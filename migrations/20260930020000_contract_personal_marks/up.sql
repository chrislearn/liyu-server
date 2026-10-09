-- A mark is one participant's private assessment, never a shared contract state.
-- Existing gift_contracts rows remain historical; do not infer either user's
-- assessment from their former shared status.
CREATE TABLE gift_contract_marks (
    gift_id BIGINT NOT NULL REFERENCES gifts(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('pending', 'fulfilled')),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (gift_id, user_id)
);
CREATE INDEX gift_contract_marks_user_idx ON gift_contract_marks(user_id, gift_id);

-- purchase and calendar extension (shared with startup upgrade)
ALTER TABLE orders ADD COLUMN IF NOT EXISTS request_hash TEXT;
CREATE TABLE IF NOT EXISTS gift_contract_schedules (
    gift_id BIGINT PRIMARY KEY REFERENCES gifts(id) ON DELETE CASCADE,
    confirmed_on DATE,
    proposed_on DATE,
    proposer_id BIGINT REFERENCES users(id),
    confirmed_by BIGINT REFERENCES users(id),
    revision BIGINT NOT NULL DEFAULT 1 CHECK (revision > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (confirmed_by IS NULL OR confirmed_by <> proposer_id)
);
-- end purchase and calendar extension
