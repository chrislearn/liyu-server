CREATE TABLE friend_details (
    owner_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    friend_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    nickname TEXT NOT NULL DEFAULT '' CHECK (char_length(nickname) <= 100),
    phone TEXT NOT NULL DEFAULT '' CHECK (char_length(phone) <= 100),
    email TEXT NOT NULL DEFAULT '' CHECK (char_length(email) <= 254),
    relationship TEXT NOT NULL DEFAULT '' CHECK (char_length(relationship) <= 100),
    birthday TEXT NOT NULL DEFAULT '' CHECK (char_length(birthday) <= 40),
    note TEXT NOT NULL DEFAULT '' CHECK (char_length(note) <= 500),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (owner_id, friend_id),
    CHECK (owner_id <> friend_id)
);
