CREATE TABLE user_profiles (
    user_id BIGINT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    phone TEXT UNIQUE,
    email TEXT UNIQUE,
    avatar_url TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT profile_phone_format CHECK (phone IS NULL OR phone ~ '^[0-9]{11}$'),
    CONSTRAINT profile_email_length CHECK (email IS NULL OR char_length(email) BETWEEN 3 AND 254)
);

CREATE TABLE shipping_addresses (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    recipient_name TEXT NOT NULL,
    phone TEXT NOT NULL,
    address TEXT NOT NULL,
    is_default BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT shipping_recipient_length CHECK (char_length(recipient_name) BETWEEN 1 AND 50),
    CONSTRAINT shipping_phone_format CHECK (phone ~ '^[0-9]{11}$'),
    CONSTRAINT shipping_address_length CHECK (char_length(address) BETWEEN 1 AND 500)
);
CREATE INDEX shipping_addresses_user_id_idx ON shipping_addresses(user_id);
CREATE UNIQUE INDEX shipping_addresses_one_default_per_user_idx
    ON shipping_addresses(user_id) WHERE is_default;

INSERT INTO user_profiles (user_id)
SELECT id FROM users
ON CONFLICT (user_id) DO NOTHING;
