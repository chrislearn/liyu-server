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
