//! Durable in-app occasion reminders. The event key makes hourly scans idempotent.

use diesel::prelude::*;
use diesel::result::QueryResult;
use diesel::sql_types::{BigInt, Nullable};

use crate::pool;

const INSERT_DUE: &str = r#"
WITH today AS (SELECT (now() AT TIME ZONE 'Asia/Shanghai')::date AS d),
events AS (
    SELECT d.owner_id,d.friend_id,
           left(COALESCE(NULLIF(d.nickname,''),u.display_name),70) AS friend_name,
           e.kind,e.event_on
    FROM friend_details d JOIN users u ON u.id=d.friend_id AND u.is_active
    JOIN friendships f ON f.status='accepted'
      AND f.user_low_id=LEAST(d.owner_id,d.friend_id)
      AND f.user_high_id=GREATEST(d.owner_id,d.friend_id)
    CROSS JOIN LATERAL (VALUES ('birthday',d.birthday),('wedding',d.wedding_date)) e(kind,event_on)
    WHERE ($1::bigint IS NULL OR d.owner_id=$1)
      AND e.event_on ~ '^[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])$'
), occasions AS (
    SELECT e.*, y.yr,
      make_date(y.yr,substring(e.event_on,6,2)::int,
        LEAST(substring(e.event_on,9,2)::int,
          extract(day from date_trunc('month',make_date(y.yr,substring(e.event_on,6,2)::int,1)::timestamp)
            + interval '1 month - 1 day')::int)) AS occasion_on
    FROM events e CROSS JOIN today t
    CROSS JOIN LATERAL generate_series(extract(year from t.d)::int,extract(year from t.d)::int+1) y(yr)
), due AS (
    SELECT o.*, (o.occasion_on-t.d) AS days_before
    FROM occasions o CROSS JOIN today t
    WHERE o.occasion_on IN (t.d,t.d+7)
)
INSERT INTO notifications(user_id,title,body,expires_at,event_key,type)
SELECT owner_id,
  CASE WHEN days_before=0 THEN '今天是' ELSE '7 天后是' END || friend_name ||
    CASE WHEN kind='birthday' THEN '的生日' ELSE '的结婚纪念日' END,
  '可以去看看 TA 的心愿单，提前准备一份心意。',
  now()+interval '30 days',
  'occasion:'||owner_id||':'||friend_id||':'||kind||':'||yr||':'||days_before,
  'occasion'
FROM due
ON CONFLICT(event_key) DO NOTHING
"#;

pub(crate) fn scan(conn: &mut PgConnection) -> QueryResult<usize> {
    scan_for_user(conn, None)
}

pub(crate) fn scan_for_user(conn: &mut PgConnection, user_id: Option<i64>) -> QueryResult<usize> {
    diesel::sql_query(INSERT_DUE)
        .bind::<Nullable<BigInt>, _>(user_id)
        .execute(conn)
}

pub(crate) fn start_worker() {
    tokio::spawn(async {
        loop {
            let db = pool().clone();
            let _ = tokio::task::spawn_blocking(move || {
                if let Ok(mut conn) = db.get() {
                    let _ = scan(&mut conn);
                }
            })
            .await;
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });
}
