# dimasik-backend

Rust + SQLite backend for the Dimasik historical portal prototype. It replaces
the Next.js route handlers and JSON stores: the browser calls the same
`/api/*` paths (which Next rewrites to this service), and Next server
components read the same data through the additional `/internal/*` endpoints.

## Commands

Run from the repository root so the default `data/` and `public/data/` paths
resolve:

```sh
cargo run --manifest-path rust-backend/Cargo.toml -- serve
cargo run --manifest-path rust-backend/Cargo.toml -- import
cargo run --manifest-path rust-backend/Cargo.toml -- import --force
```

* `serve` (default command) — applies migrations, runs the first bootstrap
  import when the database was never imported, then listens on
  `127.0.0.1:8787`.
* `import` — migrations plus the one-time bootstrap import. Re-running is a
  no-op with a notice; `--force` atomically replaces all mutable state with the
  fixtures (an invalid fixture aborts and leaves the previous database intact)
  and is meant for fixture verification.
* `--fresh` — allow an intentional empty install when source documents are
  missing (canonical publication flags plus the bundled defaults). Without it
  the importer requires all four documents.
* `--data-dir DIR`, `--import-dir DIR`, `--database-path PATH` — per-invocation
  overrides.

## Configuration

Values come from the process environment first, then from `./.env` and
`./.env.local` relative to the current working directory. Real environment
variables always win.

| Variable | Default | Purpose |
| --- | --- | --- |
| `DATABASE_PATH` | `data/dimasik.sqlite` | SQLite file (WAL, foreign keys on) |
| `DATA_DIR` | `data` | Directory holding the four JSON documents |
| `IMPORT_DIR` | `DATA_DIR` | First-bootstrap source (read-only) |
| `PUBLIC_DATA_DIR` | `public/data` | Canonical `gubernias.geojson` and `uyezds-1897/` (read-only) |
| `BIND_ADDR` | `127.0.0.1:8787` | Listen address |
| `FRONTEND_ORIGIN` | `http://localhost:3000` | Origin allowed for browser mutations |
| `ADMIN_USERNAME` / `ADMIN_PASSWORD` | unset | Login credentials; unset means login always fails |
| `ADMIN_SESSION_SECRET` | unset | HMAC secret, at least 32 characters (`SESSION_SECRET` accepted as fallback) |

`serve` fails fast when the canonical GeoJSON is missing/invalid, when the
imported JSON documents are invalid, or when the database province set does not
match the canonical 76 features.

## Import semantics

The first bootstrap reads (never writes) `gubernia-publications.json`,
`site-settings.json`, `about-content.json` and `settlement-references.json`
from `IMPORT_DIR`, validates them exactly like the previous TypeScript stores
and writes them into SQLite in one transaction together with an import marker.
The RiStat district registry (`id`, `name`, `province`) is seeded in that same
transaction from `PUBLIC_DATA_DIR/uyezds-1897/*.geojson` (703 rows for the
current atlas). Every canonical province must ship its district file: a missing
or invalid file fails the import before any write instead of committing a
partial registry. Runtime map/point-in-polygon reads stay lenient (a missing
file behaves as an empty district collection), matching the previous loader.
All four documents must exist: everything is parsed and validated before any
write, so a partial or invalid source leaves the database unmarked and
untouched (and `--force` replacement runs in a single transaction, so a bad
fixture can never destroy live state). With `--fresh`, missing documents fall
back to the runtime defaults: publication flags seeded from the canonical
GeoJSON (its `published`/`slug` properties), the bundled category and
settlement-type lists, the bundled About paragraphs, and no references.
`serve` and `import` only ever read the source documents.

Legacy shapes are preserved: posts without a category stay `null`, settlements
without a stored `url` expose the derived `/naselennyy-punkt/<id>` address and
keep `type: null`, and original ids, ordering and timestamps are untouched.

## HTTP surface

Browser API (same paths, methods, statuses and error envelopes
`{"error": "..."}` as before, always `Cache-Control: no-store`):

| Method | Path |
| --- | --- |
| GET | `/api/gubernias` |
| POST | `/api/gubernias` |
| PATCH, DELETE | `/api/gubernias/{id}` |
| POST | `/api/gubernias/{id}/posts` |
| PATCH, DELETE | `/api/gubernias/{id}/posts/{postId}` |
| POST | `/api/gubernias/{id}/settlements` |
| DELETE | `/api/gubernias/{id}/settlements/{settlementId}` |
| PATCH | `/api/naselennyy-punkt/{slug}/reference` |
| PATCH | `/api/about-content` |
| POST, PATCH, DELETE | `/api/nastroyki` |
| POST, DELETE | `/api/admin/session` |

Every mutating route except `POST /api/admin/session` (login only checks
credentials and sets the cookie) and `DELETE /api/admin/session` (which clears
it) requires the `dimasik_admin_session` cookie (HttpOnly, SameSite=Strict,
eight hours) and an Origin that either equals `FRONTEND_ORIGIN` or matches the
request host. Request bodies keep the original byte caps (posts 610,192 bytes,
settlements 4,096, settings 2,048, content 602,000).

SSR internal endpoints (for the server-only fetch adapter; forward the incoming
cookie):

| Method | Path | Response |
| --- | --- | --- |
| GET | `/internal/session` | `{isAdmin}` |
| GET | `/internal/geo` | `{provinces, settlements}` |
| GET | `/internal/gubernia/{slug}` | `PublishedGubernia` or 404 |
| GET | `/internal/settlement/{slug}` | `{settlement, gubernia}` or 404 |
| GET | `/internal/settlement-reference/{id}` | `{body}` for a published settlement; 404 when none is stored or the settlement is withdrawn (an admin cookie may read withdrawn rows) |
| GET | `/internal/about` | `{body}` |
| GET | `/internal/settings` | `{categories, settlementTypes}` |
| GET | `/internal/metrics` | `PublicationMetrics` (admin cookie required, else 401) |

## Storage notes

* Canonical province ids, names, labels and geometry stay file-owned in
  `public/data/gubernias.geojson`; historical district shapes are read lazily
  from `public/data/uyezds-1897/<province>.geojson` for validation.
* `provinces`, `districts`, `posts`, `settlements`, `settlement_references`,
  `site_settings` and `about_content` hold all mutable state. Migrations live in
  `migrations/` and are applied through `schema_migrations`.
* `districts` mirrors the `id`/`name`/`provinceId` triples of the district
  files; geometry stays in the GeoJSON files. Settlement and post writes may
  only use an existing `(province, district)` pair, checked inside the write
  transaction, and the registry is backfilled once (only while empty) for
  databases imported before migration 2.
* Post order is a stored per-province `position`; new posts prepend, moved posts
  insert before older `createdAt` values but after equal ones, matching the
  previous array semantics.
* Deleting a settlement detaches its posts (`settlementId: null`) and best-effort
  removes its reference block; unpublishing a province removes its posts and
  settlements while orphaned reference blocks survive, exactly like the JSON
  stores.
