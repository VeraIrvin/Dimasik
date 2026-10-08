-- Version 1: publication state, content, settings and references.
-- The canonical 76 province rows are seeded from public/data/gubernias.geojson
-- during the first bootstrap import; only mutable state lives here.
-- `schema_migrations` itself is created by the migration runner.

CREATE TABLE app_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE provinces (
  id TEXT PRIMARY KEY,
  published INTEGER NOT NULL DEFAULT 0 CHECK (published IN (0, 1)),
  slug TEXT,
  description TEXT NOT NULL DEFAULT '',
  CHECK (
    (published = 0 AND slug IS NULL AND description = '')
    OR (published = 1 AND slug IS NOT NULL)
  )
);

CREATE UNIQUE INDEX provinces_slug_unique ON provinces (slug) WHERE slug IS NOT NULL;

CREATE TABLE settlements (
  id TEXT PRIMARY KEY,
  province_id TEXT NOT NULL REFERENCES provinces (id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  uyezd_id TEXT NOT NULL,
  latitude REAL NOT NULL,
  longitude REAL NOT NULL,
  created_at TEXT NOT NULL,
  position INTEGER NOT NULL,
  -- NULL keeps the legacy "url not stored" state; readers derive
  -- /naselennyy-punkt/<id> exactly like the previous JSON store did.
  url TEXT,
  type TEXT
);

CREATE INDEX settlements_province_position ON settlements (province_id, position);

CREATE TABLE posts (
  id TEXT PRIMARY KEY,
  province_id TEXT NOT NULL REFERENCES provinces (id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  body_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  -- Date.parse(createdAt) in milliseconds; drives the newest-first order.
  created_ms INTEGER NOT NULL,
  position INTEGER NOT NULL,
  uyezd_id TEXT,
  settlement_id TEXT REFERENCES settlements (id) ON DELETE SET NULL,
  year TEXT NOT NULL DEFAULT '',
  archive_reference TEXT NOT NULL DEFAULT '',
  -- NULL keeps legacy "category not stored"; never matched against settings.
  category TEXT
);

CREATE INDEX posts_province_position ON posts (province_id, position);
CREATE INDEX posts_settlement ON posts (settlement_id);

-- References deliberately have no foreign key: unpublishing a province wipes
-- its settlements but (like the JSON stores) keeps orphaned reference blocks.
CREATE TABLE settlement_references (
  settlement_id TEXT PRIMARY KEY,
  body_json TEXT NOT NULL
);

CREATE TABLE site_settings (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  categories_json TEXT NOT NULL,
  settlement_types_json TEXT NOT NULL
);

CREATE TABLE about_content (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  body_json TEXT NOT NULL
);
