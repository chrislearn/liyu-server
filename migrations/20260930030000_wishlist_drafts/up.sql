-- Existing lists were published immediately. Preserve their visibility/history.
ALTER TABLE wishlists ADD COLUMN published_at TIMESTAMPTZ;
UPDATE wishlists SET published_at=created_at;
ALTER TABLE wishlists ALTER COLUMN published_at SET DEFAULT now();
CREATE INDEX wishlist_drafts_owner_idx ON wishlists(owner_id,id)
    WHERE published_at IS NULL AND closed_at IS NULL;
