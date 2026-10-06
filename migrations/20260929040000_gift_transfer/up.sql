ALTER TABLE gifts ADD COLUMN transfer_parent_id BIGINT REFERENCES gifts(id);
ALTER TABLE gifts DROP CONSTRAINT gifts_state_check;
ALTER TABLE gifts ADD CONSTRAINT gifts_state_check CHECK
    (state IN ('sealed','opened','revealed','accepted','exchanged','cashed_out','transferred','withdrawn','expired'));
CREATE UNIQUE INDEX gifts_one_active_transfer_idx ON gifts(transfer_parent_id)
    WHERE transfer_parent_id IS NOT NULL AND state IN ('sealed','opened','revealed');
