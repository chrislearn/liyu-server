# LiYu test server

Rust + Salvo REST API backed by PostgreSQL and Diesel. The implementation plan, current gaps, API inventory and privacy matrix are in [todos/app-api.md](todos/app-api.md). This backend keeps test payment and carrier flows; contact gifting now has verified identities and a configurable SMS/email delivery bridge. See [contact delivery](docs/contact-delivery.md). Bind to loopback by default; fixed test credentials must never be exposed publicly.

## Run

Install Rust, [just](https://github.com/casey/just), Python 3 and PostgreSQL (with `psql` on `PATH`). Start local PostgreSQL with a `root` role, password `root`, and permission to create databases. Then, from this repository:

```sh
cp .env.example .env # first setup only; keep existing local settings
just dev
```

The default `DATABASE_URL` is `postgres://root:root@127.0.0.1:5432/liyu_dev`. `just dev` creates `liyu_dev` if missing, then starts the server. Migrations and deterministic demo seeds run automatically. `LIYU_BIND` defaults to `127.0.0.1:8787`. Both just and the Rust server load `.env`, preserving variables already set in the environment; `.env` is ignored by Git. Direct `cargo run` also loads `.env`, but requires the database to exist already.

To clear all development data, stop the server and run `just reset`, then `just dev`. Reset forcibly disconnects database clients, drops `liyu_dev` and recreates it; the next startup restores the schema and demo seeds. Database management refuses URLs pointing outside the local `liyu_dev` database.

Seed users: `demo@liyu.test`, `linzhou@liyu.test`, `chenxiao@liyu.test`; every password is `123456`. `just dev` explicitly enables `LIYU_TEST_DELIVERY=true` unless overridden. For phone/email registration, first request `/api/v1/auth/challenges`, then submit its `challenge_id` and code; test replies expose `test_code`. Direct server startup defaults to real verification, with random codes and no code in the response. Arbitrary test identifiers are allowed only in explicit test mode. The catalog has 33 stable IDs matching the app. Product images and their provenance are in `test-data/products/`.

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

`LIYU_API_URL` lets the app reach this test server; `LIYU_IDENTIFIER` selects another seeded/registered account, and `LIYU_AUTH_TOKEN` overrides automatic login. The app falls back to local demo data when the server is unavailable. The old whole-state endpoint permanently returns `410`: its JSON would contain private gift answers, addresses and money entries. Online workflows use the domain APIs below.

## Storage and connection pool

`.env` supports `LIYU_DB_POOL_MAX_SIZE=8` (positive integer; maximum connections for this process) and `LIYU_DATA_DIR=test-data` (data root, relative to the server working directory, or an absolute path). Environment variables override `.env`. Invalid configuration fails at startup.

Avatars live in `<data-root>/avatars/`; product media in `<data-root>/products/`. The bundled product fixtures have moved from `assets/products/` to `test-data/products/`. With a custom data root, startup copies missing bundled fixtures there without overwriting existing media. Uploaded product images are atomically written as PNG to `products/admin-<id>/<variant>.png`; `card` also supplies the default image for other variants. Existing avatar IDs and public media URLs remain unchanged. When changing the data root, copy your previous `avatars/` and managed `products/admin-*/` directories before restarting; the server does not silently move or delete external data. Back up the database and media directory together.

## Web administration

Open `/admin` in a browser. This is a web-only administration interface served by the Rust server, with Diesel/PostgreSQL persistence. It supports product search, creation and editing, prices, stock, category, tags, physical/digital type, publishing/unpublishing, and uploading thumbnail/card/detail images (JPEG/PNG/WebP, up to 4 MiB, 16–4096 pixels). Products are unpublished instead of deleted so existing orders and gifts retain their references. Unpublished/out-of-stock products cannot be added to the cart or ordered; payment rechecks availability and atomically deducts stock, including multiple lines of the same product. Historical orders retain their recorded price.

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

The consolidated baseline remains `migrations/20260926000000_init/`. The additive `20260928000000_contact_delivery` migration upgrades an existing consolidated database without rebuilding it. The older pre-consolidation boundary described below still requires explicit reconciliation. A database that was built by the old incremental migration chain (profile, catalog, fulfillment, commerce, wishlist, gifting, demo_logistics, wish_claim, session_expiry — with or without the short-lived `20260926100000_avatars`) has those version strings recorded in `__diesel_schema_migrations`, which the consolidated migration does not match. Pointing the new binary at such a database either replays the whole init script (`relation "users" already exists`) or skips it and silently misses the `avatars` table.

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
| Cart and test orders | `GET/DELETE /cart`, `POST /cart/items`, `DELETE /cart/items/{id}`, `POST /orders/quote`, `GET/POST /orders`, `GET /orders/{id}`, `POST /orders/{id}/pay-test` |
| Gifts | `GET /gifts/inbox`, `GET /gifts/outbox`, `GET /gifts/{id}`, `PUT /gifts/{id}/puzzle`, `POST /gifts/{id}/open`, `/answer`, `/accept`, `/withdraw` |
| Shipment privacy | `GET /shipments/{gift_id}` and `POST /shipments/{gift_id}/confirm-receipt` for the recipient; `GET /gifts/{gift_id}/delivery-summary` for the sender |

For local logistics demonstrations only, set `LIYU_DEMO_MODE=true` and a random `LIYU_DEMO_ADMIN_KEY` of at least 32 characters. From a loopback client, `POST /api/v1/demo/shipments/{gift_id}/advance` with header `X-LiYu-Demo-Key` and JSON `{"stage":"out_for_delivery"}` advances one step (`collected` → `transit` → `out_for_delivery` → `delivered`). The default mode disables this route. Seed gift `900003` is in transit; `900001` is carrier-delivered but awaits the recipient's confirmation.

The profile's `avatar_url` is set by binary upload: `POST /me/avatar` takes a raw JPEG/PNG/WebP body (max 1 MiB, 16–4096 px per axis; magic bytes are sniffed, oversized dimensions are rejected before decode, and the image must fully decode), stores it under an unguessable UUID in `test-data/avatars/`, and points `avatar_url` at `GET /media/avatars/{id}` (public, immutable-cached). Re-uploading atomically replaces the previous avatar; `DELETE /me/avatar` removes it and clears the pointer. External avatar URLs are no longer accepted. The cart has a product, confirmed-friend recipient, and optional wishlist item per line; `POST /orders` requires an `Idempotency-Key` header. `pay-test` is a server-side simulated payment: it creates canonical gifts and atomically claims linked wishes but does not charge a real provider. The sender's delivery response contains only carrier-delivered and recipient-confirmed booleans; the recipient alone can see tracking, address and event history. Gifts have role-specific projections and server-side puzzle attempts. Recipient exchange/cash-out, wallet accounting, and complete online wishlist/order history integration remain tracked in [todos/app-api.md](todos/app-api.md).

Run `./tests/e2e.ps1` against a loopback server to verify the 33-product catalog, three-account privacy boundaries, idempotent payment, puzzle, and gift acceptance. This script creates test orders and gifts in its target database.

Run the administrator integration checks against a disposable server whose first administrator matches `LIYU_TEST_ADMIN_USERNAME` and `LIYU_TEST_ADMIN_PASSWORD`: `python3 tests/admin_e2e.py http://127.0.0.1:8787`. It creates products and test orders; run the existing user E2E on a separate fresh database because that suite expects exactly 33 fixture products.

## Expanded operations console (Dioxus)

The detailed consumer-to-management feature table and acceptance checklist are in `todos/admin-management.md`. `/admin` now uses Rust/Dioxus 0.7.10 Web (`admin-ui/`), served by Salvo at the same origin. Run `just build-admin` before `just dev` and after frontend edits. Rust API changes need a server restart. Generated `web/dist` and `admin-ui/target` are ignored; a missing build returns a 503 build instruction. Building requires Rust's `wasm32-unknown-unknown` target and `dx`. `--debug-symbols false` avoids the installed Binaryen's DWARF incompatibility.

The console includes overview, users, products, stock, prices/history, coupon rules/issuance, recovery prices, wallet, orders, gifts, shipments, friends, wishlists, contracts, notifications and audit. Money uses integer cents. Percentages use basis points: coupon 1000 = 10% off; recovery 9200 = 92% of actual paid value. Management date fields use UTC. Product ID 0 remains valid.

Management writes use the independent admin Cookie plus `X-Admin-Request: 1` and `X-CSRF-Token`. `GET /admin/api/overview` and `/admin/api/reports/{module}?q=&cursor=` provide bounded ascending-ID pages. `POST /admin/api/manage/{action}/{id}` requires a reason. Domain writes and product saves record actor and before/after data in the same transaction. Stock, price and listing changes also generate history, including checkout/refund stock movements. Disabling a user removes sessions and prevents login; separate session revocation is available. Administrators currently have full console permissions; role subdivision is not implemented.

Fresh databases still use the single init SQL pair. Startup atomically installs only section 13 on an old DB under an advisory lock, without replaying seeds or rewriting the migration ledger. Repeated startup preserves data and sequences. Back up data before upgrading; partial/unknown extension schema fails for inspection. No database is automatically reset.

Coupon templates retain immutable monetary terms; create a replacement to change rules, and enable/disable old templates. Types: promotion, new-user, compensation. Discount modes: fixed cents, percentage. Fields: threshold, cap, product/category scope, validity, total quota, per-user quota. Issuance is serialized. `GET /api/v1/me/coupons` returns the user's issued coupons. Send `X-Coupon-Id` on `POST /api/v1/orders/quote` and `/api/v1/orders` to quote/reserve a coupon. Test payment verifies validity and consumes it. Cancelling a pending order releases it; paid orders cannot use that cancellation action. Discounts are allocated to gift values so recovery cannot credit undiscounted face value.

Recovery is opt-in per product. `GET /api/v1/gifts/{id}/recovery-quote` and `POST .../cash-out` require the recipient's revealed gift and use active server policy. Fixed recovery prices are capped at paid value. `POST .../exchange` with `{ "product_id": 12 }` buys an active replacement with recovery credit; a shortfall requires wallet balance and excess is credited. Gift settlement and immutable ledger share a transaction; duplicate settlement never credits twice. `GET /api/v1/wallet` returns balance/recent ledger. Withdrawal and discovered expiry refund the sender and restore stock once. Expiry remains discovered by the existing open/answer paths; a background expiry scheduler is not implemented.

Shipment management requires an accepted/exchanged physical gift, appends a trace and can mark carrier delivery. It never writes recipient confirmation; delivered shipments cannot be edited. Contracts can be fulfilled/waived only for accepted gifts with contract text. Wishlists can be closed and friend relations removed. Targeted notifications support expiry, revocation, owner-only listing and read status via `/api/v1/notifications` and `POST /api/v1/notifications/{id}/read`.

These are test-commerce operations, without real payment or withdrawal. Some consumer Makepad pages still use local demonstrations; new APIs do not import local balances or contacts. This Dioxus migration applies to the operations UI.
