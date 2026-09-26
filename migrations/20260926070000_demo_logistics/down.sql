DELETE FROM gifts WHERE id = 900003;
DROP INDEX IF EXISTS shipment_events_gift_stage_idx;
ALTER TABLE shipment_events DROP CONSTRAINT IF EXISTS shipment_event_stage_check;
ALTER TABLE shipment_events DROP COLUMN stage;
