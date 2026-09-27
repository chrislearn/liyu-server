DROP TABLE management_schema_version;
DROP TABLE notifications;
DROP TABLE gift_contracts;
DROP TABLE wallet_ledger;
DROP TABLE recycle_policies;
ALTER TABLE orders DROP COLUMN coupon_id, DROP COLUMN discount_cents, DROP COLUMN subtotal_cents;
DROP TABLE user_coupons;
DROP TABLE coupon_templates;
DROP TRIGGER catalog_change ON catalog;
DROP FUNCTION record_catalog_change();
DROP TABLE catalog_history;
DROP TABLE admin_audit;
ALTER TABLE users DROP COLUMN is_active;
DROP TABLE admin_sessions;
DROP TABLE administrators;
-- Reverses the consolidated initial schema. Drops follow the exact reverse of
-- the original per-migration down chain (avatars → session_expiry → wish_claim
-- → demo_logistics → gifting → wishlist → commerce → fulfillment → catalog
-- → profile → init) so foreign keys never block a teardown. In particular
-- cart_items.wish_item_id / order_items.wish_item_id reference
-- wishlist_items, so the dependent columns are dropped before any wishlist
-- table, matching the original wish_claim/down.sql.
DROP TABLE avatars;
DROP INDEX IF EXISTS sessions_expires_at_idx;
ALTER TABLE sessions DROP COLUMN expires_at;
DROP INDEX IF EXISTS order_items_wish_item_idx;
DROP INDEX IF EXISTS cart_items_wish_item_idx;
ALTER TABLE order_items DROP COLUMN wish_item_id;
ALTER TABLE cart_items DROP COLUMN wish_item_id;
DROP TABLE wishlist_items;
DROP TABLE wishlist_audience;
DROP TABLE wishlists;
DROP TABLE friendships;
DROP TABLE order_items;
DROP TABLE orders;
DROP TABLE cart_items;
DROP TABLE shipment_events;
DROP TABLE shipments;
DROP TABLE gifts;
DROP TABLE catalog;
DROP TABLE shipping_addresses;
DROP TABLE user_profiles;
DROP TABLE sessions;
DROP TABLE users;
