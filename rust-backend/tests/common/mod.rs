//! Shared helpers for the integration tests. Each test binary compiles this
//! module separately, so helpers used by only one binary are expected.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use dimasik_backend::config::Config;
use dimasik_backend::{build_router, prepare};

pub const ADMIN_COOKIE: &str = "dimasik_admin_session";
pub const LEGACY_SETTLEMENT_ID: &str = "11111111-1111-4111-8111-111111111111";
pub const LEGACY_POST_ID: &str = "aaaaaaaa-0000-4000-8000-000000000001";
pub const OLDER_POST_ID: &str = "aaaaaaaa-0000-4000-8000-000000000002";

pub struct TestEnv {
    pub dir: tempfile::TempDir,
    pub app: Router,
    pub config: Config,
}

pub fn square(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Value {
    json!([
        [min_x, min_y],
        [max_x, min_y],
        [max_x, max_y],
        [min_x, max_y],
        [min_x, min_y]
    ])
}

/// 76 canonical features: `abos`, `ryazan`, `tula` plus 73 numbered provinces.
pub fn canonical_geojson(published: &[(&str, &str)]) -> Value {
    let mut features = Vec::new();
    let mut push = |id: String, name: String, slug: Option<&str>, offset: f64| {
        let is_published = slug.is_some();
        let mut properties = json!({
            "id": id,
            "name": name,
            "published": is_published,
            "labelCoordinates": [offset, offset],
            "labelMinZoom": 5,
        });
        if let Some(slug) = slug {
            properties["slug"] = Value::String(slug.to_string());
        }
        features.push(json!({
            "type": "Feature",
            "properties": properties,
            "geometry": {
                "type": "Polygon",
                "coordinates": [square(offset, offset, offset + 1.0, offset + 1.0)],
            },
        }));
    };

    let slug_for = |id: &str| -> Option<&str> {
        published
            .iter()
            .find(|(candidate, _)| *candidate == id)
            .map(|(_, slug)| *slug)
    };
    push(
        "abos".to_string(),
        "Абосская губерния".to_string(),
        slug_for("abos"),
        100.0,
    );
    push(
        "ryazan".to_string(),
        "Рязанская губерния".to_string(),
        slug_for("ryazan"),
        0.0,
    );
    push(
        "tula".to_string(),
        "Тульская губерния".to_string(),
        slug_for("tula"),
        10.0,
    );
    for index in 1..=73 {
        push(
            format!("gubernia-1897-{index}"),
            format!("Губерния {index}"),
            None,
            offset_for(index),
        );
    }

    json!({ "type": "FeatureCollection", "features": features })
}

fn offset_for(index: usize) -> f64 {
    200.0 + (index as f64) * 2.0
}

/// Ryazan districts: one polygon with a hole, one fragmented multipolygon.
pub fn ryazan_districts() -> Value {
    json!({
        "type": "FeatureCollection",
        "features": [
            {
                "type": "Feature",
                "properties": { "kind": "province", "id": "ryazan" },
                "geometry": { "type": "Polygon", "coordinates": [square(0.0, 0.0, 10.0, 10.0)] }
            },
            {
                "type": "Feature",
                "properties": { "kind": "district", "id": "uyezd-1897-1", "name": "Скопинский уезд", "provinceId": "ryazan" },
                "geometry": { "type": "Polygon", "coordinates": [
                    square(0.0, 0.0, 10.0, 10.0),
                    square(4.0, 4.0, 6.0, 6.0)
                ] }
            },
            {
                "type": "Feature",
                "properties": { "kind": "district", "id": "uyezd-1897-2", "name": "Ряжский уезд", "provinceId": "ryazan" },
                "geometry": { "type": "MultiPolygon", "coordinates": [
                    [square(20.0, 20.0, 24.0, 24.0)],
                    [square(30.0, 30.0, 34.0, 34.0)]
                ] }
            }
        ]
    })
}

pub fn tula_districts() -> Value {
    json!({
        "type": "FeatureCollection",
        "features": [
            {
                "type": "Feature",
                "properties": { "kind": "province", "id": "tula" },
                "geometry": { "type": "Polygon", "coordinates": [square(100.0, 100.0, 110.0, 110.0)] }
            },
            {
                "type": "Feature",
                "properties": { "kind": "district", "id": "uyezd-1897-9", "name": "Тульский уезд", "provinceId": "tula" },
                "geometry": { "type": "Polygon", "coordinates": [square(100.0, 100.0, 110.0, 110.0)] }
            }
        ]
    })
}

/// A one-district atlas file, used for `abos` and the 73 numbered provinces so
/// the fixture ships every canonical district file like the real atlas.
pub fn single_district(
    province_id: &str,
    district_id: &str,
    district_name: &str,
    offset: f64,
) -> Value {
    json!({
        "type": "FeatureCollection",
        "features": [
            {
                "type": "Feature",
                "properties": { "kind": "province", "id": province_id },
                "geometry": { "type": "Polygon", "coordinates": [square(offset, offset, offset + 1.0, offset + 1.0)] }
            },
            {
                "type": "Feature",
                "properties": { "kind": "district", "id": district_id, "name": district_name, "provinceId": province_id },
                "geometry": { "type": "Polygon", "coordinates": [square(offset, offset, offset + 1.0, offset + 1.0)] }
            }
        ]
    })
}

pub fn paragraph(text: &str) -> Value {
    json!({ "type": "paragraph", "content": [{ "type": "text", "text": text }] })
}

pub fn paragraph_doc(text: &str) -> Value {
    json!({ "type": "doc", "content": [paragraph(text)] })
}

pub fn publications_fixture(canonical_ids: &[String]) -> Value {
    let mut gubernias = serde_json::Map::new();
    for id in canonical_ids {
        let entry = match id.as_str() {
            "ryazan" => json!({
                "published": true,
                "slug": "ryazanskaya",
                "description": "",
                "posts": [
                    {
                        "id": LEGACY_POST_ID,
                        "title": "Первый",
                        "body": paragraph_doc("Текст первый"),
                        "createdAt": "2026-10-07T10:00:00.000Z",
                        "updatedAt": "2026-10-07T10:00:00.000Z",
                        "uyezdId": "uyezd-1897-1",
                        "settlementId": null,
                        "year": "1900",
                        "archiveReference": ""
                    },
                    {
                        "id": OLDER_POST_ID,
                        "title": "Второй",
                        "body": paragraph_doc("Текст второй"),
                        "createdAt": "2026-10-06T10:00:00.000Z",
                        "updatedAt": "2026-10-06T10:00:00.000Z",
                        "uyezdId": "uyezd-1897-1",
                        "settlementId": LEGACY_SETTLEMENT_ID,
                        "year": "",
                        "archiveReference": "Ф. 1, оп. 2"
                    }
                ],
                "settlements": [
                    {
                        "id": LEGACY_SETTLEMENT_ID,
                        "name": "Павелец",
                        "guberniaId": "ryazan",
                        "uyezdId": "uyezd-1897-1",
                        "latitude": 5,
                        "longitude": 5,
                        "createdAt": "2026-10-06T09:00:00.000Z"
                    }
                ]
            }),
            "tula" => json!({
                "published": true,
                "slug": "tulskaya",
                "description": "",
                "posts": [],
                "settlements": []
            }),
            _ => json!({
                "published": false,
                "slug": null,
                "description": "",
                "posts": [],
                "settlements": []
            }),
        };
        gubernias.insert(id.clone(), entry);
    }
    json!({ "version": 1, "gubernias": Value::Object(gubernias) })
}

pub fn settings_fixture() -> Value {
    json!({
        "version": 1,
        "categories": ["Статья", "Очерк"],
        "settlementTypes": ["Город", "Село", "Погост"],
    })
}

pub fn about_fixture() -> Value {
    json!({ "version": 1, "body": paragraph_doc("О проекте Dimasik") })
}

pub fn references_fixture() -> Value {
    json!({
        "version": 1,
        "references": {
            LEGACY_SETTLEMENT_ID: paragraph_doc("Справка о Павельце")
        }
    })
}

pub fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("fixture directory");
    }
    std::fs::write(path, format!("{value}\n")).expect("fixture write");
}

pub fn canonical_ids() -> Vec<String> {
    let mut ids = vec!["abos".to_string(), "ryazan".to_string(), "tula".to_string()];
    for index in 1..=73 {
        ids.push(format!("gubernia-1897-{index}"));
    }
    ids
}

pub fn base_config(dir: &Path) -> Config {
    Config {
        database_path: dir.join("db/dimasik.sqlite"),
        data_dir: dir.join("data"),
        import_dir: dir.join("data"),
        public_data_dir: dir.join("public/data"),
        bind_addr: "127.0.0.1:0".to_string(),
        frontend_origin: Some("http://localhost:3000".to_string()),
        admin_username: Some("admin".to_string()),
        admin_password: Some("secret".to_string()),
        admin_session_secret: Some("0123456789abcdef0123456789abcdef".to_string()),
        // Image storage stays unconfigured: uploads answer 503 and the image
        // lifecycle tests exercise the database side directly.
        s3_endpoint: None,
        s3_region: None,
        s3_bucket: None,
        s3_access_key_id: None,
        s3_secret_access_key: None,
        yandex_metrika_oauth_token: None,
        yandex_metrika_counter_id: None,
        fresh_install: false,
    }
}

/// Writes all 76 canonical district files below
/// `<root>/public/data/uyezds-1897`, mirroring the real atlas layout.
pub fn write_district_atlas(root: &Path) {
    write_json(
        &root.join("public/data/uyezds-1897/ryazan.geojson"),
        &ryazan_districts(),
    );
    write_json(
        &root.join("public/data/uyezds-1897/tula.geojson"),
        &tula_districts(),
    );
    write_json(
        &root.join("public/data/uyezds-1897/abos.geojson"),
        &single_district("abos", "uyezd-1897-900", "Абосский уезд", 100.0),
    );
    for index in 1..=73 {
        let province_id = format!("gubernia-1897-{index}");
        write_json(
            &root.join(format!("public/data/uyezds-1897/{province_id}.geojson")),
            &single_district(
                &province_id,
                &format!("uyezd-1897-{}", 1_000 + index),
                &format!("Уезд {index}"),
                offset_for(index),
            ),
        );
    }
}

/// Writes the standard fixture tree: canonical atlas (76 provinces with every
/// district file), the four JSON documents and a fresh temp directory.
pub fn standard_fixture_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    write_json(
        &root.join("public/data/gubernias.geojson"),
        &canonical_geojson(&[]),
    );
    write_district_atlas(root);
    write_json(
        &root.join("data/gubernia-publications.json"),
        &publications_fixture(&canonical_ids()),
    );
    write_json(&root.join("data/site-settings.json"), &settings_fixture());
    write_json(&root.join("data/about-content.json"), &about_fixture());
    write_json(
        &root.join("data/settlement-references.json"),
        &references_fixture(),
    );
    dir
}

impl TestEnv {
    pub fn new() -> Self {
        let dir = standard_fixture_dir();
        Self::from_dir(dir)
    }

    pub fn from_dir(dir: tempfile::TempDir) -> Self {
        let config = base_config(dir.path());
        let state = prepare(&config).expect("environment prepares");
        let app = build_router(state);
        Self { dir, app, config }
    }

    pub fn prepare_again(&self) -> Result<(), String> {
        prepare(&self.config).map(|_| ())
    }

    pub fn router_for_config(config: &Config) -> Router {
        build_router(prepare(config).expect("environment prepares"))
    }
}

pub struct TestResponse {
    pub status: StatusCode,
    pub body: Value,
    pub headers: HeaderMap,
}

impl TestResponse {
    pub fn error_message(&self) -> String {
        self.body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    pub fn cache_control(&self) -> Option<&str> {
        self.headers
            .get("cache-control")
            .and_then(|value| value.to_str().ok())
    }

    pub fn set_cookie(&self) -> Option<&str> {
        self.headers
            .get("set-cookie")
            .and_then(|value| value.to_str().ok())
    }
}

pub async fn send(app: &Router, request: Request<Body>) -> TestResponse {
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body reads");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).to_string()))
    };
    TestResponse {
        status,
        body,
        headers,
    }
}

pub fn request(method: Method, uri: &str, cookie: Option<&str>, body: Option<&Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("request builds"),
        None => builder.body(Body::empty()).expect("request builds"),
    }
}

pub fn raw_request(
    method: Method,
    uri: &str,
    cookie: Option<&str>,
    content_type: Option<&str>,
    body: &str,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    if let Some(content_type) = content_type {
        builder = builder.header("content-type", content_type);
    }
    builder.body(Body::from(body.to_string())).expect("request builds")
}

pub fn method(value: &str) -> Method {
    Method::from_bytes(value.as_bytes()).expect("method")
}

pub async fn login(app: &Router, login_name: &str, password: &str) -> TestResponse {
    send(
        app,
        request(
            Method::POST,
            "/api/admin/session",
            None,
            Some(&json!({ "login": login_name, "password": password })),
        ),
    )
    .await
}

/// Returns the `Cookie:` header value for the admin session.
pub async fn admin_cookie(app: &Router) -> String {
    let response = login(app, "admin", "secret").await;
    assert_eq!(response.status, StatusCode::OK, "login succeeds");
    response
        .set_cookie()
        .expect("set-cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string()
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}
