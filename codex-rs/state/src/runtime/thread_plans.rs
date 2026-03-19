use super::*;
use crate::ActiveThreadPlan;
use crate::ThreadPlanItem;
use crate::ThreadPlanItemCreateParams;
use crate::ThreadPlanItemStatus;
use crate::ThreadPlanSnapshot;
use crate::ThreadPlanSnapshotCreateParams;
use crate::model::ThreadPlanItemRow;
use crate::model::ThreadPlanSnapshotRow;

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

    pub async fn update_active_thread_plan_item_status(
        &self,
        thread_id: &str,
        row_id: &str,
        status: ThreadPlanItemStatus,
    ) -> anyhow::Result<Option<ActiveThreadPlan>> {
        let Some(snapshot) = self.get_active_thread_plan(thread_id).await? else {
            return Ok(None);
        };
        let now = Utc::now().timestamp();
        let completed_at = matches!(status, ThreadPlanItemStatus::Completed).then_some(now);
        sqlx::query(
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
        .bind(snapshot.snapshot.id.as_str())
        .bind(row_id)
        .execute(self.pool.as_ref())
        .await?;
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
}

fn string_list_json(values: &[String]) -> String {
    serde_json::to_string(values).expect("thread plan string list should serialize")
}
