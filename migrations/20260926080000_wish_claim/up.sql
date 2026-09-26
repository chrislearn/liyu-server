-- A cart/order may target a wishlist item. Claims are assigned only after test payment;
-- the wishlist row is locked and checked again inside that payment transaction.
ALTER TABLE cart_items ADD COLUMN wish_item_id BIGINT REFERENCES wishlist_items(id);
ALTER TABLE order_items ADD COLUMN wish_item_id BIGINT REFERENCES wishlist_items(id);
CREATE INDEX cart_items_wish_item_idx ON cart_items(wish_item_id) WHERE wish_item_id IS NOT NULL;
CREATE INDEX order_items_wish_item_idx ON order_items(wish_item_id) WHERE wish_item_id IS NOT NULL;
