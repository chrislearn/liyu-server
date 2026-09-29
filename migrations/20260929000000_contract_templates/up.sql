CREATE TABLE contract_templates (
    id BIGSERIAL PRIMARY KEY,
    label TEXT NOT NULL CHECK (char_length(label) BETWEEN 1 AND 20),
    body TEXT NOT NULL CHECK (char_length(body) BETWEEN 1 AND 24),
    sort_order INTEGER NOT NULL UNIQUE,
    is_active BOOLEAN NOT NULL DEFAULT true
);

INSERT INTO contract_templates (id, label, body, sort_order) VALUES
    (1, '回请咖啡', '下周找时间回请我喝一杯咖啡', 10),
    (2, '晒一晒', '收下要发一条朋友圈晒一晒', 20),
    (3, '陪看电影', '周末陪我看一场电影', 30),
    (4, '见面拥抱', '下次见面先给我一个拥抱', 40);

SELECT setval(pg_get_serial_sequence('contract_templates', 'id'), 4);
