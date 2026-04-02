CREATE TABLE thread_plan_items_v2 (
    snapshot_id TEXT NOT NULL,
    row_id TEXT NOT NULL,
    row_index INTEGER NOT NULL,
    status TEXT NOT NULL,
    step TEXT NOT NULL,
    path TEXT NOT NULL,
    details TEXT NOT NULL,
    inputs TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(inputs) AND json_type(inputs) = 'array'),
    outputs TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(outputs) AND json_type(outputs) = 'array'),
    depends_on TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(depends_on) AND json_type(depends_on) = 'array'),
    acceptance TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    completed_at INTEGER,
    PRIMARY KEY (snapshot_id, row_id),
    FOREIGN KEY(snapshot_id) REFERENCES thread_plan_snapshots(id) ON DELETE CASCADE
);

INSERT INTO thread_plan_items_v2 (
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
)
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
FROM thread_plan_items;

DROP TABLE thread_plan_items;

ALTER TABLE thread_plan_items_v2 RENAME TO thread_plan_items;

CREATE INDEX idx_thread_plan_items_snapshot
    ON thread_plan_items(snapshot_id, row_index ASC);
