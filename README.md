# LiYu test server

Rust + Salvo REST API backed by PostgreSQL and Diesel. The implementation plan, current gaps, API inventory and privacy matrix are in [docs/_todo.md](docs/_todo.md). This is a local test backend: no SMS/email, real payment or carrier integration. Bind to loopback by default; fixed test credentials must never be exposed publicly.

## Run

Create an empty PostgreSQL database, set `DATABASE_URL`, then run `cargo run`. Migrations and deterministic demo seeds run automatically. `LIYU_BIND` defaults to `127.0.0.1:8787`.

```powershell
$env:DATABASE_URL = 'postgres://postgres:password@127.0.0.1:5432/liyu'
cargo run
```

Seed users: `demo@liyu.test`, `linzhou@liyu.test`, `chenxiao@liyu.test`; every password is `123456`. Registration also requires verification code `123456`. Identifiers are arbitrary test strings; no real phone or email is needed. The catalog has 33 stable IDs matching the app. Product images and their provenance are in `assets/products/`.

```powershell
$base = 'http://127.0.0.1:8787'
$login = Invoke-RestMethod "$base/api/v1/auth/login" -Method Post -ContentType 'application/json' -Body '{"identifier":"demo@liyu.test","password":"123456"}'
$headers = @{Authorization="Bearer $($login.token)"}
Invoke-RestMethod "$base/api/v1/auth/register" -Method Post -ContentType 'application/json' -Body '{"identifier":"new-user","display_name":"新用户","password":"123456","code":"123456"}'
```

Session tokens expire 30 days after issuance. `POST /auth/refresh` takes the current Bearer token and returns a new token plus `expires_in_seconds`; the old token is revoked atomically. `POST /auth/logout` takes the Bearer token and returns `204`, immediately revoking it. Expired or revoked tokens receive `401` and cannot refresh. The test password and registration code remain `123456`.

```powershell
$refreshed = Invoke-RestMethod "$base/api/v1/auth/refresh" -Method Post -Headers $headers
$headers = @{Authorization="Bearer $($refreshed.token)"}
Invoke-RestMethod "$base/api/v1/auth/logout" -Method Post -Headers $headers
```

`LIYU_API_URL` lets the app reach this test server; `LIYU_IDENTIFIER` selects another seeded/registered account, and `LIYU_AUTH_TOKEN` overrides automatic login. The app falls back to local demo data when the server is unavailable. The old whole-state endpoint permanently returns `410`: its JSON would contain private gift answers, addresses and money entries. Online workflows use the domain APIs below.

## Current REST surface

All paths below are under `/api/v1`, except `/health`. Authenticated endpoints need `Authorization: Bearer <token>`.

| Area | Endpoints |
| --- | --- |
| Auth | `POST /auth/register`, `POST /auth/login`, `POST /auth/refresh`, `POST /auth/logout`, `GET /me` |
| Profile | `GET/PATCH /me/profile`, `PUT /me/phone`, `PUT /me/email`, `GET/POST /me/addresses`, `PUT/DELETE /me/addresses/{id}`, `POST/DELETE /me/avatar`, `GET /media/avatars/{id}` |
| Catalog | `GET /catalog/categories`, `GET /catalog?category=&q=&cursor=&limit=`, `GET /catalog/{id}`, `GET /media/products/{id}/{variant}` (`thumb`, `card`, `detail`) |
| Friends and wishlists | `GET /friends`, `POST /friend-requests`, `POST /friend-requests/{id}/accept`, `GET/POST /wishlists`, `GET/PATCH/DELETE /wishlists/{id}`, `POST /wishlists/{id}/close`, `POST/DELETE /wishlists/{id}/items`, `GET /friends/{id}/wishlists` |
| Cart and test orders | `GET/DELETE /cart`, `POST /cart/items`, `DELETE /cart/items/{id}`, `POST /orders/quote`, `GET/POST /orders`, `GET /orders/{id}`, `POST /orders/{id}/pay-test` |
| Gifts | `GET /gifts/inbox`, `GET /gifts/outbox`, `GET /gifts/{id}`, `PUT /gifts/{id}/puzzle`, `POST /gifts/{id}/open`, `/answer`, `/accept`, `/withdraw` |
| Shipment privacy | `GET /shipments/{gift_id}` and `POST /shipments/{gift_id}/confirm-receipt` for the recipient; `GET /gifts/{gift_id}/delivery-summary` for the sender |

For local logistics demonstrations only, set `LIYU_DEMO_MODE=true` and a random `LIYU_DEMO_ADMIN_KEY` of at least 32 characters. From a loopback client, `POST /api/v1/demo/shipments/{gift_id}/advance` with header `X-LiYu-Demo-Key` and JSON `{"stage":"out_for_delivery"}` advances one step (`collected` → `transit` → `out_for_delivery` → `delivered`). The default mode disables this route. Seed gift `900003` is in transit; `900001` is carrier-delivered but awaits the recipient's confirmation.

The profile's `avatar_url` is set by binary upload: `POST /me/avatar` takes a raw JPEG/PNG/WebP body (max 1 MiB, 16–4096 px per axis; magic bytes are sniffed, oversized dimensions are rejected before decode, and the image must fully decode), stores it under an unguessable UUID in `assets/avatars/`, and points `avatar_url` at `GET /media/avatars/{id}` (public, immutable-cached). Re-uploading atomically replaces the previous avatar; `DELETE /me/avatar` removes it and clears the pointer. External avatar URLs are no longer accepted. The cart has a product, confirmed-friend recipient, and optional wishlist item per line; `POST /orders` requires an `Idempotency-Key` header. `pay-test` is a server-side simulated payment: it creates canonical gifts and atomically claims linked wishes but does not charge a real provider. The sender's delivery response contains only carrier-delivered and recipient-confirmed booleans; the recipient alone can see tracking, address and event history. Gifts have role-specific projections and server-side puzzle attempts. Recipient exchange/cash-out, wallet accounting, and complete online wishlist/order history integration remain tracked in [docs/_todo.md](docs/_todo.md).

Run `./tests/e2e.ps1` against a loopback server to verify the 33-product catalog, three-account privacy boundaries, idempotent payment, puzzle, and gift acceptance. This script creates test orders and gifts in its target database.
