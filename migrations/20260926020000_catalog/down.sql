DROP INDEX IF EXISTS catalog_search_idx;
DROP INDEX IF EXISTS catalog_category_id_idx;
ALTER TABLE catalog DROP COLUMN is_active, DROP COLUMN stock, DROP COLUMN tags, DROP COLUMN description, DROP COLUMN spec, DROP COLUMN kind, DROP COLUMN brand;
