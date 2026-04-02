CREATE TABLE thread_plan_snapshots (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    source_turn_id TEXT NOT NULL,
    source_item_id TEXT NOT NULL,
    raw_markdown TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    superseded_at INTEGER
);

CREATE TABLE thread_plan_items (
    snapshot_id TEXT NOT NULL,
    row_id TEXT NOT NULL,
    row_index INTEGER NOT NULL,
    status TEXT NOT NULL,
    step TEXT NOT NULL,
    path TEXT NOT NULL,
    details TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    completed_at INTEGER,
    PRIMARY KEY (snapshot_id, row_id),
    FOREIGN KEY(snapshot_id) REFERENCES thread_plan_snapshots(id) ON DELETE CASCADE
);

CREATE INDEX idx_thread_plan_snapshots_active
    ON thread_plan_snapshots(thread_id, superseded_at, created_at DESC);
CREATE INDEX idx_thread_plan_items_snapshot
    ON thread_plan_items(snapshot_id, row_index ASC);
