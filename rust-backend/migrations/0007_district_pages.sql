-- Version 7: every canonical district has an automatic page. A NULL URL keeps
-- its stable fallback (/uezd/<id>); rich reference content starts empty. The
-- image table is rebuilt to add district galleries while preserving every
-- existing row, attachment and cleanup marker.

ALTER TABLE districts ADD COLUMN url TEXT;
ALTER TABLE districts ADD COLUMN description_json TEXT NOT NULL DEFAULT '';

CREATE UNIQUE INDEX districts_effective_url
  ON districts (COALESCE(url, '/uezd/' || id));

CREATE TRIGGER districts_reserve_default_url_on_update
BEFORE UPDATE OF url ON districts
WHEN NEW.url IS NOT NULL AND EXISTS (
  SELECT 1 FROM districts other
  WHERE other.id <> NEW.id AND '/uezd/' || other.id = NEW.url
)
BEGIN
  SELECT RAISE(ABORT, 'district default URL is reserved');
END;

CREATE TRIGGER districts_reserve_default_url_on_insert
BEFORE INSERT ON districts
WHEN NEW.url IS NOT NULL AND EXISTS (
  SELECT 1 FROM districts other
  WHERE other.id <> NEW.id AND '/uezd/' || other.id = NEW.url
)
BEGIN
  SELECT RAISE(ABORT, 'district default URL is reserved');
END;

CREATE TABLE post_images_v7 (
  id TEXT PRIMARY KEY,
  post_id TEXT REFERENCES posts (id) ON DELETE SET NULL,
  province_id TEXT REFERENCES provinces (id) ON DELETE SET NULL,
  settlement_id TEXT REFERENCES settlements (id) ON DELETE SET NULL,
  district_id TEXT REFERENCES districts (id) ON DELETE SET NULL,
  original_key TEXT NOT NULL,
  thumbnail_key TEXT NOT NULL,
  width INTEGER NOT NULL CHECK (width >= 0),
  height INTEGER NOT NULL CHECK (height >= 0),
  position INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  created_ms INTEGER NOT NULL,
  attached_ms INTEGER,
  CHECK (
    (post_id IS NOT NULL) +
    (province_id IS NOT NULL) +
    (settlement_id IS NOT NULL) +
    (district_id IS NOT NULL) <= 1
  )
);

INSERT INTO post_images_v7 (
  id, post_id, province_id, settlement_id, district_id,
  original_key, thumbnail_key, width, height, position,
  created_at, created_ms, attached_ms
)
SELECT
  id, post_id, province_id, settlement_id, NULL,
  original_key, thumbnail_key, width, height, position,
  created_at, created_ms, attached_ms
FROM post_images;

DROP TABLE post_images;
ALTER TABLE post_images_v7 RENAME TO post_images;

CREATE INDEX post_images_post_position ON post_images (post_id, position);
CREATE INDEX post_images_province_position ON post_images (province_id, position);
CREATE INDEX post_images_settlement_position ON post_images (settlement_id, position);
CREATE INDEX post_images_district_position ON post_images (district_id, position);
CREATE INDEX post_images_unattached ON post_images (attached_ms, created_ms)
  WHERE post_id IS NULL AND province_id IS NULL AND settlement_id IS NULL AND district_id IS NULL;
