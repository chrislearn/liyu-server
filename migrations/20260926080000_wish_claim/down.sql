DROP INDEX order_items_wish_item_idx;
DROP INDEX cart_items_wish_item_idx;
ALTER TABLE order_items DROP COLUMN wish_item_id;
ALTER TABLE cart_items DROP COLUMN wish_item_id;
