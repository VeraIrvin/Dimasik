mod common;

use axum::http::{HeaderValue, Method, StatusCode};
use axum::Router;
use rusqlite::OptionalExtension;
use serde_json::{json, Value};

use common::{
    admin_cookie, login, method, paragraph_doc, raw_request, request, send, TestEnv, TestResponse,
    LEGACY_POST_ID, LEGACY_SETTLEMENT_ID,
};

async fn api(
    app: &Router,
    http_method: Method,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> TestResponse {
    send(app, request(http_method, uri, cookie, body.as_ref())).await
}

/// Inserts a pending upload row exactly as the upload route does once the
/// storage objects exist, so the image lifecycle can be tested without S3.
fn insert_pending_image(
    database: &std::path::Path,
    id: &str,
    width: i64,
    height: i64,
    created_ms: i64,
) {
    let connection = dimasik_backend::db::open_database(database).expect("database opens");
    connection
        .execute(
            "INSERT INTO post_images (id, post_id, original_key, thumbnail_key, width, height, position, created_at, created_ms, attached_ms)\n\
             VALUES (?1, NULL, ?2, ?3, ?4, ?5, 0, '2026-10-09T00:00:00.000Z', ?6, NULL)",
            rusqlite::params![
                id,
                format!("post-images/{id}/original"),
                format!("post-images/{id}/thumbnail"),
                width,
                height,
                created_ms,
            ],
        )
        .expect("pending image row");
}

/// Reads one stored image row as `(post_id, attached_ms, original_key)`; the
/// key has to survive until the storage deletion is confirmed.
fn image_row(
    database: &std::path::Path,
    id: &str,
) -> Option<(Option<String>, Option<i64>, String)> {
    let connection = dimasik_backend::db::open_database(database).expect("database opens");
    connection
        .query_row(
            "SELECT post_id, attached_ms, original_key FROM post_images WHERE id = ?1",
            rusqlite::params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .expect("image row query")
}

/// A rich province description: a formatted paragraph with a link whose
/// `target` is validated but never persisted, a deliberately blank middle
/// paragraph and a list, so canonicalisation and blank lines stay covered.
fn rich_description() -> Value {
    json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "Справочные сведения",
                    "marks": [
                        { "type": "bold" },
                        { "type": "link", "attrs": { "href": "https://example.com", "target": "_blank" } }
                    ]
                }]
            },
            { "type": "paragraph" },
            {
                "type": "heading",
                "attrs": { "level": 2, "textAlign": "center" },
                "content": [{ "type": "text", "text": "Заголовок" }]
            },
            {
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "Пункт" }]
                    }]
                }]
            }
        ]
    })
}

/// The stored form of [`rich_description`]: `target` is dropped by the same
/// normalisation the post bodies use.
fn canonical_description() -> Value {
    let mut document = rich_description();
    document["content"][0]["content"][0]["marks"] = json!([
        { "type": "bold" },
        { "type": "link", "attrs": { "href": "https://example.com" } }
    ]);
    document
}

#[tokio::test]
async fn bootstrap_preserves_fixture_state_and_legacy_shapes() {
    let env = TestEnv::new();
    let app = env.app.clone();

    let collection = api(&app, Method::GET, "/api/gubernias", None, None).await;
    assert_eq!(collection.status, StatusCode::OK);
    assert_eq!(collection.cache_control(), Some("no-store"));
    assert_eq!(collection.body["type"], "FeatureCollection");
    let features = collection.body["features"].as_array().expect("features");
    assert_eq!(features.len(), 76);
    let ryazan = features
        .iter()
        .find(|feature| feature["properties"]["id"] == "ryazan")
        .expect("ryazan feature");
    assert_eq!(ryazan["properties"]["published"], true);
    assert_eq!(ryazan["properties"]["slug"], "ryazanskaya");
    assert!(ryazan["properties"]["labelCoordinates"].is_array());
    assert!(ryazan["geometry"]["coordinates"].is_array());
    let unpublished = features
        .iter()
        .find(|feature| feature["properties"]["id"] == "gubernia-1897-1")
        .expect("numbered feature");
    assert_eq!(unpublished["properties"]["published"], false);
    assert!(unpublished["properties"].get("slug").is_none());

    let gubernia = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    assert_eq!(gubernia.status, StatusCode::OK);
    assert_eq!(gubernia.body["id"], "ryazan");
    assert_eq!(gubernia.body["name"], "Рязанская губерния");
    let posts = gubernia.body["posts"].as_array().expect("posts");
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0]["id"], LEGACY_POST_ID);
    assert_eq!(posts[0]["category"], Value::Null, "legacy category stays null");
    assert_eq!(posts[0]["body"], paragraph_doc("Текст первый"));
    assert_eq!(posts[0]["createdAt"], "2026-10-07T10:00:00.000Z");
    assert_eq!(posts[0]["updatedAt"], "2026-10-07T10:00:00.000Z");
    assert_eq!(posts[1]["id"], common::OLDER_POST_ID);
    assert_eq!(posts[1]["archiveReference"], "Ф. 1, оп. 2");
    assert_eq!(posts[1]["settlementId"], LEGACY_SETTLEMENT_ID);

    let settlements = gubernia.body["settlements"].as_array().expect("settlements");
    assert_eq!(settlements.len(), 1);
    assert_eq!(settlements[0]["id"], LEGACY_SETTLEMENT_ID);
    assert_eq!(
        settlements[0]["url"],
        format!("/naselennyy-punkt/{LEGACY_SETTLEMENT_ID}"),
        "legacy url is derived"
    );
    assert_eq!(settlements[0]["type"], Value::Null, "legacy type stays null");
    assert_eq!(settlements[0]["latitude"], 5.0);
    assert_eq!(settlements[0]["createdAt"], "2026-10-06T09:00:00.000Z");

    let geo = api(&app, Method::GET, "/internal/geo", None, None).await;
    assert_eq!(geo.status, StatusCode::OK);
    let provinces = geo.body["provinces"].as_array().expect("provinces");
    assert_eq!(provinces.len(), 2);
    assert_eq!(provinces[0]["id"], "ryazan");
    assert_eq!(provinces[1]["id"], "tula");
    assert_eq!(geo.body["settlements"].as_array().unwrap().len(), 1);

    let settings = api(&app, Method::GET, "/internal/settings", None, None).await;
    assert_eq!(
        settings.body,
        json!({ "categories": ["Статья", "Очерк"], "settlementTypes": ["Город", "Село", "Погост"] })
    );

    let about = api(&app, Method::GET, "/internal/about", None, None).await;
    assert_eq!(about.body["body"], paragraph_doc("О проекте Dimasik"));

    let reference = api(
        &app,
        Method::GET,
        &format!("/internal/settlement-reference/{LEGACY_SETTLEMENT_ID}"),
        None,
        None,
    )
    .await;
    assert_eq!(reference.status, StatusCode::OK);
    assert_eq!(reference.body["body"], paragraph_doc("Справка о Павельце"));

    let settlement = api(
        &app,
        Method::GET,
        &format!("/internal/settlement/{LEGACY_SETTLEMENT_ID}"),
        None,
        None,
    )
    .await;
    assert_eq!(settlement.status, StatusCode::OK);
    assert_eq!(settlement.body["settlement"]["name"], "Павелец");
    assert_eq!(settlement.body["gubernia"]["slug"], "ryazanskaya");
}

#[tokio::test]
async fn internal_reads_and_missing_resources() {
    let env = TestEnv::new();
    let app = env.app.clone();

    let session = api(&app, Method::GET, "/internal/session", None, None).await;
    assert_eq!(session.status, StatusCode::OK);
    assert_eq!(session.body, json!({ "isAdmin": false }));

    assert_eq!(
        api(&app, Method::GET, "/internal/gubernia/missing", None, None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        api(&app, Method::GET, "/internal/settlement/missing", None, None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        api(
            &app,
            Method::GET,
            "/internal/settlement-reference/missing",
            None,
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );

    let metrics = api(&app, Method::GET, "/internal/metrics", None, None).await;
    assert_eq!(metrics.status, StatusCode::UNAUTHORIZED);
    assert_eq!(metrics.error_message(), "Требуется вход администратора.");

    let cookie = admin_cookie(&app).await;
    let metrics = api(&app, Method::GET, "/internal/metrics", Some(&cookie), None).await;
    assert_eq!(metrics.status, StatusCode::OK);
    assert_eq!(metrics.body["totalHistoricalGubernias"], 76);
    assert_eq!(metrics.body["publishedCount"], 2);
    assert_eq!(metrics.body["totalPublishedPosts"], 2);
    assert_eq!(metrics.body["totalPublishedSettlements"], 1);
    let provinces = metrics.body["provinces"].as_array().expect("provinces");
    assert_eq!(provinces[0]["name"], "Рязанская губерния");
    assert_eq!(provinces[0]["postsCount"], 2);
    assert_eq!(provinces[0]["settlementsCount"], 1);
    assert_eq!(provinces[1]["name"], "Тульская губерния");
}

#[tokio::test]
async fn auth_guards_mutations_with_session_content_type_and_origin() {
    let env = TestEnv::new();
    let app = env.app.clone();

    let unauthorized = api(
        &app,
        Method::POST,
        "/api/gubernias",
        None,
        Some(json!({ "id": "tula", "slug": "other" })),
    )
    .await;
    assert_eq!(unauthorized.status, StatusCode::UNAUTHORIZED);
    assert_eq!(unauthorized.error_message(), "Требуется вход администратора.");
    assert_eq!(unauthorized.cache_control(), Some("no-store"));

    let cookie = admin_cookie(&app).await;

    let wrong_type = send(
        &app,
        raw_request(
            Method::POST,
            "/api/gubernias",
            Some(&cookie),
            Some("text/plain"),
            "{}",
        ),
    )
    .await;
    assert_eq!(wrong_type.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(wrong_type.error_message(), "Ожидается JSON-запрос.");

    let invalid_json = send(
        &app,
        raw_request(
            Method::POST,
            "/api/gubernias",
            Some(&cookie),
            Some("application/json"),
            "{not json",
        ),
    )
    .await;
    assert_eq!(invalid_json.status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid_json.error_message(), "Неверный формат запроса.");

    let mut forbidden = request(
        Method::POST,
        "/api/gubernias",
        Some(&cookie),
        Some(&json!({ "id": "gubernia-1897-1", "slug": "one" })),
    );
    forbidden
        .headers_mut()
        .insert("origin", HeaderValue::from_static("http://evil.example"));
    let forbidden = send(&app, forbidden).await;
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
    assert_eq!(forbidden.error_message(), "Запрос отклонён.");

    let mut allowed = request(
        Method::POST,
        "/api/gubernias",
        Some(&cookie),
        Some(&json!({ "id": "gubernia-1897-1", "slug": "one" })),
    );
    allowed.headers_mut().insert(
        "origin",
        HeaderValue::from_static("http://localhost:3000"),
    );
    let allowed = send(&app, allowed).await;
    assert_eq!(allowed.status, StatusCode::CREATED);
    assert_eq!(allowed.body["slug"], "one");
    assert_eq!(allowed.body["name"], "Губерния 1");
}

#[tokio::test]
async fn login_logout_and_session_semantics() {
    let env = TestEnv::new();
    let app = env.app.clone();

    let wrong = login(&app, "admin", "nope").await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong.error_message(), "Неверный логин или пароль.");

    let malformed = send(
        &app,
        raw_request(
            Method::POST,
            "/api/admin/session",
            None,
            Some("application/json"),
            "not json",
        ),
    )
    .await;
    assert_eq!(malformed.status, StatusCode::BAD_REQUEST);
    assert_eq!(malformed.error_message(), "Неверный формат запроса.");

    let array_body = send(
        &app,
        raw_request(Method::POST, "/api/admin/session", None, None, "[]"),
    )
    .await;
    assert_eq!(array_body.status, StatusCode::UNAUTHORIZED);

    // No content type is required for the session endpoint.
    let ok = send(
        &app,
        raw_request(
            Method::POST,
            "/api/admin/session",
            None,
            None,
            "{\"login\":\"admin\",\"password\":\"secret\"}",
        ),
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK);
    assert_eq!(ok.body, json!({ "authenticated": true }));
    let cookie_header = ok.set_cookie().expect("set-cookie");
    assert!(cookie_header.contains("HttpOnly"));
    assert!(cookie_header.contains("SameSite=Strict"));
    assert!(cookie_header.contains("Path=/"));
    assert!(cookie_header.contains("Max-Age=28800"));
    assert!(!cookie_header.contains("Secure"));

    let cookie = cookie_header.split(';').next().unwrap().to_string();
    let session = api(&app, Method::GET, "/internal/session", Some(&cookie), None).await;
    assert_eq!(session.body, json!({ "isAdmin": true }));

    let logout = api(&app, Method::DELETE, "/api/admin/session", Some(&cookie), None).await;
    assert_eq!(logout.status, StatusCode::OK);
    assert_eq!(logout.body, json!({ "authenticated": false }));
    assert!(logout.set_cookie().unwrap().contains("Max-Age=0"));
}

#[tokio::test]
async fn login_requires_configured_credentials_and_secret() {
    let dir = common::standard_fixture_dir();
    let mut config = common::base_config(dir.path());
    config.admin_session_secret = Some("too-short".to_string());
    let app = TestEnv::router_for_config(&config);
    let response = login(&app, "admin", "secret").await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.error_message(), "Вход временно недоступен.");

    let mut config = common::base_config(dir.path());
    config.admin_username = None;
    config.admin_password = None;
    let app = TestEnv::router_for_config(&config);
    let response = login(&app, "admin", "secret").await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn gubernia_publication_lifecycle() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;

    let already = api(
        &app,
        Method::POST,
        "/api/gubernias",
        Some(&cookie),
        Some(json!({ "id": "ryazan", "slug": "other" })),
    )
    .await;
    assert_eq!(already.status, StatusCode::CONFLICT);
    assert_eq!(already.error_message(), "Губерния уже опубликована.");

    let unknown = api(
        &app,
        Method::POST,
        "/api/gubernias",
        Some(&cookie),
        Some(json!({ "id": "missing", "slug": "missing" })),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.error_message(), "Губерния не найдена.");

    let bad_id = api(
        &app,
        Method::POST,
        "/api/gubernias",
        Some(&cookie),
        Some(json!({ "id": "", "slug": "x" })),
    )
    .await;
    assert_eq!(bad_id.status, StatusCode::BAD_REQUEST);
    assert_eq!(bad_id.error_message(), "Некорректный идентификатор губернии.");

    let bad_slug = api(
        &app,
        Method::POST,
        "/api/gubernias",
        Some(&cookie),
        Some(json!({ "id": "gubernia-1897-1", "slug": "Bad Slug" })),
    )
    .await;
    assert_eq!(bad_slug.status, StatusCode::BAD_REQUEST);
    assert!(bad_slug.error_message().starts_with("Slug должен содержать"));

    let published = api(
        &app,
        Method::POST,
        "/api/gubernias",
        Some(&cookie),
        Some(json!({ "id": "gubernia-1897-1", "slug": "one" })),
    )
    .await;
    assert_eq!(published.status, StatusCode::CREATED);

    let duplicate_slug = api(
        &app,
        Method::POST,
        "/api/gubernias",
        Some(&cookie),
        Some(json!({ "id": "gubernia-1897-2", "slug": "one" })),
    )
    .await;
    assert_eq!(duplicate_slug.status, StatusCode::CONFLICT);
    assert_eq!(
        duplicate_slug.error_message(),
        "Этот slug уже используется другой губернией."
    );

    let updated = api(
        &app,
        Method::PATCH,
        "/api/gubernias/gubernia-1897-1",
        Some(&cookie),
        Some(json!({ "slug": "one-updated", "description": paragraph_doc("Описание") })),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_eq!(updated.body["slug"], "one-updated");
    assert_eq!(updated.body["description"], paragraph_doc("Описание"));

    // The clean cutover has no string shim: plain text is not a document.
    let legacy_string = api(
        &app,
        Method::PATCH,
        "/api/gubernias/gubernia-1897-1",
        Some(&cookie),
        Some(json!({ "slug": "one-updated", "description": "Описание" })),
    )
    .await;
    assert_eq!(legacy_string.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        legacy_string.error_message(),
        "Некорректное содержимое описания."
    );

    let unpublished = api(
        &app,
        Method::DELETE,
        "/api/gubernias/gubernia-1897-1",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(unpublished.status, StatusCode::NO_CONTENT);
    let gone = api(&app, Method::GET, "/internal/gubernia/one-updated", None, None).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    let again = api(
        &app,
        Method::DELETE,
        "/api/gubernias/gubernia-1897-1",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    assert_eq!(again.error_message(), "Губерния не опубликована.");
}

#[tokio::test]
async fn province_description_documents_survive_restart_and_embed_in_snapshots() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;

    // Imported provinces start with no description at all.
    let before = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    assert_eq!(before.status, StatusCode::OK);
    assert_eq!(before.body["description"], Value::Null);

    let updated = api(
        &app,
        Method::PATCH,
        "/api/gubernias/ryazan",
        Some(&cookie),
        Some(json!({ "slug": "ryazanskaya", "description": rich_description() })),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_eq!(updated.body["description"], canonical_description());

    // The public snapshot that feeds the province page returns the document.
    let public = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    assert_eq!(public.status, StatusCode::OK);
    assert_eq!(public.body["description"], canonical_description());

    // The settlement snapshot embeds the same province document.
    let settlement = api(
        &app,
        Method::GET,
        &format!("/internal/settlement/{LEGACY_SETTLEMENT_ID}"),
        None,
        None,
    )
    .await;
    assert_eq!(settlement.status, StatusCode::OK);
    assert_eq!(
        settlement.body["gubernia"]["description"],
        canonical_description()
    );

    // A restart reads the stored document back unchanged.
    env.prepare_again().expect("restart is idempotent");
    let restarted = TestEnv::router_for_config(&env.config);
    let after = api(&restarted, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    assert_eq!(after.status, StatusCode::OK);
    assert_eq!(after.body["description"], canonical_description());
}

#[tokio::test]
async fn province_description_rejects_unsafe_and_oversized_documents() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;
    let url = "/api/gubernias/ryazan";

    let unsafe_node = api(
        &app,
        Method::PATCH,
        url,
        Some(&cookie),
        Some(json!({
            "slug": "ryazanskaya",
            "description": {
                "type": "doc",
                "content": [{ "type": "image", "src": "https://example.com/x.png" }]
            }
        })),
    )
    .await;
    assert_eq!(unsafe_node.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        unsafe_node.error_message(),
        "Некорректное содержимое описания."
    );

    let unsafe_link = api(
        &app,
        Method::PATCH,
        url,
        Some(&cookie),
        Some(json!({
            "slug": "ryazanskaya",
            "description": {
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{
                    "type": "text",
                    "text": "x",
                    "marks": [{ "type": "link", "attrs": { "href": "javascript:alert(1)" } }]
                }]}]
            }
        })),
    )
    .await;
    assert_eq!(unsafe_link.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        unsafe_link.error_message(),
        "Некорректное содержимое описания."
    );

    // Omitting the field was rejected before the cutover and stays rejected,
    // so a slug-only edit can never silently wipe the description.
    let missing = api(
        &app,
        Method::PATCH,
        url,
        Some(&cookie),
        Some(json!({ "slug": "ryazanskaya" })),
    )
    .await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST);
    assert_eq!(missing.error_message(), "Некорректное содержимое описания.");

    // Bodies beyond the document bound are refused before validation.
    let oversized = api(
        &app,
        Method::PATCH,
        url,
        Some(&cookie),
        Some(json!({
            "slug": "ryazanskaya",
            "description": {
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "x".repeat(620_000) }]
                }]
            }
        })),
    )
    .await;
    assert_eq!(oversized.status, StatusCode::BAD_REQUEST);
    assert_eq!(oversized.error_message(), "Тело запроса слишком большое.");

    // Rejected writes never touched the stored state.
    let stored = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    assert_eq!(stored.body["description"], Value::Null);

    // A blank document and `null` both clear the field.
    let set = api(
        &app,
        Method::PATCH,
        url,
        Some(&cookie),
        Some(json!({ "slug": "ryazanskaya", "description": rich_description() })),
    )
    .await;
    assert_eq!(set.status, StatusCode::OK);
    assert_eq!(set.body["description"], canonical_description());

    let cleared_with_blank = api(
        &app,
        Method::PATCH,
        url,
        Some(&cookie),
        Some(json!({
            "slug": "ryazanskaya",
            "description": { "type": "doc", "content": [] }
        })),
    )
    .await;
    assert_eq!(cleared_with_blank.status, StatusCode::OK);
    assert_eq!(cleared_with_blank.body["description"], Value::Null);

    let set_again = api(
        &app,
        Method::PATCH,
        url,
        Some(&cookie),
        Some(json!({ "slug": "ryazanskaya", "description": rich_description() })),
    )
    .await;
    assert_eq!(set_again.body["description"], canonical_description());

    let cleared_with_null = api(
        &app,
        Method::PATCH,
        url,
        Some(&cookie),
        Some(json!({ "slug": "ryazanskaya", "description": null })),
    )
    .await;
    assert_eq!(cleared_with_null.status, StatusCode::OK);
    assert_eq!(cleared_with_null.body["description"], Value::Null);
}

#[tokio::test]
async fn unpublish_clears_content_and_metrics_but_keeps_orphan_references() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;

    let before = api(&app, Method::GET, "/internal/metrics", Some(&cookie), None).await;
    assert_eq!(before.body["totalPublishedPosts"], 2);

    let response = api(
        &app,
        Method::DELETE,
        "/api/gubernias/ryazan",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);

    let metrics = api(&app, Method::GET, "/internal/metrics", Some(&cookie), None).await;
    assert_eq!(metrics.body["publishedCount"], 1);
    assert_eq!(metrics.body["totalPublishedPosts"], 0);
    assert_eq!(metrics.body["totalPublishedSettlements"], 0);

    assert_eq!(
        api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    // The reference row survives unpublishing, but it is withdrawn from public
    // reads: anonymous callers get 404, administrators still see the block.
    let anonymous = api(
        &app,
        Method::GET,
        &format!("/internal/settlement-reference/{LEGACY_SETTLEMENT_ID}"),
        None,
        None,
    )
    .await;
    assert_eq!(anonymous.status, StatusCode::NOT_FOUND);
    let admin = api(
        &app,
        Method::GET,
        &format!("/internal/settlement-reference/{LEGACY_SETTLEMENT_ID}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(admin.status, StatusCode::OK);
    assert_eq!(admin.body["body"], paragraph_doc("Справка о Павельце"));
}

#[tokio::test]
async fn post_crud_placement_ordering_and_moves() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;

    let created = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "  Новый  ",
            "body": paragraph_doc("Тело"),
            "category": "Статья",
            "settlementId": LEGACY_SETTLEMENT_ID,
            "year": " 1910 ",
            "archiveReference": ""
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let post_id = created.body["id"].as_str().unwrap().to_string();
    assert_eq!(created.body["title"], "Новый");
    assert_eq!(created.body["uyezdId"], "uyezd-1897-1");
    assert_eq!(created.body["settlementId"], LEGACY_SETTLEMENT_ID);
    assert_eq!(created.body["year"], "1910");
    assert_eq!(created.body["category"], "Статья");

    // Newest first: the new post heads the list.
    let gubernia = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    assert_eq!(gubernia.body["posts"][0]["id"], post_id);

    let mismatch = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Новый",
            "body": paragraph_doc("Тело"),
            "category": "Статья",
            "settlementId": LEGACY_SETTLEMENT_ID,
            "uyezdId": "uyezd-1897-2"
        })),
    )
    .await;
    assert_eq!(mismatch.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        mismatch.error_message(),
        "Уезд не соответствует выбранному населённому пункту."
    );

    let foreign_settlement = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Новый",
            "body": paragraph_doc("Тело"),
            "category": "Статья",
            "settlementId": "missing"
        })),
    )
    .await;
    assert_eq!(
        foreign_settlement.error_message(),
        "Населённый пункт не найден в этой губернии."
    );

    let district_only = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Новый",
            "body": paragraph_doc("Тело"),
            "category": "Статья",
            "uyezdId": "uyezd-1897-2"
        })),
    )
    .await;
    assert_eq!(district_only.status, StatusCode::OK);
    assert_eq!(district_only.body["uyezdId"], "uyezd-1897-2");
    assert_eq!(district_only.body["settlementId"], Value::Null);

    let bad_category = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Новый",
            "body": paragraph_doc("Тело"),
            "category": "Прочее"
        })),
    )
    .await;
    assert_eq!(bad_category.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        bad_category.error_message(),
        "Категория должна быть одной из: «Статья», «Очерк»."
    );

    let javascript_link = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Ссылка",
            "body": {
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": "клик",
                        "marks": [{ "type": "link", "attrs": { "href": "javascript:alert(1)" } }]
                    }]
                }]
            },
            "category": "Статья"
        })),
    )
    .await;
    assert_eq!(javascript_link.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        javascript_link.error_message(),
        "Некорректное содержимое публикации."
    );

    let unknown_node = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Узел",
            "body": { "type": "doc", "content": [{ "type": "image", "src": "x" }] },
            "category": "Статья"
        })),
    )
    .await;
    assert_eq!(unknown_node.status, StatusCode::BAD_REQUEST);

    let empty_document = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Пусто",
            "body": { "type": "doc", "content": [] },
            "category": "Статья"
        })),
    )
    .await;
    assert_eq!(
        empty_document.error_message(),
        "Текст публикации не должен быть пустым."
    );

    // Move to another published province; createdAt survives.
    let moved = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Новый",
            "body": paragraph_doc("Тело"),
            "category": "Статья",
            "targetGuberniaId": "tula",
            "uyezdId": "uyezd-1897-9"
        })),
    )
    .await;
    assert_eq!(moved.status, StatusCode::OK);
    assert_eq!(moved.body["id"], post_id);
    assert_eq!(
        moved.body["createdAt"], created.body["createdAt"],
        "createdAt is preserved across moves"
    );

    let ryazan = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    assert_eq!(ryazan.body["posts"].as_array().unwrap().len(), 2);
    let tula = api(&app, Method::GET, "/internal/gubernia/tulskaya", None, None).await;
    assert_eq!(tula.body["posts"].as_array().unwrap().len(), 1);
    assert_eq!(tula.body["posts"][0]["id"], post_id);

    let delete_from_wrong_province = api(
        &app,
        Method::DELETE,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(delete_from_wrong_province.status, StatusCode::NOT_FOUND);
    assert_eq!(
        delete_from_wrong_province.error_message(),
        "Публикация не найдена."
    );

    let deleted = api(
        &app,
        Method::DELETE,
        &format!("/api/gubernias/tula/posts/{post_id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let deleted_again = api(
        &app,
        Method::DELETE,
        &format!("/api/gubernias/tula/posts/{post_id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(deleted_again.status, StatusCode::NOT_FOUND);

    let unpublished_province = api(
        &app,
        Method::POST,
        "/api/gubernias/gubernia-1897-1/posts",
        Some(&cookie),
        Some(json!({ "title": "x", "body": paragraph_doc("y"), "category": "Статья" })),
    )
    .await;
    assert_eq!(unpublished_province.status, StatusCode::NOT_FOUND);
    assert_eq!(
        unpublished_province.error_message(),
        "Губерния не опубликована."
    );
}

/// Marks an unattached row for object cleanup (what the DELETE route does
/// before the storage objects are removed).
fn mark_image_for_cleanup(database: &std::path::Path, id: &str, attached_ms: i64) {
    let connection = dimasik_backend::db::open_database(database).expect("database opens");
    connection
        .execute(
            "UPDATE post_images SET attached_ms = ?1 WHERE id = ?2",
            rusqlite::params![attached_ms, id],
        )
        .expect("cleanup marker");
}

#[tokio::test]
async fn image_upload_requires_admin_and_configured_storage() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let boundary = "dimasik-boundary";
    let body = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"file\"; filename=\"photo.jpg\"\r\n\
         Content-Type: image/jpeg\r\n\
         \r\n\
         not-a-real-image\r\n\
         --{boundary}--\r\n"
    );
    let content_type = format!("multipart/form-data; boundary={boundary}");

    // Anonymous uploads are rejected before anything else happens.
    let anonymous = send(
        &app,
        raw_request(
            Method::POST,
            "/api/post-images",
            None,
            Some(&content_type),
            &body,
        ),
    )
    .await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);

    // With a session but without S3_* configuration the upload answers 503.
    let cookie = admin_cookie(&app).await;
    let unconfigured = send(
        &app,
        raw_request(
            Method::POST,
            "/api/post-images",
            Some(&cookie),
            Some(&content_type),
            &body,
        ),
    )
    .await;
    assert_eq!(unconfigured.status, StatusCode::SERVICE_UNAVAILABLE);

    // Unknown ids and unknown variants stay ordinary 404s without storage.
    let unknown_read = api(
        &app,
        Method::GET,
        "/api/post-images/00000000-0000-4000-8000-000000000000/original",
        None,
        None,
    )
    .await;
    assert_eq!(unknown_read.status, StatusCode::NOT_FOUND);
    let bad_variant = api(
        &app,
        Method::GET,
        "/api/post-images/00000000-0000-4000-8000-000000000000/medium",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(bad_variant.status, StatusCode::NOT_FOUND);
    let unknown_delete = api(
        &app,
        Method::DELETE,
        "/api/post-images/00000000-0000-4000-8000-000000000000",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(unknown_delete.status, StatusCode::NOT_FOUND);

    // Empty posts keep working without storage, images array included.
    let created = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Без изображений",
            "body": paragraph_doc("Текст"),
            "category": "Статья"
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(created.body["images"], json!([]));
}

#[tokio::test]
async fn post_creation_claims_pending_images_in_order() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;
    let database = env.config.database_path.clone();
    let now_ms = dimasik_backend::util::now_unix() * 1_000;
    insert_pending_image(&database, "img-second", 640, 480, now_ms);
    insert_pending_image(&database, "img-first", 1200, 900, now_ms);

    let created = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "С изображениями",
            "body": paragraph_doc("Текст"),
            "category": "Статья",
            "imageIds": ["img-second", "img-first"]
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let post_id = created.body["id"].as_str().unwrap().to_string();
    let images = created.body["images"].as_array().expect("images");
    assert_eq!(images.len(), 2);
    assert_eq!(images[0]["id"], "img-second");
    assert_eq!(images[1]["id"], "img-first");
    assert_eq!(images[0]["width"], 640);
    assert_eq!(images[0]["height"], 480);
    assert_eq!(images[0]["originalUrl"], "/api/post-images/img-second/original");
    assert_eq!(
        images[0]["thumbnailUrl"],
        "/api/post-images/img-second/thumbnail"
    );

    // The feed serializes the same DTO order, and legacy posts carry an empty
    // images array rather than a missing key.
    let gubernia = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    let posts = gubernia.body["posts"].as_array().expect("posts");
    let feed_post = posts
        .iter()
        .find(|post| post["id"] == post_id.as_str())
        .expect("created post");
    assert_eq!(feed_post["images"][0]["id"], "img-second");
    assert_eq!(feed_post["images"][1]["id"], "img-first");
    let legacy = posts
        .iter()
        .find(|post| post["id"] == LEGACY_POST_ID)
        .expect("legacy post");
    assert_eq!(legacy["images"], json!([]));
    let (post_ref, attached_ms, key) = image_row(&database, "img-first").expect("row");
    assert_eq!(post_ref.as_deref(), Some(post_id.as_str()));
    assert!(attached_ms.is_some());
    assert_eq!(key, "post-images/img-first/original");

    // Rejected claims never create a post (the fixture has exactly two, so
    // anything but three after a failure means a partial write).
    let post_count = |body: &Value| body["posts"].as_array().unwrap().len();
    let stolen = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Кража",
            "body": paragraph_doc("Текст"),
            "category": "Статья",
            "imageIds": ["img-second"]
        })),
    )
    .await;
    assert_eq!(stolen.status, StatusCode::CONFLICT);
    assert_eq!(
        stolen.error_message(),
        "Изображение уже привязано к публикации."
    );

    let unknown = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Неизвестное",
            "body": paragraph_doc("Текст"),
            "category": "Статья",
            "imageIds": ["img-unknown"]
        })),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);

    let duplicates = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Дубли",
            "body": paragraph_doc("Текст"),
            "category": "Статья",
            "imageIds": ["img-first", "img-first"]
        })),
    )
    .await;
    assert_eq!(duplicates.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        duplicates.error_message(),
        "Идентификаторы изображений не должны повторяться."
    );

    let too_many: Vec<String> = (0..11).map(|index| format!("img-{index}")).collect();
    let overflow = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Слишком много",
            "body": paragraph_doc("Текст"),
            "category": "Статья",
            "imageIds": too_many
        })),
    )
    .await;
    assert_eq!(overflow.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        overflow.error_message(),
        "К публикации можно приложить не более 10 изображений."
    );

    insert_pending_image(
        &database,
        "img-expired",
        10,
        10,
        now_ms - dimasik_backend::store::PENDING_IMAGE_TTL_MS - 1_000,
    );
    let expired = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Просроченное",
            "body": paragraph_doc("Текст"),
            "category": "Статья",
            "imageIds": ["img-expired"]
        })),
    )
    .await;
    assert_eq!(expired.status, StatusCode::NOT_FOUND);
    assert_eq!(
        expired.error_message(),
        "Срок загрузки изображения истёк."
    );

    insert_pending_image(&database, "img-detached", 10, 10, now_ms);
    mark_image_for_cleanup(&database, "img-detached", now_ms);
    let detached = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Отсоединённое",
            "body": paragraph_doc("Текст"),
            "category": "Статья",
            "imageIds": ["img-detached"]
        })),
    )
    .await;
    assert_eq!(detached.status, StatusCode::NOT_FOUND);
    assert_eq!(
        detached.error_message(),
        "Изображение не найдено: загрузка удалена."
    );

    let after_failures = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    assert_eq!(
        post_count(&after_failures.body),
        3,
        "failed claims must not leave partial posts"
    );

    // Editing without image fields keeps every attachment.
    let patched = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "С изображениями",
            "body": paragraph_doc("Обновлённый текст"),
            "category": "Статья"
        })),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK);
    let patched_images = patched.body["images"].as_array().expect("images");
    assert_eq!(patched_images.len(), 2);
    assert_eq!(patched_images[0]["id"], "img-second");
    assert_eq!(patched_images[1]["id"], "img-first");

    // A restart (a fresh connection over the same database file) keeps the
    // attachments.
    let reopened = TestEnv::router_for_config(&env.config);
    let gubernia = api(
        &reopened,
        Method::GET,
        "/internal/gubernia/ryazanskaya",
        None,
        None,
    )
    .await;
    let restarted = gubernia.body["posts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|post| post["id"] == post_id.as_str())
        .expect("post after restart");
    assert_eq!(restarted["images"].as_array().unwrap().len(), 2);

    // A cross-province move keeps the post id and its images (the previous
    // DELETE + INSERT lost them).
    let moved = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "С изображениями",
            "body": paragraph_doc("Обновлённый текст"),
            "category": "Статья",
            "targetGuberniaId": "tula"
        })),
    )
    .await;
    assert_eq!(moved.status, StatusCode::OK);
    assert_eq!(moved.body["images"].as_array().unwrap().len(), 2);
    assert_eq!(
        image_row(&database, "img-second").expect("row").0.as_deref(),
        Some(post_id.as_str())
    );
    let tula = api(&app, Method::GET, "/internal/gubernia/tulskaya", None, None).await;
    assert_eq!(tula.body["posts"][0]["id"], post_id.as_str());
    assert_eq!(tula.body["posts"][0]["images"][0]["id"], "img-second");

    // Deleting the post detaches the rows but keeps their keys so the storage
    // cleanup can finish the job; detached images stop being readable.
    let deleted = api(
        &app,
        Method::DELETE,
        &format!("/api/gubernias/tula/posts/{post_id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let (post_ref, attached_ms, key) = image_row(&database, "img-second").expect("row");
    assert_eq!(post_ref, None);
    assert!(
        attached_ms.is_some(),
        "detached rows stay marked for cleanup"
    );
    assert_eq!(key, "post-images/img-second/original");
    let gone = api(
        &app,
        Method::GET,
        "/api/post-images/img-second/original",
        None,
        None,
    )
    .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

/// Image-only publications: an empty body is stored as the canonical empty
/// document (never `null`) only while the request claims images, and the same
/// allowance keeps the record editable and movable. A body without images
/// still has to carry visible text.
#[tokio::test]
async fn image_only_post_create_edit_move_and_rollback() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;
    let database = env.config.database_path.clone();
    let now_ms = dimasik_backend::util::now_unix() * 1_000;
    let canonical_empty = json!({ "type": "doc", "content": [{ "type": "paragraph" }] });
    insert_pending_image(&database, "img-photo-second", 800, 600, now_ms);
    insert_pending_image(&database, "img-photo-first", 1024, 768, now_ms);

    // The raw empty form is accepted with images and stored canonically, in
    // the submitted order.
    let created = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Только фотографии",
            "body": { "type": "doc", "content": [] },
            "category": "Статья",
            "imageIds": ["img-photo-second", "img-photo-first"]
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let post_id = created.body["id"].as_str().unwrap().to_string();
    assert_eq!(created.body["body"], canonical_empty);
    let images = created.body["images"].as_array().expect("images");
    assert_eq!(images.len(), 2);
    assert_eq!(images[0]["id"], "img-photo-second");
    assert_eq!(images[1]["id"], "img-photo-first");

    // The feed serialises the same canonical body and image order.
    let gubernia = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    let feed_post = gubernia.body["posts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|post| post["id"] == post_id.as_str())
        .expect("created post");
    assert_eq!(feed_post["body"], canonical_empty);
    assert_eq!(feed_post["images"][0]["id"], "img-photo-second");
    assert_eq!(feed_post["images"][1]["id"], "img-photo-first");

    // Editing an image-only post with another empty body keeps the images.
    let patched = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Только фотографии",
            "body": { "type": "doc", "content": [{ "type": "paragraph" }] },
            "category": "Статья"
        })),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK);
    assert_eq!(patched.body["body"], canonical_empty);
    let patched_images = patched.body["images"].as_array().expect("images");
    assert_eq!(patched_images.len(), 2);
    assert_eq!(patched_images[0]["id"], "img-photo-second");
    assert_eq!(patched_images[1]["id"], "img-photo-first");

    // A cross-province move keeps the canonical body and the attachments.
    let moved = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Только фотографии",
            "body": { "type": "doc", "content": [{ "type": "paragraph" }] },
            "category": "Статья",
            "targetGuberniaId": "tula",
            "uyezdId": "uyezd-1897-9"
        })),
    )
    .await;
    assert_eq!(moved.status, StatusCode::OK);
    assert_eq!(moved.body["images"].as_array().unwrap().len(), 2);
    assert_eq!(
        image_row(&database, "img-photo-first").expect("row").0.as_deref(),
        Some(post_id.as_str())
    );
    let tula = api(&app, Method::GET, "/internal/gubernia/tulskaya", None, None).await;
    assert_eq!(tula.body["posts"][0]["id"], post_id.as_str());
    assert_eq!(tula.body["posts"][0]["body"], canonical_empty);
    assert_eq!(tula.body["posts"][0]["images"][1]["id"], "img-photo-first");

    // Empty bodies without images stay rejected for both forms on create...
    for body in [
        json!({ "type": "doc", "content": [] }),
        canonical_empty.clone(),
    ] {
        let rejected = api(
            &app,
            Method::POST,
            "/api/gubernias/ryazan/posts",
            Some(&cookie),
            Some(json!({ "title": "Без текста", "body": body, "category": "Статья" })),
        )
        .await;
        assert_eq!(rejected.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            rejected.error_message(),
            "Текст публикации не должен быть пустым."
        );
    }

    // An explicitly empty claim list does not authorise an empty body.
    let empty_list = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Пустой список",
            "body": { "type": "doc", "content": [] },
            "category": "Статья",
            "imageIds": []
        })),
    )
    .await;
    assert_eq!(empty_list.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        empty_list.error_message(),
        "Текст публикации не должен быть пустым."
    );

    // Title rules are unchanged for image-only creates.
    let untitled = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "   ",
            "body": { "type": "doc", "content": [] },
            "category": "Статья",
            "imageIds": ["img-photo-first"]
        })),
    )
    .await;
    assert_eq!(untitled.status, StatusCode::BAD_REQUEST);
    assert_eq!(untitled.error_message(), "Заголовок не должен быть пустым.");

    // ... and for a stored post without images.
    let legacy_empty = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{LEGACY_POST_ID}"),
        Some(&cookie),
        Some(json!({
            "title": "Текст первый",
            "body": canonical_empty.clone(),
            "category": "Статья"
        })),
    )
    .await;
    assert_eq!(legacy_empty.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        legacy_empty.error_message(),
        "Текст публикации не должен быть пустым."
    );

    // A forged id cannot authorise an empty record: the claim failure rolls
    // the whole create back, including the id claimed before it.
    insert_pending_image(&database, "img-photo-orphan", 10, 10, now_ms);
    let before = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    let posts_before = before.body["posts"].as_array().unwrap().len();
    let forged = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Подделка",
            "body": { "type": "doc", "content": [] },
            "category": "Статья",
            "imageIds": ["img-photo-orphan", "img-photo-unknown"]
        })),
    )
    .await;
    assert_eq!(forged.status, StatusCode::NOT_FOUND);
    assert_eq!(forged.error_message(), "Изображение не найдено.");
    let after = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    assert_eq!(
        after.body["posts"].as_array().unwrap().len(),
        posts_before,
        "a failed claim must not leave an image-only post behind"
    );
    let (post_ref, attached_ms, _) = image_row(&database, "img-photo-orphan").expect("row");
    assert_eq!(post_ref, None);
    assert_eq!(attached_ms, None);
}

/// PATCHes a post in ryazan with an edited title/body, passing the
/// `imageIds` field the rollback tests need (`None` omits it entirely).
async fn patch_edited_post(
    app: &Router,
    cookie: &str,
    post_id: &str,
    image_ids: Option<Value>,
) -> TestResponse {
    let mut payload = json!({
        "title": "Изменённый",
        "body": paragraph_doc("Изменённый текст"),
        "category": "Статья",
    });
    if let Some(image_ids) = image_ids {
        payload["imageIds"] = image_ids;
    }
    api(
        app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(cookie),
        Some(payload),
    )
    .await
}

/// PATCH `imageIds` is a full ordered replacement: retained ids keep their
/// rows, fresh pending ids are claimed, the store rewrites the order, and
/// omitted ids are detached with their durable marker because the save
/// committed; a re-order alone changes nothing but `position`.
#[tokio::test]
async fn post_patch_replaces_images_in_order_and_detaches_removed() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;
    let database = env.config.database_path.clone();
    let now_ms = dimasik_backend::util::now_unix() * 1_000;
    insert_pending_image(&database, "img-patch-first", 640, 480, now_ms);
    insert_pending_image(&database, "img-patch-second", 800, 600, now_ms);
    insert_pending_image(&database, "img-patch-added", 1024, 768, now_ms);

    let created = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Замена изображений",
            "body": paragraph_doc("Текст"),
            "category": "Статья",
            "imageIds": ["img-patch-first", "img-patch-second"]
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let post_id = created.body["id"].as_str().unwrap().to_string();

    let replaced = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Замена изображений",
            "body": paragraph_doc("Обновлённый текст"),
            "category": "Статья",
            "imageIds": ["img-patch-second", "img-patch-added"]
        })),
    )
    .await;
    assert_eq!(replaced.status, StatusCode::OK);
    assert_eq!(replaced.body["body"], paragraph_doc("Обновлённый текст"));
    let images = replaced.body["images"].as_array().expect("images");
    assert_eq!(images.len(), 2);
    assert_eq!(images[0]["id"], "img-patch-second");
    assert_eq!(images[1]["id"], "img-patch-added");
    assert_eq!(
        images[1]["originalUrl"],
        "/api/post-images/img-patch-added/original"
    );

    assert_eq!(
        image_row(&database, "img-patch-second").expect("row").0.as_deref(),
        Some(post_id.as_str()),
        "a retained id keeps its owning post"
    );
    assert_eq!(
        image_row(&database, "img-patch-added").expect("row").0.as_deref(),
        Some(post_id.as_str()),
        "a fresh pending id is claimed by the save"
    );
    let (post_ref, attached_ms, key) = image_row(&database, "img-patch-first").expect("row");
    assert_eq!(post_ref, None, "an id left out of the replacement detaches");
    assert!(
        attached_ms.is_some_and(|ms| ms > 0),
        "a detached row keeps a positive cleanup marker"
    );
    assert_eq!(key, "post-images/img-patch-first/original");
    let detached_read = api(
        &app,
        Method::GET,
        "/api/post-images/img-patch-first/original",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(detached_read.status, StatusCode::NOT_FOUND);

    // The feed serializes the replacement order too.
    let gubernia = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    let feed_post = gubernia.body["posts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|post| post["id"] == post_id.as_str())
        .expect("post");
    assert_eq!(feed_post["images"][0]["id"], "img-patch-second");
    assert_eq!(feed_post["images"][1]["id"], "img-patch-added");

    // Re-ordering alone keeps every row attached.
    let reordered = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Замена изображений",
            "body": paragraph_doc("Обновлённый текст"),
            "category": "Статья",
            "imageIds": ["img-patch-added", "img-patch-second"]
        })),
    )
    .await;
    assert_eq!(reordered.status, StatusCode::OK);
    assert_eq!(reordered.body["images"][0]["id"], "img-patch-added");
    assert_eq!(reordered.body["images"][1]["id"], "img-patch-second");
    assert_eq!(
        image_row(&database, "img-patch-added").expect("row").0.as_deref(),
        Some(post_id.as_str()),
        "re-ordering never detaches"
    );

    // A detached id can never come back through a later save.
    let reattach = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Замена изображений",
            "body": paragraph_doc("Обновлённый текст"),
            "category": "Статья",
            "imageIds": ["img-patch-first"]
        })),
    )
    .await;
    assert_eq!(reattach.status, StatusCode::NOT_FOUND);
    assert_eq!(
        reattach.error_message(),
        "Изображение не найдено: загрузка удалена."
    );
    assert_eq!(
        image_row(&database, "img-patch-first").expect("row").0,
        None,
        "the rejected save must not re-own the detached row"
    );
    assert_eq!(
        image_row(&database, "img-patch-added").expect("row").0.as_deref(),
        Some(post_id.as_str()),
        "the rejected save must not disturb the current attachments"
    );
}

/// A replacement that names an id which is malformed, unknown, expired,
/// already detached, owned by another post or missing rolls the entire save
/// back, including earlier claims of the same request and the metadata.
#[tokio::test]
async fn post_patch_image_replacement_rejects_and_rolls_back() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;
    let database = env.config.database_path.clone();
    let now_ms = dimasik_backend::util::now_unix() * 1_000;
    insert_pending_image(&database, "img-rb-own", 100, 80, now_ms);
    insert_pending_image(&database, "img-rb-fresh", 200, 160, now_ms);
    insert_pending_image(&database, "img-rb-foreign", 300, 240, now_ms);
    insert_pending_image(
        &database,
        "img-rb-expired",
        10,
        10,
        now_ms - dimasik_backend::store::PENDING_IMAGE_TTL_MS - 1_000,
    );
    insert_pending_image(&database, "img-rb-cancelled", 10, 10, now_ms);
    mark_image_for_cleanup(&database, "img-rb-cancelled", now_ms);

    let owner = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Исходный",
            "body": paragraph_doc("Исходный текст"),
            "category": "Статья",
            "imageIds": ["img-rb-own"]
        })),
    )
    .await;
    assert_eq!(owner.status, StatusCode::CREATED);
    let post_id = owner.body["id"].as_str().unwrap().to_string();
    let other = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Чужой",
            "body": paragraph_doc("Чужой текст"),
            "category": "Статья",
            "imageIds": ["img-rb-foreign"]
        })),
    )
    .await;
    assert_eq!(other.status, StatusCode::CREATED);
    let other_id = other.body["id"].as_str().unwrap().to_string();

    // Shape errors are rejected before any row is touched.
    let null = patch_edited_post(&app, &cookie, &post_id, Some(json!(null))).await;
    assert_eq!(null.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        null.error_message(),
        "Список изображений должен быть массивом."
    );
    let not_array = patch_edited_post(&app, &cookie, &post_id, Some(json!("img-rb-own"))).await;
    assert_eq!(not_array.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        not_array.error_message(),
        "Список изображений должен быть массивом."
    );
    let not_string =
        patch_edited_post(&app, &cookie, &post_id, Some(json!(["img-rb-own", 5]))).await;
    assert_eq!(not_string.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        not_string.error_message(),
        "Некорректный идентификатор изображения."
    );
    let duplicates = patch_edited_post(
        &app,
        &cookie,
        &post_id,
        Some(json!(["img-rb-own", "img-rb-own"])),
    )
    .await;
    assert_eq!(duplicates.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        duplicates.error_message(),
        "Идентификаторы изображений не должны повторяться."
    );
    let mut overflow: Vec<String> = (0..dimasik_backend::store::MAX_POST_IMAGES)
        .map(|index| format!("img-rb-over-{index}"))
        .collect();
    overflow.push("img-rb-own".to_string());
    let too_many = patch_edited_post(&app, &cookie, &post_id, Some(json!(overflow))).await;
    assert_eq!(too_many.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        too_many.error_message(),
        "К публикации можно приложить не более 10 изображений."
    );

    // Row-level failures are classified like creation and abort the save.
    let unknown = patch_edited_post(&app, &cookie, &post_id, Some(json!(["img-rb-missing"]))).await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.error_message(), "Изображение не найдено.");
    let expired = patch_edited_post(
        &app,
        &cookie,
        &post_id,
        Some(json!(["img-rb-own", "img-rb-expired"])),
    )
    .await;
    assert_eq!(expired.status, StatusCode::NOT_FOUND);
    assert_eq!(
        expired.error_message(),
        "Срок загрузки изображения истёк."
    );
    let cancelled =
        patch_edited_post(&app, &cookie, &post_id, Some(json!(["img-rb-cancelled"]))).await;
    assert_eq!(cancelled.status, StatusCode::NOT_FOUND);
    assert_eq!(
        cancelled.error_message(),
        "Изображение не найдено: загрузка удалена."
    );
    let foreign = patch_edited_post(&app, &cookie, &post_id, Some(json!(["img-rb-foreign"]))).await;
    assert_eq!(foreign.status, StatusCode::CONFLICT);
    assert_eq!(
        foreign.error_message(),
        "Изображение уже привязано к публикации."
    );

    // A request that claims a fresh id before failing on a foreign one must
    // roll the successful claim back too (nothing half-attached).
    let mixed = patch_edited_post(
        &app,
        &cookie,
        &post_id,
        Some(json!(["img-rb-fresh", "img-rb-foreign"])),
    )
    .await;
    assert_eq!(mixed.status, StatusCode::CONFLICT);
    let (post_ref, attached_ms, _) = image_row(&database, "img-rb-fresh").expect("row");
    assert_eq!(
        post_ref, None,
        "a claim made before the failing id must roll back"
    );
    assert_eq!(attached_ms, None);

    // The failed saves left the post metadata, its attachment set and every
    // other row exactly as they were.
    let gubernia = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    let feed_post = gubernia.body["posts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|post| post["id"] == post_id.as_str())
        .expect("post");
    assert_eq!(feed_post["title"], "Исходный");
    assert_eq!(feed_post["body"], paragraph_doc("Исходный текст"));
    assert_eq!(feed_post["images"].as_array().unwrap().len(), 1);
    assert_eq!(feed_post["images"][0]["id"], "img-rb-own");
    assert_eq!(
        image_row(&database, "img-rb-own").expect("row").0.as_deref(),
        Some(post_id.as_str())
    );
    assert_eq!(
        image_row(&database, "img-rb-foreign").expect("row").0.as_deref(),
        Some(other_id.as_str()),
        "a foreign attachment is never taken over"
    );
    assert_eq!(image_row(&database, "img-rb-expired").expect("row").0, None);
    assert_eq!(image_row(&database, "img-rb-cancelled").expect("row").0, None);

    // The post stays editable once the ids are correct.
    let accepted = patch_edited_post(
        &app,
        &cookie,
        &post_id,
        Some(json!(["img-rb-own", "img-rb-fresh"])),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::OK);
    assert_eq!(accepted.body["title"], "Изменённый");
    assert_eq!(accepted.body["images"][0]["id"], "img-rb-own");
    assert_eq!(accepted.body["images"][1]["id"], "img-rb-fresh");
    assert_eq!(
        image_row(&database, "img-rb-fresh").expect("row").0.as_deref(),
        Some(post_id.as_str())
    );
}

/// A PATCH without the `imageIds` field keeps the stored attachments in order,
/// including across a cross-province move; a replacement afterwards works
/// from that authoritative set.
#[tokio::test]
async fn post_patch_image_omission_and_moves_preserve_attachments() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;
    let database = env.config.database_path.clone();
    let now_ms = dimasik_backend::util::now_unix() * 1_000;
    insert_pending_image(&database, "img-move-first", 100, 80, now_ms);
    insert_pending_image(&database, "img-move-second", 200, 160, now_ms);

    let created = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Переезд",
            "body": paragraph_doc("Текст"),
            "category": "Статья",
            "imageIds": ["img-move-first", "img-move-second"]
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let post_id = created.body["id"].as_str().unwrap().to_string();

    // Omitting the field keeps the attachments untouched.
    let edited = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Переезд: правка",
            "body": paragraph_doc("Другой текст"),
            "category": "Статья"
        })),
    )
    .await;
    assert_eq!(edited.status, StatusCode::OK);
    assert_eq!(edited.body["images"][0]["id"], "img-move-first");
    assert_eq!(edited.body["images"][1]["id"], "img-move-second");

    // A move with the field omitted keeps them too; the rows never leave the
    // post id.
    let moved = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Переезд: правка",
            "body": paragraph_doc("Другой текст"),
            "category": "Статья",
            "targetGuberniaId": "tula"
        })),
    )
    .await;
    assert_eq!(moved.status, StatusCode::OK);
    assert_eq!(moved.body["images"].as_array().unwrap().len(), 2);
    assert_eq!(
        image_row(&database, "img-move-first").expect("row").0.as_deref(),
        Some(post_id.as_str())
    );

    // A replacement in the new province drops one image and keeps the other.
    let replaced = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/tula/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Переезд: правка",
            "body": paragraph_doc("Другой текст"),
            "category": "Статья",
            "imageIds": ["img-move-second"]
        })),
    )
    .await;
    assert_eq!(replaced.status, StatusCode::OK);
    assert_eq!(replaced.body["images"].as_array().unwrap().len(), 1);
    assert_eq!(replaced.body["images"][0]["id"], "img-move-second");
    assert_eq!(image_row(&database, "img-move-first").expect("row").0, None);

    // Moving back with the field omitted keeps the surviving image.
    let returned = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/tula/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Переезд: правка",
            "body": paragraph_doc("Другой текст"),
            "category": "Статья",
            "targetGuberniaId": "ryazan"
        })),
    )
    .await;
    assert_eq!(returned.status, StatusCode::OK);
    assert_eq!(returned.body["images"].as_array().unwrap().len(), 1);
    assert_eq!(returned.body["images"][0]["id"], "img-move-second");
    assert_eq!(
        image_row(&database, "img-move-second").expect("row").0.as_deref(),
        Some(post_id.as_str())
    );
    let tula = api(&app, Method::GET, "/internal/gubernia/tulskaya", None, None).await;
    assert!(
        tula.body["posts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|post| post["id"] != post_id.as_str()),
        "the moved post is no longer in the previous province"
    );
}

/// The blank-body rule follows the final attachment set of the save: dropping
/// the last image with an empty body is rejected (nothing detaches), the same
/// drop with real text commits, and adding a fresh image may store the
/// canonical empty document again.
#[tokio::test]
async fn post_patch_blank_body_depends_on_final_image_set() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;
    let database = env.config.database_path.clone();
    let now_ms = dimasik_backend::util::now_unix() * 1_000;
    let canonical_empty = json!({ "type": "doc", "content": [{ "type": "paragraph" }] });
    insert_pending_image(&database, "img-last", 100, 80, now_ms);

    let created = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Одна фотография",
            "body": { "type": "doc", "content": [] },
            "category": "Статья",
            "imageIds": ["img-last"]
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let post_id = created.body["id"].as_str().unwrap().to_string();
    assert_eq!(created.body["body"], canonical_empty);

    // Removing the last image while the body stays empty is rejected and the
    // image stays attached.
    let rejected = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Одна фотография",
            "body": { "type": "doc", "content": [] },
            "category": "Статья",
            "imageIds": []
        })),
    )
    .await;
    assert_eq!(rejected.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        rejected.error_message(),
        "Текст публикации не должен быть пустым."
    );
    assert_eq!(
        image_row(&database, "img-last").expect("row").0.as_deref(),
        Some(post_id.as_str()),
        "a rejected save must not detach the image"
    );

    // With real text the same removal commits and the row is detached.
    let cleared = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Одна фотография",
            "body": paragraph_doc("Теперь только текст"),
            "category": "Статья",
            "imageIds": []
        })),
    )
    .await;
    assert_eq!(cleared.status, StatusCode::OK);
    assert_eq!(cleared.body["body"], paragraph_doc("Теперь только текст"));
    assert_eq!(cleared.body["images"], json!([]));
    let (post_ref, attached_ms, key) = image_row(&database, "img-last").expect("row");
    assert_eq!(post_ref, None);
    assert!(attached_ms.is_some_and(|ms| ms > 0));
    assert_eq!(key, "post-images/img-last/original");

    // A replacement that brings a fresh image can store the canonical empty
    // body again...
    insert_pending_image(&database, "img-last-replacement", 50, 50, now_ms);
    let refilled = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Одна фотография",
            "body": { "type": "doc", "content": [] },
            "category": "Статья",
            "imageIds": ["img-last-replacement"]
        })),
    )
    .await;
    assert_eq!(refilled.status, StatusCode::OK);
    assert_eq!(refilled.body["body"], canonical_empty);
    assert_eq!(refilled.body["images"][0]["id"], "img-last-replacement");

    // ...but it cannot be emptied again by the save that drops it.
    let rejected_again = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Одна фотография",
            "body": canonical_empty.clone(),
            "category": "Статья",
            "imageIds": []
        })),
    )
    .await;
    assert_eq!(rejected_again.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        rejected_again.error_message(),
        "Текст публикации не должен быть пустым."
    );
    assert_eq!(
        image_row(&database, "img-last-replacement").expect("row").0.as_deref(),
        Some(post_id.as_str())
    );
}

#[tokio::test]
async fn pending_images_stay_admin_only_until_claimed() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;
    let database = env.config.database_path.clone();
    let now_ms = dimasik_backend::util::now_unix() * 1_000;
    insert_pending_image(&database, "img-pending", 100, 50, now_ms);

    // Anonymous readers never see a pending upload; a session alone is not
    // enough without storage to sign the redirect.
    let anonymous = api(
        &app,
        Method::GET,
        "/api/post-images/img-pending/thumbnail",
        None,
        None,
    )
    .await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    let admin = api(
        &app,
        Method::GET,
        "/api/post-images/img-pending/thumbnail",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(admin.status, StatusCode::SERVICE_UNAVAILABLE);

    // Without storage the objects cannot be removed, so the pending row (and
    // its keys) must stay put instead of leaking the objects.
    let refused_delete = api(
        &app,
        Method::DELETE,
        "/api/post-images/img-pending",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(refused_delete.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        image_row(&database, "img-pending").is_some(),
        "row stays until the objects are deleted"
    );

    // Claiming works without storage: the objects already exist.
    let created = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "С картинкой",
            "body": paragraph_doc("Текст"),
            "category": "Статья",
            "imageIds": ["img-pending"]
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let post_id = created.body["id"].as_str().unwrap().to_string();
    assert_eq!(
        image_row(&database, "img-pending").expect("row").0.as_deref(),
        Some(post_id.as_str())
    );

    // Attached images cannot be cancelled.
    let attached_delete = api(
        &app,
        Method::DELETE,
        "/api/post-images/img-pending",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(attached_delete.status, StatusCode::CONFLICT);
    assert_eq!(
        attached_delete.error_message(),
        "Изображение уже привязано к публикации."
    );

    // Unpublishing the province deletes its posts, which detaches the image;
    // the detached row is invisible and keeps its keys for the cleanup.
    let unpublished = api(
        &app,
        Method::DELETE,
        "/api/gubernias/ryazan",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(unpublished.status, StatusCode::NO_CONTENT);
    assert_eq!(image_row(&database, "img-pending").expect("row").0, None);
    let detached = api(
        &app,
        Method::GET,
        "/api/post-images/img-pending/original",
        None,
        None,
    )
    .await;
    assert_eq!(detached.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn category_snapshot_stays_valid_after_settings_change() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;

    let created = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Очерк",
            "body": paragraph_doc("Текст"),
            "category": "Очерк"
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let post_id = created.body["id"].as_str().unwrap().to_string();

    let removed = api(
        &app,
        Method::DELETE,
        "/api/nastroyki",
        Some(&cookie),
        Some(json!({ "kind": "categories", "name": "Очерк" })),
    )
    .await;
    assert_eq!(removed.status, StatusCode::OK);
    assert_eq!(removed.body["settings"]["categories"], json!(["Статья"]));

    // A new post can no longer choose the removed option.
    let rejected = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({ "title": "x", "body": paragraph_doc("y"), "category": "Очерк" })),
    )
    .await;
    assert_eq!(rejected.status, StatusCode::BAD_REQUEST);

    // The stored snapshot stays valid when the category is unchanged.
    let updated = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Очерк",
            "body": paragraph_doc("Текст изменён"),
            "category": "Очерк"
        })),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_eq!(updated.body["category"], "Очерк");
}

#[tokio::test]
async fn settlement_creation_validates_geometry_types_and_urls() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;

    let created = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/settlements",
        Some(&cookie),
        Some(json!({
            "name": "Новое",
            "uyezdId": "uyezd-1897-2",
            "coordinates": "22, 22",
            "url": "/naselennyy-punkt/novoe",
            "type": "Село"
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let settlement_id = created.body["id"].as_str().unwrap().to_string();
    assert_eq!(created.body["url"], "/naselennyy-punkt/novoe");
    assert_eq!(created.body["type"], "Село");
    assert_eq!(created.body["uyezdId"], "uyezd-1897-2");

    // The multipolygon's second landmass also counts as inside.
    let second_landmass = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/settlements",
        Some(&cookie),
        Some(json!({
            "name": "Второй массив",
            "uyezdId": "uyezd-1897-2",
            "coordinates": "31, 31",
            "url": "/naselennyy-punkt/vtoroy-massiv",
            "type": "Город"
        })),
    )
    .await;
    assert_eq!(second_landmass.status, StatusCode::CREATED);

    let outside = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/settlements",
        Some(&cookie),
        Some(json!({
            "name": "Снаружи",
            "uyezdId": "uyezd-1897-2",
            "coordinates": "40, 40",
            "url": "/naselennyy-punkt/snaruzhi",
            "type": "Село"
        })),
    )
    .await;
    assert_eq!(outside.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        outside.error_message(),
        "Точка находится за пределами выбранного уезда."
    );

    // Inside the hole of the first district: not part of the uyezd.
    let in_hole = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/settlements",
        Some(&cookie),
        Some(json!({
            "name": "Дыра",
            "uyezdId": "uyezd-1897-1",
            "coordinates": "5, 5",
            "url": "/naselennyy-punkt/dyra",
            "type": "Село"
        })),
    )
    .await;
    assert_eq!(in_hole.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        in_hole.error_message(),
        "Точка находится за пределами выбранного уезда."
    );

    let derived_collision = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/settlements",
        Some(&cookie),
        Some(json!({
            "name": "Коллизия",
            "uyezdId": "uyezd-1897-1",
            "coordinates": "1, 1",
            "url": format!("/naselennyy-punkt/{LEGACY_SETTLEMENT_ID}"),
            "type": "Село"
        })),
    )
    .await;
    assert_eq!(derived_collision.status, StatusCode::CONFLICT);
    assert_eq!(
        derived_collision.error_message(),
        "Этот адрес страницы уже используется другим населённым пунктом."
    );

    let duplicate = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/settlements",
        Some(&cookie),
        Some(json!({
            "name": "Дубль",
            "uyezdId": "uyezd-1897-1",
            "coordinates": "1, 1",
            "url": "/naselennyy-punkt/novoe",
            "type": "Село"
        })),
    )
    .await;
    assert_eq!(duplicate.status, StatusCode::CONFLICT);

    let unknown_uyezd = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/settlements",
        Some(&cookie),
        Some(json!({
            "name": "x",
            "uyezdId": "uyezd-1897-999",
            "coordinates": "1, 1",
            "url": "/naselennyy-punkt/x1",
            "type": "Село"
        })),
    )
    .await;
    assert_eq!(
        unknown_uyezd.error_message(),
        "Уезд не найден в этой губернии."
    );

    let missing_uyezd = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/settlements",
        Some(&cookie),
        Some(json!({
            "name": "x",
            "coordinates": "1, 1",
            "url": "/naselennyy-punkt/x2",
            "type": "Село"
        })),
    )
    .await;
    assert_eq!(
        missing_uyezd.error_message(),
        "Уезд обязателен для населённого пункта."
    );

    let bad_type = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/settlements",
        Some(&cookie),
        Some(json!({
            "name": "x",
            "uyezdId": "uyezd-1897-1",
            "coordinates": "1, 1",
            "url": "/naselennyy-punkt/x3",
            "type": "Станица"
        })),
    )
    .await;
    assert_eq!(
        bad_type.error_message(),
        "Тип населённого пункта должен быть одним из: «Город», «Село», «Погост»."
    );

    let bad_url = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/settlements",
        Some(&cookie),
        Some(json!({
            "name": "x",
            "uyezdId": "uyezd-1897-1",
            "coordinates": "1, 1",
            "url": "novoe",
            "type": "Село"
        })),
    )
    .await;
    assert!(bad_url.error_message().starts_with("Адрес страницы должен"));

    let bad_coordinates = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/settlements",
        Some(&cookie),
        Some(json!({
            "name": "x",
            "uyezdId": "uyezd-1897-1",
            "coordinates": "1;1",
            "url": "/naselennyy-punkt/x4",
            "type": "Село"
        })),
    )
    .await;
    assert_eq!(
        bad_coordinates.error_message(),
        "Координаты должны содержать широту и долготу через запятую."
    );

    // Deleting a settlement detaches its posts and drops its reference block.
    let bound_post = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Привязан",
            "body": paragraph_doc("Текст"),
            "category": "Статья",
            "settlementId": settlement_id,
            "year": "1911"
        })),
    )
    .await;
    assert_eq!(bound_post.status, StatusCode::CREATED);
    let bound_post_id = bound_post.body["id"].as_str().unwrap().to_string();

    let saved_reference = api(
        &app,
        Method::PATCH,
        "/api/naselennyy-punkt/novoe/reference",
        Some(&cookie),
        Some(json!({ "body": paragraph_doc("Справка") })),
    )
    .await;
    assert_eq!(saved_reference.status, StatusCode::OK);

    let missing_reference_target = api(
        &app,
        Method::PATCH,
        "/api/naselennyy-punkt/missing/reference",
        Some(&cookie),
        Some(json!({ "body": paragraph_doc("Справка") })),
    )
    .await;
    assert_eq!(missing_reference_target.status, StatusCode::NOT_FOUND);
    assert_eq!(
        missing_reference_target.error_message(),
        "Населённый пункт не найден."
    );

    let deleted = api(
        &app,
        Method::DELETE,
        &format!("/api/gubernias/ryazan/settlements/{settlement_id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);

    let gubernia = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    let detached = gubernia.body["posts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|post| post["id"] == bound_post_id)
        .expect("post survives settlement deletion");
    assert_eq!(detached["settlementId"], Value::Null);
    assert_eq!(detached["year"], "1911");
    assert_eq!(detached["uyezdId"], "uyezd-1897-2");

    let reference = api(
        &app,
        Method::GET,
        &format!("/internal/settlement-reference/{settlement_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(reference.status, StatusCode::NOT_FOUND);

    let second_delete = api(
        &app,
        Method::DELETE,
        &format!("/api/gubernias/ryazan/settlements/{settlement_id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(second_delete.status, StatusCode::NOT_FOUND);
    assert_eq!(
        second_delete.error_message(),
        "Населённый пункт не найден."
    );
}

#[tokio::test]
async fn request_size_caps_are_enforced() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;

    let oversized_settings = api(
        &app,
        Method::POST,
        "/api/nastroyki",
        Some(&cookie),
        Some(json!({ "kind": "categories", "name": "x".repeat(3_000) })),
    )
    .await;
    assert_eq!(oversized_settings.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        oversized_settings.error_message(),
        "Тело запроса слишком большое."
    );

    let oversized_settlement = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/settlements",
        Some(&cookie),
        Some(json!({
            "name": "x",
            "uyezdId": "uyezd-1897-1",
            "coordinates": "1, 1",
            "url": "/naselennyy-punkt/x",
            "type": "Село",
            "pad": "x".repeat(5_000)
        })),
    )
    .await;
    assert_eq!(oversized_settlement.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        oversized_settlement.error_message(),
        "Тело запроса слишком большое."
    );

    let oversized_post = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "x",
            "body": paragraph_doc("y"),
            "category": "Статья",
            "pad": "x".repeat(620_000)
        })),
    )
    .await;
    assert_eq!(oversized_post.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        oversized_post.error_message(),
        "Тело запроса слишком большое."
    );
}

#[tokio::test]
async fn settings_mutations_follow_store_rules() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;

    let wrong_keys = api(
        &app,
        Method::POST,
        "/api/nastroyki",
        Some(&cookie),
        Some(json!({ "kind": "categories", "name": "x", "extra": 1 })),
    )
    .await;
    assert_eq!(wrong_keys.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        wrong_keys.error_message(),
        "Ожидается название и список настроек."
    );

    let unknown_kind = api(
        &app,
        Method::POST,
        "/api/nastroyki",
        Some(&cookie),
        Some(json!({ "kind": "nope", "name": "x" })),
    )
    .await;
    assert_eq!(unknown_kind.error_message(), "Неизвестный список настроек.");

    let duplicate = api(
        &app,
        Method::POST,
        "/api/nastroyki",
        Some(&cookie),
        Some(json!({ "kind": "categories", "name": "Статья" })),
    )
    .await;
    assert_eq!(
        duplicate.error_message(),
        "Такое название уже есть в списке."
    );

    let added = api(
        &app,
        Method::POST,
        "/api/nastroyki",
        Some(&cookie),
        Some(json!({ "kind": "categories", "name": "  Новое   название  " })),
    )
    .await;
    assert_eq!(added.status, StatusCode::OK);
    assert_eq!(
        added.body["settings"]["categories"],
        json!(["Статья", "Очерк", "Новое название"])
    );

    let renamed = api(
        &app,
        Method::PATCH,
        "/api/nastroyki",
        Some(&cookie),
        Some(json!({
            "kind": "categories",
            "originalName": "Очерк",
            "name": "Очерк обновлённый"
        })),
    )
    .await;
    assert_eq!(renamed.status, StatusCode::OK);
    assert_eq!(
        renamed.body["settings"]["categories"],
        json!(["Статья", "Очерк обновлённый", "Новое название"]),
        "rename keeps the position"
    );

    let missing_original = api(
        &app,
        Method::PATCH,
        "/api/nastroyki",
        Some(&cookie),
        Some(json!({ "kind": "categories", "originalName": "Нет", "name": "x" })),
    )
    .await;
    assert_eq!(
        missing_original.error_message(),
        "Исходное название не найдено."
    );

    let deleted = api(
        &app,
        Method::DELETE,
        "/api/nastroyki",
        Some(&cookie),
        Some(json!({ "kind": "settlementTypes", "name": "Погост" })),
    )
    .await;
    assert_eq!(deleted.status, StatusCode::OK);
    assert_eq!(
        deleted.body["settings"]["settlementTypes"],
        json!(["Город", "Село"])
    );

    let missing_delete = api(
        &app,
        Method::DELETE,
        "/api/nastroyki",
        Some(&cookie),
        Some(json!({ "kind": "settlementTypes", "name": "Погост" })),
    )
    .await;
    assert_eq!(missing_delete.error_message(), "Название не найдено.");

    let wrong_patch_keys = api(
        &app,
        Method::PATCH,
        "/api/nastroyki",
        Some(&cookie),
        Some(json!({ "kind": "categories", "name": "x" })),
    )
    .await;
    assert_eq!(
        wrong_patch_keys.error_message(),
        "Ожидается исходное название, новое название и список настроек."
    );

    // The persisted lists are visible to the SSR endpoint and to validation.
    let settings = api(&app, Method::GET, "/internal/settings", None, None).await;
    assert_eq!(settings.body["settlementTypes"], json!(["Город", "Село"]));
}

#[tokio::test]
async fn about_content_patch_round_trips() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;

    let wrong_keys = api(
        &app,
        Method::PATCH,
        "/api/about-content",
        Some(&cookie),
        Some(json!({ "body": paragraph_doc("x"), "extra": 1 })),
    )
    .await;
    assert_eq!(wrong_keys.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        wrong_keys.error_message(),
        "Ожидается документ содержимого страницы."
    );

    let invalid = api(
        &app,
        Method::PATCH,
        "/api/about-content",
        Some(&cookie),
        Some(json!({ "body": { "type": "doc", "content": [] } })),
    )
    .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        invalid.error_message(),
        "Текст публикации не должен быть пустым."
    );

    let updated = api(
        &app,
        Method::PATCH,
        "/api/about-content",
        Some(&cookie),
        Some(json!({ "body": paragraph_doc("Новый текст") })),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_eq!(updated.body["body"], paragraph_doc("Новый текст"));

    let about = api(&app, Method::GET, "/internal/about", None, None).await;
    assert_eq!(about.body["body"], paragraph_doc("Новый текст"));
}

#[tokio::test]
async fn publishing_another_province_sorts_metrics_with_russian_collation() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;

    let published = api(
        &app,
        Method::POST,
        "/api/gubernias",
        Some(&cookie),
        Some(json!({ "id": "abos", "slug": "abosskaya" })),
    )
    .await;
    assert_eq!(published.status, StatusCode::CREATED);

    let metrics = api(&app, Method::GET, "/internal/metrics", Some(&cookie), None).await;
    assert_eq!(metrics.body["publishedCount"], 3);
    let names: Vec<&str> = metrics.body["provinces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|province| province["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "Абосская губерния",
            "Рязанская губерния",
            "Тульская губерния"
        ]
    );

    let geo = api(&app, Method::GET, "/internal/geo", None, None).await;
    assert_eq!(geo.body["provinces"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn every_response_is_no_store() {
    let env = TestEnv::new();
    let app = env.app.clone();

    for (method_name, uri) in [
        ("GET", "/api/gubernias"),
        ("GET", "/internal/geo"),
        ("GET", "/internal/about"),
        ("GET", "/internal/settings"),
        ("GET", "/internal/session"),
        ("GET", "/internal/gubernia/ryazanskaya"),
        ("POST", "/api/gubernias"),
    ] {
        let response = api(&app, method(method_name), uri, None, None).await;
        assert_eq!(
            response.cache_control(),
            Some("no-store"),
            "{method_name} {uri}"
        );
    }
}

#[tokio::test]
async fn json_objects_only_for_mutation_bodies() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;

    let array_body = send(
        &app,
        raw_request(
            Method::POST,
            "/api/gubernias",
            Some(&cookie),
            Some("application/json"),
            "[]",
        ),
    )
    .await;
    assert_eq!(array_body.status, StatusCode::BAD_REQUEST);
    assert_eq!(array_body.error_message(), "Неверный формат запроса.");

    let null_body = send(
        &app,
        raw_request(
            Method::POST,
            "/api/gubernias",
            Some(&cookie),
            Some("application/json; charset=utf-8"),
            "null",
        ),
    )
    .await;
    assert_eq!(null_body.status, StatusCode::BAD_REQUEST);

    // DELETE routes carry no body and never check the content type.
    let delete = send(
        &app,
        raw_request(
            Method::DELETE,
            "/api/gubernias/gubernia-1897-5",
            Some(&cookie),
            None,
            "",
        ),
    )
    .await;
    assert_eq!(delete.status, StatusCode::NOT_FOUND);
    assert_eq!(delete.error_message(), "Губерния не опубликована.");
}

#[tokio::test]
async fn ordered_list_start_accepts_integral_floats_over_http() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;

    let created = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Список",
            "category": "Статья",
            "body": {
                "type": "doc",
                "content": [{
                    "type": "orderedList",
                    "attrs": { "start": 3.0 },
                    "content": [{
                        "type": "listItem",
                        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Пункт" }] }]
                    }]
                }]
            }
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(
        created.body["body"]["content"][0]["attrs"]["start"],
        json!(3),
        "integral float start canonicalises to an integer"
    );
}

#[tokio::test]
async fn district_relations_are_enforced_from_the_database() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;

    // A district of another province is invisible to this province.
    let cross_province = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/settlements",
        Some(&cookie),
        Some(json!({
            "name": "Чужая",
            "uyezdId": "uyezd-1897-9",
            "coordinates": "50, 50",
            "url": "/naselennyy-punkt/chuzhaya",
            "type": "Село"
        })),
    )
    .await;
    assert_eq!(cross_province.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        cross_province.error_message(),
        "Уезд не найден в этой губернии."
    );

    // Post placement uses the same relation.
    let post_with_foreign_district = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "x",
            "body": paragraph_doc("y"),
            "category": "Статья",
            "uyezdId": "uyezd-1897-9"
        })),
    )
    .await;
    assert_eq!(post_with_foreign_district.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        post_with_foreign_district.error_message(),
        "Уезд не найден в этой губернии."
    );

    // Removing the registry row proves SQLite owns the check: the GeoJSON file
    // still contains the district, but writes must fail.
    let connection = dimasik_backend::db::open_database(&env.config.database_path)
        .expect("database opens");
    connection
        .execute("DELETE FROM districts WHERE id = 'uyezd-1897-1'", [])
        .unwrap();
    drop(connection);

    let after_delete = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/settlements",
        Some(&cookie),
        Some(json!({
            "name": "После удаления",
            "uyezdId": "uyezd-1897-1",
            "coordinates": "1, 1",
            "url": "/naselennyy-punkt/posle-udaleniya",
            "type": "Село"
        })),
    )
    .await;
    assert_eq!(after_delete.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        after_delete.error_message(),
        "Уезд не найден в этой губернии."
    );

    let post_after_delete = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "x",
            "body": paragraph_doc("y"),
            "category": "Статья",
            "uyezdId": "uyezd-1897-1"
        })),
    )
    .await;
    assert_eq!(post_after_delete.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        post_after_delete.error_message(),
        "Уезд не найден в этой губернии."
    );
}

/// Reads one stored image row as `(post_id, province_id, settlement_id,
/// attached_ms)`; entity ownership and the durable cleanup marker have to
/// survive every save.
fn image_owners(
    database: &std::path::Path,
    id: &str,
) -> (Option<String>, Option<String>, Option<String>, Option<i64>) {
    let connection = dimasik_backend::db::open_database(database).expect("database opens");
    connection
        .query_row(
            "SELECT post_id, province_id, settlement_id, attached_ms FROM post_images WHERE id = ?1",
            rusqlite::params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("image row")
}

/// The province PATCH keeps its slug/description contract and gains the
/// optional ordered gallery: omitted fields keep the stored images, a null
/// description is allowed next to photos, and the public publication exposes
/// the gallery exactly while the province is published.
#[tokio::test]
async fn province_patch_manages_gallery_and_publishes_it() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;
    let database = env.config.database_path.clone();
    let now_ms = dimasik_backend::util::now_unix() * 1_000;
    insert_pending_image(&database, "img-prov-first", 640, 480, now_ms);
    insert_pending_image(&database, "img-prov-second", 800, 600, now_ms);

    // The default publication carries an empty gallery, and pending uploads
    // stay administrator-only while attached images of a published province
    // are readable (the storage layer is unconfigured, so the read stops at
    // 503 instead of 404).
    let before = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    assert_eq!(before.body["images"], json!([]));
    let pending_anonymous = api(
        &app,
        Method::GET,
        "/api/post-images/img-prov-first/original",
        None,
        None,
    )
    .await;
    assert_eq!(pending_anonymous.status, StatusCode::UNAUTHORIZED);
    let pending_admin = api(
        &app,
        Method::GET,
        "/api/post-images/img-prov-first/original",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(pending_admin.status, StatusCode::SERVICE_UNAVAILABLE);

    // A gallery with a null description: allowed for a province, and the
    // submitted order is the stored order.
    let saved = api(
        &app,
        Method::PATCH,
        "/api/gubernias/ryazan",
        Some(&cookie),
        Some(json!({
            "slug": "ryazanskaya",
            "description": Value::Null,
            "imageIds": ["img-prov-second", "img-prov-first"]
        })),
    )
    .await;
    assert_eq!(saved.status, StatusCode::OK);
    assert_eq!(saved.body["description"], Value::Null);
    assert_eq!(saved.body["images"][0]["id"], "img-prov-second");
    assert_eq!(saved.body["images"][1]["id"], "img-prov-first");
    assert_eq!(saved.body["images"][1]["width"], 640);
    assert_eq!(
        saved.body["images"][1]["originalUrl"],
        "/api/post-images/img-prov-first/original"
    );
    let (post_owner, province_owner, settlement_owner, attached) =
        image_owners(&database, "img-prov-first");
    assert_eq!(post_owner, None);
    assert_eq!(province_owner.as_deref(), Some("ryazan"));
    assert_eq!(settlement_owner, None);
    assert!(attached.is_some_and(|ms| ms > 0));

    // The public SSR read exposes the same ordered gallery, and its images
    // are publicly readable.
    let published = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    assert_eq!(published.body["images"][0]["id"], "img-prov-second");
    assert_eq!(published.body["images"][1]["id"], "img-prov-first");
    let public_read = api(
        &app,
        Method::GET,
        "/api/post-images/img-prov-first/original",
        None,
        None,
    )
    .await;
    assert_eq!(public_read.status, StatusCode::SERVICE_UNAVAILABLE);

    // An omitted field keeps the stored gallery untouched.
    let omitted = api(
        &app,
        Method::PATCH,
        "/api/gubernias/ryazan",
        Some(&cookie),
        Some(json!({ "slug": "ryazanskaya", "description": paragraph_doc("Сведения") })),
    )
    .await;
    assert_eq!(omitted.status, StatusCode::OK);
    assert_eq!(omitted.body["description"], paragraph_doc("Сведения"));
    assert_eq!(omitted.body["images"][0]["id"], "img-prov-second");
    assert_eq!(omitted.body["images"][1]["id"], "img-prov-first");

    // A replacement that leaves one image out detaches exactly that row.
    let reduced = api(
        &app,
        Method::PATCH,
        "/api/gubernias/ryazan",
        Some(&cookie),
        Some(json!({
            "slug": "ryazanskaya",
            "description": paragraph_doc("Сведения"),
            "imageIds": ["img-prov-first"]
        })),
    )
    .await;
    assert_eq!(reduced.status, StatusCode::OK);
    assert_eq!(reduced.body["images"][0]["id"], "img-prov-first");
    let (_, detached_owner, _, detached_marker) = image_owners(&database, "img-prov-second");
    assert_eq!(detached_owner, None);
    assert!(detached_marker.is_some_and(|ms| ms > 0));
    let gone = api(
        &app,
        Method::GET,
        "/api/post-images/img-prov-second/original",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

/// Cross-owner ids and malformed lists fail the whole province save without
/// touching slug, description or gallery, and unpublishing detaches the
/// province gallery together with the images of its deleted posts.
#[tokio::test]
async fn province_gallery_conflicts_roll_back_and_unpublish_detaches() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;
    let database = env.config.database_path.clone();
    let now_ms = dimasik_backend::util::now_unix() * 1_000;
    insert_pending_image(&database, "img-owner-prov", 100, 80, now_ms);
    insert_pending_image(&database, "img-owner-post", 200, 160, now_ms);
    insert_pending_image(&database, "img-owner-free", 300, 240, now_ms);

    let post = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Пост",
            "body": paragraph_doc("Текст"),
            "category": "Статья",
            "imageIds": ["img-owner-post"]
        })),
    )
    .await;
    assert_eq!(post.status, StatusCode::CREATED);

    let seeded = api(
        &app,
        Method::PATCH,
        "/api/gubernias/ryazan",
        Some(&cookie),
        Some(json!({
            "slug": "ryazanskaya",
            "description": paragraph_doc("Сведения"),
            "imageIds": ["img-owner-prov"]
        })),
    )
    .await;
    assert_eq!(seeded.status, StatusCode::OK);
    assert_eq!(seeded.body["images"][0]["id"], "img-owner-prov");

    // A request that claims a fresh image before failing on the post-owned
    // one must roll the whole save back.
    let conflict = api(
        &app,
        Method::PATCH,
        "/api/gubernias/ryazan",
        Some(&cookie),
        Some(json!({
            "slug": "drugoy-slug",
            "description": paragraph_doc("Испорченное"),
            "imageIds": ["img-owner-free", "img-owner-post"]
        })),
    )
    .await;
    assert_eq!(conflict.status, StatusCode::CONFLICT);
    let (_, free_owner, _, free_attached) = image_owners(&database, "img-owner-free");
    assert_eq!(free_owner, None, "the failed save must not claim the fresh id");
    assert_eq!(free_attached, None);
    let gubernia = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    assert_eq!(gubernia.body["slug"], "ryazanskaya");
    assert_eq!(gubernia.body["description"], paragraph_doc("Сведения"));
    assert_eq!(gubernia.body["images"][0]["id"], "img-owner-prov");

    // Malformed lists are rejected before anything is written.
    let duplicates = api(
        &app,
        Method::PATCH,
        "/api/gubernias/ryazan",
        Some(&cookie),
        Some(json!({
            "slug": "drugoy-slug",
            "description": paragraph_doc("Испорченное"),
            "imageIds": ["img-owner-free", "img-owner-free"]
        })),
    )
    .await;
    assert_eq!(duplicates.status, StatusCode::BAD_REQUEST);
    let after = api(&app, Method::GET, "/internal/gubernia/ryazanskaya", None, None).await;
    assert_eq!(after.body["slug"], "ryazanskaya", "no partial slug write");

    // Unpublishing deletes the posts and detaches both galleries.
    let unpublished = api(
        &app,
        Method::DELETE,
        "/api/gubernias/ryazan",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(unpublished.status, StatusCode::NO_CONTENT);
    let (_, province_owner, _, province_marker) = image_owners(&database, "img-owner-prov");
    assert_eq!(province_owner, None);
    assert!(province_marker.is_some_and(|ms| ms > 0));
    let (post_owner, _, _, post_marker) = image_owners(&database, "img-owner-post");
    assert_eq!(post_owner, None, "the deleted post detaches its image");
    assert!(post_marker.is_some_and(|ms| ms > 0));
    let gone = api(
        &app,
        Method::GET,
        "/api/post-images/img-owner-prov/original",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

/// The settlement reference PATCH gains the optional ordered gallery: an
/// empty document is accepted while images remain, body-only saves keep the
/// photos, a foreign id rolls everything back and the About editor stays
/// without a photo field.
#[tokio::test]
async fn settlement_reference_accepts_images_and_preserves_body_only_saves() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;
    let database = env.config.database_path.clone();
    let now_ms = dimasik_backend::util::now_unix() * 1_000;
    insert_pending_image(&database, "img-ref-first", 640, 480, now_ms);
    insert_pending_image(&database, "img-ref-second", 800, 600, now_ms);

    let created = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/settlements",
        Some(&cookie),
        Some(json!({
            "name": "С фотографиями",
            "uyezdId": "uyezd-1897-2",
            "coordinates": "22, 22",
            "url": "/naselennyy-punkt/ref-photos",
            "type": "Село"
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let settlement_id = created.body["id"].as_str().unwrap().to_string();
    let canonical_empty = json!({ "type": "doc", "content": [{ "type": "paragraph" }] });
    let text = paragraph_doc("Справка");

    // An image-only reference: the empty document is accepted with photos and
    // stored canonically, in the submitted order.
    let saved = api(
        &app,
        Method::PATCH,
        "/api/naselennyy-punkt/ref-photos/reference",
        Some(&cookie),
        Some(json!({
            "body": { "type": "doc", "content": [] },
            "imageIds": ["img-ref-second", "img-ref-first"]
        })),
    )
    .await;
    assert_eq!(saved.status, StatusCode::OK);
    assert_eq!(saved.body["body"], canonical_empty);
    assert_eq!(saved.body["images"][0]["id"], "img-ref-second");
    assert_eq!(saved.body["images"][1]["id"], "img-ref-first");
    let (post_owner, province_owner, settlement_owner, _) =
        image_owners(&database, "img-ref-first");
    assert_eq!(post_owner, None);
    assert_eq!(province_owner, None);
    assert_eq!(settlement_owner.as_deref(), Some(settlement_id.as_str()));

    // The public SSR read returns the body plus the gallery.
    let reference = api(
        &app,
        Method::GET,
        &format!("/internal/settlement-reference/{settlement_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(reference.status, StatusCode::OK);
    assert_eq!(reference.body["body"], canonical_empty);
    assert_eq!(reference.body["images"].as_array().unwrap().len(), 2);
    assert_eq!(reference.body["images"][0]["id"], "img-ref-second");

    // A body-only save (the old payload shape) preserves the gallery.
    let body_only = api(
        &app,
        Method::PATCH,
        "/api/naselennyy-punkt/ref-photos/reference",
        Some(&cookie),
        Some(json!({ "body": text })),
    )
    .await;
    assert_eq!(body_only.status, StatusCode::OK);
    assert_eq!(body_only.body["images"].as_array().unwrap().len(), 2);

    // Empty text without photos is still rejected, and the failed save may
    // not drop the stored images either.
    let rejected = api(
        &app,
        Method::PATCH,
        "/api/naselennyy-punkt/ref-photos/reference",
        Some(&cookie),
        Some(json!({ "body": { "type": "doc", "content": [] }, "imageIds": [] })),
    )
    .await;
    assert_eq!(rejected.status, StatusCode::BAD_REQUEST);
    let unchanged = api(
        &app,
        Method::GET,
        &format!("/internal/settlement-reference/{settlement_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(unchanged.body["body"], text);
    assert_eq!(unchanged.body["images"].as_array().unwrap().len(), 2);

    // Removing one image detaches only that row.
    let reduced = api(
        &app,
        Method::PATCH,
        "/api/naselennyy-punkt/ref-photos/reference",
        Some(&cookie),
        Some(json!({ "body": text, "imageIds": ["img-ref-first"] })),
    )
    .await;
    assert_eq!(reduced.status, StatusCode::OK);
    assert_eq!(reduced.body["images"][0]["id"], "img-ref-first");
    let (_, _, detached_owner, detached_marker) = image_owners(&database, "img-ref-second");
    assert_eq!(detached_owner, None);
    assert!(detached_marker.is_some_and(|ms| ms > 0));

    // A post-owned image is refused and changes nothing.
    insert_pending_image(&database, "img-ref-post", 10, 10, now_ms);
    let post = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Владелец",
            "body": paragraph_doc("Текст"),
            "category": "Статья",
            "imageIds": ["img-ref-post"]
        })),
    )
    .await;
    assert_eq!(post.status, StatusCode::CREATED);
    let conflict = api(
        &app,
        Method::PATCH,
        "/api/naselennyy-punkt/ref-photos/reference",
        Some(&cookie),
        Some(json!({ "body": text, "imageIds": ["img-ref-post"] })),
    )
    .await;
    assert_eq!(conflict.status, StatusCode::CONFLICT);
    let after_conflict = api(
        &app,
        Method::GET,
        &format!("/internal/settlement-reference/{settlement_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(after_conflict.body["body"], text);
    assert_eq!(after_conflict.body["images"][0]["id"], "img-ref-first");

    // The About editor did not grow a photo field.
    let about = api(
        &app,
        Method::PATCH,
        "/api/about-content",
        Some(&cookie),
        Some(json!({ "body": paragraph_doc("О проекте"), "imageIds": [] })),
    )
    .await;
    assert_eq!(about.status, StatusCode::BAD_REQUEST);

    // Deleting the settlement detaches the whole reference gallery.
    let deleted = api(
        &app,
        Method::DELETE,
        &format!("/api/gubernias/ryazan/settlements/{settlement_id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let (_, _, gone_owner, gone_marker) = image_owners(&database, "img-ref-first");
    assert_eq!(gone_owner, None);
    assert!(gone_marker.is_some_and(|ms| ms > 0));
    let gone = api(
        &app,
        Method::GET,
        "/api/post-images/img-ref-first/original",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}
