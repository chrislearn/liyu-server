-- Removing the visibility boundary must never expose or discard private drafts.
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM wishlists WHERE published_at IS NULL) THEN
        RAISE EXCEPTION 'Cannot remove draft support while private drafts exist';
    END IF;
END $$;
DROP INDEX wishlist_drafts_owner_idx;
ALTER TABLE wishlists DROP COLUMN published_at;
