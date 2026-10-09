# LIYU 服务端

Rust + Salvo REST API backed by PostgreSQL and Diesel. The implementation plan, current gaps, API inventory and privacy matrix are in [todos/app-api.md](todos/app-api.md). This backend keeps test payment and carrier flows; contact gifting now has verified identities and a configurable SMS/email delivery bridge. See [contact delivery](docs/contact-delivery.md). Bind to loopback by default; fixed test credentials must never be exposed publicly.

LIYU-MINI now signs in through a server-owned browser authorization page and calls business APIs over standard HTTPS, without a custom LIYU host service. See [browser authorization](docs/browser-authorization.md) for protocol, expiry, cancellation and testing.

数据范围、供应商传输、会话撤销及删除边界见[服务端隐私说明](PRIVACY.md)。

## 容器部署与持续集成

完整步骤见[部署指南](docs/deployment.md)：Docker Compose 一键启动 PostgreSQL、服务端和 Caddy，本地内部 HTTPS、线上域名及证书、GHCR 镜像发布、验证服务、升级与备份。镜像包含管理后台与商品图片，无需 LIYU 专用宿主。

实际线上入口为 `https://liyu.taidge.com`，健康检查为 `/health`，管理后台为 `/admin`；2026-10-09 公网健康检查返回 HTTP 200。使用 [compose.deploy.yaml](compose.deploy.yaml) 部署数据库和服务端，不包含 Caddy；已有宿主机 Caddy 反代到 `127.0.0.1:8787`。复制 `deploy/production.env.example`、填写数据库/管理员密码及邮件短信桥后执行：

```sh
docker compose --env-file deploy/production.env -f compose.deploy.yaml pull
docker compose --env-file deploy/production.env -f compose.deploy.yaml up -d --wait
```

默认镜像标签为 `v0.1.2`，已发布并包含固定验证码体验开关。详细配置与核验步骤见部署指南。


## Run

Install Rust, [just](https://github.com/casey/just), Python 3 and PostgreSQL (with `psql` and `pg_ctl` on `PATH`). The local PostgreSQL cluster needs a `root` role, password `root`, and permission to create databases. Then, from this repository:

```sh
cp .env.example .env # first setup only; keep existing local settings
just dev
```

The default `DATABASE_URL` is `postgres://root:root@127.0.0.1:5432/liyu_dev`. `just dev` starts an existing local PostgreSQL cluster if the configured port is closed, creates `liyu_dev` if missing, then starts the server. On macOS it detects the Homebrew cluster matching `pg_ctl`; for another data directory, set `PGDATA` in `.env`. It does not initialize a new cluster. Migrations and deterministic demo seeds run automatically. `LIYU_BIND` defaults to `127.0.0.1:8787`. Both just and the Rust server load `.env`, preserving variables already set in the environment; `.env` is ignored by Git. Direct `cargo run` also loads `.env`, but requires the database to exist already.

To clear all development data, stop the server and run `just reset`, then `just dev`. Reset forcibly disconnects database clients, drops `liyu_dev` and recreates it; the next startup restores the schema and demo seeds. Database management refuses URLs pointing outside the local `liyu_dev` database.

Seed users: `demo@liyu.test`, `linzhou@liyu.test`, `chenxiao@liyu.test`; every password is `123456`. `just dev` explicitly enables `LIYU_TEST_DELIVERY=true` unless overridden. For phone/email registration, first request `/api/v1/auth/challenges`, then submit its `challenge_id` and code; test replies expose `test_code`. Direct server startup defaults to real verification, with random codes and no code in the response. Arbitrary test identifiers are allowed only in explicit test mode. The catalog has 33 stable IDs matching the app. Product images and their provenance are in `test-data/products/`.

For a populated local social graph, run `python3 scripts/seed-social-demo.py` after the server has applied migrations. It defaults to `chris@veco.id`; use `--anchor <existing-account-identifier>` for another account. This one-step script stays under `scripts/` rather than Diesel migrations, accepts only the local `liyu_dev` database, preserves existing accounts and sessions, and is repeatable. It adds 108 Chinese-named test users with unique 11-digit profile phone numbers (some also have email), 2–100 confirmed friends per generated user, address-book contacts, 108 generated-user wishlists plus two for the anchor, 180 historical gifts and orders, and independent completed/pending contract marks for each participant. If the anchor has no unexpired unopened gifts, it adds three fresh ones from confirmed friends, with corresponding paid test orders and a 24-hour expiry. Every **new** test account uses password `123456`; existing account passwords are unchanged. Normal phone login accepts the 11-digit number. Back up the local database before importing if you need to restore its prior state.

Contract presets live in the `contract_templates` table, seeded by the additive migration. Authenticated `GET /api/v1/contract-templates` returns active templates in display order; `GET /api/v1/contracts` returns accepted contracts involving the caller, with only the caller's own `pending` or `fulfilled` status. `PUT /api/v1/contracts/{id}/status` saves `{ "status": "pending" | "fulfilled" }` for the authenticated participant alone. Each participant can mark or undo fulfillment independently. Gift details expose this same personal value as `contract_status` (null before acceptance or without a contract). The additive `20260930020000_contract_personal_marks` migration stores marks by `(gift_id, user_id)`; absent marks default to pending. Legacy shared `gift_contracts` rows remain historical and are not copied into personal decisions.

New gifts and wishlists default to a 24-hour validity period. A single or batch gift quote/order or wishlist create request may set `expires_hours` to an integer from 1 through 720 (30 days); the order retains the choice until payment creates its gifts. Wishlist edits may omit the field to keep the existing deadline or provide it to start a new period. Gift and wishlist views return `expires_at` as Unix seconds. An unopened/unrevealed gift expires at the deadline and is refunded on the next gift inbox, outbox, detail, or open request. Expired wishlists remain visible to their owner but cannot be edited or claimed.

Every newly paid gift starts sealed with `ready: false` until its sender calls `PUT /api/v1/gifts/{id}/puzzle` with `unlock_kind: "free"`, `"guess_who"`, or `"question"` (legacy `"passphrase"` remains accepted). The recipient can see the product and gift price before opening, but not the sender, answer, message, or agreement. Only the recipient's explicit `POST .../open` can begin opening. `free` reveals the sender on that action; puzzle modes move to `opened` and reveal the sender only after a correct `POST .../answer`. Wrong answers never reveal the sender, including after three attempts. `guess_who` checks the sender's current account display name or the recipient's private friend nickname; other puzzle answers remain server-side digests. Existing gifts retain their configured mode during migration.

Friend details accept `birthday` and `wedding_date` as `YYYY-MM-DD`; these dates are private to the owner. The reminder worker creates owner-only `occasion` notifications at 7 days before and on the day each year in Asia/Shanghai time, with event keys that prevent duplicates. `GET /api/v1/notifications` also scans due reminders so they appear immediately when the app opens. This is an in-app reminder; no device push is sent.

```powershell
$base = 'http://127.0.0.1:8787'
$login = Invoke-RestMethod "$base/api/v1/auth/login" -Method Post -ContentType 'application/json' -Body '{"identifier":"demo@liyu.test","password":"123456"}'
$headers = @{Authorization="Bearer $($login.token)"}
$challenge = Invoke-RestMethod "$base/api/v1/auth/challenges" -Method Post -ContentType 'application/json' -Body '{"kind":"email","value":"new@example.test","purpose":"register"}'
$registration = @{ identifier='new@example.test'; password='new-password-2026'; code=$challenge.test_code; challenge_id=$challenge.challenge_id } | ConvertTo-Json
Invoke-RestMethod "$base/api/v1/auth/register" -Method Post -ContentType 'application/json' -Body $registration
```

Session tokens expire 30 days after issuance. `POST /auth/refresh` takes the current Bearer token and returns a new token plus `expires_in_seconds`; the old token is revoked atomically. `POST /auth/logout` takes the Bearer token and returns `204`, immediately revoking it. Expired or revoked tokens receive `401` and cannot refresh. Legacy seeded account passwords remain `123456`; new accounts use salted Argon2 passwords. Registration and binding use an expiring, one-use challenge even in test mode for phone/email identities.

```powershell
$refreshed = Invoke-RestMethod "$base/api/v1/auth/refresh" -Method Post -Headers $headers
$headers = @{Authorization="Bearer $($refreshed.token)"}
Invoke-RestMethod "$base/api/v1/auth/logout" -Method Post -Headers $headers
```

Legacy native LIYU integrations may use `LIYU_API_URL`, `LIYU_IDENTIFIER`, and `LIYU_AUTH_TOKEN`. LIYU-MINI instead calls standard HTTPS directly and obtains its own session through browser authorization. It never falls back to demo data on network failure; local demo is an explicit separate entry. The old whole-state endpoint permanently returns `410`: its JSON would contain private gift answers, addresses and money entries. Online workflows use the domain APIs below.

## Storage and connection pool

`.env` supports `LIYU_DB_POOL_MAX_SIZE=8` (positive integer; maximum connections for this process) and `LIYU_DATA_DIR=test-data` (data root, relative to the server working directory, or an absolute path). Environment variables override `.env`. Invalid configuration fails at startup.

Avatars live in `<data-root>/avatars/`; product media in `<data-root>/products/`. The bundled product fixtures have moved from `assets/products/` to `test-data/products/`. With a custom data root, startup copies missing bundled fixtures there without overwriting existing media. Uploaded product images are atomically written as PNG to `products/admin-<id>/<variant>.png`; `card` also supplies the default image for other variants. Existing avatar IDs and public media URLs remain unchanged. When changing the data root, copy your previous `avatars/` and managed `products/admin-*/` directories before restarting; the server does not silently move or delete external data. Back up the database and media directory together.

## Web administration

Open `/admin` in a browser. This is a web-only administration interface served by the Rust server, with Diesel/PostgreSQL persistence. It supports product search, creation and editing, prices, stock, category, tags, physical/digital type, publishing/unpublishing, and uploading thumbnail/card/detail images (JPEG/PNG/WebP, up to 4 MiB, 16–4096 pixels). Products are unpublished instead of deleted so existing orders and gifts retain their references. Unpublished/out-of-stock products cannot be ordered; payment rechecks availability and atomically deducts stock. Existing multi-item historical orders remain payable under the same stock checks. Historical orders retain their recorded price.

Configure an administrator by setting these values in `.env` before starting:

```dotenv
LIYU_ADMIN_USERNAME=admin
LIYU_ADMIN_PASSWORD=<your-password>
LIYU_ADMIN_COOKIE_SECURE=false
```

No administrator or default admin password is seeded. Admin usernames and passwords have no application length requirements. When both values are empty, configured-admin synchronization is skipped. On each startup, the named administrator is created if absent, or its password is synchronized with the configuration if changed. Other administrators are preserved. Password changes revoke that administrator’s existing sessions; unchanged passwords preserve sessions. Restart after editing `.env`; exported environment variables still take precedence. Clear both values to stop managing the account through configuration. HTTPS deployments should set `LIYU_ADMIN_COOKIE_SECURE=true`.

`administrators` and `admin_sessions` are independent of `users` and `sessions`. Admin passwords use salted Argon2id hashes. Admin sessions expire after 8 hours and use an HttpOnly, SameSite=Strict cookie scoped to `/admin`. User Bearer tokens cannot authorize management operations, and admin sessions cannot authorize user APIs. Mutations require `X-Admin-Request: 1` plus the CSRF token returned by login/`me`; supplied Origin headers must match the server host. The web page handles these headers automatically.

Admin endpoints are `/admin/api/login`, `/logout`, `/me`, `/products` (GET/POST), `/products/{id}` (PUT), and `/products/{id}/images/{variant}` (POST raw image bytes), all under `/admin/api`. Products use integer `price_cents` in the API; the page displays yuan. Product list responses include `items` and `next_cursor` (100 per page).

**Existing databases:** startup automatically installs the admin tables and product ID sequence when all three are absent, even if the consolidated migration is already marked `20260926000000`. It applies only section 12 from the existing init SQL in one locked transaction, preserving users, products, orders and the migration ledger. Repeated startups preserve other administrator accounts and the product sequence; the configured account follows `.env` as described above. Partially installed admin schemas are reported for inspection. `just upgrade-admin` remains available for an explicit offline upgrade. Fresh databases receive everything through the normal migration.

## Upgrading a previously migrated database

Profiles without an uploaded avatar return `/api/v1/media/default-avatars/{user_id}` in `avatar_url`. This endpoint serves a deterministic 140×140 PNG identicon derived from the user ID, so it stays the same across logins and server restarts without storing an extra file. Uploaded avatars take precedence; deleting an upload restores the same default avatar on the next profile read.

The consolidated baseline remains `migrations/20260926000000_init/`. The additive `20260928000000_contact_delivery` and `20260929000000_contract_templates` migrations upgrade an existing consolidated database without rebuilding it. The older pre-consolidation boundary described below still requires explicit reconciliation. A database that was built by the old incremental migration chain (profile, catalog, fulfillment, commerce, wishlist, gifting, demo_logistics, wish_claim, session_expiry — with or without the short-lived `20260926100000_avatars`) has those version strings recorded in `__diesel_schema_migrations`, which the consolidated migration does not match. Pointing the new binary at such a database either replays the whole init script (`relation "users" already exists`) or skips it and silently misses the `avatars` table.

For this local test backend the safe options are:

1. **Rebuild (recommended).** This server stores only test data; drop and recreate the database and let migrations run on startup. This deletes all stored users, orders and administrator accounts; back up anything you want to keep first.
2. **Hand-mark an existing database**, only if you must keep its rows:
   - Take a backup first (`pg_dump`), and stop the server so no writes interleave.
   - Verify the live schema really matches the old chain's final state; if it drifted, stop and rebuild instead.
   - Create the avatars table if the old `20260926100000_avatars` migration never ran (see `migrations/20260926000000_init/up.sql`, section 11).
   - Apply section 12 (admin tables and catalog ID sequence) with `just upgrade-admin` if absent.
   - Replace the version ledger so the consolidated migration is considered applied:
     ```sql
     TRUNCATE __diesel_schema_migrations;
     INSERT INTO __diesel_schema_migrations (version, run_on)
     VALUES ('20260926000000', now());
     ```
   - Restart and confirm the additive contact-delivery migration applies successfully, then is skipped on subsequent starts.

Do not edit `up.sql` to "make it fit" an old database; the baseline migration must stay an exact description of its original schema.

## Current REST surface

All paths below are under `/api/v1`, except `/health`. Authenticated endpoints need `Authorization: Bearer <token>`.

| Area | Endpoints |
| --- | --- |
| Auth | `POST /auth/register`, `POST /auth/login`, `POST /auth/refresh`, `POST /auth/logout`, `GET /me` |
| Profile | `GET/PATCH /me/profile`, `PUT /me/phone`, `PUT /me/email`, `GET/POST /me/addresses`, `PUT/DELETE /me/addresses/{id}`, `POST/DELETE /me/avatar`, `GET /media/avatars/{id}` |
| Catalog | `GET /catalog/categories`, `GET /catalog?category=&q=&cursor=&limit=`, `GET /catalog/{id}`, `GET /media/products/{id}/{variant}` (`thumb`, `card`, `detail`) |
| Friends and wishlists | `GET /friends`, `POST /friends/requests`, `POST /friends/requests/{id}/accept`, `GET/POST /wishlists`, `GET/PATCH/DELETE /wishlists/{id}`, `POST /wishlists/{id}/close`, `POST/DELETE /wishlists/{id}/items`, `GET /friends/{id}/wishlists` |
| Direct gift orders | `POST /orders/quote`, `GET/POST /orders`, `GET /orders/{id}`, `POST /orders/{id}/pay-test` |
| Gifts | `GET /gifts/inbox`, `GET /gifts/outbox`, `GET /gifts/{id}`, `PUT /gifts/{id}/puzzle`, `POST /gifts/{id}/open`, `/answer`, `/accept`, `/withdraw` |
| Shipment privacy | `GET /shipments/{gift_id}` and `POST /shipments/{gift_id}/confirm-receipt` for the recipient; `GET /gifts/{gift_id}/delivery-summary` for the sender |

For local logistics demonstrations only, set `LIYU_DEMO_MODE=true` and a random `LIYU_DEMO_ADMIN_KEY` of at least 32 characters. From a loopback client, `POST /api/v1/demo/shipments/{gift_id}/advance` with header `X-LiYu-Demo-Key` and JSON `{"stage":"out_for_delivery"}` advances one step (`collected` → `transit` → `out_for_delivery` → `delivered`). The default mode disables this route. Seed gift `900003` is in transit; `900001` is carrier-delivered but awaits the recipient's confirmation.

The profile's `avatar_url` is set by binary upload: `POST /me/avatar` takes a raw JPEG/PNG/WebP body (max 1 MiB, 16–4096 px per axis; magic bytes are sniffed, oversized dimensions are rejected before decode, and the image must fully decode), stores it under an unguessable UUID in `test-data/avatars/`, and points `avatar_url` at `GET /media/avatars/{id}` (public, immutable-cached). Re-uploading atomically replaces the previous avatar; `DELETE /me/avatar` removes it and clears the pointer. External avatar URLs are no longer accepted. `GET /friends` returns confirmed friends plus this account's private nickname, phone, email, relationship, birthday, and note fields. `PUT /friends/{id}/details` updates those private fields only for a confirmed friend; it never edits the friend's account identity or verified contact methods. Sent gift summaries include the recipient ID for filtering; recipient gift summaries expose a sender only after the gift reveals that identity. A sender's `GET /gifts/{id}` detail additionally returns their own `contract_text`, while the list omits it and sealed recipient views keep it hidden. `POST /orders/quote` and `POST /orders` accept the legacy single-gift body with one `product_id` plus either `recipient_id` or `recipient: {kind,value,label}`. They also accept `{ "items": [{ "product_id": 0, "recipient_id": 12 }, { "product_id": 0, "recipient_id": 13 }] }` for 1–100 distinct recipients in one order. Each item may use the legacy contact or wish fields. `POST /orders` requires an `Idempotency-Key` header. `pay-test` is a server-side simulated payment: it creates canonical gifts and atomically claims linked wishes but does not charge a real provider. The sender's delivery response contains only carrier-delivered and recipient-confirmed booleans; the recipient alone can see tracking, address and event history. Gifts have role-specific projections and server-side puzzle attempts. Recipient exchange/cash-out, wallet accounting, and complete online wishlist/order history integration remain tracked in [todos/app-api.md](todos/app-api.md).

Run `./tests/e2e.ps1` against a loopback server to verify the 33-product catalog, three-account privacy boundaries, idempotent payment, puzzle, and gift acceptance. This script creates test orders and gifts in its target database.

Run the administrator integration checks against a disposable server whose first administrator matches `LIYU_TEST_ADMIN_USERNAME` and `LIYU_TEST_ADMIN_PASSWORD`: `python3 tests/admin_e2e.py http://127.0.0.1:8787`. It creates products and test orders; run the existing user E2E on a separate fresh database because that suite expects exactly 33 fixture products.

## Expanded operations console (Dioxus)

The detailed consumer-to-management feature table and acceptance checklist are in `todos/admin-management.md`. `/admin` now uses Rust/Dioxus 0.7.10 Web (`admin-ui/`), served by Salvo at the same origin. Run `just build-admin` before `just dev` and after frontend edits. Rust API changes need a server restart. Generated `web/dist` and `admin-ui/target` are ignored; a missing build returns a 503 build instruction. Building requires Rust's `wasm32-unknown-unknown` target and `dx`. `--debug-symbols false` avoids the installed Binaryen's DWARF incompatibility.

The console includes overview, users, products, stock, prices/history, coupon rules/issuance, recovery prices, wallet, orders, gifts, shipments, friends, wishlists, contracts, notifications and audit. Money uses integer cents. Percentages use basis points: coupon 1000 = 10% off; recovery 9200 = 92% of actual paid value. Management date fields use UTC. Product ID 0 remains valid.

Management writes use the independent admin Cookie plus `X-Admin-Request: 1` and `X-CSRF-Token`. `GET /admin/api/overview` and `/admin/api/reports/{module}?q=&cursor=` provide bounded ascending-ID pages. `POST /admin/api/manage/{action}/{id}` requires a reason. Domain writes and product saves record actor and before/after data in the same transaction. Stock, price and listing changes also generate history, including checkout/refund stock movements. Disabling a user removes sessions and prevents login; separate session revocation is available. Administrators currently have full console permissions; role subdivision is not implemented.

Fresh databases still use the single init SQL pair. Startup atomically installs only section 13 on an old DB under an advisory lock, without replaying seeds or rewriting the migration ledger. Repeated startup preserves data and sequences. Back up data before upgrading; partial/unknown extension schema fails for inspection. No database is automatically reset.

Coupon templates retain immutable monetary terms; create a replacement to change rules, and enable/disable old templates. Types: promotion, new-user, compensation. Discount modes: fixed cents, percentage. Fields: threshold, cap, product/category scope, validity, total quota, per-user quota. Issuance is serialized. `GET /api/v1/me/coupons` returns the user's issued coupons. Send `X-Coupon-Id` on `POST /api/v1/orders/quote` and `/api/v1/orders` to quote/reserve a coupon across the entire order (including a multi-recipient order). Test payment verifies validity and consumes it. Cancelling a pending order releases it; paid orders cannot use that cancellation action. Discounts are allocated to gift values so recovery cannot credit undiscounted face value.

Recovery is opt-in per product. `GET /api/v1/gifts/{id}/recovery-quote` and `POST .../cash-out` require the recipient's revealed gift and use active server policy. Fixed recovery prices are capped at paid value. `POST .../exchange` with `{ "product_id": 12 }` buys an active replacement with recovery credit; a shortfall requires wallet balance and excess is credited. Gift settlement and immutable ledger share a transaction; duplicate settlement never credits twice. `GET /api/v1/wallet` returns balance/recent ledger. Withdrawal and discovered expiry refund the sender and restore stock once. Expiry remains discovered by the existing open/answer paths; a background expiry scheduler is not implemented.

Shipment management requires an accepted/exchanged physical gift, appends a trace and can mark carrier delivery. It never writes recipient confirmation; delivered shipments cannot be edited. Contract reports show separate `sender_status` and `recipient_status` for accepted gifts with contract text. Administrators cannot set a shared contract state; only participants can save their own marks. Wishlists can be closed and friend relations removed. Targeted notifications support expiry, revocation, owner-only listing and read status via `/api/v1/notifications` and `POST /api/v1/notifications/{id}/read`.

These are test-commerce operations, without real payment or withdrawal. Some consumer Makepad pages still use local demonstrations; new APIs do not import local balances or contacts. This Dioxus migration applies to the operations UI.

## Wishlist drafts and publication

`POST /api/v1/wishlists/drafts` creates an owner-only draft with `{ "title": "生日心愿", "note": "optional", "items": [{ "product_id": 0 }] }`. Items can be omitted for an empty draft. `GET /wishlists/mine` and details include `status` (`draft`, `published`, `closed`, `expired`) and `item_count`. Drafts never appear in friend lists/details or qualify for wish-linked orders.

Drafts support up to eight items through `POST /wishlists/{id}/items` and `DELETE /wishlists/{id}/items/{item_id}`. Adding the same exact product again returns the existing item and `already_present: true`; the last draft item may be removed. Parent row locks serialize additions, removals and publishing. Published item sets are fixed; existing metadata editing, closing and gift-claim rules remain.

`PUT /wishlists/{id}` saves draft metadata and audience. `POST /wishlists/{id}/publish` accepts the same metadata body (`title`, `note`, `occasion`, `event_on`, `audience_user_ids`, `expires_hours`), validates a nonempty item set and current catalog/audience, and starts the expiry countdown at publication. An omitted audience preserves saved selections; an explicit empty array means all confirmed friends. Publication retries do not extend expiry or create another list.

The additive `20260930030000_wishlist_drafts` migration marks existing lists as already published. Legacy `POST /wishlists` still creates a published list for older clients. Rollback refuses while private drafts remain.

## 产品名词

中文用户文案使用：商品（目录及心愿单内容）、礼盒（用户送出的对象）、礼物（收礼人成功拆开礼盒后收到的商品）。未指定具体商品的种类及预算条目标为「商品需求」。解谜中的礼盒尚未拆开；送礼通知和送出记录均称「礼盒」。商品原名和用户自填标题、寄语不作改写。API 路径与数据库字段继续使用既有标识。

## 购买核验、双方约定日期与日历（v0.1.3 源码，未部署）

客户端创建订单可传 `X-Expected-Total-Cents`，重新计价与确认金额不符返回 409 且不建单。同一个 Idempotency-Key 只允许原请求和原确认金额，改变内容返回冲突。旧版没有请求摘要的订单仍可读取，不能凭旧 key 盲目重试。送礼人能读取自己的拆盒设置、寄语与约定以核验配置，但答案及摘要不返回。

已接受的礼物约定支持 GET/PUT `/api/v1/contracts/{id}/schedule`；PUT 提交 `{proposed_on:"YYYY-MM-DD",expected_revision:N}`，空日期代表提议清除。另一参与者 POST `/schedule/confirm` 提交 `{expected_revision:N}` 后生效；提出者可 POST `/schedule/cancel` 撤回。过期版本、自确认及越权操作拒绝。新提议不覆盖原生效日期，各自兑现状态保持独立。启动时幂等升级原约定迁移，不要求已经执行过该迁移的数据库手动补表。

POST `/calendar-export` 创建五分钟导出页，随机密钥在 URL fragment 中，下载时重新核验原会话、双方关系及已确认日期。下载一次后密钥失效。生成 UTF-8/CRLF iCalendar 全天事件，可手动导入日历；无对方联系方式、无自动邀请、无系统日历读写或自动同步。改期或清除后需用户自行核对已导入事件。

此项目按 [Apache License 2.0](LICENSE) 授权，Cargo 元数据为 Apache-2.0。
