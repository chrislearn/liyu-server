-- A verified address belongs to one account, while an account may own many addresses.
ALTER TABLE contact_identities DROP CONSTRAINT contact_identities_user_id_kind_key;
CREATE INDEX contact_identities_user_kind_idx ON contact_identities(user_id, kind);

-- The sender's address book keeps the first resolved account as an identity anchor.
-- Display names and address values are mutable; bound_user_id is never silently rewritten.
CREATE TABLE sender_contacts (
 id BIGSERIAL PRIMARY KEY,
 owner_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 label TEXT NOT NULL DEFAULT '' CHECK (char_length(label) <= 100),
 bound_user_id BIGINT REFERENCES users(id),
 expired_at TIMESTAMPTZ,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 UNIQUE(id,owner_id)
);
CREATE INDEX sender_contacts_owner_idx ON sender_contacts(owner_id, id) WHERE expired_at IS NULL;

CREATE TABLE sender_contact_methods (
 id BIGSERIAL PRIMARY KEY,
 contact_id BIGINT NOT NULL,
 owner_id BIGINT NOT NULL,
 kind TEXT NOT NULL CHECK (kind IN ('phone','email')),
 value TEXT NOT NULL,
 expired_at TIMESTAMPTZ,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 FOREIGN KEY (contact_id,owner_id) REFERENCES sender_contacts(id,owner_id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX sender_contact_methods_active_unique
 ON sender_contact_methods(owner_id,kind,value) WHERE expired_at IS NULL;
CREATE INDEX sender_contact_methods_contact_idx ON sender_contact_methods(contact_id);

-- A checkout snapshot remains available if the address book changes before payment.
ALTER TABLE order_items ADD COLUMN recipient_bound_user_id BIGINT REFERENCES users(id);
ALTER TABLE order_items ADD COLUMN recipient_change_confirmed BOOLEAN NOT NULL DEFAULT false;
UPDATE order_items SET recipient_bound_user_id=recipient_id
 WHERE recipient_contact IS NOT NULL AND recipient_id IS NOT NULL;
