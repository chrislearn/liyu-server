CREATE TABLE contact_identities (
 id BIGSERIAL PRIMARY KEY, user_id BIGINT NOT NULL REFERENCES users(id),
 kind TEXT NOT NULL CHECK(kind IN ('phone','email')), value TEXT NOT NULL,
 verified_at TIMESTAMPTZ NOT NULL DEFAULT now(), UNIQUE(kind,value)
);
CREATE TABLE contact_challenges (
 id TEXT PRIMARY KEY, kind TEXT NOT NULL, value TEXT NOT NULL,
 purpose TEXT NOT NULL CHECK(purpose IN ('register','bind')),
 owner_id BIGINT REFERENCES users(id), code_hash TEXT NOT NULL, source_hash TEXT NOT NULL,
 attempts INTEGER NOT NULL DEFAULT 0, consumed_at TIMESTAMPTZ,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(), expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '10 minutes'
);
CREATE INDEX challenge_rate_idx ON contact_challenges(kind,value,created_at);
CREATE TABLE delivery_outbox (
 id BIGSERIAL PRIMARY KEY, event_key TEXT NOT NULL UNIQUE,
 gift_id BIGINT REFERENCES gifts(id), kind TEXT NOT NULL CHECK(kind IN ('phone','email')),
 destination TEXT NOT NULL, payload JSONB NOT NULL,
 status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','sending','sent','failed','cancelled')),
 attempts INTEGER NOT NULL DEFAULT 0, next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 lease_until TIMESTAMPTZ, last_error TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), sent_at TIMESTAMPTZ
);
CREATE INDEX outbox_due_idx ON delivery_outbox(status,next_attempt_at);
ALTER TABLE cart_items ALTER COLUMN recipient_id DROP NOT NULL;
ALTER TABLE order_items ALTER COLUMN recipient_id DROP NOT NULL;
ALTER TABLE gifts ALTER COLUMN recipient_id DROP NOT NULL;
ALTER TABLE cart_items ADD COLUMN recipient_contact JSONB;
ALTER TABLE order_items ADD COLUMN recipient_contact JSONB;
ALTER TABLE gifts ADD COLUMN recipient_contact JSONB;
ALTER TABLE gifts ADD COLUMN invitation_hash TEXT UNIQUE;
ALTER TABLE cart_items ADD CONSTRAINT cart_recipient CHECK(recipient_id IS NOT NULL OR recipient_contact IS NOT NULL);
ALTER TABLE order_items ADD CONSTRAINT order_recipient CHECK(recipient_id IS NOT NULL OR recipient_contact IS NOT NULL);
ALTER TABLE gifts ADD CONSTRAINT gift_recipient CHECK(recipient_id IS NOT NULL OR recipient_contact IS NOT NULL);
ALTER TABLE notifications ADD COLUMN gift_id BIGINT REFERENCES gifts(id);
ALTER TABLE notifications ADD COLUMN event_key TEXT UNIQUE;
ALTER TABLE notifications ADD COLUMN type TEXT NOT NULL DEFAULT 'general';

-- contact book extension (shared with startup upgrade)
-- A verified address belongs to one account, while an account may own many addresses.
ALTER TABLE contact_identities DROP CONSTRAINT IF EXISTS contact_identities_user_id_kind_key;
CREATE INDEX IF NOT EXISTS contact_identities_user_kind_idx ON contact_identities(user_id, kind);

-- The sender's address book keeps the first resolved account as an identity anchor.
-- Display names and address values are mutable; bound_user_id is never silently rewritten.
CREATE TABLE IF NOT EXISTS sender_contacts (
 id BIGSERIAL PRIMARY KEY,
 owner_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 label TEXT NOT NULL DEFAULT '' CHECK (char_length(label) <= 100),
 bound_user_id BIGINT REFERENCES users(id),
 expired_at TIMESTAMPTZ,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 UNIQUE(id,owner_id)
);
CREATE INDEX IF NOT EXISTS sender_contacts_owner_idx ON sender_contacts(owner_id, id) WHERE expired_at IS NULL;

CREATE TABLE IF NOT EXISTS sender_contact_methods (
 id BIGSERIAL PRIMARY KEY,
 contact_id BIGINT NOT NULL,
 owner_id BIGINT NOT NULL,
 kind TEXT NOT NULL CHECK (kind IN ('phone','email')),
 value TEXT NOT NULL,
 expired_at TIMESTAMPTZ,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 FOREIGN KEY (contact_id,owner_id) REFERENCES sender_contacts(id,owner_id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX IF NOT EXISTS sender_contact_methods_active_unique
 ON sender_contact_methods(owner_id,kind,value) WHERE expired_at IS NULL;
CREATE INDEX IF NOT EXISTS sender_contact_methods_contact_idx ON sender_contact_methods(contact_id);

-- A checkout snapshot remains available if the address book changes before payment.
ALTER TABLE order_items ADD COLUMN IF NOT EXISTS recipient_bound_user_id BIGINT REFERENCES users(id);
ALTER TABLE order_items ADD COLUMN IF NOT EXISTS recipient_change_confirmed BOOLEAN NOT NULL DEFAULT false;
UPDATE order_items SET recipient_bound_user_id=recipient_id
 WHERE recipient_contact IS NOT NULL AND recipient_id IS NOT NULL AND recipient_bound_user_id IS NULL;
-- end contact book extension
