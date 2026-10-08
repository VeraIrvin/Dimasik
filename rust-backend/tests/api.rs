mod common;

use axum::http::{HeaderValue, Method, StatusCode};
use axum::Router;
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
    assert_eq!(
        published.body,
        json!({
            "id": "gubernia-1897-1",
            "name": "Губерния 1",
            "slug": "one",
            "description": "",
            "posts": [],
            "settlements": [],
        })
    );

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
        Some(json!({ "slug": "one-updated", "description": "Описание" })),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_eq!(updated.body["slug"], "one-updated");
    assert_eq!(updated.body["description"], "Описание");

    let too_long = api(
        &app,
        Method::PATCH,
        "/api/gubernias/gubernia-1897-1",
        Some(&cookie),
        Some(json!({ "slug": "one-updated", "description": "x".repeat(20_001) })),
    )
    .await;
    assert_eq!(too_long.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        too_long.error_message(),
        "Описание должно быть строкой не длиннее 20000 символов."
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
