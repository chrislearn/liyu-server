DROP INDEX IF EXISTS wishlists_expiry_idx;
ALTER TABLE wishlists DROP COLUMN expires_at;
ALTER TABLE gifts ALTER COLUMN expires_at SET DEFAULT (now() + INTERVAL '7 days');
ALTER TABLE orders DROP COLUMN gift_expires_hours;
