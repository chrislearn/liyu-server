CREATE TABLE friend_tags (
    id BIGSERIAL PRIMARY KEY,
    owner_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (owner_id, name)
);

CREATE TABLE friend_tag_members (
    tag_id BIGINT NOT NULL REFERENCES friend_tags(id) ON DELETE CASCADE,
    friend_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (tag_id, friend_id)
);
CREATE INDEX friend_tag_members_friend_idx ON friend_tag_members(friend_id);
