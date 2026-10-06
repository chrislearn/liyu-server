-- Only correct system-generated copy; user titles, notes and product names stay intact.
UPDATE notifications SET title='有礼盒等你拆', body='你收到一个礼盒，打开后按提示完成拆盒。'
WHERE type='gift' AND gift_id IS NOT NULL
  AND title='有礼物等你拆' AND body='一份神秘礼物已经放进你的礼盒。';
UPDATE delivery_outbox SET payload=jsonb_set(payload, '{title}', to_jsonb('有一个礼盒等你领取'::text))
WHERE gift_id IS NOT NULL AND status IN ('pending','failed')
  AND payload->>'type'='gift_invitation' AND payload->>'title'='有一份礼物等你领取';
