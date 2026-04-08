-- Extra session columns
ALTER TABLE sessions ADD COLUMN context_used_pct REAL;
ALTER TABLE sessions ADD COLUMN total_input_tokens INTEGER;
ALTER TABLE sessions ADD COLUMN total_output_tokens INTEGER;
ALTER TABLE sessions ADD COLUMN cost_usd REAL;
ALTER TABLE sessions ADD COLUMN git_branch TEXT;

-- Claude's internal tasks (TodoWrite)
CREATE TABLE IF NOT EXISTS session_tasks (
    id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    subject TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'pending',
    PRIMARY KEY (id, session_id)
);
