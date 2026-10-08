CREATE TABLE browser_authorizations (
 id TEXT PRIMARY KEY,
 poll_hash TEXT NOT NULL,
 browser_hash TEXT NOT NULL,
 purpose TEXT NOT NULL CHECK (purpose IN ('login','email','phone')),
 owner_id BIGINT REFERENCES users(id),
 approved_user_id BIGINT REFERENCES users(id),
 state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','approved','consumed','cancelled')),
 source_hash TEXT NOT NULL,
 login_attempts INTEGER NOT NULL DEFAULT 0,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '5 minutes'
);
CREATE INDEX browser_authorizations_source ON browser_authorizations(source_hash,created_at);
