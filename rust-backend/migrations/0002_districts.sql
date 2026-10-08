-- Version 2: RiStat district registry (id, name, province) seeded from the
-- canonical public/data/uyezds-1897/*.geojson files. District geometry stays in
-- those static files; SQLite owns the identity/province relations that writes
-- must satisfy.

CREATE TABLE districts (
  id TEXT PRIMARY KEY,
  province_id TEXT NOT NULL REFERENCES provinces (id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  position INTEGER NOT NULL
);

CREATE INDEX districts_province_position ON districts (province_id, position);
