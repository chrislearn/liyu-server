-- liyu-server initial schema.
--
-- This single pair of migrations replaces the historical incremental chain
-- (init → profile → catalog → fulfillment → commerce → wishlist → gifting →
-- demo_logistics → wish_claim → session_expiry → avatars). A fresh database
-- converges to exactly the same schema and demo-data semantics; seed rows are
-- written in their final form with their historical keys and fixture values.
--
-- Section order follows the original migration order so foreign keys resolve:
--   1. users / sessions / catalog
--   2. user_profiles / shipping_addresses
--   3. catalog columns + full product seed
--   4. gifts / shipments / shipment_events (+ demo gift 900001)
--   5. cart_items / orders / order_items
--   6. friendships / wishlists / wishlist_audience / wishlist_items (+ demo rows)
--   7. gifting columns on gifts
--   8. shipment stage codes + demo gift 900003
--   9. wish_item_id on cart_items / order_items
--  10. sessions.expires_at
--  11. avatars

-- 1. init --------------------------------------------------------------------

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

INSERT INTO users (identifier, display_name, password_hash) VALUES
 ('demo@liyu.test', '演示用户', '8d969eef6ecad3c29a3a629280e686cf0c3f5d5a86aff3ca12020c923adc6c92'),
 ('linzhou@liyu.test', '林舟', '8d969eef6ecad3c29a3a629280e686cf0c3f5d5a86aff3ca12020c923adc6c92'),
 ('chenxiao@liyu.test', '陈晓', '8d969eef6ecad3c29a3a629280e686cf0c3f5d5a86aff3ca12020c923adc6c92')
ON CONFLICT (identifier) DO NOTHING;

-- 2. profile -----------------------------------------------------------------

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

-- 3. catalog (final shape + full product seed) --------------------------------

ALTER TABLE catalog
    ADD COLUMN brand TEXT NOT NULL DEFAULT '',
    ADD COLUMN kind TEXT NOT NULL DEFAULT '',
    ADD COLUMN spec TEXT NOT NULL DEFAULT '',
    ADD COLUMN description TEXT NOT NULL DEFAULT '',
    ADD COLUMN tags TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN stock INTEGER NOT NULL DEFAULT 100 CHECK (stock >= 0),
    ADD COLUMN is_active BOOLEAN NOT NULL DEFAULT true;

INSERT INTO catalog (id, name, category, price_cents, physical, brand, kind, spec, description, tags, stock)
VALUES
(0, '三顿半精品咖啡礼盒', 'coffee', 10900, true, '三顿半', '咖啡', '24 颗装 · 包邮', '超即溶的精品咖啡，冷水牛奶都能三秒化开。24 颗 6 种风味，一个月的早晨都有着落。', ARRAY['冷萃','速溶','礼盒']::TEXT[], 100),
(1, '星巴克中杯拿铁电子券', 'coffee', 3500, false, '星巴克', '咖啡', '全国门店通用 · 30 天有效', '一杯中杯拿铁，想喝的时候去门店出示券码就行。', ARRAY['电子券','门店']::TEXT[], 100),
(2, '喜茶多肉葡萄兑换券', 'coffee', 2900, false, '喜茶', '奶茶', '全国门店通用 · 30 天有效', '招牌多肉葡萄，一整杯的果肉。门店或小程序都能兑。', ARRAY['电子券','水果茶']::TEXT[], 100),
(3, '电影通兑票', 'movie', 4900, false, '猫眼', '电影票', '2D 场次通兑 · 60 天有效', '全国大部分影院 2D 场次通兑，挑一部想看的片子就好。', ARRAY['电子券','2D']::TEXT[], 100),
(4, '电影双人套票', 'movie', 9800, false, '猫眼', '电影票', '两张通兑票 + 爆米花套餐', '两张票加一份爆米花套餐 —— 送了这个，下次见面就有了理由。', ARRAY['电子券','双人','爆米花']::TEXT[], 100),
(5, '帆布托特包', 'trendy', 7900, true, '野帆', '包', '米白 · 加厚帆布 · 包邮', '16 安加厚帆布，装得下电脑和一天的零碎。越用越软。', ARRAY['米白','大容量','帆布']::TEXT[], 100),
(6, '香薰蜡烛', 'trendy', 8800, true, '栖木', '香薰', '无花果香 · 200g · 包邮', '无花果叶和一点木质调，点燃 40 小时。睡前点一会儿，房间是暖的。', ARRAY['无花果','助眠','200g']::TEXT[], 100),
(7, '拍立得相纸', 'trendy', 5900, true, '即影', '相纸', 'mini 白边 · 40 张 · 包邮', 'mini 规格白边相纸 40 张，适配常见的拍立得相机。', ARRAY['mini','40张']::TEXT[], 100),
(8, '潮玩盲盒', 'blind', 6900, true, '泡泡岛', '盲盒', '随机一款 · 有隐藏款 · 包邮', '12 款常规加 1 款隐藏，拆之前谁也不知道是哪一只。', ARRAY['潮玩','隐藏款','摆件']::TEXT[], 100),
(9, '文具盲盒', 'blind', 3900, true, '纸上', '盲盒', '6 件随机 · 包邮', '胶带、便签、印章、钢笔…… 随机 6 件，做手帐的人会喜欢。', ARRAY['文具','手帐']::TEXT[], 100),
(10, '小蛋糕兑换券', 'sweet', 12800, false, '好利来', '蛋糕', '6 寸 · 指定门店自提', '6 寸鲜奶蛋糕，提前一天在小程序预约，门店自提。', ARRAY['电子券','6寸','生日']::TEXT[], 100),
(11, '向日葵花束', 'sweet', 9900, true, '花点时间', '鲜花', '3 枝装 · 同城配送', '三枝向日葵配尤加利叶，同城当天配送。', ARRAY['向日葵','同城']::TEXT[], 100),
(12, '澄光 Q5 55 英寸 4K 电视', 'digital', 269900, true, '澄光', '电视', '55 英寸 · 4K · 包邮包安装', '55 英寸 4K 屏，低蓝光护眼模式。开机无广告，老人小孩都会用。', ARRAY['55寸','4K','护眼','客厅']::TEXT[], 100),
(13, '澄光 Q7 65 英寸 4K 电视', 'digital', 399900, true, '澄光', '电视', '65 英寸 · 4K 120Hz · 包邮包安装', '65 英寸 4K 120Hz 高刷，接游戏机不拖影。客厅一面墙刚刚好。', ARRAY['65寸','4K','120Hz','游戏','客厅']::TEXT[], 100),
(14, '声屿 Mini 43 英寸电视', 'digital', 129900, true, '声屿', '电视', '43 英寸 · 全高清 · 包邮', '43 英寸全高清，放卧室或出租屋正合适。自带音箱不闷。', ARRAY['43寸','全高清','卧室']::TEXT[], 100),
(15, '澄光 Q9 75 英寸 Mini LED 电视', 'digital', 699900, true, '澄光', '电视', '75 英寸 · Mini LED · 包邮包安装', '75 英寸 Mini LED，暗场景也看得清。给新家客厅的一份大礼。', ARRAY['75寸','4K','MiniLED','120Hz','客厅']::TEXT[], 100),
(16, '声屿 Pebble 蓝牙音箱', 'digital', 39900, true, '声屿', '音箱', '蓝牙 5.3 · IP67 防水 · 包邮', '鹅卵石大小，扔进包里就走。防水，浴室和野餐都能放。', ARRAY['蓝牙','便携','防水']::TEXT[], 100),
(17, '声屿 Air 降噪耳机', 'digital', 89900, true, '声屿', '耳机', '头戴式 · 主动降噪 · 包邮', '主动降噪，地铁上也能听清轻音乐。一次充电用 40 小时。', ARRAY['降噪','蓝牙','头戴','长续航']::TEXT[], 100),
(18, '光语 便携投影仪', 'digital', 199900, true, '光语', '投影仪', '1080P · 自动对焦 · 包邮', '往白墙一照就是 100 寸。自动对焦，躺在床上看电影。', ARRAY['1080P','便携','卧室']::TEXT[], 100),
(19, '暖物 可视空气炸锅 5L', 'home', 32900, true, '暖物', '空气炸锅', '5L · 可视窗 · 包邮', '透明可视窗，炸到几分熟一眼就知道。5L 够三四个人吃。', ARRAY['5L','可视','大容量']::TEXT[], 100),
(20, '暖物 保温电热水壶', 'home', 15900, true, '暖物', '水壶', '1.7L · 恒温 · 包邮', '五档恒温，泡茶冲奶都合适。304 不锈钢内胆。', ARRAY['1.7L','恒温','泡茶']::TEXT[], 100),
(21, '栖木 全棉四件套', 'home', 45900, true, '栖木', '床品', '1.8m 床 · 60 支全棉 · 包邮', '60 支长绒棉，奶油色，越洗越软。搬新家换一套新床品。', ARRAY['1.8m','全棉','奶油色']::TEXT[], 100),
(22, '栖木 陶瓷餐具礼盒', 'home', 26900, true, '栖木', '餐具', '8 件套 · 可进洗碗机 · 包邮', '两人份的碗盘杯 8 件，哑光釉面，可进洗碗机和微波炉。', ARRAY['陶瓷','8件','洗碗机']::TEXT[], 100),
(23, '栖木 氛围落地灯', 'home', 23900, true, '栖木', '灯', '暖光 · 三档调光 · 包邮', '暖白两色、三档亮度。沙发边放一盏，晚上就不想开大灯了。', ARRAY['暖光','调光','卧室']::TEXT[], 100),
(24, '琴叶榕盆栽', 'home', 13900, true, '青田', '绿植', '80cm 高 · 含盆 · 同城配送', '80 厘米的琴叶榕，叶子大、好养活。放新家客厅一角。', ARRAY['大盆','好养','客厅']::TEXT[], 100),
(25, '多肉组合盆栽', 'home', 5900, true, '青田', '绿植', '6 株 · 陶盆 · 包邮', '六株多肉拼在一个陶盆里，两周浇一次水就行。', ARRAY['小盆','好养','桌面']::TEXT[], 100),
(26, '小橡 轻便婴儿推车', 'baby', 129900, true, '小橡', '推车', '可登机 · 5.8kg · 包邮', '一只手就能收起来，5.8 公斤，能带上飞机。', ARRAY['轻便','可登机','可躺']::TEXT[], 100),
(27, '小橡 新生儿礼盒', 'baby', 36900, true, '小橡', '母婴礼盒', '0–6 个月 · 纯棉 9 件 · 包邮', '纯棉和尚服、口水巾、小袜子 9 件，满月礼刚刚好。', ARRAY['纯棉','0-6月','满月']::TEXT[], 100),
(28, '木作益智积木', 'baby', 19900, true, '小橡', '玩具', '100 块 · 3 岁以上 · 包邮', '100 块榉木积木，水性漆，棱角都磨圆了。', ARRAY['3岁+','益智','木质']::TEXT[], 100),
(29, '随行保温杯', 'trendy', 12900, true, '野帆', '杯子', '480ml · 12 小时保温 · 包邮', '一只手能开盖，放包里不漏。热水放到下午还烫。', ARRAY['480ml','保温','通勤']::TEXT[], 100),
(30, '暖物 扫拖机器人', 'home', 189900, true, '暖物', '扫地机', '扫拖一体 · 自动集尘 · 包邮', '扫拖一体，自动倒尘。搬进新家，地板交给它。', ARRAY['扫拖一体','自动集尘','新家']::TEXT[], 100),
(31, '声屿 S6 55 英寸 QLED 电视', 'digital', 249900, true, '声屿', '电视', '55 英寸 · QLED · 包邮包安装', '55 英寸量子点屏，颜色比普通 4K 更鲜亮。窄边框，挂在卧室不占地方。', ARRAY['55寸','4K','QLED','卧室']::TEXT[], 100),
(32, '澄光 Q6 65 英寸 4K 电视', 'digital', 299900, true, '澄光', '电视', '65 英寸 · 4K · 包邮包安装', '65 英寸 4K，三千块以内能买到的大屏。护眼模式、开机无广告。', ARRAY['65寸','4K','护眼','客厅']::TEXT[], 100)
ON CONFLICT (id) DO UPDATE SET
    name = EXCLUDED.name,
    category = EXCLUDED.category,
    price_cents = EXCLUDED.price_cents,
    physical = EXCLUDED.physical,
    brand = EXCLUDED.brand,
    kind = EXCLUDED.kind,
    spec = EXCLUDED.spec,
    description = EXCLUDED.description,
    tags = EXCLUDED.tags;

CREATE INDEX catalog_category_id_idx ON catalog(category, id);
CREATE INDEX catalog_search_idx ON catalog USING GIN (to_tsvector('simple', name || ' ' || brand || ' ' || kind || ' ' || description));

-- 4. fulfillment -------------------------------------------------------------

-- One gift is shared by its sender and recipient. Every API must project by role.
CREATE TABLE gifts (
    id BIGSERIAL PRIMARY KEY,
    sender_id BIGINT NOT NULL REFERENCES users(id),
    recipient_id BIGINT NOT NULL REFERENCES users(id),
    product_id INTEGER NOT NULL REFERENCES catalog(id),
    state TEXT NOT NULL DEFAULT 'sealed',
    exchanged_item_id INTEGER REFERENCES catalog(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (sender_id <> recipient_id)
);
CREATE INDEX gifts_sender_id_idx ON gifts(sender_id);
CREATE INDEX gifts_recipient_id_idx ON gifts(recipient_id);

CREATE TABLE shipments (
    gift_id BIGINT PRIMARY KEY REFERENCES gifts(id) ON DELETE CASCADE,
    carrier TEXT NOT NULL,
    tracking_number TEXT NOT NULL,
    recipient_name TEXT NOT NULL,
    recipient_phone TEXT NOT NULL,
    recipient_address TEXT NOT NULL,
    delivered_at TIMESTAMPTZ,
    recipient_confirmed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (recipient_confirmed_at IS NULL OR delivered_at IS NOT NULL)
);

CREATE TABLE shipment_events (
    id BIGSERIAL PRIMARY KEY,
    gift_id BIGINT NOT NULL REFERENCES shipments(gift_id) ON DELETE CASCADE,
    event_at TIMESTAMPTZ NOT NULL,
    description TEXT NOT NULL
);
CREATE INDEX shipment_events_gift_at_idx ON shipment_events(gift_id, event_at);

-- Fictitious data exercises all three roles: demo sends, Lin Zhou receives, Chen Xiao is unrelated.
INSERT INTO gifts (id, sender_id, recipient_id, product_id, state)
SELECT 900001, s.id, r.id, 5, 'accepted'
FROM users s CROSS JOIN users r
WHERE s.identifier = 'demo@liyu.test' AND r.identifier = 'linzhou@liyu.test'
ON CONFLICT (id) DO NOTHING;

INSERT INTO shipments
    (gift_id, carrier, tracking_number, recipient_name, recipient_phone, recipient_address, delivered_at)
SELECT 900001, '礼遇演示快递', 'LY-DEMO-20260926-0001', '林舟',
       '13800138000', '上海市虚构区演示路 123 号', now() - INTERVAL '1 day'
WHERE EXISTS (SELECT 1 FROM gifts WHERE id = 900001)
ON CONFLICT (gift_id) DO NOTHING;

INSERT INTO shipment_events (gift_id, event_at, description)
SELECT 900001, now() - INTERVAL '2 days', '演示包裹已揽收'
WHERE EXISTS (SELECT 1 FROM shipments WHERE gift_id = 900001);
INSERT INTO shipment_events (gift_id, event_at, description)
SELECT 900001, now() - INTERVAL '1 day', '演示包裹已签收'
WHERE EXISTS (SELECT 1 FROM shipments WHERE gift_id = 900001);

-- 5. commerce ----------------------------------------------------------------

CREATE TABLE cart_items (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    product_id INTEGER NOT NULL REFERENCES catalog(id),
    recipient_id BIGINT NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (user_id <> recipient_id)
);
CREATE INDEX cart_items_user_id_idx ON cart_items(user_id);

CREATE TABLE orders (
    id BIGSERIAL PRIMARY KEY,
    buyer_id BIGINT NOT NULL REFERENCES users(id),
    total_cents BIGINT NOT NULL CHECK (total_cents >= 0),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'paid_test', 'cancelled')),
    idempotency_key TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    paid_at TIMESTAMPTZ,
    UNIQUE (buyer_id, idempotency_key)
);

CREATE TABLE order_items (
    id BIGSERIAL PRIMARY KEY,
    order_id BIGINT NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    product_id INTEGER NOT NULL REFERENCES catalog(id),
    recipient_id BIGINT NOT NULL REFERENCES users(id),
    price_cents BIGINT NOT NULL CHECK (price_cents >= 0),
    gift_id BIGINT UNIQUE REFERENCES gifts(id)
);
CREATE INDEX order_items_order_id_idx ON order_items(order_id);

-- 6. wishlist ----------------------------------------------------------------

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

-- 7. gifting -----------------------------------------------------------------

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

-- 8. demo_logistics ----------------------------------------------------------

-- Stable stage codes make progression deterministic and idempotent.
ALTER TABLE shipment_events ADD COLUMN stage TEXT;
ALTER TABLE shipment_events ADD CONSTRAINT shipment_event_stage_check
    CHECK (stage IS NULL OR stage IN ('collected', 'transit', 'out_for_delivery', 'delivered'));

UPDATE shipment_events SET stage = 'collected'
WHERE gift_id = 900001 AND description = '演示包裹已揽收';
UPDATE shipment_events SET stage = 'delivered'
WHERE gift_id = 900001 AND description = '演示包裹已签收';
CREATE UNIQUE INDEX shipment_events_gift_stage_idx ON shipment_events(gift_id, stage);

-- A second fictional gift is in transit. The first fixture is delivered but
-- deliberately not recipient-confirmed. Migrations run once and never reset user data.
INSERT INTO gifts (id, sender_id, recipient_id, product_id, state, price_cents,
                   identity_known, revealed_at, settled_at)
SELECT 900003, s.id, r.id, 12, 'accepted', c.price_cents, true,
       now() - INTERVAL '3 days', now() - INTERVAL '3 days'
FROM users s CROSS JOIN users r CROSS JOIN catalog c
WHERE s.identifier = 'linzhou@liyu.test'
  AND r.identifier = 'demo@liyu.test' AND c.id = 12
ON CONFLICT (id) DO NOTHING;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM gifts g JOIN users u ON u.id = g.recipient_id
        WHERE g.id = 900003 AND g.state = 'accepted'
          AND u.identifier = 'demo@liyu.test' AND g.product_id = 12
    ) THEN
        RAISE EXCEPTION 'demo shipment fixture 900003 has unexpected gift state or recipient';
    END IF;
END $$;

INSERT INTO shipments (gift_id, carrier, tracking_number, recipient_name,
                       recipient_phone, recipient_address)
SELECT 900003, '礼遇演示快递', 'LY-DEMO-20260926-0003', '阿岚',
       '000-0000-0000', '演示市虚构区样例路 20 号'
WHERE EXISTS (SELECT 1 FROM gifts WHERE id = 900003)
ON CONFLICT (gift_id) DO NOTHING;

INSERT INTO shipment_events (gift_id, event_at, stage, description)
SELECT 900003, now() - INTERVAL '2 days', 'collected', '演示包裹已揽收'
WHERE EXISTS (SELECT 1 FROM shipments WHERE gift_id = 900003)
ON CONFLICT (gift_id, stage) DO NOTHING;
INSERT INTO shipment_events (gift_id, event_at, stage, description)
SELECT 900003, now() - INTERVAL '1 day', 'transit', '演示包裹运输中'
WHERE EXISTS (SELECT 1 FROM shipments WHERE gift_id = 900003)
ON CONFLICT (gift_id, stage) DO NOTHING;

SELECT setval(pg_get_serial_sequence('gifts', 'id'),
              GREATEST((SELECT MAX(id) FROM gifts), 1));

-- 9. wish_claim --------------------------------------------------------------

-- A cart/order may target a wishlist item. Claims are assigned only after test payment;
-- the wishlist row is locked and checked again inside that payment transaction.
ALTER TABLE cart_items ADD COLUMN wish_item_id BIGINT REFERENCES wishlist_items(id);
ALTER TABLE order_items ADD COLUMN wish_item_id BIGINT REFERENCES wishlist_items(id);
CREATE INDEX cart_items_wish_item_idx ON cart_items(wish_item_id) WHERE wish_item_id IS NOT NULL;
CREATE INDEX order_items_wish_item_idx ON order_items(wish_item_id) WHERE wish_item_id IS NOT NULL;

-- 10. session_expiry ---------------------------------------------------------

ALTER TABLE sessions ADD COLUMN expires_at TIMESTAMPTZ NOT NULL DEFAULT (now() + INTERVAL '30 days');
CREATE INDEX sessions_expires_at_idx ON sessions(expires_at);

-- 11. avatars ----------------------------------------------------------------

CREATE TABLE avatars (
    id UUID PRIMARY KEY,
    owner_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    content_type TEXT NOT NULL,
    byte_len INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT avatar_content_type_check
        CHECK (content_type IN ('image/jpeg', 'image/png', 'image/webp')),
    CONSTRAINT avatar_byte_len_check CHECK (byte_len > 0 AND byte_len <= 1048576)
);
CREATE INDEX avatars_owner_id_idx ON avatars(owner_id);

-- 12. web administration (intentionally independent of users/sessions) -------
CREATE TABLE administrators (
    id BIGSERIAL PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE admin_sessions (
    token_hash TEXT PRIMARY KEY,
    administrator_id BIGINT NOT NULL REFERENCES administrators(id) ON DELETE CASCADE,
    csrf_token TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT (now() + INTERVAL '8 hours')
);
CREATE INDEX admin_sessions_expiry_idx ON admin_sessions(expires_at);
CREATE INDEX admin_sessions_administrator_idx ON admin_sessions(administrator_id);
-- Existing stable fixture IDs remain unchanged; new products receive IDs > 32.
CREATE SEQUENCE catalog_id_seq OWNED BY catalog.id;
SELECT setval('catalog_id_seq', GREATEST((SELECT MAX(id) FROM catalog), 32), true);
ALTER TABLE catalog ALTER COLUMN id SET DEFAULT nextval('catalog_id_seq');

-- 13. management domains -----------------------------------------------------
ALTER TABLE users ADD COLUMN is_active BOOLEAN NOT NULL DEFAULT true;
CREATE TABLE admin_audit (
 id BIGSERIAL PRIMARY KEY, administrator_id BIGINT REFERENCES administrators(id),
 action TEXT NOT NULL, entity TEXT NOT NULL, entity_id BIGINT NOT NULL,
 reason TEXT NOT NULL DEFAULT '', before_data JSONB, after_data JSONB,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE catalog_history (
 id BIGSERIAL PRIMARY KEY, product_id INTEGER NOT NULL REFERENCES catalog(id),
 old_price_cents BIGINT NOT NULL, new_price_cents BIGINT NOT NULL,
 old_stock INTEGER NOT NULL, new_stock INTEGER NOT NULL,
 old_active BOOLEAN NOT NULL, new_active BOOLEAN NOT NULL,
 administrator_id BIGINT REFERENCES administrators(id), reason TEXT NOT NULL DEFAULT 'business',
 created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE FUNCTION record_catalog_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF (OLD.price_cents,OLD.stock,OLD.is_active) IS DISTINCT FROM (NEW.price_cents,NEW.stock,NEW.is_active) THEN
 INSERT INTO catalog_history(product_id,old_price_cents,new_price_cents,old_stock,new_stock,old_active,new_active,administrator_id,reason)
 VALUES(NEW.id,OLD.price_cents,NEW.price_cents,OLD.stock,NEW.stock,OLD.is_active,NEW.is_active,
 NULLIF(current_setting('liyu.administrator',true),'')::bigint,COALESCE(NULLIF(current_setting('liyu.reason',true),''),'business'));
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER catalog_change AFTER UPDATE ON catalog FOR EACH ROW EXECUTE FUNCTION record_catalog_change();
CREATE TABLE coupon_templates (
 id BIGSERIAL PRIMARY KEY, name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
 kind TEXT NOT NULL CHECK(kind IN ('promotion','new_user','compensation')),
 discount_kind TEXT NOT NULL CHECK(discount_kind IN ('fixed','percentage')),
 value BIGINT NOT NULL CHECK(value>0), min_spend_cents BIGINT NOT NULL DEFAULT 0 CHECK(min_spend_cents>=0),
 max_discount_cents BIGINT NOT NULL DEFAULT 0 CHECK(max_discount_cents>=0),
 product_id INTEGER REFERENCES catalog(id), category TEXT NOT NULL DEFAULT '',
 starts_at TIMESTAMPTZ NOT NULL, expires_at TIMESTAMPTZ NOT NULL,
 issue_limit INTEGER NOT NULL CHECK(issue_limit>0), per_user_limit INTEGER NOT NULL DEFAULT 1 CHECK(per_user_limit>0),
 is_active BOOLEAN NOT NULL DEFAULT true, CHECK(expires_at>starts_at),
 CHECK(discount_kind<>'percentage' OR value<=10000), CHECK(value<=1000000000),
 CHECK(category IN ('','coffee','movie','trendy','blind','sweet','digital','home','baby'))
);
CREATE TABLE user_coupons (
 id BIGSERIAL PRIMARY KEY, template_id BIGINT NOT NULL REFERENCES coupon_templates(id),
 user_id BIGINT NOT NULL REFERENCES users(id), status TEXT NOT NULL DEFAULT 'available' CHECK(status IN ('available','reserved','used','revoked')),
 order_id BIGINT UNIQUE REFERENCES orders(id), issued_at TIMESTAMPTZ NOT NULL DEFAULT now(), used_at TIMESTAMPTZ
);
CREATE INDEX user_coupons_owner_idx ON user_coupons(user_id,id);
ALTER TABLE orders ADD COLUMN subtotal_cents BIGINT NOT NULL DEFAULT 0,
 ADD COLUMN discount_cents BIGINT NOT NULL DEFAULT 0 CHECK(discount_cents>=0),
 ADD COLUMN coupon_id BIGINT REFERENCES user_coupons(id);
UPDATE orders SET subtotal_cents=total_cents;
CREATE TABLE recycle_policies (
 product_id INTEGER PRIMARY KEY REFERENCES catalog(id), is_active BOOLEAN NOT NULL DEFAULT true,
 mode TEXT NOT NULL CHECK(mode IN ('fixed','percentage')), value BIGINT NOT NULL CHECK(value>=0 AND value<=1000000000),
 expires_at TIMESTAMPTZ, CHECK(mode<>'percentage' OR value<=10000)
);
CREATE TABLE wallet_ledger (
 id BIGSERIAL PRIMARY KEY,user_id BIGINT NOT NULL REFERENCES users(id),
 amount_cents BIGINT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('cash_out','exchange','withdraw','expired')),
 gift_id BIGINT NOT NULL REFERENCES gifts(id), created_at TIMESTAMPTZ NOT NULL DEFAULT now(), UNIQUE(gift_id,kind)
);
CREATE INDEX wallet_ledger_owner_idx ON wallet_ledger(user_id,id);
CREATE TABLE gift_contracts (
 gift_id BIGINT PRIMARY KEY REFERENCES gifts(id), status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','fulfilled','waived')),
 updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE notifications (
 id BIGSERIAL PRIMARY KEY,user_id BIGINT NOT NULL REFERENCES users(id),
 title TEXT NOT NULL CHECK(char_length(title) BETWEEN 1 AND 100),body TEXT NOT NULL CHECK(char_length(body)<=2000),
 expires_at TIMESTAMPTZ NOT NULL, revoked_at TIMESTAMPTZ,read_at TIMESTAMPTZ,created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE management_schema_version (version INTEGER PRIMARY KEY);
INSERT INTO management_schema_version VALUES(1);
