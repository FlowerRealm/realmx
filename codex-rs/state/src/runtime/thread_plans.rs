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
use crate::thread_plan_csv::parse_thread_plan_snapshot_csv;

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

        let snapshot = ThreadPlanSnapshot::try_from(snapshot_row)?;
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

    pub async fn update_active_thread_plan_item_status(
        &self,
        thread_id: &str,
        row_id: &str,
        status: ThreadPlanItemStatus,
    ) -> anyhow::Result<Option<ActiveThreadPlan>> {
        let Some(active_plan) = self.get_active_thread_plan(thread_id).await? else {
            return Ok(None);
        };

        let mut rows = active_plan
            .items
            .iter()
            .map(thread_plan_item_to_create_params)
            .collect::<Vec<_>>();
        let Some(row) = rows.iter_mut().find(|item| item.row_id == row_id) else {
            return Err(anyhow::anyhow!(
                "active thread plan row not found: {row_id}"
            ));
        };
        row.status = status;
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
    use pretty_assertions::assert_eq;

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
}
