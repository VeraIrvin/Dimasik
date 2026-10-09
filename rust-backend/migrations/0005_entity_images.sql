-- Version 5: the existing image pipeline may attach an upload to exactly one
-- post, province or settlement. Rebuilding the table adds cross-column CHECKs
-- while preserving every version-4 row and its post attachment.

CREATE TABLE post_images_v5 (
  id TEXT PRIMARY KEY,
  post_id TEXT REFERENCES posts (id) ON DELETE SET NULL,
  province_id TEXT REFERENCES provinces (id) ON DELETE SET NULL,
  settlement_id TEXT REFERENCES settlements (id) ON DELETE SET NULL,
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
    (settlement_id IS NOT NULL) <= 1
  )
);

INSERT INTO post_images_v5 (
  id, post_id, province_id, settlement_id, original_key, thumbnail_key,
  width, height, position, created_at, created_ms, attached_ms
)
SELECT
  id, post_id, NULL, NULL, original_key, thumbnail_key,
  width, height, position, created_at, created_ms, attached_ms
FROM post_images;

DROP TABLE post_images;
ALTER TABLE post_images_v5 RENAME TO post_images;

CREATE INDEX post_images_post_position ON post_images (post_id, position);
CREATE INDEX post_images_province_position ON post_images (province_id, position);
CREATE INDEX post_images_settlement_position ON post_images (settlement_id, position);
CREATE INDEX post_images_unattached ON post_images (attached_ms, created_ms)
  WHERE post_id IS NULL AND province_id IS NULL AND settlement_id IS NULL;
