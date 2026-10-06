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
