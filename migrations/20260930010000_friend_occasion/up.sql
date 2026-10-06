ALTER TABLE friend_details
    ADD COLUMN wedding_date TEXT NOT NULL DEFAULT '' CHECK (char_length(wedding_date) <= 40);
