use super::*;
use crate::ActiveThreadPlan;
use crate::ThreadPlanItem;
use crate::ThreadPlanItemCreateParams;
use crate::ThreadPlanItemStatus;
use crate::ThreadPlanSnapshot;
use crate::ThreadPlanSnapshotCreateParams;
use crate::canonicalize_thread_plan_csv;
use crate::model::ThreadPlanSnapshotRow;
use crate::render_thread_plan_csv;
use crate::thread_plan_csv::canonicalize_thread_plan_snapshot_csv;
use crate::thread_plan_csv::parse_thread_plan_snapshot_csv;
use std::collections::HashMap;
use std::collections::HashSet;

impl StateRuntime {
    pub async fn replace_active_thread_plan(
        &self,
        snapshot: &ThreadPlanSnapshotCreateParams,
    ) -> anyhow::Result<ActiveThreadPlan> {
        let raw_csv = canonicalize_thread_plan_csv(snapshot.raw_csv.as_str())?;
        let rows = parse_thread_plan_snapshot_csv(raw_csv.as_str())?;
        let now = Utc::now().timestamp();
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            r#"
UPDATE thread_plan_snapshots
SET superseded_at = ?
WHERE thread_id = ? AND superseded_at IS NULL
            "#,
        )
        .bind(now)
        .bind(snapshot.thread_id.as_str())
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
INSERT INTO thread_plan_snapshots (
    id,
    thread_id,
    source_turn_id,
    source_item_id,
    raw_csv,
    created_at,
    superseded_at
) VALUES (?, ?, ?, ?, ?, ?, NULL)
            "#,
        )
        .bind(snapshot.id.as_str())
        .bind(snapshot.thread_id.as_str())
        .bind(snapshot.source_turn_id.as_str())
        .bind(snapshot.source_item_id.as_str())
        .bind(raw_csv.as_str())
        .bind(now)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(ActiveThreadPlan {
            snapshot: ThreadPlanSnapshot {
                id: snapshot.id.clone(),
                thread_id: snapshot.thread_id.clone(),
                source_turn_id: snapshot.source_turn_id.clone(),
                source_item_id: snapshot.source_item_id.clone(),
                raw_csv,
                created_at: DateTime::<Utc>::from_timestamp(now, 0)
                    .ok_or_else(|| anyhow::anyhow!("invalid unix timestamp: {now}"))?,
                superseded_at: None,
            },
            items: rows_to_thread_plan_items(snapshot.id.as_str(), rows.as_slice()),
        })
    }

    pub async fn get_active_thread_plan(
        &self,
        thread_id: &str,
    ) -> anyhow::Result<Option<ActiveThreadPlan>> {
        let snapshot_row = sqlx::query_as::<_, ThreadPlanSnapshotRow>(
            r#"
SELECT
    id,
    thread_id,
    source_turn_id,
    source_item_id,
    raw_csv,
    created_at,
    superseded_at
FROM thread_plan_snapshots
WHERE thread_id = ? AND superseded_at IS NULL
ORDER BY created_at DESC
LIMIT 1
            "#,
        )
        .bind(thread_id)
        .fetch_optional(self.pool.as_ref())
        .await?;
        let Some(snapshot_row) = snapshot_row else {
            return Ok(None);
        };

        let mut snapshot = ThreadPlanSnapshot::try_from(snapshot_row)?;
        snapshot.raw_csv = match canonicalize_thread_plan_snapshot_csv(snapshot.raw_csv.as_str()) {
            Ok(raw_csv) => raw_csv,
            Err(err) => {
                warn!(
                    "ignoring incompatible active thread plan snapshot {}: {err}",
                    snapshot.id
                );
                return Ok(None);
            }
        };
        let rows = match parse_thread_plan_snapshot_csv(snapshot.raw_csv.as_str()) {
            Ok(rows) => rows,
            Err(err) => {
                warn!(
                    "ignoring incompatible active thread plan snapshot {}: {err}",
                    snapshot.id
                );
                return Ok(None);
            }
        };

        Ok(Some(ActiveThreadPlan {
            items: rows_to_thread_plan_items(snapshot.id.as_str(), rows.as_slice()),
            snapshot,
        }))
    }

    pub async fn clear_active_thread_plan(&self, thread_id: &str) -> anyhow::Result<bool> {
        let now = Utc::now().timestamp();
        let result = sqlx::query(
            r#"
UPDATE thread_plan_snapshots
SET superseded_at = ?
WHERE thread_id = ? AND superseded_at IS NULL
            "#,
        )
        .bind(now)
        .bind(thread_id)
        .execute(self.pool.as_ref())
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn update_active_thread_plan_item_status(
        &self,
        thread_id: &str,
        row_id: &str,
        status: ThreadPlanItemStatus,
    ) -> anyhow::Result<Option<ActiveThreadPlan>> {
        self.update_active_thread_plan_item_statuses(thread_id, &[(row_id.to_string(), status)])
            .await
    }

    pub async fn update_active_thread_plan_item_statuses(
        &self,
        thread_id: &str,
        updates: &[(String, ThreadPlanItemStatus)],
    ) -> anyhow::Result<Option<ActiveThreadPlan>> {
        let Some(active_plan) = self.get_active_thread_plan(thread_id).await? else {
            return Ok(None);
        };

        let mut updates_by_row_id = HashMap::with_capacity(updates.len());
        let mut seen_row_ids = HashSet::with_capacity(updates.len());
        for (row_id, status) in updates {
            if !seen_row_ids.insert(row_id.as_str()) {
                return Err(anyhow::anyhow!(
                    "duplicate active thread plan row update: {row_id}"
                ));
            }
            updates_by_row_id.insert(row_id.clone(), *status);
        }

        let mut rows = active_plan
            .items
            .iter()
            .map(thread_plan_item_to_create_params)
            .collect::<Vec<_>>();
        let mut found_row_ids = HashSet::with_capacity(updates.len());
        for row in &mut rows {
            if let Some(status) = updates_by_row_id.get(row.row_id.as_str()).copied() {
                row.status = status;
                found_row_ids.insert(row.row_id.clone());
            }
        }

        for (row_id, _) in updates {
            if !found_row_ids.contains(row_id.as_str()) {
                return Err(anyhow::anyhow!(
                    "active thread plan row not found: {row_id}"
                ));
            }
        }

        let in_progress_count = rows
            .iter()
            .filter(|row| matches!(row.status, ThreadPlanItemStatus::InProgress))
            .count();
        if in_progress_count > 1 {
            return Err(anyhow::anyhow!(
                "active thread plan may include at most one in_progress row"
            ));
        }

        let raw_csv = render_thread_plan_csv(rows.as_slice())?;
        sqlx::query(
            r#"
UPDATE thread_plan_snapshots
SET raw_csv = ?
WHERE id = ?
            "#,
        )
        .bind(raw_csv.as_str())
        .bind(active_plan.snapshot.id.as_str())
        .execute(self.pool.as_ref())
        .await?;

        self.get_active_thread_plan(thread_id).await
    }
}

fn rows_to_thread_plan_items(
    snapshot_id: &str,
    rows: &[ThreadPlanItemCreateParams],
) -> Vec<ThreadPlanItem> {
    rows.iter()
        .cloned()
        .map(|row| ThreadPlanItem {
            snapshot_id: snapshot_id.to_string(),
            row_id: row.row_id,
            row_index: row.row_index,
            status: row.status,
            step: row.step,
            path: row.path,
            details: row.details,
            inputs: row.inputs,
            outputs: row.outputs,
            depends_on: row.depends_on,
            acceptance: row.acceptance,
        })
        .collect()
}

fn thread_plan_item_to_create_params(item: &ThreadPlanItem) -> ThreadPlanItemCreateParams {
    ThreadPlanItemCreateParams {
        row_id: item.row_id.clone(),
        row_index: item.row_index,
        status: item.status,
        step: item.step.clone(),
        path: item.path.clone(),
        details: item.details.clone(),
        inputs: item.inputs.clone(),
        outputs: item.outputs.clone(),
        depends_on: item.depends_on.clone(),
        acceptance: item.acceptance.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::StateRuntime;
    use super::test_support::unique_temp_dir;
    use crate::ThreadPlanItemStatus;
    use crate::ThreadPlanSnapshotCreateParams;
    use crate::migrations::STATE_MIGRATOR;
    use pretty_assertions::assert_eq;
    use sqlx::SqlitePool;
    use sqlx::migrate::Migrator;
    use sqlx::sqlite::SqliteConnectOptions;
    use std::borrow::Cow;

    const RAW_CSV: &str = "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,in_progress,Persist plan,codex-rs/state/src/runtime/thread_plans.rs,write active snapshot,plan csv,active plan rows,,active plan reloads
plan-02,pending,Render plan,codex-rs/tui/src/history_cell.rs,,active plan rows,history cell update,plan-01,
";

    #[tokio::test]
    async fn replace_get_and_update_active_thread_plan() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        let thread_id = "thread-1";
        let snapshot = ThreadPlanSnapshotCreateParams {
            id: "snapshot-1".to_string(),
            thread_id: thread_id.to_string(),
            source_turn_id: "turn-1".to_string(),
            source_item_id: "item-1".to_string(),
            raw_csv: RAW_CSV.to_string(),
        };

        let created = runtime
            .replace_active_thread_plan(&snapshot)
            .await
            .expect("create active plan");
        assert_eq!(created.snapshot.id, "snapshot-1");
        assert_eq!(created.snapshot.raw_csv, RAW_CSV);
        assert_eq!(created.items.len(), 2);
        assert_eq!(created.items[0].row_id, "plan-01");
        assert_eq!(created.items[0].status, ThreadPlanItemStatus::InProgress);
        assert_eq!(created.items[1].depends_on, vec!["plan-01".to_string()]);

        let loaded = runtime
            .get_active_thread_plan(thread_id)
            .await
            .expect("load active plan")
            .expect("active plan should exist");
        assert_eq!(loaded.snapshot.id, "snapshot-1");
        assert_eq!(loaded.items.len(), 2);
        assert_eq!(loaded.items[1].step, "Render plan");

        let updated = runtime
            .update_active_thread_plan_item_status(
                thread_id,
                "plan-02",
                ThreadPlanItemStatus::Completed,
            )
            .await
            .expect("update status")
            .expect("active plan should still exist");
        assert_eq!(updated.items[1].status, ThreadPlanItemStatus::Completed);
        assert!(
            updated
                .snapshot
                .raw_csv
                .contains("plan-02,completed,Render plan")
        );
    }

    #[tokio::test]
    async fn get_active_thread_plan_restores_legacy_markdown_snapshots() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        sqlx::query(
            r#"
INSERT INTO thread_plan_snapshots (
    id,
    thread_id,
    source_turn_id,
    source_item_id,
    raw_csv,
    created_at,
    superseded_at
) VALUES (?, ?, ?, ?, ?, ?, NULL)
            "#,
        )
        .bind("snapshot-legacy")
        .bind("thread-legacy")
        .bind("turn-1")
        .bind("item-1")
        .bind("# Plan\n\n```csv\nid,status,step,path,details\nplan-01,pending,Legacy,codex-rs/core/src/plan_csv.rs,\n```")
        .bind(1_i64)
        .execute(runtime.pool.as_ref())
        .await
        .expect("insert legacy snapshot");

        let loaded = runtime
            .get_active_thread_plan("thread-legacy")
            .await
            .expect("load active plan")
            .expect("legacy active plan should load");
        assert_eq!(loaded.snapshot.id, "snapshot-legacy");
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].row_id, "plan-01");
        assert_eq!(loaded.items[0].step, "Legacy");
        assert_eq!(loaded.items[0].inputs, Vec::<String>::new());
        assert_eq!(loaded.items[0].depends_on, Vec::<String>::new());
    }

    #[tokio::test]
    async fn update_active_thread_plan_rejects_multiple_in_progress_rows() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        let thread_id = "thread-2";
        let snapshot = ThreadPlanSnapshotCreateParams {
            id: "snapshot-2".to_string(),
            thread_id: thread_id.to_string(),
            source_turn_id: "turn-2".to_string(),
            source_item_id: "item-2".to_string(),
            raw_csv: RAW_CSV.to_string(),
        };

        runtime
            .replace_active_thread_plan(&snapshot)
            .await
            .expect("create active plan");

        let err = runtime
            .update_active_thread_plan_item_status(
                thread_id,
                "plan-02",
                ThreadPlanItemStatus::InProgress,
            )
            .await
            .expect_err("should reject a second in_progress row");
        assert_eq!(
            err.to_string(),
            "active thread plan may include at most one in_progress row"
        );

        let loaded = runtime
            .get_active_thread_plan(thread_id)
            .await
            .expect("load active plan")
            .expect("active plan should exist");
        assert_eq!(loaded.items[0].status, ThreadPlanItemStatus::InProgress);
        assert_eq!(loaded.items[1].status, ThreadPlanItemStatus::Pending);
    }

    #[tokio::test]
    async fn batch_status_updates_are_atomic() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        let thread_id = "thread-3";
        let snapshot = ThreadPlanSnapshotCreateParams {
            id: "snapshot-3".to_string(),
            thread_id: thread_id.to_string(),
            source_turn_id: "turn-3".to_string(),
            source_item_id: "item-3".to_string(),
            raw_csv: RAW_CSV.to_string(),
        };

        runtime
            .replace_active_thread_plan(&snapshot)
            .await
            .expect("create active plan");

        let err = runtime
            .update_active_thread_plan_item_statuses(
                thread_id,
                &[
                    ("plan-01".to_string(), ThreadPlanItemStatus::Completed),
                    ("plan-99".to_string(), ThreadPlanItemStatus::InProgress),
                ],
            )
            .await
            .expect_err("unknown row should fail the whole batch");
        assert_eq!(err.to_string(), "active thread plan row not found: plan-99");

        let loaded = runtime
            .get_active_thread_plan(thread_id)
            .await
            .expect("load active plan")
            .expect("active plan should exist");
        assert_eq!(loaded.items[0].status, ThreadPlanItemStatus::InProgress);
        assert_eq!(loaded.items[1].status, ThreadPlanItemStatus::Pending);
    }

    #[tokio::test]
    async fn clear_active_thread_plan_supersedes_current_snapshot() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        let thread_id = "thread-4";
        let snapshot = ThreadPlanSnapshotCreateParams {
            id: "snapshot-4".to_string(),
            thread_id: thread_id.to_string(),
            source_turn_id: "turn-4".to_string(),
            source_item_id: "item-4".to_string(),
            raw_csv: RAW_CSV.to_string(),
        };

        runtime
            .replace_active_thread_plan(&snapshot)
            .await
            .expect("create active plan");

        assert!(
            runtime
                .clear_active_thread_plan(thread_id)
                .await
                .expect("clear active plan"),
            "clearing an existing active plan should report a change"
        );
        assert_eq!(
            runtime
                .get_active_thread_plan(thread_id)
                .await
                .expect("load active plan after clear"),
            None
        );
        assert!(
            !runtime
                .clear_active_thread_plan(thread_id)
                .await
                .expect("clearing a missing active plan should be a no-op"),
            "clearing a missing active plan should report no change"
        );
    }

    #[tokio::test]
    async fn init_migrates_legacy_plan_rows_into_snapshot_csv() {
        let codex_home = unique_temp_dir();
        tokio::fs::create_dir_all(&codex_home)
            .await
            .expect("create codex home");
        let state_path = super::super::state_db_path(codex_home.as_path());
        let old_state_migrator = Migrator {
            migrations: Cow::Owned(
                STATE_MIGRATOR
                    .migrations
                    .iter()
                    .filter(|migration| migration.version < 22)
                    .cloned()
                    .collect(),
            ),
            ignore_missing: false,
            locking: true,
            no_tx: false,
        };
        let pool = SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(&state_path)
                .create_if_missing(true),
        )
        .await
        .expect("open legacy state db");
        old_state_migrator
            .run(&pool)
            .await
            .expect("apply legacy state schema");
        sqlx::query(
            r#"
INSERT INTO thread_plan_snapshots (
    id,
    thread_id,
    source_turn_id,
    source_item_id,
    raw_markdown,
    created_at,
    superseded_at
) VALUES (?, ?, ?, ?, ?, ?, NULL)
            "#,
        )
        .bind("snapshot-migrated")
        .bind("thread-migrated")
        .bind("turn-1")
        .bind("item-1")
        .bind(
            "<proposed_plan>\n```csv\nid,status,step,path,details\nplan-01,pending,Legacy row,codex-rs/core/src/plan_csv.rs,stale markdown\nplan-02,pending,Needs status,codex-rs/core/src/codex.rs,stale markdown\n```\n</proposed_plan>\n",
        )
        .bind(1_i64)
        .execute(&pool)
        .await
        .expect("insert legacy snapshot");
        sqlx::query(
            r#"
INSERT INTO thread_plan_items (
    snapshot_id,
    row_id,
    row_index,
    status,
    step,
    path,
    details,
    created_at,
    updated_at,
    completed_at,
    inputs,
    outputs,
    depends_on,
    acceptance
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind("snapshot-migrated")
        .bind("plan-01")
        .bind(0_i64)
        .bind("completed")
        .bind("Legacy row")
        .bind("codex-rs/core/src/plan_csv.rs")
        .bind("now complete")
        .bind(1_i64)
        .bind(2_i64)
        .bind(2_i64)
        .bind(r#"["plan markdown"]"#)
        .bind(r#"["review output"]"#)
        .bind("[]")
        .bind("done")
        .execute(&pool)
        .await
        .expect("insert migrated row 1");
        sqlx::query(
            r#"
INSERT INTO thread_plan_items (
    snapshot_id,
    row_id,
    row_index,
    status,
    step,
    path,
    details,
    created_at,
    updated_at,
    completed_at,
    inputs,
    outputs,
    depends_on,
    acceptance
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind("snapshot-migrated")
        .bind("plan-02")
        .bind(1_i64)
        .bind("in_progress")
        .bind("Needs status")
        .bind("codex-rs/core/src/codex.rs")
        .bind("current state from rows")
        .bind(1_i64)
        .bind(3_i64)
        .bind(Option::<i64>::None)
        .bind("[]")
        .bind(r#"["artifact"]"#)
        .bind(r#"["plan-01"]"#)
        .bind(Option::<&str>::None)
        .execute(&pool)
        .await
        .expect("insert migrated row 2");
        pool.close().await;

        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        let loaded = runtime
            .get_active_thread_plan("thread-migrated")
            .await
            .expect("load migrated active plan")
            .expect("migrated active plan should exist");
        assert_eq!(loaded.snapshot.id, "snapshot-migrated");
        assert_eq!(
            loaded.snapshot.raw_csv,
            "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,completed,Legacy row,codex-rs/core/src/plan_csv.rs,now complete,plan markdown,review output,,done
plan-02,in_progress,Needs status,codex-rs/core/src/codex.rs,current state from rows,,artifact,plan-01,
"
        );
        assert_eq!(loaded.items.len(), 2);
        assert_eq!(loaded.items[0].status, ThreadPlanItemStatus::Completed);
        assert_eq!(loaded.items[0].inputs, vec!["plan markdown".to_string()]);
        assert_eq!(loaded.items[0].outputs, vec!["review output".to_string()]);
        assert_eq!(loaded.items[0].acceptance.as_deref(), Some("done"));
        assert_eq!(loaded.items[1].status, ThreadPlanItemStatus::InProgress);
        assert_eq!(loaded.items[1].depends_on, vec!["plan-01".to_string()]);
    }

    #[tokio::test]
    async fn init_keeps_legacy_markdown_snapshots_readable_without_row_table_data() {
        let codex_home = unique_temp_dir();
        tokio::fs::create_dir_all(&codex_home)
            .await
            .expect("create codex home");
        let state_path = super::super::state_db_path(codex_home.as_path());
        let old_state_migrator = Migrator {
            migrations: Cow::Owned(
                STATE_MIGRATOR
                    .migrations
                    .iter()
                    .filter(|migration| migration.version < 22)
                    .cloned()
                    .collect(),
            ),
            ignore_missing: false,
            locking: true,
            no_tx: false,
        };
        let pool = SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(&state_path)
                .create_if_missing(true),
        )
        .await
        .expect("open legacy state db");
        old_state_migrator
            .run(&pool)
            .await
            .expect("apply legacy state schema");
        sqlx::query(
            r#"
INSERT INTO thread_plan_snapshots (
    id,
    thread_id,
    source_turn_id,
    source_item_id,
    raw_markdown,
    created_at,
    superseded_at
) VALUES (?, ?, ?, ?, ?, ?, NULL)
            "#,
        )
        .bind("snapshot-legacy-only")
        .bind("thread-legacy-only")
        .bind("turn-1")
        .bind("item-1")
        .bind(
            "<proposed_plan>\n```csv\nid,status,step,path,details\nplan-01,pending,Legacy only,codex-rs/core/src/plan_csv.rs,still readable\n```\n</proposed_plan>\n",
        )
        .bind(1_i64)
        .execute(&pool)
        .await
        .expect("insert legacy-only snapshot");
        pool.close().await;

        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string())
            .await
            .expect("initialize runtime");

        let loaded = runtime
            .get_active_thread_plan("thread-legacy-only")
            .await
            .expect("load migrated legacy-only active plan")
            .expect("legacy-only active plan should exist");
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].row_id, "plan-01");
        assert_eq!(loaded.items[0].status, ThreadPlanItemStatus::Pending);
        assert_eq!(loaded.items[0].step, "Legacy only");
        assert_eq!(
            loaded.snapshot.raw_csv,
            "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,pending,Legacy only,codex-rs/core/src/plan_csv.rs,still readable,,,,
"
        );
    }
}
