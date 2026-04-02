use super::*;
use crate::ActiveThreadPlan;
use crate::ThreadPlanItem;
use crate::ThreadPlanItemCreateParams;
use crate::ThreadPlanItemStatus;
use crate::ThreadPlanSnapshot;
use crate::ThreadPlanSnapshotCreateParams;
use crate::model::ThreadPlanItemRow;
use crate::model::ThreadPlanSnapshotRow;
use std::collections::HashMap;
use std::collections::HashSet;

impl StateRuntime {
    pub async fn replace_active_thread_plan(
        &self,
        snapshot: &ThreadPlanSnapshotCreateParams,
        items: &[ThreadPlanItemCreateParams],
    ) -> anyhow::Result<ActiveThreadPlan> {
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
    raw_markdown,
    created_at,
    superseded_at
) VALUES (?, ?, ?, ?, ?, ?, NULL)
            "#,
        )
        .bind(snapshot.id.as_str())
        .bind(snapshot.thread_id.as_str())
        .bind(snapshot.source_turn_id.as_str())
        .bind(snapshot.source_item_id.as_str())
        .bind(snapshot.raw_markdown.as_str())
        .bind(now)
        .execute(&mut *tx)
        .await?;

        for item in items {
            let completed_at =
                matches!(item.status, ThreadPlanItemStatus::Completed).then_some(now);
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
    inputs,
    outputs,
    depends_on,
    acceptance,
    created_at,
    updated_at,
    completed_at
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(snapshot.id.as_str())
            .bind(item.row_id.as_str())
            .bind(item.row_index)
            .bind(item.status.as_str())
            .bind(item.step.as_str())
            .bind(item.path.as_str())
            .bind(item.details.as_str())
            .bind(string_list_json(item.inputs.as_slice()))
            .bind(string_list_json(item.outputs.as_slice()))
            .bind(string_list_json(item.depends_on.as_slice()))
            .bind(item.acceptance.as_deref())
            .bind(now)
            .bind(now)
            .bind(completed_at)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        let active_plan = self
            .get_active_thread_plan(snapshot.thread_id.as_str())
            .await?;
        active_plan.ok_or_else(|| anyhow::anyhow!("failed to load active thread plan"))
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
    raw_markdown,
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
        let item_rows = sqlx::query_as::<_, ThreadPlanItemRow>(
            r#"
SELECT
    snapshot_id,
    row_id,
    row_index,
    status,
    step,
    path,
    details,
    inputs,
    outputs,
    depends_on,
    acceptance,
    created_at,
    updated_at,
    completed_at
FROM thread_plan_items
WHERE snapshot_id = ?
ORDER BY row_index ASC
            "#,
        )
        .bind(snapshot.id.as_str())
        .fetch_all(self.pool.as_ref())
        .await?;
        let items = item_rows
            .into_iter()
            .map(ThreadPlanItem::try_from)
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Some(ActiveThreadPlan { snapshot, items }))
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
            updates_by_row_id.insert(row_id.as_str(), *status);
        }

        let mut in_progress_count = 0usize;
        let mut found_row_ids = HashSet::with_capacity(updates.len());
        for item in &active_plan.items {
            let next_status = updates_by_row_id
                .get(item.row_id.as_str())
                .copied()
                .unwrap_or(item.status);
            if updates_by_row_id.contains_key(item.row_id.as_str()) {
                found_row_ids.insert(item.row_id.as_str());
            }
            if matches!(next_status, ThreadPlanItemStatus::InProgress) {
                in_progress_count = in_progress_count.saturating_add(1);
            }
        }

        for (row_id, _) in updates {
            if !found_row_ids.contains(row_id.as_str()) {
                return Err(anyhow::anyhow!(
                    "active thread plan row not found: {row_id}"
                ));
            }
        }
        if in_progress_count > 1 {
            return Err(anyhow::anyhow!(
                "active thread plan may include at most one in_progress row"
            ));
        }

        let now = Utc::now().timestamp();
        let mut tx = self.pool.begin().await?;
        for item in &active_plan.items {
            let Some(status) = updates_by_row_id.get(item.row_id.as_str()).copied() else {
                continue;
            };
            let completed_at = match status {
                ThreadPlanItemStatus::Completed => item
                    .completed_at
                    .map(|value| value.timestamp())
                    .or(Some(now)),
                ThreadPlanItemStatus::Pending | ThreadPlanItemStatus::InProgress => None,
            };
            let result = sqlx::query(
                r#"
UPDATE thread_plan_items
SET
    status = ?,
    updated_at = ?,
    completed_at = ?
WHERE snapshot_id = ? AND row_id = ?
                "#,
            )
            .bind(status.as_str())
            .bind(now)
            .bind(completed_at)
            .bind(active_plan.snapshot.id.as_str())
            .bind(item.row_id.as_str())
            .execute(&mut *tx)
            .await?;
            if result.rows_affected() != 1 {
                return Err(anyhow::anyhow!(
                    "failed to update active thread plan row {}: expected 1 row, got {}",
                    item.row_id,
                    result.rows_affected()
                ));
            }
        }
        tx.commit().await?;

        self.get_active_thread_plan(thread_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::StateRuntime;
    use super::test_support::unique_temp_dir;
    use crate::ThreadPlanItemCreateParams;
    use crate::ThreadPlanItemStatus;
    use crate::ThreadPlanSnapshotCreateParams;
    use pretty_assertions::assert_eq;

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
            raw_markdown: "plan markdown".to_string(),
        };
        let items = vec![
            ThreadPlanItemCreateParams {
                row_id: "plan-01".to_string(),
                row_index: 0,
                status: ThreadPlanItemStatus::InProgress,
                step: "Persist plan".to_string(),
                path: "codex-rs/state/src/runtime/thread_plans.rs".to_string(),
                details: "write active snapshot".to_string(),
                inputs: vec!["plan markdown".to_string()],
                outputs: vec!["active plan rows".to_string()],
                depends_on: Vec::new(),
                acceptance: Some("active plan reloads".to_string()),
            },
            ThreadPlanItemCreateParams {
                row_id: "plan-02".to_string(),
                row_index: 1,
                status: ThreadPlanItemStatus::Pending,
                step: "Render plan".to_string(),
                path: "codex-rs/tui/src/history_cell.rs".to_string(),
                details: String::new(),
                inputs: vec!["active plan rows".to_string()],
                outputs: vec!["history cell update".to_string()],
                depends_on: vec!["plan-01".to_string()],
                acceptance: None,
            },
        ];

        let created = runtime
            .replace_active_thread_plan(&snapshot, items.as_slice())
            .await
            .expect("create active plan");
        assert_eq!(created.snapshot.id, "snapshot-1");
        assert_eq!(created.items.len(), 2);
        assert_eq!(created.items[0].row_id, "plan-01");
        assert_eq!(created.items[0].status, ThreadPlanItemStatus::InProgress);
        assert_eq!(created.items[0].inputs, vec!["plan markdown".to_string()]);
        assert_eq!(created.items[1].depends_on, vec!["plan-01".to_string()]);

        let loaded = runtime
            .get_active_thread_plan(thread_id)
            .await
            .expect("load active plan")
            .expect("active plan should exist");
        assert_eq!(loaded.snapshot.id, "snapshot-1");
        assert_eq!(loaded.items, created.items);

        let updated = runtime
            .update_active_thread_plan_item_status(
                thread_id,
                "plan-02",
                ThreadPlanItemStatus::Completed,
            )
            .await
            .expect("update active plan")
            .expect("updated plan should exist");
        assert_eq!(updated.items[1].status, ThreadPlanItemStatus::Completed);
        assert!(updated.items[1].completed_at.is_some());

        let _ = tokio::fs::remove_dir_all(codex_home).await;
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
            raw_markdown: "plan markdown".to_string(),
        };
        let items = vec![
            ThreadPlanItemCreateParams {
                row_id: "plan-01".to_string(),
                row_index: 0,
                status: ThreadPlanItemStatus::InProgress,
                step: "Persist plan".to_string(),
                path: "codex-rs/state/src/runtime/thread_plans.rs".to_string(),
                details: "write active snapshot".to_string(),
                inputs: Vec::new(),
                outputs: Vec::new(),
                depends_on: Vec::new(),
                acceptance: None,
            },
            ThreadPlanItemCreateParams {
                row_id: "plan-02".to_string(),
                row_index: 1,
                status: ThreadPlanItemStatus::Pending,
                step: "Render plan".to_string(),
                path: "codex-rs/tui/src/history_cell.rs".to_string(),
                details: String::new(),
                inputs: Vec::new(),
                outputs: Vec::new(),
                depends_on: Vec::new(),
                acceptance: None,
            },
        ];

        runtime
            .replace_active_thread_plan(&snapshot, items.as_slice())
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

        let _ = tokio::fs::remove_dir_all(codex_home).await;
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
            raw_markdown: "plan markdown".to_string(),
        };
        let items = vec![
            ThreadPlanItemCreateParams {
                row_id: "plan-01".to_string(),
                row_index: 0,
                status: ThreadPlanItemStatus::InProgress,
                step: "Persist plan".to_string(),
                path: "codex-rs/state/src/runtime/thread_plans.rs".to_string(),
                details: String::new(),
                inputs: Vec::new(),
                outputs: Vec::new(),
                depends_on: Vec::new(),
                acceptance: None,
            },
            ThreadPlanItemCreateParams {
                row_id: "plan-02".to_string(),
                row_index: 1,
                status: ThreadPlanItemStatus::Pending,
                step: "Render plan".to_string(),
                path: "codex-rs/tui/src/history_cell.rs".to_string(),
                details: String::new(),
                inputs: Vec::new(),
                outputs: Vec::new(),
                depends_on: Vec::new(),
                acceptance: None,
            },
        ];

        runtime
            .replace_active_thread_plan(&snapshot, items.as_slice())
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

        let _ = tokio::fs::remove_dir_all(codex_home).await;
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
            raw_markdown: "plan markdown".to_string(),
        };
        let items = vec![ThreadPlanItemCreateParams {
            row_id: "plan-01".to_string(),
            row_index: 0,
            status: ThreadPlanItemStatus::InProgress,
            step: "Persist plan".to_string(),
            path: "codex-rs/state/src/runtime/thread_plans.rs".to_string(),
            details: String::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            depends_on: Vec::new(),
            acceptance: None,
        }];

        runtime
            .replace_active_thread_plan(&snapshot, items.as_slice())
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

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }
}

fn string_list_json(values: &[String]) -> String {
    serde_json::to_string(values).expect("thread plan string list should serialize")
}
