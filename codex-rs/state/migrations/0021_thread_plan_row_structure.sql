ALTER TABLE thread_plan_items
    ADD COLUMN inputs TEXT NOT NULL DEFAULT '[]';

ALTER TABLE thread_plan_items
    ADD COLUMN outputs TEXT NOT NULL DEFAULT '[]';

ALTER TABLE thread_plan_items
    ADD COLUMN depends_on TEXT NOT NULL DEFAULT '[]';

ALTER TABLE thread_plan_items
    ADD COLUMN acceptance TEXT;
