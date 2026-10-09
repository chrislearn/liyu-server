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

-- host PKCE extension
CREATE TABLE IF NOT EXISTS host_oauth_requests (
 auth_id TEXT PRIMARY KEY REFERENCES browser_authorizations(id) ON DELETE CASCADE,
 challenge TEXT NOT NULL, redirect_uri TEXT NOT NULL, state TEXT NOT NULL,
 code_hash TEXT UNIQUE, user_id BIGINT REFERENCES users(id), consumed BOOLEAN NOT NULL DEFAULT false,
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '5 minutes'
);
CREATE TABLE IF NOT EXISTS host_oauth_grants (
 refresh_hash TEXT PRIMARY KEY, access_hash TEXT NOT NULL UNIQUE, user_id BIGINT NOT NULL REFERENCES users(id),
 family TEXT NOT NULL, used BOOLEAN NOT NULL DEFAULT false, revoked BOOLEAN NOT NULL DEFAULT false,
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '90 days'
);
CREATE INDEX IF NOT EXISTS host_oauth_grants_family ON host_oauth_grants(family);
