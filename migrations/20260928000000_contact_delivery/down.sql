-- Refuse to lose address-book anchors or checkout snapshots on rollback.
DO $$ DECLARE has_data BOOLEAN; BEGIN
 IF to_regclass('sender_contacts') IS NOT NULL THEN
   EXECUTE 'SELECT EXISTS(SELECT 1 FROM sender_contacts)' INTO has_data;
   IF has_data THEN RAISE EXCEPTION 'contact book contains saved contacts'; END IF;
 END IF;
 IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema=current_schema() AND table_name='order_items' AND column_name='recipient_bound_user_id') THEN
   EXECUTE 'SELECT EXISTS(SELECT 1 FROM order_items WHERE recipient_bound_user_id IS NOT NULL OR recipient_change_confirmed)' INTO has_data;
   IF has_data THEN RAISE EXCEPTION 'contact book contains checkout snapshots'; END IF;
 END IF;
 IF EXISTS (SELECT 1 FROM contact_identities GROUP BY user_id,kind HAVING count(*)>1) THEN
   RAISE EXCEPTION 'contact book contains multiple verified addresses';
 END IF;
END $$;
ALTER TABLE order_items DROP COLUMN IF EXISTS recipient_bound_user_id;
ALTER TABLE order_items DROP COLUMN IF EXISTS recipient_change_confirmed;
DROP TABLE IF EXISTS sender_contact_methods;
DROP TABLE IF EXISTS sender_contacts;
DROP INDEX IF EXISTS contact_identities_user_kind_idx;

-- Refuse rollback while unresolved gifts/orders exist; never discard paid gifts.
ALTER TABLE cart_items ALTER COLUMN recipient_id SET NOT NULL;
ALTER TABLE order_items ALTER COLUMN recipient_id SET NOT NULL;
ALTER TABLE gifts ALTER COLUMN recipient_id SET NOT NULL;
ALTER TABLE notifications DROP COLUMN gift_id, DROP COLUMN event_key, DROP COLUMN type;
DROP TABLE delivery_outbox;
DROP TABLE contact_challenges;
DROP TABLE contact_identities;
ALTER TABLE cart_items DROP CONSTRAINT cart_recipient, DROP COLUMN recipient_contact;
ALTER TABLE order_items DROP CONSTRAINT order_recipient, DROP COLUMN recipient_contact;
ALTER TABLE gifts DROP CONSTRAINT gift_recipient, DROP COLUMN recipient_contact, DROP COLUMN invitation_hash;
