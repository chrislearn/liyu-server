#!/usr/bin/env python3
"""Add repeatable social/gift fixtures to the local liyu_dev database only.

All newly created accounts use the test password 123456. Existing accounts,
passwords and sessions are preserved. Re-running the script does not duplicate
rows. The anchor defaults to chris@veco.id. When it has no live unopened gift,
the script adds three fresh gifts from its friends for testing.
"""

import argparse
import hashlib
import os
import random
import subprocess
from pathlib import Path
from urllib.parse import urlsplit


ROOT = Path(__file__).resolve().parents[1]
SEED = 20260929
N = 108


def literal(value):
    return "'" + str(value).replace("'", "''") + "'"


def values(rows):
    return ",\n".join("(" + ",".join(literal(v) for v in row) + ")" for row in rows)


def database_url():
    raw = os.environ.get("DATABASE_URL")
    if not raw:
        raw = next(
            line.split("=", 1)[1]
            for line in (ROOT / ".env").read_text().splitlines()
            if line.startswith("DATABASE_URL=")
        )
    url = urlsplit(raw)
    if url.scheme not in ("postgres", "postgresql") or url.hostname not in (
        "localhost", "127.0.0.1", "::1"
    ) or url.path != "/liyu_dev" or url.query or url.fragment:
        raise SystemExit("Only a local PostgreSQL /liyu_dev database may be seeded")
    return raw


def psql(url, sql):
    result = subprocess.run(
        ["psql", url, "-X", "-q", "-v", "ON_ERROR_STOP=1", "-A", "-t", "-f", "-"],
        input=sql, text=True, capture_output=True,
        env={**os.environ, "PGCONNECT_TIMEOUT": "5"}, check=False,
    )
    if result.returncode:
        raise SystemExit(result.stderr.strip())
    return result.stdout.strip()


def enrichment_sql():
    """Fill owner-private details and multi-item previews for seeded fixtures only."""
    return [
        "WITH pairs AS (SELECT user_low_id AS owner_id,user_high_id AS friend_id FROM friendships WHERE status='accepted' UNION ALL SELECT user_high_id,user_low_id FROM friendships WHERE status='accepted') "
        "INSERT INTO friend_details(owner_id,friend_id,nickname,phone,email,relationship,birthday,note) "
        "SELECT pairs.owner_id,pairs.friend_id,u.display_name,p.phone,COALESCE(p.email,''),"
        "CASE pairs.friend_id % 4 WHEN 0 THEN '朋友' WHEN 1 THEN '同学' WHEN 2 THEN '同事' ELSE '老友' END,"
        "to_char(date '1990-01-01' + (pairs.friend_id % 10000)::integer,'YYYY-MM-DD'),'一起分享过礼物' "
        "FROM pairs JOIN user_profiles p ON p.user_id=pairs.friend_id JOIN users u ON u.id=pairs.friend_id "
        "WHERE p.phone ~ '^1999000[0-9]{4}$' "
        "ON CONFLICT(owner_id,friend_id) DO UPDATE SET "
        "nickname=COALESCE(NULLIF(friend_details.nickname,''),EXCLUDED.nickname),"
        "phone=COALESCE(NULLIF(friend_details.phone,''),EXCLUDED.phone),"
        "email=COALESCE(NULLIF(friend_details.email,''),EXCLUDED.email),"
        "relationship=COALESCE(NULLIF(friend_details.relationship,''),EXCLUDED.relationship),"
        "birthday=COALESCE(NULLIF(friend_details.birthday,''),EXCLUDED.birthday),"
        "note=COALESCE(NULLIF(friend_details.note,''),EXCLUDED.note);",
        "INSERT INTO wishlist_items(id,wishlist_id,ordinal,product_id,kind,wants) "
        "SELECT 970000+(w.id-940000)*3+n.ordinal,w.id,n.ordinal,"
        "(((w.id-940000)*7+n.ordinal*5)%33)::integer,'','适合分享的小惊喜' "
        "FROM wishlists w CROSS JOIN generate_series(1,3) AS n(ordinal) "
        "WHERE w.id BETWEEN 940001 AND 940108 "
        "AND NOT EXISTS (SELECT 1 FROM wishlist_items existing WHERE existing.wishlist_id=w.id AND existing.ordinal=n.ordinal) "
        "ON CONFLICT DO NOTHING;",
        "SELECT setval(pg_get_serial_sequence('wishlist_items','id'),(SELECT max(id) FROM wishlist_items));",
    ]


def ensure_unopened_sql(anchor):
    """Replenish unopened gifts without duplicating an existing live batch."""
    return f"""
BEGIN;
DO $seed$
DECLARE
    recipient bigint;
    sender bigint;
    product record;
    new_order bigint;
    new_gift bigint;
    gift_count integer := 0;
BEGIN
    SELECT id INTO STRICT recipient FROM users WHERE identifier = {literal(anchor)};
    PERFORM pg_advisory_xact_lock(20260929, recipient::integer);
    IF EXISTS (SELECT 1 FROM gifts WHERE recipient_id = recipient
               AND state = 'sealed' AND expires_at > now()) THEN
        RETURN;
    END IF;
    FOR sender IN
        SELECT CASE WHEN f.user_low_id = recipient THEN f.user_high_id ELSE f.user_low_id END
        FROM friendships f
        WHERE f.status = 'accepted' AND recipient IN (f.user_low_id, f.user_high_id)
        ORDER BY f.id LIMIT 3
    LOOP
        SELECT id, price_cents INTO STRICT product
        FROM catalog WHERE is_active = true AND stock > 0
        ORDER BY id OFFSET gift_count + 5 LIMIT 1;
        INSERT INTO orders(buyer_id,total_cents,subtotal_cents,status,idempotency_key,paid_at)
        VALUES (sender,product.price_cents,product.price_cents,'paid_test',
                'social-unopened-' || recipient || '-' || sender || '-' ||
                to_char(clock_timestamp(),'YYYYMMDDHH24MISSUS'),now())
        RETURNING id INTO new_order;
        INSERT INTO gifts(sender_id,recipient_id,product_id,state,price_cents,
                          unlock_kind,message,available_at,expires_at)
        VALUES (sender,recipient,product.id,'sealed',product.price_cents,'free',
                '送你一份小惊喜，打开看看吧。',now(),now() + interval '24 hours')
        RETURNING id INTO new_gift;
        INSERT INTO order_items(order_id,product_id,recipient_id,price_cents,gift_id)
        VALUES (new_order,product.id,recipient,product.price_cents,new_gift);
        gift_count := gift_count + 1;
    END LOOP;
    IF gift_count < 3 THEN
        RAISE EXCEPTION 'Expected three accepted friends for unopened gift fixtures';
    END IF;
END
$seed$;
COMMIT;
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--anchor", default="chris@veco.id", help="existing user identifier to connect (default: chris@veco.id)")
    parser.add_argument("--enrich-only", action="store_true", help="backfill seeded friend details and wishlist items")
    args = parser.parse_args()
    url = database_url()
    if args.enrich_only:
        psql(url, "\n".join(["BEGIN;", *enrichment_sql(), "COMMIT;"]))
        print(psql(url, "SELECT 'friend details='||count(*) FROM friend_details WHERE phone ~ '^1999000[0-9]{4}$'"))
        return
    anchor = args.anchor.strip().lower()
    if psql(url, f"SELECT count(*) FROM users WHERE identifier={literal(anchor)}") != "1":
        raise SystemExit("anchor account not found")
    existing = psql(url, "SELECT identifier FROM users WHERE identifier NOT LIKE 'liyu-seed-%' AND identifier NOT LIKE '1999000%' AND identifier NOT LIKE '+861999000%' ORDER BY id").splitlines()
    rng = random.Random(SEED)
    surnames = "林陈王李张赵周吴郑何许孙".split()
    if len(surnames) == 1:
        surnames = list(surnames[0])
    given = ["小满", "晓宁", "子涵", "思远", "雨桐", "嘉禾", "知夏", "以安", "星河"]
    people = []
    for i in range(N):
        name = surnames[i // len(given)] + given[i % len(given)]
        phone = f"1999000{i + 1:04d}"
        email = f"liyu-seed-{i + 1:03d}@example.test" if i % 3 else ""
        identifier = email or "+86" + phone
        people.append((identifier, name, phone, email))
    ids = [person[0] for person in people]

    edges = set()
    degree = {identifier: 0 for identifier in ids + existing}

    def edge(a, b):
        if a == b or degree[a] >= 100 or degree[b] >= 100:
            return
        pair = tuple(sorted((a, b)))
        if pair not in edges:
            edges.add(pair)
            degree[a] += 1
            degree[b] += 1

    # A ring guarantees that every generated person has at least two friends.
    for i in range(N):
        edge(ids[i], ids[(i + 1) % N])
    for i in range(N):
        for j in range(i + 2, N):
            if rng.random() < 0.08 + 0.30 * (i / N) * (j / N):
                edge(ids[i], ids[j])
    for hub, target in ((0, 100), (35, 78), (70, 55)):
        for peer in rng.sample(ids, len(ids)):
            if degree[ids[hub]] >= target:
                break
            edge(ids[hub], peer)
    for account in existing:
        target = 36 if account == anchor else 4
        for peer in rng.sample(ids, len(ids)):
            if degree[account] >= target:
                break
            edge(account, peer)
    if not all(2 <= degree[identifier] <= 100 for identifier in ids):
        raise SystemExit("generated friendship degree out of range")

    # Every account gets a small address book, using a subset of its confirmed
    # friends. It is separate from the friendship graph and has real phone data.
    neighbors = {identifier: [] for identifier in degree}
    for a, b in sorted(edges):
        neighbors[a].append(b)
        neighbors[b].append(a)
    contacts = []
    code = 0
    for owner in [anchor] + ids:
        candidates = [p for p in neighbors[owner] if p in ids]
        for peer in candidates[:(16 if owner == anchor else 3)]:
            code += 1
            contacts.append((960000 + code, owner, peer))

    gift_pairs = []
    anchor_peers = neighbors[anchor][:20]
    for i, peer in enumerate(anchor_peers):
        gift_pairs.append((anchor, peer) if i % 2 else (peer, anchor))
    social_edges = [pair for pair in sorted(edges) if anchor not in pair]
    for a, b in rng.sample(social_edges, min(160, len(social_edges))):
        gift_pairs.append((a, b) if rng.randrange(2) else (b, a))
    pact_texts = [
        "下周找时间回请我喝一杯咖啡", "收下要发一条朋友圈晒一晒",
        "周末陪我看一场电影", "下次见面先给我一个拥抱",
    ]
    gifts = []
    for i, (sender, recipient) in enumerate(gift_pairs, 1):
        state = ["accepted", "accepted", "accepted", "revealed", "opened", "sealed"][i % 6]
        contract = pact_texts[i % 4] if state == "accepted" and i % 4 != 0 else ""
        pact_status = ["fulfilled", "fulfilled", "pending", "waived"][i % 4] if contract else ""
        age_days = (i * 7) % 170 + 5 if state == "accepted" else i % 4
        gifts.append((920000 + i, sender, recipient, i % 33, state, contract,
                      pact_status, age_days, f"{recipient}，这份心意送给你。"))

    # Staging tables make identifiers resolve at execution time. They also keep
    # this script independent of sequence values and existing database rows.
    sql = ["BEGIN;",
           "UPDATE users SET identifier='+86'||identifier WHERE identifier ~ '^1999000[0-9]{4}$';",
           "UPDATE sender_contact_methods SET value='+86'||value WHERE id BETWEEN 1060001 AND 1060999 AND value ~ '^1999000[0-9]{4}$';",
           "CREATE TEMP TABLE seed_people(identifier text,name text,phone text,email text) ON COMMIT DROP;",
           "INSERT INTO seed_people VALUES\n" + values(people) + ";",
           "CREATE TEMP TABLE seed_edges(a text,b text) ON COMMIT DROP;",
           "INSERT INTO seed_edges VALUES\n" + values(sorted(edges)) + ";",
           "CREATE TEMP TABLE seed_contacts(id bigint,owner text,peer text) ON COMMIT DROP;",
           "INSERT INTO seed_contacts VALUES\n" + values(contacts) + ";",
           "CREATE TEMP TABLE seed_gifts(id bigint,sender text,recipient text,product_id integer,state text,contract_text text,pact_status text,age_days integer,message text) ON COMMIT DROP;",
           "INSERT INTO seed_gifts VALUES\n" + values(gifts) + ";"]
    password_hash = hashlib.sha256(b"123456").hexdigest()
    sql += [
        f"INSERT INTO users(identifier,display_name,password_hash) SELECT identifier,name,{literal(password_hash)} FROM seed_people ON CONFLICT(identifier) DO NOTHING;",
        "INSERT INTO user_profiles(user_id,phone,email) SELECT u.id,p.phone,NULLIF(p.email,'') FROM seed_people p JOIN users u USING(identifier) ON CONFLICT(user_id) DO NOTHING;",
        "INSERT INTO contact_identities(user_id,kind,value) SELECT u.id,'phone','+86'||p.phone FROM seed_people p JOIN users u USING(identifier) ON CONFLICT(kind,value) DO NOTHING;",
        "INSERT INTO contact_identities(user_id,kind,value) SELECT u.id,'email',p.email FROM seed_people p JOIN users u USING(identifier) WHERE p.email<>'' ON CONFLICT(kind,value) DO NOTHING;",
        "INSERT INTO friendships(user_low_id,user_high_id,requested_by_user_id,status,accepted_at) SELECT LEAST(a.id,b.id),GREATEST(a.id,b.id),a.id,'accepted',now()-interval '90 days' FROM seed_edges e JOIN users a ON a.identifier=e.a JOIN users b ON b.identifier=e.b ON CONFLICT(user_low_id,user_high_id) DO NOTHING;",
        "INSERT INTO sender_contacts(id,owner_id,label,bound_user_id) SELECT c.id,o.id,p.display_name,p.id FROM seed_contacts c JOIN users o ON o.identifier=c.owner JOIN users p ON p.identifier=c.peer ON CONFLICT(id) DO NOTHING;",
        "INSERT INTO sender_contact_methods(id,contact_id,owner_id,kind,value) SELECT c.id+100000,c.id,o.id,'phone','+86'||p.phone FROM seed_contacts c JOIN users o ON o.identifier=c.owner JOIN users b ON b.identifier=c.peer JOIN user_profiles p ON p.user_id=b.id ON CONFLICT(id) DO NOTHING;",
        "INSERT INTO wishlists(id,owner_id,title,note,occasion,event_on,created_at) SELECT 940000+row_number() OVER(ORDER BY p.identifier),u.id,p.name||'的生日心愿','想和熟人一起分享喜欢的东西','birthday',current_date+((row_number() OVER(ORDER BY p.identifier))::integer % 90+10),now()-interval '12 days' FROM seed_people p JOIN users u USING(identifier) ON CONFLICT(id) DO NOTHING;",
        "INSERT INTO wishlist_items(id,wishlist_id,ordinal,product_id,kind,wants) SELECT 950000+(w.id-940000),w.id,0,((w.id-940000)*7 % 33)::integer,'','喜欢实用又有惊喜的礼物' FROM wishlists w WHERE w.id BETWEEN 940001 AND 940108 ON CONFLICT(id) DO NOTHING;",
        f"INSERT INTO wishlists(id,owner_id,title,note,occasion,event_on,created_at) SELECT 940201,id,'今年想收到的礼物','给熟人一点灵感','birthday',current_date+interval '25 days',now()-interval '5 days' FROM users WHERE identifier={literal(anchor)} ON CONFLICT(id) DO NOTHING;",
        f"INSERT INTO wishlists(id,owner_id,title,note,occasion,event_on,created_at) SELECT 940202,id,'周末的小惊喜','不必贵重，心意最重要','other',current_date+interval '45 days',now()-interval '2 days' FROM users WHERE identifier={literal(anchor)} ON CONFLICT(id) DO NOTHING;",
        "INSERT INTO wishlist_items(id,wishlist_id,ordinal,product_id,kind,wants) VALUES (950201,940201,0,7,'','喜欢有设计感的小物件'),(950202,940202,0,12,'','适合周末放松') ON CONFLICT(id) DO NOTHING;",
        *enrichment_sql(),
        "INSERT INTO gifts(id,sender_id,recipient_id,product_id,state,price_cents,unlock_kind,message,contract_text,identity_known,created_at,available_at,expires_at,opened_at,revealed_at,settled_at,voucher_code) SELECT s.id,u.id,r.id,s.product_id,s.state,c.price_cents,'free',s.message,s.contract_text,s.state IN ('revealed','accepted'),now()-(s.age_days||' days')::interval,now()-(s.age_days||' days')::interval,now()+interval '24 hours',CASE WHEN s.state IN ('opened','revealed','accepted') THEN now()-(s.age_days||' days')::interval END,CASE WHEN s.state IN ('revealed','accepted') THEN now()-(s.age_days||' days')::interval END,CASE WHEN s.state='accepted' THEN now()-(s.age_days||' days')::interval END,CASE WHEN s.state='accepted' AND NOT c.physical THEN 'LY-TEST-'||s.id END FROM seed_gifts s JOIN users u ON u.identifier=s.sender JOIN users r ON r.identifier=s.recipient JOIN catalog c ON c.id=s.product_id ON CONFLICT(id) DO NOTHING;",
        "INSERT INTO orders(id,buyer_id,total_cents,status,idempotency_key,created_at,paid_at) SELECT s.id+100000,u.id,c.price_cents,'paid_test','social-seed-'||s.id,g.created_at,g.created_at FROM seed_gifts s JOIN gifts g ON g.id=s.id JOIN users u ON u.identifier=s.sender JOIN catalog c ON c.id=s.product_id ON CONFLICT(id) DO NOTHING;",
        "INSERT INTO order_items(id,order_id,product_id,recipient_id,price_cents,gift_id) SELECT s.id+200000,s.id+100000,s.product_id,r.id,c.price_cents,s.id FROM seed_gifts s JOIN users r ON r.identifier=s.recipient JOIN catalog c ON c.id=s.product_id ON CONFLICT(id) DO NOTHING;",
        "INSERT INTO gift_contracts(gift_id,status,updated_at) SELECT s.id,s.pact_status,g.settled_at+interval '2 days' FROM seed_gifts s JOIN gifts g ON g.id=s.id WHERE s.pact_status<>'' ON CONFLICT(gift_id) DO NOTHING;",
        "INSERT INTO shipments(gift_id,carrier,tracking_number,recipient_name,recipient_phone,recipient_address,delivered_at,recipient_confirmed_at) SELECT s.id,'礼遇测试快递','LY-SOCIAL-'||s.id,r.display_name,p.phone,'北京市朝阳区测试路 '||(s.id-920000)||' 号',g.settled_at,g.settled_at FROM seed_gifts s JOIN gifts g ON g.id=s.id JOIN catalog c ON c.id=g.product_id JOIN users r ON r.id=g.recipient_id JOIN user_profiles p ON p.user_id=r.id WHERE s.state='accepted' AND c.physical AND p.phone IS NOT NULL ON CONFLICT(gift_id) DO NOTHING;",
        "SELECT setval(pg_get_serial_sequence('users','id'),(SELECT max(id) FROM users));",
        "SELECT setval(pg_get_serial_sequence('friendships','id'),(SELECT max(id) FROM friendships));",
        "SELECT setval(pg_get_serial_sequence('sender_contacts','id'),(SELECT max(id) FROM sender_contacts));",
        "SELECT setval(pg_get_serial_sequence('sender_contact_methods','id'),(SELECT max(id) FROM sender_contact_methods));",
        "SELECT setval(pg_get_serial_sequence('wishlists','id'),(SELECT max(id) FROM wishlists));",
        "SELECT setval(pg_get_serial_sequence('wishlist_items','id'),(SELECT max(id) FROM wishlist_items));",
        "SELECT setval(pg_get_serial_sequence('gifts','id'),(SELECT max(id) FROM gifts));",
        "SELECT setval(pg_get_serial_sequence('orders','id'),(SELECT max(id) FROM orders));",
        "SELECT setval(pg_get_serial_sequence('order_items','id'),(SELECT max(id) FROM order_items));",
        "COMMIT;",
        "SELECT 'seeded users='||(SELECT count(*) FROM user_profiles WHERE phone LIKE '1999000%')||', gifts='||(SELECT count(*) FROM gifts WHERE id BETWEEN 920001 AND 920999)||', fulfilled contracts='||(SELECT count(*) FROM gift_contracts WHERE gift_id BETWEEN 920001 AND 920999 AND status='fulfilled');",
    ]
    summary = psql(url, "\n".join(sql)).splitlines()[-1]
    psql(url, ensure_unopened_sql(anchor))
    unopened = psql(url, f"SELECT count(*) FROM gifts WHERE recipient_id=(SELECT id FROM users WHERE identifier={literal(anchor)}) AND state='sealed' AND expires_at>now()")
    print(f"{summary}, unopened gifts for {anchor}={unopened}")


if __name__ == "__main__":
    main()
