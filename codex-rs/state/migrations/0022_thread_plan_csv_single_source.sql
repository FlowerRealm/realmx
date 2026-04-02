ALTER TABLE thread_plan_snapshots
    RENAME COLUMN raw_markdown TO raw_csv;

UPDATE thread_plan_snapshots
SET raw_csv = (
    WITH csv_rows AS (
        SELECT
            row_index,
            '"' || replace(row_id, '"', '""') || '",' ||
            '"' || replace(status, '"', '""') || '",' ||
            '"' || replace(step, '"', '""') || '",' ||
            '"' || replace(path, '"', '""') || '",' ||
            '"' || replace(details, '"', '""') || '",' ||
            '"' || replace(
                COALESCE(
                    (
                        SELECT group_concat(value, '|')
                        FROM (
                            SELECT value
                            FROM json_each(
                                CASE
                                    WHEN json_valid(inputs) THEN
                                        CASE
                                            WHEN json_type(inputs) = 'array' THEN inputs
                                            ELSE '[]'
                                        END
                                    ELSE '[]'
                                END
                            )
                            ORDER BY key
                        )
                    ),
                    ''
                ),
                '"',
                '""'
            ) || '",' ||
            '"' || replace(
                COALESCE(
                    (
                        SELECT group_concat(value, '|')
                        FROM (
                            SELECT value
                            FROM json_each(
                                CASE
                                    WHEN json_valid(outputs) THEN
                                        CASE
                                            WHEN json_type(outputs) = 'array' THEN outputs
                                            ELSE '[]'
                                        END
                                    ELSE '[]'
                                END
                            )
                            ORDER BY key
                        )
                    ),
                    ''
                ),
                '"',
                '""'
            ) || '",' ||
            '"' || replace(
                COALESCE(
                    (
                        SELECT group_concat(value, '|')
                        FROM (
                            SELECT value
                            FROM json_each(
                                CASE
                                    WHEN json_valid(depends_on) THEN
                                        CASE
                                            WHEN json_type(depends_on) = 'array' THEN depends_on
                                            ELSE '[]'
                                        END
                                    ELSE '[]'
                                END
                            )
                            ORDER BY key
                        )
                    ),
                    ''
                ),
                '"',
                '""'
            ) || '",' ||
            '"' || replace(COALESCE(acceptance, ''), '"', '""') || '"' AS csv_line
        FROM thread_plan_items
        WHERE snapshot_id = thread_plan_snapshots.id
    )
    SELECT
        'id,status,step,path,details,inputs,outputs,depends_on,acceptance' || char(10) ||
        group_concat(csv_line, char(10)) || char(10)
    FROM (
        SELECT csv_line
        FROM csv_rows
        ORDER BY row_index ASC
    )
)
WHERE EXISTS (
    SELECT 1
    FROM thread_plan_items
    WHERE snapshot_id = thread_plan_snapshots.id
);

-- Keep thread_plan_items in place for now. Another 0022_* migration still rewrites
-- that table, and sqlx only sorts duplicate versions by version number, so the
-- relative order is not something we should bet user data on. Runtime reads from
-- raw_csv after this migration, and a later dedicated migration can safely remove
-- the legacy row table once the 0022_* sequence is untangled.
