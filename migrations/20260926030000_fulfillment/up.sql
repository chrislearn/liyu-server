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
