CREATE TABLE contact_identities (
 id BIGSERIAL PRIMARY KEY, user_id BIGINT NOT NULL REFERENCES users(id),
 kind TEXT NOT NULL CHECK(kind IN ('phone','email')), value TEXT NOT NULL,
 verified_at TIMESTAMPTZ NOT NULL DEFAULT now(), UNIQUE(kind,value), UNIQUE(user_id,kind)
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
