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
