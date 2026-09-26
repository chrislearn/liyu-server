diesel::table! {
    users (id) {
        id -> Int8,
        identifier -> Text,
        display_name -> Text,
        password_hash -> Text,
        state -> Nullable<Jsonb>,
        state_revision -> Int8,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    sessions (token_hash) {
        token_hash -> Text,
        user_id -> Int8,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    catalog (id) {
        id -> Int4,
        name -> Text,
        category -> Text,
        price_cents -> Int8,
        physical -> Bool,
    }
}

diesel::table! {
    user_profiles (user_id) {
        user_id -> Int8,
        phone -> Nullable<Text>,
        email -> Nullable<Text>,
        avatar_url -> Nullable<Text>,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    shipping_addresses (id) {
        id -> Int8,
        user_id -> Int8,
        recipient_name -> Text,
        phone -> Text,
        address -> Text,
        is_default -> Bool,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    avatars (id) {
        id -> Uuid,
        owner_id -> Int8,
        content_type -> Text,
        byte_len -> Int4,
        created_at -> Timestamptz,
    }
}

diesel::joinable!(sessions -> users (user_id));
diesel::joinable!(user_profiles -> users (user_id));
diesel::joinable!(shipping_addresses -> users (user_id));
diesel::joinable!(avatars -> users (owner_id));
diesel::allow_tables_to_appear_in_same_query!(
    users,
    sessions,
    catalog,
    user_profiles,
    shipping_addresses,
    avatars
);
