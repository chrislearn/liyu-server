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
