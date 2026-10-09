mod common;

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use common::{
    admin_cookie, base_config, canonical_geojson, canonical_ids, paragraph_doc, request, send,
    standard_fixture_dir, write_district_atlas, write_json, TestEnv,
};

use dimasik_backend::config::Config;
use dimasik_backend::db;
use dimasik_backend::geo::{CanonicalProvinces, GeoRuntime};
use dimasik_backend::import::{import_bootstrap, run_import_command};
use dimasik_backend::{build_router, prepare};

fn canonical(public_data_dir: &std::path::Path) -> CanonicalProvinces {
    CanonicalProvinces::load(&public_data_dir.join("gubernias.geojson")).expect("canonical loads")
}

/// `prepare` returns `Result<AppState, String>`; `AppState` is not `Debug`, so
/// failure expectations use this helper instead of `expect_err`.
fn prepare_error(config: &Config) -> String {
    match prepare(config) {
        Ok(_) => panic!("expected prepare to fail"),
        Err(error) => error,
    }
}

#[tokio::test]
async fn second_prepare_against_the_same_database_keeps_state() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;

    let published = send(
        &app,
        request(
            Method::POST,
            "/api/gubernias",
            Some(&cookie),
            Some(&json!({ "id": "gubernia-1897-1", "slug": "one" })),
        ),
    )
    .await;
    assert_eq!(published.status, StatusCode::CREATED);

    env.prepare_again().expect("second prepare is idempotent");

    let app2 = TestEnv::router_for_config(&env.config);
    let geo = send(&app2, request(Method::GET, "/internal/geo", None, None)).await;
    let provinces = geo.body["provinces"].as_array().unwrap();
    assert_eq!(provinces.len(), 3, "import must not run twice");
    assert!(provinces.iter().any(|province| province["slug"] == "one"));

    let ryazan = send(
        &app2,
        request(Method::GET, "/internal/gubernia/ryazanskaya", None, None),
    )
    .await;
    assert_eq!(ryazan.body["posts"].as_array().unwrap().len(), 2);

    let connection = db::open_database(&env.config.database_path).unwrap();
    let districts: i64 = connection
        .query_row("SELECT COUNT(*) FROM districts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(districts, 77, "district registry is seeded exactly once");
}

#[tokio::test]
async fn import_command_skips_when_imported_and_force_reimports() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;

    let unpublished = send(
        &app,
        request(Method::DELETE, "/api/gubernias/ryazan", Some(&cookie), None),
    )
    .await;
    assert_eq!(unpublished.status, StatusCode::NO_CONTENT);

    let mut connection = db::open_database(&env.config.database_path).expect("database opens");
    db::migrate(&mut connection).expect("migrations");
    let canonical_collection = canonical(&env.config.public_data_dir);
    let geo = GeoRuntime::new(&env.config.public_data_dir);

    run_import_command(&mut connection, &env.config, &canonical_collection, &geo, false)
        .expect("skip is not an error");
    let app2 = TestEnv::router_for_config(&env.config);
    assert_eq!(
        send(
            &app2,
            request(Method::GET, "/internal/gubernia/ryazanskaya", None, None)
        )
        .await
        .status,
        StatusCode::NOT_FOUND,
        "without --force the changed state is kept"
    );

    run_import_command(&mut connection, &env.config, &canonical_collection, &geo, true)
        .expect("force re-import");
    let app3 = TestEnv::router_for_config(&env.config);
    let restored = send(
        &app3,
        request(Method::GET, "/internal/gubernia/ryazanskaya", None, None),
    )
    .await;
    assert_eq!(restored.status, StatusCode::OK);
    assert_eq!(restored.body["posts"].as_array().unwrap().len(), 2);
    assert_eq!(restored.body["settlements"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn invalid_publication_state_rolls_back_and_can_be_retried() {
    let dir = standard_fixture_dir();
    let publication_path = dir.path().join("data/gubernia-publications.json");
    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&publication_path).unwrap()).unwrap();
    value["gubernias"]["ryazan"]["posts"][0]["extra"] = json!(1);
    write_json(&publication_path, &value);

    let config = base_config(dir.path());
    let error = prepare_error(&config);
    assert!(
        error.contains("unrecognised post fields"),
        "clear error message, got: {error}"
    );

    let connection = db::open_database(&config.database_path).expect("database opens");
    let provinces: i64 = connection
        .query_row("SELECT COUNT(*) FROM provinces", [], |row| row.get(0))
        .unwrap();
    assert_eq!(provinces, 0, "transaction rolled back");
    assert_eq!(db::imported_at(&connection).unwrap(), None, "no marker");

    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&publication_path).unwrap()).unwrap();
    value["gubernias"]["ryazan"]["posts"][0]
        .as_object_mut()
        .unwrap()
        .remove("extra");
    write_json(&publication_path, &value);

    prepare(&config).expect("retry with the fixed fixture succeeds");
}

#[tokio::test]
async fn invalid_slug_or_settlement_state_is_rejected_transactionally() {
    let dir = standard_fixture_dir();
    let publication_path = dir.path().join("data/gubernia-publications.json");

    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&publication_path).unwrap()).unwrap();
    value["gubernias"]["tula"]["slug"] = json!("Not A Slug");
    write_json(&publication_path, &value);
    let config = base_config(dir.path());
    let error = prepare_error(&config);
    assert!(error.contains("invalid slug"), "got: {error}");

    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&publication_path).unwrap()).unwrap();
    value["gubernias"]["tula"]["slug"] = json!("tulskaya");
    value["gubernias"]["ryazan"]["settlements"][0]["latitude"] = json!(120);
    write_json(&publication_path, &value);
    let error = prepare_error(&config);
    assert!(error.contains("invalid settlement fields"), "got: {error}");

    let connection = db::open_database(&config.database_path).unwrap();
    let provinces: i64 = connection
        .query_row("SELECT COUNT(*) FROM provinces", [], |row| row.get(0))
        .unwrap();
    assert_eq!(provinces, 0);
}

#[tokio::test]
async fn unpublished_entries_with_content_are_rejected() {
    let dir = standard_fixture_dir();
    let publication_path = dir.path().join("data/gubernia-publications.json");
    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&publication_path).unwrap()).unwrap();
    value["gubernias"]["gubernia-1897-1"]["posts"] = json!([{
        "id": "bbbbbbbb-0000-4000-8000-000000000001",
        "title": "Скрытое",
        "body": paragraph_doc("x"),
        "createdAt": "2026-10-01T00:00:00.000Z",
        "updatedAt": "2026-10-01T00:00:00.000Z",
        "uyezdId": null,
        "settlementId": null,
        "year": "",
        "archiveReference": ""
    }]);
    write_json(&publication_path, &value);

    let error = prepare_error(&base_config(dir.path()));
    assert!(
        error.contains("must not retain publication content"),
        "got: {error}"
    );
}

#[tokio::test]
async fn fresh_install_seeds_canonical_flags_and_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_json(
        &root.join("public/data/gubernias.geojson"),
        &canonical_geojson(&[("ryazan", "ryazanskaya")]),
    );
    write_district_atlas(root);
    let mut config = base_config(root);
    config.fresh_install = true;
    let state = prepare(&config).expect("fresh install");
    let app = build_router(state);
    let geo = send(&app, request(Method::GET, "/internal/geo", None, None)).await;
    let provinces = geo.body["provinces"].as_array().unwrap();
    assert_eq!(provinces.len(), 1);
    assert_eq!(provinces[0]["id"], "ryazan");
    assert_eq!(provinces[0]["slug"], "ryazanskaya");
    assert_eq!(provinces[0]["name"], "Рязанская губерния");
    assert_eq!(geo.body["settlements"].as_array().unwrap().len(), 0);

    let gubernia = send(
        &app,
        request(Method::GET, "/internal/gubernia/ryazanskaya", None, None),
    )
    .await;
    assert_eq!(gubernia.status, StatusCode::OK);
    assert_eq!(gubernia.body["posts"], json!([]));
    assert_eq!(gubernia.body["settlements"], json!([]));

    let settings = send(&app, request(Method::GET, "/internal/settings", None, None)).await;
    assert_eq!(
        settings.body["categories"],
        json!(["Статья", "Персона", "Ссылка на источник", "Источник с индексацией"])
    );
    assert_eq!(
        settings.body["settlementTypes"],
        json!(["Город", "Село", "Деревня"])
    );

    let about = send(&app, request(Method::GET, "/internal/about", None, None)).await;
    assert_eq!(about.body["body"]["content"].as_array().unwrap().len(), 5);
}

#[tokio::test]
async fn database_province_set_must_match_canonical_geojson() {
    let env = TestEnv::new();
    let connection = db::open_database(&env.config.database_path).unwrap();
    connection
        .execute("DELETE FROM provinces WHERE id = 'abos'", [])
        .unwrap();
    let error = env.prepare_again().expect_err("tampered database rejected");
    assert!(error.contains("does not match"), "got: {error}");
}

#[tokio::test]
async fn district_registry_backfills_for_pre_migration_databases() {
    let env = TestEnv::new();
    let connection = db::open_database(&env.config.database_path).unwrap();
    connection.execute("DELETE FROM districts", []).unwrap();
    drop(connection);

    env.prepare_again().expect("backfill prepare");

    let connection = db::open_database(&env.config.database_path).unwrap();
    let districts: i64 = connection
        .query_row("SELECT COUNT(*) FROM districts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(districts, 77, "backfill repopulates the registry once");
}

#[tokio::test]
async fn missing_or_invalid_district_file_rejects_the_import_before_writing() {
    let dir = standard_fixture_dir();
    std::fs::remove_file(dir.path().join("public/data/uyezds-1897/ryazan.geojson")).unwrap();
    let config = base_config(dir.path());
    let error = prepare_error(&config);
    assert!(error.contains("ryazan.geojson"), "got: {error}");
    assert!(error.contains("missing"), "got: {error}");
    let connection = db::open_database(&config.database_path).unwrap();
    let provinces: i64 = connection
        .query_row("SELECT COUNT(*) FROM provinces", [], |row| row.get(0))
        .unwrap();
    let districts: i64 = connection
        .query_row("SELECT COUNT(*) FROM districts", [], |row| row.get(0))
        .unwrap();
    assert_eq!((provinces, districts), (0, 0), "no partial registry");
    assert_eq!(db::imported_at(&connection).unwrap(), None, "no marker");
    drop(connection);

    // A district file that contradicts its province fails the same way.
    let dir = standard_fixture_dir();
    let path = dir.path().join("public/data/uyezds-1897/tula.geojson");
    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    value["features"][1]["properties"]["provinceId"] = json!("ryazan");
    write_json(&path, &value);
    let config = base_config(dir.path());
    let error = prepare_error(&config);
    assert!(error.contains("invalid metadata"), "got: {error}");
    let connection = db::open_database(&config.database_path).unwrap();
    assert_eq!(db::imported_at(&connection).unwrap(), None);
}

#[tokio::test]
async fn concurrent_bootstrap_processes_import_exactly_once() {
    let dir = standard_fixture_dir();
    let config = base_config(dir.path());

    // Two full cold starts, like two `serve` processes launched together: each
    // opens the database, migrates and imports.
    let spawn = |config: Config| {
        std::thread::spawn(move || {
            let canonical =
                CanonicalProvinces::load(&config.public_data_dir.join("gubernias.geojson"))
                    .expect("canonical loads");
            let geo = GeoRuntime::new(&config.public_data_dir);
            let mut connection = db::open_database(&config.database_path).expect("database opens");
            db::migrate(&mut connection).expect("migrations apply");
            import_bootstrap(&mut connection, &config, &canonical, &geo)
                .map(|summary| summary.is_some())
        })
    };
    let handle_a = spawn(config.clone());
    let handle_b = spawn(config.clone());
    let imported_a = handle_a.join().unwrap().expect("cold start A completes");
    let imported_b = handle_b.join().unwrap().expect("cold start B completes");
    assert_eq!(
        [imported_a, imported_b]
            .iter()
            .filter(|imported| **imported)
            .count(),
        1,
        "exactly one concurrent process seeds the database"
    );

    let connection = db::open_database(&config.database_path).unwrap();
    let provinces: i64 = connection
        .query_row("SELECT COUNT(*) FROM provinces", [], |row| row.get(0))
        .unwrap();
    assert_eq!(provinces as usize, canonical_ids().len());
    let posts: i64 = connection
        .query_row("SELECT COUNT(*) FROM posts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(posts, 2, "second importer must not double-insert");
    let districts: i64 = connection
        .query_row("SELECT COUNT(*) FROM districts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(districts, 77);
    let migrations: i64 = connection
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(migrations, 4, "each migration is recorded once");
}

#[test]
fn migration_converts_plain_text_province_descriptions_once() {
    let dir = tempfile::tempdir().unwrap();
    let database_path = dir.path().join("legacy/dimasik.sqlite");
    let mut connection = db::open_database(&database_path).unwrap();
    // Rebuild the version-2 schema exactly as an upgraded live database has it.
    connection
        .execute_batch(include_str!("../migrations/0001_init.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0002_districts.sql"))
        .unwrap();
    connection
        .execute_batch(
            "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);\n\
             INSERT INTO schema_migrations (version, applied_at) VALUES (1, '2026-10-01T00:00:00.000Z'), (2, '2026-10-01T00:00:00.000Z');",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO provinces (id, published, slug, description) VALUES\n\
             ('ryazan', 1, 'ryazanskaya', ?1),\n\
             ('tula', 0, NULL, ''),\n\
             ('abos', 1, 'abos', ?2)",
            rusqlite::params!["Первая строка\n\nВторая", "   "],
        )
        .unwrap();

    db::migrate(&mut connection).unwrap();

    let versions: Vec<i64> = {
        let mut statement = connection
            .prepare("SELECT version FROM schema_migrations ORDER BY version")
            .unwrap();
        let rows = statement.query_map([], |row| row.get::<_, i64>(0)).unwrap();
        rows.map(|row| row.unwrap()).collect()
    };
    assert_eq!(versions, vec![1, 2, 3, 4]);
    // Migration 4 adds the image table even to an upgraded legacy database.
    let image_table: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'post_images'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(image_table, 1, "migration 4 creates post_images");

    let columns: Vec<String> = {
        let mut statement = connection.prepare("PRAGMA table_info(provinces)").unwrap();
        let rows = statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap();
        rows.map(|row| row.unwrap()).collect()
    };
    assert!(columns.contains(&"description_json".to_string()));
    assert!(
        !columns.contains(&"description".to_string()),
        "the legacy column is gone: {columns:?}"
    );

    let converted: String = connection
        .query_row(
            "SELECT description_json FROM provinces WHERE id = 'ryazan'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&converted).unwrap(),
        json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": "Первая строка" }] },
                { "type": "paragraph" },
                { "type": "paragraph", "content": [{ "type": "text", "text": "Вторая" }] }
            ]
        })
    );
    // Whitespace-only legacy text keeps the "no description" sentinel.
    let sentinel: String = connection
        .query_row(
            "SELECT description_json FROM provinces WHERE id = 'abos'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(sentinel, "");
    // Publication state survives the conversion.
    let state: (i64, Option<String>) = connection
        .query_row(
            "SELECT published, slug FROM provinces WHERE id = 'ryazan'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, (1, Some("ryazanskaya".to_string())));
    // The renamed CHECK still ties the sentinel to the unpublished state.
    assert!(connection
        .execute(
            "UPDATE provinces SET description_json = '{\"type\":\"doc\"}' WHERE id = 'tula'",
            [],
        )
        .is_err());

    // Re-running the migration keeps the same version and the same rows.
    db::migrate(&mut connection).unwrap();
    let versions_again: Vec<i64> = {
        let mut statement = connection
            .prepare("SELECT version FROM schema_migrations ORDER BY version")
            .unwrap();
        let rows = statement.query_map([], |row| row.get::<_, i64>(0)).unwrap();
        rows.map(|row| row.unwrap()).collect()
    };
    assert_eq!(versions_again, vec![1, 2, 3, 4]);
    let converted_again: String = connection
        .query_row(
            "SELECT description_json FROM provinces WHERE id = 'ryazan'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(converted_again, converted);
}

#[tokio::test]
async fn integral_float_ordered_list_starts_import_as_integers() {
    let dir = standard_fixture_dir();
    let publication_path = dir.path().join("data/gubernia-publications.json");
    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&publication_path).unwrap()).unwrap();
    value["gubernias"]["ryazan"]["posts"][0]["body"] = json!({
        "type": "doc",
        "content": [{
            "type": "orderedList",
            "attrs": { "start": 2.0 },
            "content": [{
                "type": "listItem",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Пункт" }] }]
            }]
        }]
    });
    write_json(&publication_path, &value);
    assert!(
        std::fs::read_to_string(&publication_path)
            .unwrap()
            .contains("\"start\":2.0"),
        "the fixture carries an integral float representation"
    );

    let config = base_config(dir.path());
    let state = prepare(&config).expect("integral float start imports");
    let app = build_router(state);
    let gubernia = send(
        &app,
        request(Method::GET, "/internal/gubernia/ryazanskaya", None, None),
    )
    .await;
    assert_eq!(
        gubernia.body["posts"][0]["body"]["content"][0]["attrs"]["start"],
        json!(2)
    );
}

#[tokio::test]
async fn legacy_plain_text_descriptions_import_as_rich_documents() {
    let dir = standard_fixture_dir();
    let publication_path = dir.path().join("data/gubernia-publications.json");
    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&publication_path).unwrap()).unwrap();
    value["gubernias"]["ryazan"]["description"] =
        json!("Строка «раз»\n\nСтрока с \"кавычками\" и \\слэшем");
    value["gubernias"]["tula"]["description"] = json!("   \n\t");
    write_json(&publication_path, &value);

    let env = TestEnv::from_dir(dir);
    let app = env.app.clone();

    let ryazan = send(
        &app,
        request(Method::GET, "/internal/gubernia/ryazanskaya", None, None),
    )
    .await;
    assert_eq!(ryazan.status, StatusCode::OK);
    assert_eq!(
        ryazan.body["description"],
        json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": "Строка «раз»" }] },
                { "type": "paragraph" },
                {
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "Строка с \"кавычками\" и \\слэшем" }]
                }
            ]
        })
    );

    // Whitespace-only legacy text becomes the "no description" state.
    let tula = send(
        &app,
        request(Method::GET, "/internal/gubernia/tulskaya", None, None),
    )
    .await;
    assert_eq!(tula.body["description"], Value::Null);

    // The historical 20 000 UTF-16 unit boundary still imports...
    let boundary_dir = standard_fixture_dir();
    let boundary_path = boundary_dir.path().join("data/gubernia-publications.json");
    let mut boundary: Value =
        serde_json::from_str(&std::fs::read_to_string(&boundary_path).unwrap()).unwrap();
    boundary["gubernias"]["tula"]["description"] = json!("x".repeat(20_000));
    write_json(&boundary_path, &boundary);
    let boundary_state = prepare(&base_config(boundary_dir.path())).expect("20 000 units import");
    let boundary_app = build_router(boundary_state);
    let tula = send(
        &boundary_app,
        request(Method::GET, "/internal/gubernia/tulskaya", None, None),
    )
    .await;
    assert_eq!(tula.body["description"], paragraph_doc(&"x".repeat(20_000)));

    // ...while 20 001 units are rejected exactly like before the cutover.
    let oversized_dir = standard_fixture_dir();
    let oversized_path = oversized_dir.path().join("data/gubernia-publications.json");
    let mut oversized: Value =
        serde_json::from_str(&std::fs::read_to_string(&oversized_path).unwrap()).unwrap();
    oversized["gubernias"]["tula"]["description"] = json!("x".repeat(20_001));
    write_json(&oversized_path, &oversized);
    let error = prepare_error(&base_config(oversized_dir.path()));
    assert!(error.contains("invalid fields"), "got: {error}");
}

#[tokio::test]
async fn missing_documents_are_rejected_and_leave_the_database_untouched() {
    let dir = standard_fixture_dir();
    std::fs::remove_file(dir.path().join("data/about-content.json")).unwrap();
    let config = base_config(dir.path());
    let error = prepare_error(&config);
    assert!(error.contains("about-content.json"), "got: {error}");
    assert!(error.contains("--fresh"), "error points at --fresh: {error}");

    let connection = db::open_database(&config.database_path).unwrap();
    let provinces: i64 = connection
        .query_row("SELECT COUNT(*) FROM provinces", [], |row| row.get(0))
        .unwrap();
    assert_eq!(provinces, 0);
    assert_eq!(db::imported_at(&connection).unwrap(), None, "no marker");

    // Entirely empty source directory: still rejected without --fresh.
    let empty = tempfile::tempdir().unwrap();
    write_json(
        &empty.path().join("public/data/gubernias.geojson"),
        &canonical_geojson(&[]),
    );
    let error = prepare_error(&base_config(empty.path()));
    assert!(error.contains("gubernia-publications.json"), "got: {error}");
}

#[tokio::test]
async fn force_reimport_with_an_invalid_fixture_keeps_the_previous_database() {
    let dir = standard_fixture_dir();
    let config = base_config(dir.path());
    let canonical_collection = canonical(&config.public_data_dir);
    let geo = GeoRuntime::new(&config.public_data_dir);
    let mut connection = db::open_database(&config.database_path).unwrap();
    db::migrate(&mut connection).unwrap();
    run_import_command(&mut connection, &config, &canonical_collection, &geo, false).unwrap();

    // Simulate live edits that a failed `--force` re-import must not destroy.
    connection
        .execute(
            "UPDATE provinces SET published = 1, slug = 'one' WHERE id = 'gubernia-1897-1'",
            [],
        )
        .unwrap();

    let publication_path = dir.path().join("data/gubernia-publications.json");
    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&publication_path).unwrap()).unwrap();
    value["gubernias"]["ryazan"]["slug"] = json!("Bad Slug");
    write_json(&publication_path, &value);

    let error = run_import_command(&mut connection, &config, &canonical_collection, &geo, true)
        .expect_err("invalid fixture is rejected before any write");
    assert!(error.contains("invalid slug"), "got: {error}");

    let edited: (i64, Option<String>) = connection
        .query_row(
            "SELECT published, slug FROM provinces WHERE id = 'gubernia-1897-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(edited, (1, Some("one".to_string())));
    let posts: i64 = connection
        .query_row("SELECT COUNT(*) FROM posts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(posts, 2, "fixture content still present");
    let districts: i64 = connection
        .query_row("SELECT COUNT(*) FROM districts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(districts, 77);
    assert!(
        db::imported_at(&connection).unwrap().is_some(),
        "marker preserved"
    );
}

#[tokio::test]
async fn real_repository_data_imports_faithfully() {
    let root = common::repo_root();
    let data_dir = root.join("data");
    let public_dir = root.join("public/data");
    let publication_path = data_dir.join("gubernia-publications.json");
    if !publication_path.exists() || !public_dir.join("gubernias.geojson").exists() {
        eprintln!("skipping real-data test: repository data is not available");
        return;
    }

    let source: Value =
        serde_json::from_str(&std::fs::read_to_string(&publication_path).unwrap()).unwrap();
    let source_settings: Value =
        serde_json::from_str(&std::fs::read_to_string(data_dir.join("site-settings.json")).unwrap())
            .unwrap();
    let source_references: Value = serde_json::from_str(
        &std::fs::read_to_string(data_dir.join("settlement-references.json")).unwrap(),
    )
    .unwrap();

    let entries = source["gubernias"].as_object().unwrap();
    let published_count = entries
        .values()
        .filter(|entry| entry["published"] == true)
        .count();
    let posts_count: usize = entries
        .values()
        .map(|entry| entry["posts"].as_array().map(Vec::len).unwrap_or(0))
        .sum();
    let settlements_count: usize = entries
        .values()
        .map(|entry| entry["settlements"].as_array().map(Vec::len).unwrap_or(0))
        .sum();

    // Live atlas snapshot: ten published gubernias, seven posts, four
    // settlements, one reference and the custom settings lists.
    assert_eq!(published_count, 10, "live published provinces");
    assert_eq!(posts_count, 7, "live posts");
    assert_eq!(settlements_count, 4, "live settlements");
    assert_eq!(
        source_references["references"].as_object().unwrap().len(),
        1
    );
    assert_eq!(source_settings["categories"].as_array().unwrap().len(), 4);
    assert_eq!(
        source_settings["settlementTypes"].as_array().unwrap().len(),
        6
    );

    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        database_path: dir.path().join("db.sqlite"),
        data_dir: data_dir.clone(),
        import_dir: data_dir.clone(),
        public_data_dir: public_dir.clone(),
        bind_addr: "127.0.0.1:0".to_string(),
        frontend_origin: Some("http://localhost:3000".to_string()),
        admin_username: Some("admin".to_string()),
        admin_password: Some("secret".to_string()),
        admin_session_secret: Some("0123456789abcdef0123456789abcdef".to_string()),
        // The live-data fixture test does not touch image uploads.
        s3_endpoint: None,
        s3_region: None,
        s3_bucket: None,
        s3_access_key_id: None,
        s3_secret_access_key: None,
        fresh_install: false,
    };
    let state = prepare(&config).expect("real data imports");
    let app = build_router(state);
    let cookie = admin_cookie(&app).await;

    let geo = send(&app, request(Method::GET, "/internal/geo", None, None)).await;
    assert_eq!(
        geo.body["provinces"].as_array().unwrap().len(),
        published_count
    );
    assert_eq!(
        geo.body["settlements"].as_array().unwrap().len(),
        settlements_count
    );

    let metrics = send(
        &app,
        request(Method::GET, "/internal/metrics", Some(&cookie), None),
    )
    .await;
    assert_eq!(metrics.body["totalHistoricalGubernias"], 76);
    assert_eq!(metrics.body["publishedCount"], published_count);
    assert_eq!(metrics.body["totalPublishedPosts"], posts_count as i64);
    assert_eq!(
        metrics.body["totalPublishedSettlements"],
        settlements_count as i64
    );

    let settings = send(&app, request(Method::GET, "/internal/settings", None, None)).await;
    assert_eq!(settings.body["categories"], source_settings["categories"]);
    assert_eq!(
        settings.body["settlementTypes"],
        source_settings["settlementTypes"]
    );

    // RiStat district registry: 703 rows whose (id, province) pairs match the
    // canonical district files exactly.
    let district_dir = public_dir.join("uyezds-1897");
    let mut expected_districts: Vec<(String, String)> = Vec::new();
    for entry in std::fs::read_dir(&district_dir).expect("district directory") {
        let path = entry.expect("district entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("geojson") {
            continue;
        }
        let value: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let features = value["features"].as_array().expect("features");
        let province_id = features[0]["properties"]["id"]
            .as_str()
            .expect("province id")
            .to_string();
        for feature in &features[1..] {
            if feature["properties"]["kind"] != "district" {
                continue;
            }
            expected_districts.push((
                feature["properties"]["id"].as_str().unwrap().to_string(),
                province_id.clone(),
            ));
        }
    }
    expected_districts.sort();
    assert_eq!(expected_districts.len(), 703, "live RiStat district count");

    let connection = db::open_database(&config.database_path).expect("database opens");
    let imported_districts: Vec<(String, String)> = {
        let mut statement = connection
            .prepare("SELECT id, province_id FROM districts")
            .unwrap();
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap();
        rows.map(|row| row.unwrap()).collect()
    };
    let mut imported_districts = imported_districts;
    imported_districts.sort();
    assert_eq!(imported_districts, expected_districts);
    let empty_names: i64 = connection
        .query_row("SELECT COUNT(*) FROM districts WHERE name = ''", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(empty_names, 0);

    // Ryazan holds the only content; legacy posts and settlements stay intact.
    let ryazan_source = &source["gubernias"]["ryazan"];
    let ryazan = send(
        &app,
        request(Method::GET, "/internal/gubernia/ryazanskaya", None, None),
    )
    .await;
    assert_eq!(ryazan.status, StatusCode::OK);
    let source_posts = ryazan_source["posts"].as_array().unwrap();
    let source_settlements = ryazan_source["settlements"].as_array().unwrap();
    assert_eq!(ryazan.body["posts"].as_array().unwrap().len(), source_posts.len());
    assert_eq!(
        ryazan.body["settlements"].as_array().unwrap().len(),
        source_settlements.len()
    );
    for (index, source_post) in source_posts.iter().enumerate() {
        let imported = &ryazan.body["posts"][index];
        assert_eq!(imported["id"], source_post["id"]);
        assert_eq!(imported["title"], source_post["title"]);
        assert_eq!(imported["body"], source_post["body"]);
        assert_eq!(imported["createdAt"], source_post["createdAt"]);
        assert_eq!(imported["updatedAt"], source_post["updatedAt"]);
        match source_post.get("category") {
            None | Some(Value::Null) => assert_eq!(
                imported["category"],
                Value::Null,
                "legacy category stays null"
            ),
            Some(category) => assert_eq!(imported["category"], *category),
        }
    }
    for (index, source_settlement) in source_settlements.iter().enumerate() {
        let imported = &ryazan.body["settlements"][index];
        assert_eq!(imported["id"], source_settlement["id"]);
        assert_eq!(imported["name"], source_settlement["name"]);
        assert_eq!(imported["createdAt"], source_settlement["createdAt"]);
        let expected_url = match source_settlement.get("url").and_then(Value::as_str) {
            Some(url) => url.to_string(),
            None => format!(
                "/naselennyy-punkt/{}",
                source_settlement["id"].as_str().unwrap()
            ),
        };
        assert_eq!(imported["url"], expected_url);
        match source_settlement.get("type") {
            None | Some(Value::Null) => assert_eq!(imported["type"], Value::Null),
            Some(settlement_type) => assert_eq!(imported["type"], *settlement_type),
        }
    }

    // Reference blocks follow settlement publication: published settlements
    // are public, withdrawn ones are admin-only, and stored rows always survive.
    let published_settlement_ids: Vec<&str> = geo.body["settlements"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|settlement| settlement["id"].as_str())
        .collect();
    for (settlement_id, body) in source_references["references"].as_object().unwrap() {
        let anonymous = send(
            &app,
            request(
                Method::GET,
                &format!("/internal/settlement-reference/{settlement_id}"),
                None,
                None,
            ),
        )
        .await;
        let admin = send(
            &app,
            request(
                Method::GET,
                &format!("/internal/settlement-reference/{settlement_id}"),
                Some(&cookie),
                None,
            ),
        )
        .await;
        assert_eq!(admin.status, StatusCode::OK, "stored rows survive");
        assert_eq!(admin.body["body"], *body);
        if published_settlement_ids.contains(&settlement_id.as_str()) {
            assert_eq!(anonymous.status, StatusCode::OK);
            assert_eq!(anonymous.body["body"], *body);
        } else {
            assert_eq!(
                anonymous.status,
                StatusCode::NOT_FOUND,
                "withdrawn settlement references are admin-only"
            );
        }
    }

    // Re-running against the same database keeps every record.
    prepare(&config).expect("idempotent restart");
    let app2 = build_router(prepare(&config).expect("idempotent prepare"));
    let metrics_again = send(
        &app2,
        request(Method::GET, "/internal/metrics", Some(&cookie), None),
    )
    .await;
    assert_eq!(metrics_again.body, metrics.body);
}

#[tokio::test]
async fn fixture_publication_state_imports_every_canonical_entry() {
    let dir = standard_fixture_dir();
    let config = base_config(dir.path());
    let collection = canonical(&config.public_data_dir);
    let geo = GeoRuntime::new(&config.public_data_dir);
    let mut connection = db::open_database(&config.database_path).unwrap();
    db::migrate(&mut connection).unwrap();
    run_import_command(&mut connection, &config, &collection, &geo, false).unwrap();

    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM provinces", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows as usize, canonical_ids().len());
    let migration_version: i64 = connection
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(migration_version, 4);

    // The district registry mirrors the canonical district files: all 76
    // provinces ship a file in the fixture (ryazan has two districts).
    let total_districts: i64 = connection
        .query_row("SELECT COUNT(*) FROM districts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(total_districts, 77);
    let provinces_with_districts: i64 = connection
        .query_row("SELECT COUNT(DISTINCT province_id) FROM districts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(provinces_with_districts, 76);
    let ryazan_and_tula: Vec<(String, String, String)> = {
        let mut statement = connection
            .prepare(
                "SELECT id, province_id, name FROM districts\n\
                 WHERE province_id IN ('ryazan', 'tula')\n\
                 ORDER BY province_id, position",
            )
            .unwrap();
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap();
        rows.map(|row| row.unwrap()).collect()
    };
    assert_eq!(
        ryazan_and_tula,
        vec![
            (
                "uyezd-1897-1".to_string(),
                "ryazan".to_string(),
                "Скопинский уезд".to_string()
            ),
            (
                "uyezd-1897-2".to_string(),
                "ryazan".to_string(),
                "Ряжский уезд".to_string()
            ),
            (
                "uyezd-1897-9".to_string(),
                "tula".to_string(),
                "Тульский уезд".to_string()
            ),
        ]
    );
    let published: i64 = connection
        .query_row("SELECT COUNT(*) FROM provinces WHERE published = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(published, 2);
    let posts: i64 = connection
        .query_row("SELECT COUNT(*) FROM posts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(posts, 2);
    let settlements: i64 = connection
        .query_row("SELECT COUNT(*) FROM settlements", [], |row| row.get(0))
        .unwrap();
    assert_eq!(settlements, 1);
    let references: i64 = connection
        .query_row("SELECT COUNT(*) FROM settlement_references", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(references, 1);

    // Legacy storage keeps NULL url/type/category until a write persists them.
    let legacy_settlement: (Option<String>, Option<String>) = connection
        .query_row("SELECT url, type FROM settlements", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(legacy_settlement, (None, None));
    let legacy_category: Option<String> = connection
        .query_row(
            "SELECT category FROM posts WHERE id = ?1",
            rusqlite::params![common::LEGACY_POST_ID],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(legacy_category, None);
}
