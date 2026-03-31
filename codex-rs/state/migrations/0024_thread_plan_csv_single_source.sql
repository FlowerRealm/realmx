ALTER TABLE thread_plan_snapshots
    RENAME COLUMN raw_markdown TO raw_csv;

DROP INDEX IF EXISTS idx_thread_plan_items_snapshot;

DROP TABLE IF EXISTS thread_plan_items;
