set dotenv-load := true
set dotenv-required := true

default:
    @just --list

# Create the local development database if missing, then migrate and serve.
dev: build-admin
    python3 scripts/dev-db.py ensure
    LIYU_TEST_DELIVERY="${LIYU_TEST_DELIVERY:-true}" cargo run --bin liyu-server

# Delete all development data; migrations and demo seeds return on just dev.
reset:
    python3 scripts/dev-db.py reset

# After a backup: add admin schema to an already migrated database, preserving rows.
upgrade-admin:
    python3 scripts/upgrade-admin.py

# Rebuild the Dioxus management interface served at /admin.
build-admin:
    python3 scripts/build-admin.py
