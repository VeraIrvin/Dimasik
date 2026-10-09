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
| `S3_ENDPOINT` | unset | Bucket-less HTTPS base endpoint of the Selectel Object Storage S3 API; the bucket is addressed as a host subdomain |
| `S3_REGION` | unset | Region of the bucket |
| `S3_BUCKET` | unset | Pre-existing bucket name (the backend never creates it) |
| `S3_ACCESS_KEY_ID` | unset | Selectel service-user access key |
| `S3_SECRET_ACCESS_KEY` | unset | Matching secret; never sent to the browser or baked into images |

All five `S3_*` values are optional but must be set together: when any is
missing or empty the upload API answers `503` and the sweep is a no-op, while
every non-image route keeps working. There is no local filesystem fallback and
startup never fails on missing S3 configuration. `S3_ENDPOINT` must be an
absolute scheme+host+port URL with no path, query or fragment, and requests use
virtual-hosted addressing: `S3_BUCKET` is prepended to the endpoint host
(`https://<bucket>.s3.ru-6.storage.selcloud.ru`) for uploads, deletes and signed
GET URLs. `https://` is required for Selectel, while `http://` is accepted for a
local S3-compatible server during development.

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
| POST | `/api/post-images` |
| GET | `/api/post-images/{id}/{variant}` (`original` or `thumbnail`) |
| DELETE | `/api/post-images/{id}` |
| POST | `/api/gubernias/{id}/settlements` |
| DELETE | `/api/gubernias/{id}/settlements/{settlementId}` |
| PATCH | `/api/naselennyy-punkt/{slug}/reference` |
| PATCH | `/api/about-content` |
| POST, PATCH, DELETE | `/api/nastroyki` |
| POST, DELETE | `/api/admin/session` |
| GET | `/api/metrika/traffic` (administrator session required) |

Every mutating route except `POST /api/admin/session` (login only checks
credentials and sets the cookie) and `DELETE /api/admin/session` (which clears
it) requires the `dimasik_admin_session` cookie (HttpOnly, SameSite=Strict,
eight hours) and an Origin that either equals `FRONTEND_ORIGIN` or matches the
request host. Request bodies keep the original byte caps (posts 610,192 bytes,
settlements 4,096, settings 2,048, content 602,000).

Traffic reports use `GET /api/metrika/traffic?report=overview|daily|pages|sources|devices&period=today|7d|30d|custom`.
Custom periods require inclusive ISO dates `from` and `to`; dates use Moscow time
(`+03:00`). The handler checks the signed administrator session before checking
configuration, cache, or Yandex; anonymous and forged-cookie reads answer 401,
including when a report is already cached.

Set `YANDEX_METRIKA_OAUTH_TOKEN` (scope `metrika:read`, counter read access) and
`YANDEX_METRIKA_COUNTER_ID` in the ignored root `.env.metrika.local` and restart
the backend. Only these two keys are loaded from that file, after `.env.local`;
process environment wins, and an explicit empty process value disables a key.
Compose v2.24+ injects the optional file only into the backend at runtime.
Missing configuration answers 503; upstream failures answer 502/503 with
`{error, code}`, never zero traffic. OAuth values and raw upstream error bodies
are not returned to browsers or logged.

Five report-specific queries use the Yandex Reporting API. Period visitors come
from the ungrouped `ym:s:users` total, not the sum of daily visitors. Daily rows
use `ym:s:date`; popular pages use the separate hit-family `ym:pv:URL` and
`ym:pv:pageviews`. Successful normalized responses include the resolved period,
fetch timestamp, sampling and lag metadata. They are cached as serialized bytes
for ten minutes by report/date range, with per-key single-flight and a bounded
256-entry cache. Errors are not cached as successful reports. The global
`Cache-Control: no-store` middleware still protects the browser-facing response.


Image endpoints (all `Cache-Control: no-store` like the rest):

* `POST /api/post-images` — admin cookie plus the Origin/host guard;
  `multipart/form-data` with exactly one `file` part, request cap 10 MiB + 64
  KiB. The decoded image must be JPEG, PNG or WebP, at most 10 MiB, at most
  8192 px per side and at most 24 MP, otherwise the answer is 400. EXIF
  orientation is applied before the thumbnail is rendered, so `width`/`height`
  describe the oriented image and the thumbnail matches what the browser shows
  for the original. A provisional cleanup record with the deterministic keys
  is written before the first PUT, so a partial or failed attempt is never
  left untracked: the row is dropped immediately once the rollback deletes
  both objects, and the sweep retries from the persisted keys otherwise. A PUT
  that finishes after the sweep already fenced its provisional row cannot
  complete it: the request answers 404 and the objects it wrote are deleted
  again. Every object PUT and DELETE runs under an application deadline of
  `S3_OPERATION_TIMEOUT` (60 seconds), so an endpoint that accepts the
  connection and then stalls fails the request instead of pinning it. Success
  is 201 with
  `{id, width, height, originalUrl, thumbnailUrl}`: the original bytes are
  stored unchanged with their detected content type and a WebP thumbnail
  bounded to 480 px on its longest side (never upscaled) is put next to it. An
  S3, transport or deadline failure answers 502; when the `S3_*` set is
  absent or incomplete the route answers 503 for the administrator before it
  reads the body, and every non-image route keeps working.
* `GET /api/post-images/{id}/original` and `.../thumbnail` — 302 with a
  `Location` header pointing at a SigV4 signed URL that expires after 600
  seconds. Pending uploads require the admin cookie and are only reachable
  while fresh (24 h TTL); attached images are public only while their province
  is published. Unknown, expired, cancelled, in-flight or otherwise hidden ids
  answer 404, and storage keys are never accepted from query parameters.
* `DELETE /api/post-images/{id}` — admin plus Origin guard; completed pending
  uploads only (an attached image answers 409, a provisional in-flight row,
  a row already fenced for cleanup or an unknown id answers 404, and without
  S3 configuration the request answers 503 keeping the row and its keys).
  Both objects are deleted before the row: a storage failure answers 502 and
  keeps the row with its keys so the sweep retries, success answers 204
  without a body.

Post creation and editing accept `imageIds`. On
`POST /api/gubernias/{id}/posts` it is the ordered list of fresh pending
uploads to claim (a missing or `null` field means no attachments, as before).
On `PATCH /api/gubernias/{id}/posts/{postId}`, an absent field preserves all
current attachments and their order; a present array is the complete ordered
replacement (`[]` removes all), while an explicit `null` is not a list and
answers 400. Every retained
id must still belong to that post and every added id must be a fresh pending
upload; ids attached to another post answer 409, unknown, expired or already
detached ids answer 404, and a non-array value, duplicate id or more than ten
ids answers 400. Submitted order becomes `position`.

The create or update, including title, body, metadata, province placement or
move, image claims, detaches and ordering, runs in one `BEGIN IMMEDIATE`
transaction. Any invalid id or other failure rolls the entire operation back,
so neither post fields nor attachments change. These routes have the same
administrator-cookie and Origin/host authorization requirements as the other
mutating routes.

Post bodies keep strict rich-document validation with one image-only
exception: an empty body is accepted exactly when the final attachment list is
non-empty, and the canonical empty document
`{"type":"doc","content":[{"type":"paragraph"}]}` is stored, never `null`.
For PATCH this validation uses the preserved attachments when `imageIds` is
absent, and the replacement when it is present. A final empty list
with an empty body answers 400 with
`Текст публикации не должен быть пустым.`, including a PATCH that removes the
last image. Because image validation and claims occur in the same transaction,
a forged or unusable id cannot authorise an image-only record.

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
* `post_images` (migration 4) stores one row per uploaded image: both object
  keys, the oriented `width`/`height`, the 0-based `position` and lifecycle
  timestamps. It is the only owner of the storage key strings — URLs are never
  re-parsed into keys. Objects live under `post-images/{id}/original` and
  `post-images/{id}/thumbnail`, where `id` is the same server-generated row id;
  the thumbnail is a bounded WebP (max side 480 px, never upscaled, rendered
  from EXIF-oriented pixels so it needs no further rotation) and the original
  keeps its uploaded bytes, EXIF metadata and detected JPEG/PNG/WebP content
  type. Migration 4 adds the table without backfilling, so every post created
  before it simply has no image rows and serializes an empty `images` array.
* `POST /api/post-images` first writes a **provisional** row
  (`attached_ms = 0`, zero dimensions, the deterministic object keys) and only
  then PUTs the objects; once both exist the same row is completed with the
  returned keys and dimensions and becomes an ordinary claimable pending
  upload. A later failure — a partial PUT, a database error, a failed
  rollback delete — is recorded durably and both objects are deleted again;
  when those deletions succeed the row is dropped immediately instead of
  waiting for the next sweep. Only a failed deletion or a failed final
  database step leaves the row behind with its persisted keys, so a failed or
  interrupted upload can never leave untracked objects.
* Post creation may claim up to ten unique pending ids (`imageIds`), while a
  post PATCH may atomically keep, reorder, add and remove images. For PATCH an
  absent field preserves the attachments; a present array is the complete
  replacement and its order becomes `position` (an explicit `null`, a
  non-array, a duplicate or an oversized list answers 400). Existing ids must
  belong to that post, and new ids must be fresh pending uploads. A missing,
  expired or detached id answers 404, an id attached to another post 409.
  Provisional in-flight rows and
  rows already fenced for cleanup are never claimable. Claims, detaches and
  ordering run in the same `BEGIN IMMEDIATE` transaction as all other post
  fields and a province move, so any failure rolls everything back. An empty
  post body is accepted only when the final attachment list is non-empty and
  stores the canonical empty document rather than `null`.
* Completed pending rows stay readable only by their uploader while fresh;
  after `PENDING_IMAGE_TTL_MS` (24 hours) they become unclaimable and
  unreadable. The cleanup sweep deletes objects first and then rows, and treats
  three categories as ready: detached rows immediately, provisional rows
  abandoned past `IMAGE_UPLOAD_GRACE_MS` (30 minutes), and completed uploads
  past the TTL. Before touching S3 the sweep fences each candidate with an
  atomic compare-and-set, so an upload whose PUTs finish after the listing can
  no longer complete its row and no post can claim it. A row whose deletion
  fails stays fenced with its persisted keys for a later pass, and each pass
  walks the queue behind a `(sort key, id)` cursor that lasts for that pass
  only, so a batch of persistently undeletable objects never starves the rows
  behind it.
* Deleting an attached image is refused (409) and a provisional in-flight row
  answers 404; without S3 configuration the request answers 503 and keeps the
  row and its keys. Deleting a completed pending upload — including a new
  upload discarded from or cancelled with the form — marks the row for
  cleanup, best-effort deletes both objects and only then drops the row; if
  storage deletion fails, the row with both keys remains for a later sweep.
  Cancellation never sends this delete for an image that was already attached
  when editing began. A successful post replacement detaches each removed
  image by clearing `post_id` and stamping `attached_ms` while keeping both
  keys; transaction rollback leaves it attached. The sweep durably
  completes cleanup by deleting both S3 objects before dropping the detached
  row, retaining the row and keys whenever storage deletion fails. Deleting a
  post uses the same detached lifecycle. The sweep runs at startup and
  opportunistically after uploads, image deletes, post updates that detach
  images, post deletes and province deletes, which are the operations that can
  leave cleanup work behind.
* `images` is serialized on every post object (possibly empty), so the feed
  never distinguishes "absent" from "none"; feeds load a whole province's
  images in one query instead of one per post.
