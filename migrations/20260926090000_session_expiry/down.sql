DROP INDEX IF EXISTS sessions_expires_at_idx;
ALTER TABLE sessions DROP COLUMN expires_at;
