ALTER TABLE sessions ADD COLUMN expires_at TIMESTAMPTZ;
UPDATE sessions SET expires_at = created_at + INTERVAL '30 days';
ALTER TABLE sessions ALTER COLUMN expires_at SET NOT NULL;
ALTER TABLE sessions ALTER COLUMN expires_at SET DEFAULT (now() + INTERVAL '30 days');
CREATE INDEX sessions_expires_at_idx ON sessions(expires_at);
