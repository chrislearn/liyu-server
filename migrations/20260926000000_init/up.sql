CREATE TABLE users (
    id BIGSERIAL PRIMARY KEY,
    identifier TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    state JSONB,
    state_revision BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX sessions_user_id_idx ON sessions(user_id);

CREATE TABLE catalog (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    category TEXT NOT NULL,
    price_cents BIGINT NOT NULL CHECK (price_cents >= 0),
    physical BOOLEAN NOT NULL
);

INSERT INTO catalog (id, name, category, price_cents, physical) VALUES
 (0, '三顿半精品咖啡礼盒', 'coffee', 10900, true),
 (1, '星巴克中杯拿铁电子券', 'coffee', 3500, false),
 (2, '喜茶多肉葡萄兑换券', 'coffee', 2900, false),
 (3, '电影通兑票', 'movie', 4900, false),
 (4, '电影双人套票', 'movie', 9800, false),
 (5, '帆布托特包', 'trendy', 7900, true);

INSERT INTO users (identifier, display_name, password_hash) VALUES
 ('demo@liyu.test', '演示用户', '8d969eef6ecad3c29a3a629280e686cf0c3f5d5a86aff3ca12020c923adc6c92'),
 ('linzhou@liyu.test', '林舟', '8d969eef6ecad3c29a3a629280e686cf0c3f5d5a86aff3ca12020c923adc6c92'),
 ('chenxiao@liyu.test', '陈晓', '8d969eef6ecad3c29a3a629280e686cf0c3f5d5a86aff3ca12020c923adc6c92')
ON CONFLICT (identifier) DO NOTHING;
