CREATE TABLE friendships (
    id BIGSERIAL PRIMARY KEY,
    user_low_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    user_high_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    requested_by_user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'accepted')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    accepted_at TIMESTAMPTZ,
    UNIQUE (user_low_id, user_high_id),
    CHECK (user_low_id < user_high_id),
    CHECK (requested_by_user_id IN (user_low_id, user_high_id)),
    CHECK ((status = 'accepted') = (accepted_at IS NOT NULL))
);
CREATE INDEX friendships_high_status_idx ON friendships(user_high_id, status);

CREATE TABLE wishlists (
    id BIGSERIAL PRIMARY KEY,
    owner_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL CHECK (char_length(title) BETWEEN 1 AND 50),
    note TEXT NOT NULL DEFAULT '' CHECK (char_length(note) <= 200),
    occasion TEXT NOT NULL DEFAULT 'other',
    event_on DATE NOT NULL,
    closed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX wishlists_owner_event_idx ON wishlists(owner_id, event_on);

-- No rows means all accepted friends. Otherwise only listed accepted friends see it.
CREATE TABLE wishlist_audience (
    wishlist_id BIGINT NOT NULL REFERENCES wishlists(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (wishlist_id, user_id)
);

CREATE TABLE wishlist_items (
    id BIGSERIAL PRIMARY KEY,
    wishlist_id BIGINT NOT NULL REFERENCES wishlists(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    product_id INTEGER REFERENCES catalog(id),
    kind TEXT NOT NULL DEFAULT '',
    max_price_cents BIGINT NOT NULL DEFAULT 0 CHECK (max_price_cents >= 0),
    wants TEXT NOT NULL DEFAULT '' CHECK (char_length(wants) <= 100),
    claimed_gift_id BIGINT UNIQUE REFERENCES gifts(id),
    claimed_by_user_id BIGINT REFERENCES users(id),
    claimed_at TIMESTAMPTZ,
    UNIQUE (wishlist_id, ordinal),
    CHECK (product_id IS NOT NULL OR char_length(kind) > 0),
    CHECK ((claimed_gift_id IS NULL) = (claimed_by_user_id IS NULL))
);
CREATE INDEX wishlist_items_list_idx ON wishlist_items(wishlist_id, ordinal);

-- Demo users are confirmed friends, but each relationship still has a canonical pair.
INSERT INTO friendships (user_low_id, user_high_id, requested_by_user_id, status, accepted_at)
SELECT LEAST(a.id, b.id), GREATEST(a.id, b.id), LEAST(a.id, b.id), 'accepted', now()
FROM users a JOIN users b ON a.id < b.id
WHERE a.identifier IN ('demo@liyu.test', 'linzhou@liyu.test', 'chenxiao@liyu.test')
  AND b.identifier IN ('demo@liyu.test', 'linzhou@liyu.test', 'chenxiao@liyu.test')
ON CONFLICT (user_low_id, user_high_id) DO NOTHING;

-- Three realistic demo lists. The second gift is a sealed, unrevealed claim.
INSERT INTO gifts (id, sender_id, recipient_id, product_id, state)
SELECT 900002, s.id, r.id, 17, 'sealed'
FROM users s CROSS JOIN users r
WHERE s.identifier='chenxiao@liyu.test' AND r.identifier='demo@liyu.test'
ON CONFLICT (id) DO NOTHING;

INSERT INTO wishlists (id, owner_id, title, note, occasion, event_on)
SELECT 900101, id, '阿岚的生日心愿单', '想要的不多，心意最重要', 'birthday', CURRENT_DATE + 14
FROM users WHERE identifier='demo@liyu.test'
ON CONFLICT (id) DO NOTHING;
INSERT INTO wishlists (id, owner_id, title, note, occasion, event_on)
SELECT 900102, id, '林舟的新家暖房', '新客厅想添点温暖', 'housewarming', CURRENT_DATE + 21
FROM users WHERE identifier='linzhou@liyu.test'
ON CONFLICT (id) DO NOTHING;
INSERT INTO wishlists (id, owner_id, title, note, occasion, event_on)
SELECT 900103, id, '陈晓的毕业礼物', '要去新城市上班啦', 'graduation', CURRENT_DATE + 10
FROM users WHERE identifier='chenxiao@liyu.test'
ON CONFLICT (id) DO NOTHING;

INSERT INTO wishlist_items (id,wishlist_id,ordinal,product_id,kind,claimed_gift_id,claimed_by_user_id,claimed_at)
SELECT 900201,900101,0,17,'耳机',900002,u.id,now()
FROM users u WHERE u.identifier='chenxiao@liyu.test'
ON CONFLICT (id) DO NOTHING;
INSERT INTO wishlist_items (id,wishlist_id,ordinal,kind,max_price_cents,wants)
VALUES (900202,900101,1,'咖啡',30000,'冷萃'),
       (900204,900102,1,'电视',300000,'55 寸以上'),
       (900205,900103,0,'音箱',50000,'便携')
ON CONFLICT (id) DO NOTHING;
INSERT INTO wishlist_items (id,wishlist_id,ordinal,product_id,kind,claimed_gift_id,claimed_by_user_id,claimed_at)
SELECT 900203,900102,0,5,'包',900001,u.id,now()
FROM users u WHERE u.identifier='demo@liyu.test'
ON CONFLICT (id) DO NOTHING;
INSERT INTO wishlist_items (id,wishlist_id,ordinal,product_id,kind)
VALUES (900206,900103,1,29,'杯子')
ON CONFLICT (id) DO NOTHING;

-- Chen Xiao shares this list with demo only, not every accepted friend.
INSERT INTO wishlist_audience (wishlist_id,user_id)
SELECT 900103,id FROM users WHERE identifier='demo@liyu.test'
ON CONFLICT DO NOTHING;
