ALTER TABLE gifts
    ADD COLUMN price_cents BIGINT NOT NULL DEFAULT 0 CHECK (price_cents >= 0),
    ADD COLUMN unlock_kind TEXT NOT NULL DEFAULT 'free'
        CHECK (unlock_kind IN ('free','guess_who','question','passphrase')),
    ADD COLUMN clue TEXT NOT NULL DEFAULT '',
    ADD COLUMN answer_hash TEXT,
    ADD COLUMN message TEXT NOT NULL DEFAULT '',
    ADD COLUMN contract_text TEXT NOT NULL DEFAULT '',
    ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 3),
    ADD COLUMN identity_known BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ADD COLUMN expires_at TIMESTAMPTZ NOT NULL DEFAULT (now() + INTERVAL '7 days'),
    ADD COLUMN opened_at TIMESTAMPTZ,
    ADD COLUMN revealed_at TIMESTAMPTZ,
    ADD COLUMN settled_at TIMESTAMPTZ,
    ADD COLUMN voucher_code TEXT,
    ADD CONSTRAINT gifts_state_check CHECK
        (state IN ('sealed','opened','revealed','accepted','exchanged','cashed_out','withdrawn','expired'));

UPDATE gifts SET price_cents = catalog.price_cents
FROM catalog WHERE gifts.product_id = catalog.id;
UPDATE gifts SET identity_known=true, revealed_at=created_at, settled_at=created_at
WHERE state='accepted';

CREATE INDEX gifts_inbox_idx ON gifts(recipient_id, available_at DESC, id DESC);
CREATE INDEX gifts_outbox_idx ON gifts(sender_id, created_at DESC, id DESC);
