-- New gifts and wishlists default to one day; callers may choose 1..720 hours.
ALTER TABLE orders
    ADD COLUMN gift_expires_hours INTEGER NOT NULL DEFAULT 24
    CHECK (gift_expires_hours BETWEEN 1 AND 720);

ALTER TABLE gifts
    ALTER COLUMN expires_at SET DEFAULT (now() + INTERVAL '24 hours');
-- Give existing unopened gifts a one-day grace period under the new policy.
UPDATE gifts SET expires_at = now() + INTERVAL '24 hours'
WHERE state IN ('sealed','opened') AND expires_at > now() + INTERVAL '24 hours';

ALTER TABLE wishlists
    ADD COLUMN expires_at TIMESTAMPTZ NOT NULL DEFAULT (now() + INTERVAL '24 hours');
-- Existing lists had no deadline. Give them a full month from migration.
UPDATE wishlists SET expires_at = now() + INTERVAL '30 days';
CREATE INDEX wishlists_expiry_idx ON wishlists(expires_at);
