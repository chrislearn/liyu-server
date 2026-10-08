# syntax=docker/dockerfile:1
ARG RUST_IMAGE=rust:1-bookworm
# WebAssembly assets are architecture independent; build once on the runner.
FROM --platform=$BUILDPLATFORM ${RUST_IMAGE} AS ui
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends clang lld pkg-config libssl-dev python3 && rm -rf /var/lib/apt/lists/*
RUN rustup target add wasm32-unknown-unknown
RUN --mount=type=cache,target=/usr/local/cargo/registry cargo install dioxus-cli --version 0.7.10 --locked
COPY admin-ui/ admin-ui/
COPY scripts/build-admin.py scripts/build-admin.py
COPY web/admin.css web/admin.css
RUN --mount=type=cache,target=/usr/local/cargo/registry python3 scripts/build-admin.py

FROM ${RUST_IMAGE} AS server
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends libpq-dev pkg-config && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY migrations/ migrations/
COPY web/ web/
RUN --mount=type=cache,target=/usr/local/cargo/registry cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends libpq5 ca-certificates curl && rm -rf /var/lib/apt/lists/* && useradd --uid 10001 --create-home liyu && mkdir /data && chown liyu:liyu /data
WORKDIR /app
COPY --from=server /app/target/release/liyu-server /usr/local/bin/liyu-server
COPY --from=ui /app/web/dist /app/web/dist
COPY test-data/products/ /app/test-data/products/
ENV LIYU_BIND=0.0.0.0:8787 LIYU_DATA_DIR=/data LIYU_TEST_DELIVERY=false LIYU_ENV=production LIYU_ADMIN_COOKIE_SECURE=true
USER liyu
EXPOSE 8787
HEALTHCHECK --interval=15s --timeout=3s --start-period=30s CMD curl --fail --silent http://127.0.0.1:8787/health || exit 1
CMD ["liyu-server"]
