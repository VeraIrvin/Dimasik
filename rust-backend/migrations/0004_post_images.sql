-- Version 4: uploaded post images.
--
-- A row is created before its post exists: `post_id IS NULL` with a NULL
-- `attached_ms` is a pending upload. Creating a post claims pending rows in
-- the order of the submitted ids and stamps `attached_ms`.
--
-- Every upload first writes a provisional row with the deterministic object
-- keys (`post-images/{id}/original` + `/thumbnail`) and `attached_ms = 0`;
-- only after both storage PUTs succeed does the row become a claimable
-- pending one. A failed attempt (partial PUT, database error, failed
-- rollback delete) therefore always leaves a durable cleanup record, and the
-- sweep retries the object deletion from the persisted keys.
--
-- The two object keys are persisted per row, and a row is only ever dropped
-- after both storage objects are confirmed deleted. Deleting a post leaves its
-- rows behind with `post_id = NULL` and `attached_ms` set (the orphan marker),
-- so object keys are never lost before the storage deletion succeeds; the
-- cleanup sweep handles such rows together with pending uploads past their TTL.

CREATE TABLE post_images (
  id TEXT PRIMARY KEY,
  post_id TEXT REFERENCES posts (id) ON DELETE SET NULL,
  original_key TEXT NOT NULL,
  thumbnail_key TEXT NOT NULL,
  -- 0 while an upload is still in flight (the row is written before the
  -- storage PUTs and completed after both objects exist); the generated
  -- image always has real dimensions.
  width INTEGER NOT NULL CHECK (width >= 0),
  height INTEGER NOT NULL CHECK (height >= 0),
  -- Pending rows keep 0; claiming assigns the 0-based order within the post.
  position INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  created_ms INTEGER NOT NULL,
  -- NULL = upload completed, never attached (claimable while fresh);
  -- 0 = provisional row written before the storage PUTs (abandoned ones are
  -- cleaned after the upload grace period); a positive timestamp = claimed-at
  -- and, once `post_id` is NULL again, the marker for object cleanup.
  attached_ms INTEGER
);

-- Ordered per-post reads (the feed serializer).
CREATE INDEX post_images_post_position ON post_images (post_id, position);
-- Cleanup scans: orphaned rows and pending rows past their TTL.
CREATE INDEX post_images_unattached ON post_images (attached_ms, created_ms) WHERE post_id IS NULL;
