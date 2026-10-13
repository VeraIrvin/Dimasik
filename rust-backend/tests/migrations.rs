use dimasik_backend::db;
use rusqlite::types::Value;
use rusqlite::{params, Connection};

fn version_eight_database() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    connection
        .execute_batch(include_str!("../migrations/0001_init.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0002_districts.sql"))
        .unwrap();
    connection
        .execute_batch("ALTER TABLE provinces RENAME COLUMN description TO description_json;")
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0004_post_images.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0005_entity_images.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0006_post_author.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0007_district_pages.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0008_district_display_names.sql"))
        .unwrap();
    connection
        .execute_batch(
            "CREATE TABLE schema_migrations (
               version INTEGER PRIMARY KEY,
               applied_at TEXT NOT NULL
             );
             INSERT INTO schema_migrations (version, applied_at) VALUES
               (1, 'old'), (2, 'old'), (3, 'old'), (4, 'old'),
               (5, 'old'), (6, 'old'), (7, 'old'), (8, 'old');",
        )
        .unwrap();
    connection
}

fn insert_attached_fixture(connection: &Connection, latitude: f64, longitude: f64) {
    connection
        .execute_batch(
            "INSERT INTO provinces (id, published, slug, description_json)
               VALUES ('province', 1, 'province-slug', '{\"type\":\"doc\"}');
             INSERT INTO districts (
               id, province_id, name, position, url, description_json, display_name
             ) VALUES (
               'district', 'province', 'Canonical district', 4,
               '/district/custom', '{\"type\":\"doc\"}', 'Current district'
             );",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO settlements (
               id, province_id, name, uyezd_id, latitude, longitude,
               created_at, position, url, type
             ) VALUES (
               'settlement', 'province', 'Settlement', 'district', ?1, ?2,
               '1897-01-02T03:04:05Z', 7, '/settlement/custom', 'село'
             )",
            params![latitude, longitude],
        )
        .unwrap();
    connection
        .execute_batch(
            "INSERT INTO posts (
               id, province_id, title, body_json, created_at, updated_at,
               created_ms, position, uyezd_id, settlement_id, year,
               archive_reference, category, author
             ) VALUES (
               'post', 'province', 'Title', '{\"type\":\"doc\"}',
               'created', 'updated', 123, 8, 'district', 'settlement',
               '1901', 'archive', 'category', 'author'
             );
             INSERT INTO post_images (
               id, post_id, province_id, settlement_id, district_id,
               original_key, thumbnail_key, width, height, position,
               created_at, created_ms, attached_ms
             ) VALUES (
               'settlement-image', NULL, NULL, 'settlement', NULL,
               'original', 'thumbnail', 640, 480, 3,
               'image-created', 456, 789
             );",
        )
        .unwrap();
}

fn row_values(connection: &Connection, sql: &str) -> Vec<Value> {
    let mut statement = connection.prepare(sql).unwrap();
    let column_count = statement.column_count();
    statement
        .query_row([], |row| {
            (0..column_count)
                .map(|column| row.get(column))
                .collect::<rusqlite::Result<Vec<Value>>>()
        })
        .unwrap()
}

#[test]
fn version_nine_preserves_settlement_owners_and_all_metadata() {
    let mut connection = version_eight_database();
    insert_attached_fixture(&connection, 54.25, 40.75);

    let settlement_before = row_values(
        &connection,
        "SELECT id, province_id, name, uyezd_id, latitude, longitude,
                created_at, position, url, type
         FROM settlements WHERE id = 'settlement'",
    );
    let post_before = row_values(
        &connection,
        "SELECT id, province_id, title, body_json, created_at, updated_at,
                created_ms, position, uyezd_id, settlement_id, year,
                archive_reference, category, author
         FROM posts WHERE id = 'post'",
    );
    let image_before = row_values(
        &connection,
        "SELECT id, post_id, province_id, settlement_id, district_id,
                original_key, thumbnail_key, width, height, position,
                created_at, created_ms, attached_ms
         FROM post_images WHERE id = 'settlement-image'",
    );

    db::migrate(&mut connection).unwrap();

    assert_eq!(
        settlement_before,
        row_values(
            &connection,
            "SELECT id, province_id, name, uyezd_id, latitude, longitude,
                    created_at, position, url, type
             FROM settlements WHERE id = 'settlement'",
        )
    );
    assert_eq!(
        post_before,
        row_values(
            &connection,
            "SELECT id, province_id, title, body_json, created_at, updated_at,
                    created_ms, position, uyezd_id, settlement_id, year,
                    archive_reference, category, author
             FROM posts WHERE id = 'post'",
        )
    );
    assert_eq!(
        image_before,
        row_values(
            &connection,
            "SELECT id, post_id, province_id, settlement_id, district_id,
                    original_key, thumbnail_key, width, height, position,
                    created_at, created_ms, attached_ms
             FROM post_images WHERE id = 'settlement-image'",
        )
    );

    let version: i64 = connection
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 9);
    let foreign_keys: i64 = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(foreign_keys, 1);
    let violation_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_check",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(violation_count, 0);
    let settlement_index: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'index' AND name = 'settlements_province_position'
               AND tbl_name = 'settlements'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(settlement_index, 1);

    connection
        .execute_batch(
            "INSERT INTO settlements (
               id, province_id, name, uyezd_id, latitude, longitude,
               created_at, position
             ) VALUES (
               'without-coordinates', 'province', 'No point', 'district',
               NULL, NULL, 'now', 9
             );
             INSERT INTO settlements (
               id, province_id, name, uyezd_id, latitude, longitude,
               created_at, position
             ) VALUES (
               'boundary-point', 'province', 'Boundary', 'district',
               -90, 180, 'now', 10
             );",
        )
        .unwrap();
    assert!(connection
        .execute_batch(
            "INSERT INTO settlements (
               id, province_id, name, uyezd_id, latitude, longitude,
               created_at, position
             ) VALUES (
               'partial-point', 'province', 'Partial', 'district',
               10, NULL, 'now', 11
             );"
        )
        .is_err());
    assert!(connection
        .execute_batch(
            "INSERT INTO settlements (
               id, province_id, name, uyezd_id, latitude, longitude,
               created_at, position
             ) VALUES (
               'bad-latitude', 'province', 'Bad latitude', 'district',
               90.01, 10, 'now', 12
             );"
        )
        .is_err());
    assert!(connection
        .execute_batch(
            "INSERT INTO settlements (
               id, province_id, name, uyezd_id, latitude, longitude,
               created_at, position
             ) VALUES (
               'bad-longitude', 'province', 'Bad longitude', 'district',
               10, -180.01, 'now', 13
             );"
        )
        .is_err());
}

#[test]
fn failed_version_nine_is_atomic_and_reenables_foreign_keys() {
    let mut connection = version_eight_database();
    insert_attached_fixture(&connection, 91.0, 40.75);

    let error = db::migrate(&mut connection).unwrap_err();
    assert!(error.contains("Migration 9 failed"), "{error}");

    let foreign_keys: i64 = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(foreign_keys, 1);
    let version: i64 = connection
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 8);
    let coordinates: (f64, f64) = connection
        .query_row(
            "SELECT latitude, longitude FROM settlements WHERE id = 'settlement'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(coordinates, (91.0, 40.75));
    let owners: (String, String) = connection
        .query_row(
            "SELECT posts.settlement_id, post_images.settlement_id
             FROM posts JOIN post_images ON post_images.id = 'settlement-image'
             WHERE posts.id = 'post'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(owners, ("settlement".to_string(), "settlement".to_string()));
}
