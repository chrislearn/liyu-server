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

`LIYU_API_URL` lets the current app try test-account login; `LIYU_IDENTIFIER` selects another seeded/registered account, and `LIYU_AUTH_TOKEN` overrides automatic login. The app falls back to local demo data when the server is unavailable. The old whole-state endpoint is **disabled by default** because that JSON contains private gift answers, addresses and money entries. It can only be enabled for isolated legacy demos with `LIYU_ENABLE_LEGACY_STATE=1`. Online workflows must move to the domain APIs listed below; do not use legacy state sync for multi-user testing.

## Current REST surface

All paths below are under `/api/v1`, except `/health`. Authenticated endpoints need `Authorization: Bearer <token>`.

| Area | Endpoints |
| --- | --- |
| Auth | `POST /auth/register`, `POST /auth/login`, `GET /me` |
| Profile | `GET/PATCH /me/profile`, `PUT /me/phone`, `PUT /me/email`, `GET/POST /me/addresses`, `PUT/DELETE /me/addresses/{id}` |
| Catalog | `GET /catalog/categories`, `GET /catalog?category=&q=&cursor=&limit=`, `GET /catalog/{id}`, `GET /media/products/{id}/{variant}` (`thumb`, `card`, `detail`) |
| Cart and test orders | `GET/DELETE /cart`, `POST /cart/items`, `DELETE /cart/items/{id}`, `POST /orders/quote`, `GET/POST /orders`, `GET /orders/{id}`, `POST /orders/{id}/pay-test` |
| Shipment privacy | `GET /shipments/{gift_id}` and `POST /shipments/{gift_id}/confirm-receipt` for the recipient; `GET /gifts/{gift_id}/delivery-summary` for the sender |

The profile's `avatar_url` is currently a validated HTTPS reference; binary upload is planned. The cart currently has one product and recipient per line, and `POST /orders` requires an `Idempotency-Key` header. `pay-test` is a server-side simulated payment: it creates canonical gifts but does not charge a real provider. The sender's delivery response contains only carrier-delivered and recipient-confirmed booleans; the recipient alone can see tracking, address and event history. Friendship enforcement, the full gift/puzzle state machine, wishlist claims, wallet, and the app's shopping-cart/parcel UI remain tracked in [docs/_todo.md](docs/_todo.md).
