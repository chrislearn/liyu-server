-- Refuse to lose address-book anchors or checkout snapshots on rollback.
DO $$ BEGIN
 IF EXISTS (SELECT 1 FROM sender_contacts) OR
    EXISTS (SELECT 1 FROM order_items WHERE recipient_bound_user_id IS NOT NULL OR recipient_change_confirmed) OR
    EXISTS (SELECT 1 FROM contact_identities GROUP BY user_id,kind HAVING count(*)>1) THEN
   RAISE EXCEPTION 'contact book contains data that cannot be represented by the old schema';
 END IF;
END $$;
ALTER TABLE order_items DROP COLUMN recipient_bound_user_id;
ALTER TABLE order_items DROP COLUMN recipient_change_confirmed;
DROP TABLE sender_contact_methods;
DROP TABLE sender_contacts;
DROP INDEX contact_identities_user_kind_idx;
ALTER TABLE contact_identities ADD CONSTRAINT contact_identities_user_id_kind_key UNIQUE(user_id,kind);
