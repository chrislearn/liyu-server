-- Existing gifts keep their mode. New paid gifts wait for the sender's choice.
ALTER TABLE gifts ADD COLUMN puzzle_configured BOOLEAN NOT NULL DEFAULT true;
ALTER TABLE gifts ALTER COLUMN puzzle_configured SET DEFAULT false;
