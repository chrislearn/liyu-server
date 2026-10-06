DROP INDEX gifts_one_active_transfer_idx;
ALTER TABLE gifts DROP CONSTRAINT gifts_state_check;
ALTER TABLE gifts ADD CONSTRAINT gifts_state_check CHECK
    (state IN ('sealed','opened','revealed','accepted','exchanged','cashed_out','withdrawn','expired'));
ALTER TABLE gifts DROP COLUMN transfer_parent_id;
