# syntax=docker/dockerfile:1
#
# Single multi-stage build for the whole stack. Always build with an explicit
# target:
#   frontend — Next.js production server (node:24-bookworm-slim, non-root)
#   backend  — Rust/axum server with bundled SQLite (debian:bookworm-slim, non-root)
#
# BACKEND_URL is read at BUILD time by next.config.ts: the `/api/*` rewrite is
# baked into .next/routes-manifest.json. It is also read at runtime for SSR
# `/internal/*` fetches (lib/backend.ts). Build and run with the same value —
# the default below matches compose's in-cluster service URL. Renaming the
# backend service therefore requires rebuilding the frontend image.

ARG NODE_VERSION=24

# ---------------------------------------------------------------------------
# frontend: install dependencies (build stage)
# ---------------------------------------------------------------------------
FROM node:${NODE_VERSION}-bookworm-slim AS frontend-deps
ENV NEXT_TELEMETRY_DISABLED=1
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci

# ---------------------------------------------------------------------------
# frontend: production build (Turbopack; no backend required at build time)
# ---------------------------------------------------------------------------
FROM node:${NODE_VERSION}-bookworm-slim AS frontend-build
ENV NEXT_TELEMETRY_DISABLED=1
WORKDIR /app
ARG BACKEND_URL=http://backend:8787
ENV BACKEND_URL=${BACKEND_URL}
COPY --from=frontend-deps /app/node_modules ./node_modules
COPY package.json package-lock.json next.config.ts tsconfig.json next-env.d.ts ./
COPY app ./app
COPY components ./components
COPY lib ./lib
COPY public ./public
COPY scripts ./scripts
# `prebuild` regenerates public/maplibre/*.mjs from node_modules before next build.
RUN npm run build

# ---------------------------------------------------------------------------
# frontend: runtime dependencies only
# ---------------------------------------------------------------------------
FROM node:${NODE_VERSION}-bookworm-slim AS frontend-prod-deps
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --omit=dev

# ---------------------------------------------------------------------------
# frontend: runtime
# ---------------------------------------------------------------------------
FROM node:${NODE_VERSION}-bookworm-slim AS frontend
ENV NODE_ENV=production \
    NEXT_TELEMETRY_DISABLED=1
WORKDIR /app
RUN groupadd --gid 10001 app \
 && useradd --uid 10001 --gid app --no-create-home --home-dir /app --shell /usr/sbin/nologin app
COPY --from=frontend-prod-deps --chown=app:app /app/node_modules ./node_modules
COPY --from=frontend-build --chown=app:app /app/.next ./.next
# public/ is taken from the build stage so the generated public/maplibre worker
# (created by the prebuild script) is present; it is also what lib/uyezd-data.ts
# reads from process.cwd() at runtime.
COPY --from=frontend-build --chown=app:app /app/public ./public
COPY --from=frontend-build --chown=app:app /app/package.json /app/next.config.ts ./

USER 10001:10001
EXPOSE 3000
# `/dokumenty` is an informational page that does not depend on the backend;
# `/` is SSR and legitimately fails while the backend is down.
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
  CMD ["node", "-e", "fetch('http://127.0.0.1:3000/dokumenty').then((response) => process.exit(response.ok ? 0 : 1)).catch(() => process.exit(1))"]
CMD ["node", "node_modules/next/dist/bin/next", "start", "-H", "0.0.0.0", "-p", "3000"]

# ---------------------------------------------------------------------------
# backend: compile (MSRV 1.89; Cargo.lock pins everything, migrations are
# embedded via include_str! so the runtime image needs only the binary)
# ---------------------------------------------------------------------------
FROM rust:1.99-bookworm AS backend-build
WORKDIR /build
COPY rust-backend/Cargo.toml rust-backend/Cargo.lock ./rust-backend/
COPY rust-backend/migrations ./rust-backend/migrations
COPY rust-backend/src ./rust-backend/src
RUN cargo build --release --locked --manifest-path rust-backend/Cargo.toml

# ---------------------------------------------------------------------------
# backend: runtime
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS backend
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*
# /var/lib/dimasik is created and owned by UID 10001 before any volume is
# mounted, so a fresh named volume inherits writable ownership.
RUN groupadd --gid 10001 app \
 && useradd --uid 10001 --gid app --no-create-home --home-dir /app --shell /usr/sbin/nologin app \
 && mkdir -p /var/lib/dimasik \
 && chown 10001:10001 /var/lib/dimasik
WORKDIR /app
COPY --from=backend-build /build/rust-backend/target/release/dimasik-backend /usr/local/bin/dimasik-backend
# Read-only first-bootstrap sources: the four JSON documents, when present.
# They are used only when the database has never been imported (no bootstrap
# marker); an existing SQLite database is never overwritten by `serve`.
# The directory is always in the build context (data/.gitkeep) but the documents
# themselves are gitignored, so an image built from a fresh clone has an empty
# /app/seed: a fresh database then fails fast with the backend's own "provide
# all four JSON documents ... or pass --fresh" error, and an intentional empty
# install is started once with `import --fresh` (see compose.yaml).
COPY --chown=app:app data/ /app/seed/
# Canonical atlas geometry: 76-province gubernias.geojson + uyezds-1897/ district files.
COPY --chown=app:app public/data /app/public/data
# BIND_ADDR must leave loopback so sibling containers can connect; 8787 is only
# reachable on the private compose network, never published on the host.
ENV BIND_ADDR=0.0.0.0:8787 \
    DATABASE_PATH=/var/lib/dimasik/dimasik.sqlite \
    IMPORT_DIR=/app/seed \
    PUBLIC_DATA_DIR=/app/public/data

USER 10001:10001
EXPOSE 8787
# /internal/session is unauthenticated, no-store and reads the migrated database.
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
  CMD ["curl", "-fsS", "http://127.0.0.1:8787/internal/session"]
ENTRYPOINT ["dimasik-backend"]
CMD ["serve"]
