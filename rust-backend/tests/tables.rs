mod common;

use axum::http::{Method, StatusCode};
use axum::Router;
use serde_json::{json, Value};

use common::{admin_cookie, request, send, TestEnv, TestResponse, LEGACY_SETTLEMENT_ID};

async fn api(
    app: &Router,
    http_method: Method,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> TestResponse {
    send(app, request(http_method, uri, cookie, body.as_ref())).await
}

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

/// A three-column grid whose right header spans both rows. It deliberately
/// exercises formatted text, both span directions, widths, alignment and a
/// nested table. Every cell uses TipTap's canonical four-attribute shape.
fn rich_table_doc(label: &str) -> Value {
    json!({
        "type": "doc",
        "content": [{
            "type": "table",
            "content": [
                {
                    "type": "tableRow",
                    "content": [
                        {
                            "type": "tableHeader",
                            "attrs": {
                                "colspan": 2,
                                "rowspan": 1,
                                "colwidth": [180, 220],
                                "align": "center"
                            },
                            "content": [{
                                "type": "paragraph",
                                "attrs": { "textAlign": "left" },
                                "content": [{
                                    "type": "text",
                                    "text": label,
                                    "marks": [{ "type": "bold" }]
                                }]
                            }]
                        },
                        {
                            "type": "tableHeader",
                            "attrs": {
                                "colspan": 1,
                                "rowspan": 2,
                                "colwidth": [140],
                                "align": "right"
                            },
                            "content": [{
                                "type": "paragraph",
                                "attrs": { "textAlign": "left" },
                                "content": [{ "type": "text", "text": "Источник" }]
                            }, {
                                "type": "heading",
                                "attrs": { "level": 2, "textAlign": "left" },
                                "content": [{ "type": "text", "text": "Выравнивание слева" }]
                            }]
                        }
                    ]
                },
                {
                    "type": "tableRow",
                    "content": [
                        {
                            "type": "tableCell",
                            "attrs": {
                                "colspan": 1,
                                "rowspan": 1,
                                "colwidth": null,
                                "align": null
                            },
                            "content": [{
                                "type": "paragraph",
                                "content": [{
                                    "type": "text",
                                    "text": "Левая ячейка",
                                    "marks": [{ "type": "italic" }]
                                }]
                            }]
                        },
                        {
                            "type": "tableCell",
                            "attrs": {
                                "colspan": 1,
                                "rowspan": 1,
                                "colwidth": [220],
                                "align": "left"
                            },
                            "content": [
                                { "type": "paragraph" },
                                {
                                    "type": "table",
                                    "content": [{
                                        "type": "tableRow",
                                        "content": [{
                                            "type": "tableCell",
                                            "attrs": {
                                                "colspan": 1,
                                                "rowspan": 1,
                                                "colwidth": null,
                                                "align": null
                                            },
                                            "content": [{
                                                "type": "paragraph",
                                                "content": [{ "type": "text", "text": "Вложенная" }]
                                            }]
                                        }]
                                    }]
                                }
                            ]
                        }
                    ]
                }
            ]
        }]
    })
}

fn blank_table_doc() -> Value {
    json!({
        "type": "doc",
        "content": [{
            "type": "table",
            "content": [{
                "type": "tableRow",
                "content": [{
                    "type": "tableCell",
                    "attrs": {
                        "colspan": 1,
                        "rowspan": 1,
                        "colwidth": null,
                        "align": null
                    },
                    "content": [{ "type": "paragraph" }]
                }]
            }]
        }]
    })
}

fn canonical_empty_doc() -> Value {
    json!({ "type": "doc", "content": [{ "type": "paragraph" }] })
}

fn invalid_table_attrs_doc() -> Value {
    json!({
        "type": "doc",
        "content": [{
            "type": "table",
            "content": [{
                "type": "tableRow",
                "content": [{
                    "type": "tableCell",
                    "attrs": {
                        "colspan": 0,
                        "rowspan": 1,
                        "colwidth": null,
                        "align": null
                    },
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "Недопустимо" }]
                    }]
                }]
            }]
        }]
    })
}

fn ragged_table_doc() -> Value {
    let cell = |text: &str| json!({
        "type": "tableCell",
        "attrs": {
            "colspan": 1,
            "rowspan": 1,
            "colwidth": null,
            "align": null
        },
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": text }]
        }]
    });
    json!({
        "type": "doc",
        "content": [{
            "type": "table",
            "content": [
                { "type": "tableRow", "content": [cell("A"), cell("B")] },
                { "type": "tableRow", "content": [cell("C")] }
            ]
        }]
    })
}

fn find_post<'a>(snapshot: &'a Value, post_id: &str) -> &'a Value {
    snapshot["posts"]
        .as_array()
        .expect("posts")
        .iter()
        .find(|post| post["id"] == post_id)
        .expect("post in snapshot")
}

#[tokio::test]
async fn table_post_create_edit_move_and_reopen_preserve_body_metadata_and_images() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;
    let now_ms = dimasik_backend::util::now_unix() * 1_000;
    insert_pending_image(
        &env.config.database_path,
        "img-table-post",
        1280,
        720,
        now_ms,
    );
    insert_pending_image(
        &env.config.database_path,
        "img-table-blank-post",
        800,
        600,
        now_ms,
    );

    // A table containing no text is still blank and is rejected without an
    // image. The same body is accepted with an image and stored using the
    // existing canonical empty-document representation.
    let rejected_blank = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Пустая таблица",
            "body": blank_table_doc(),
            "category": "Очерк"
        })),
    )
    .await;
    assert_eq!(rejected_blank.status, StatusCode::BAD_REQUEST);

    let image_backed_blank = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Только снимок",
            "body": blank_table_doc(),
            "category": "Очерк",
            "imageIds": ["img-table-blank-post"]
        })),
    )
    .await;
    assert_eq!(image_backed_blank.status, StatusCode::CREATED);
    assert_eq!(image_backed_blank.body["body"], canonical_empty_doc());
    assert_eq!(
        image_backed_blank.body["images"][0]["id"],
        "img-table-blank-post"
    );

    // Creation itself exercises a nonblank formatted table and all post
    // metadata while retaining settlement placement and a gallery.
    let initial_body = rich_table_doc("Перепись 1912 года");
    let created = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/posts",
        Some(&cookie),
        Some(json!({
            "title": "Табличная публикация",
            "body": initial_body,
            "category": "Очерк",
            "settlementId": LEGACY_SETTLEMENT_ID,
            "year": "1912",
            "archiveReference": "Ф. 7, оп. 3, д. 2",
            "author": "А. Н. Автор",
            "imageIds": ["img-table-post"]
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(created.body["body"], initial_body);
    assert_eq!(created.body["settlementId"], LEGACY_SETTLEMENT_ID);
    assert_eq!(created.body["uyezdId"], "uyezd-1897-1");
    assert_eq!(created.body["images"][0]["id"], "img-table-post");
    let post_id = created.body["id"].as_str().expect("post id").to_owned();

    // Editing replaces one formatted, spanned grid with another without
    // disturbing settlement placement, metadata or the omitted gallery.
    let body = rich_table_doc("Перепись после правки");
    let edited = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Табличная публикация",
            "body": body,
            "category": "Очерк",
            "settlementId": LEGACY_SETTLEMENT_ID,
            "year": "1912",
            "archiveReference": "Ф. 7, оп. 3, д. 2",
            "author": "А. Н. Автор"
        })),
    )
    .await;
    assert_eq!(edited.status, StatusCode::OK);
    assert_eq!(edited.body["body"], body);
    assert_eq!(edited.body["images"][0]["id"], "img-table-post");

    let settlement = api(
        &app,
        Method::GET,
        &format!("/internal/settlement/{LEGACY_SETTLEMENT_ID}"),
        None,
        None,
    )
    .await;
    let settled = settlement.body["gubernia"]["posts"]
        .as_array()
        .expect("settlement posts")
        .iter()
        .find(|post| post["id"] == post_id)
        .expect("settled post");
    assert_eq!(settled["body"], body);
    assert_eq!(settled["category"], "Очерк");
    assert_eq!(settled["year"], "1912");
    assert_eq!(settled["archiveReference"], "Ф. 7, оп. 3, д. 2");
    assert_eq!(settled["author"], "А. Н. Автор");
    assert_eq!(settled["settlementId"], LEGACY_SETTLEMENT_ID);

    // Invalid attributes reject the entire full-save payload, including its
    // attempted metadata edits and gallery removal.
    let rejected = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Не сохранять",
            "body": invalid_table_attrs_doc(),
            "category": "Статья",
            "uyezdId": "uyezd-1897-2",
            "year": "2000",
            "archiveReference": "Другая ссылка",
            "author": "Другой автор",
            "imageIds": []
        })),
    )
    .await;
    assert_eq!(rejected.status, StatusCode::BAD_REQUEST);
    let after_rejection = api(
        &app,
        Method::GET,
        "/internal/gubernia/ryazanskaya",
        None,
        None,
    )
    .await;
    let unchanged = find_post(&after_rejection.body, &post_id);
    assert_eq!(unchanged["title"], "Табличная публикация");
    assert_eq!(unchanged["body"], body);
    assert_eq!(unchanged["category"], "Очерк");
    assert_eq!(unchanged["year"], "1912");
    assert_eq!(unchanged["archiveReference"], "Ф. 7, оп. 3, д. 2");
    assert_eq!(unchanged["author"], "А. Н. Автор");
    assert_eq!(unchanged["settlementId"], LEGACY_SETTLEMENT_ID);
    assert_eq!(unchanged["images"][0]["id"], "img-table-post");

    // A real cross-province move preserves the document, all metadata and the
    // image while replacing the now-inapplicable settlement placement.
    let moved = api(
        &app,
        Method::PATCH,
        &format!("/api/gubernias/ryazan/posts/{post_id}"),
        Some(&cookie),
        Some(json!({
            "title": "Табличная публикация",
            "body": body,
            "category": "Очерк",
            "targetGuberniaId": "tula",
            "uyezdId": "uyezd-1897-9",
            "year": "1912",
            "archiveReference": "Ф. 7, оп. 3, д. 2",
            "author": "А. Н. Автор"
        })),
    )
    .await;
    assert_eq!(moved.status, StatusCode::OK);
    assert_eq!(moved.body["body"], body);
    assert_eq!(moved.body["settlementId"], Value::Null);
    assert_eq!(moved.body["uyezdId"], "uyezd-1897-9");
    assert_eq!(moved.body["images"][0]["id"], "img-table-post");

    env.prepare_again().expect("table post database reopens");
    let reopened = TestEnv::router_for_config(&env.config);
    let tula = api(
        &reopened,
        Method::GET,
        "/internal/gubernia/tulskaya",
        None,
        None,
    )
    .await;
    let persisted = find_post(&tula.body, &post_id);
    assert_eq!(persisted["body"], body);
    assert_eq!(persisted["category"], "Очерк");
    assert_eq!(persisted["year"], "1912");
    assert_eq!(persisted["archiveReference"], "Ф. 7, оп. 3, д. 2");
    assert_eq!(persisted["author"], "А. Н. Автор");
    assert_eq!(persisted["uyezdId"], "uyezd-1897-9");
    assert_eq!(persisted["images"][0]["id"], "img-table-post");
}

#[tokio::test]
async fn province_and_about_tables_round_trip_reopen_and_reject_invalid_grids_atomically() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;
    let now_ms = dimasik_backend::util::now_unix() * 1_000;
    insert_pending_image(
        &env.config.database_path,
        "img-table-province",
        900,
        600,
        now_ms,
    );
    let province_body = rich_table_doc("Губернские итоги");
    let about_body = rich_table_doc("О таблицах проекта");

    let province = api(
        &app,
        Method::PATCH,
        "/api/gubernias/ryazan",
        Some(&cookie),
        Some(json!({
            "slug": "ryazanskaya",
            "description": province_body,
            "imageIds": ["img-table-province"]
        })),
    )
    .await;
    assert_eq!(province.status, StatusCode::OK);
    assert_eq!(province.body["description"], province_body);
    assert_eq!(province.body["images"][0]["id"], "img-table-province");

    let about = api(
        &app,
        Method::PATCH,
        "/api/about-content",
        Some(&cookie),
        Some(json!({ "body": about_body })),
    )
    .await;
    assert_eq!(about.status, StatusCode::OK);
    assert_eq!(about.body["body"], about_body);

    let public_province = api(
        &app,
        Method::GET,
        "/internal/gubernia/ryazanskaya",
        None,
        None,
    )
    .await;
    assert_eq!(public_province.body["description"], province_body);
    let public_about = api(&app, Method::GET, "/internal/about", None, None).await;
    assert_eq!(public_about.body["body"], about_body);

    env.prepare_again().expect("shared table content reopens");
    let reopened = TestEnv::router_for_config(&env.config);
    let reopened_province = api(
        &reopened,
        Method::GET,
        "/internal/gubernia/ryazanskaya",
        None,
        None,
    )
    .await;
    assert_eq!(reopened_province.body["description"], province_body);
    assert_eq!(reopened_province.body["images"][0]["id"], "img-table-province");
    let reopened_about = api(&reopened, Method::GET, "/internal/about", None, None).await;
    assert_eq!(reopened_about.body["body"], about_body);

    // Ragged geometry must not partially change slug, description or gallery.
    let invalid_province = api(
        &reopened,
        Method::PATCH,
        "/api/gubernias/ryazan",
        Some(&cookie),
        Some(json!({
            "slug": "ne-sohranyat",
            "description": ragged_table_doc(),
            "imageIds": []
        })),
    )
    .await;
    assert_eq!(invalid_province.status, StatusCode::BAD_REQUEST);
    let province_after = api(
        &reopened,
        Method::GET,
        "/internal/gubernia/ryazanskaya",
        None,
        None,
    )
    .await;
    assert_eq!(province_after.body["slug"], "ryazanskaya");
    assert_eq!(province_after.body["description"], province_body);
    assert_eq!(province_after.body["images"][0]["id"], "img-table-province");

    let invalid_about = api(
        &reopened,
        Method::PATCH,
        "/api/about-content",
        Some(&cookie),
        Some(json!({ "body": invalid_table_attrs_doc() })),
    )
    .await;
    assert_eq!(invalid_about.status, StatusCode::BAD_REQUEST);
    let blank_about = api(
        &reopened,
        Method::PATCH,
        "/api/about-content",
        Some(&cookie),
        Some(json!({ "body": blank_table_doc() })),
    )
    .await;
    assert_eq!(blank_about.status, StatusCode::BAD_REQUEST);
    let about_after = api(&reopened, Method::GET, "/internal/about", None, None).await;
    assert_eq!(about_after.body["body"], about_body);

    // Province descriptions are optional, so an otherwise valid blank grid is
    // treated as no visible description rather than fabricated text. Its
    // existing gallery remains attached when imageIds is omitted.
    let blank_province = api(
        &reopened,
        Method::PATCH,
        "/api/gubernias/ryazan",
        Some(&cookie),
        Some(json!({
            "slug": "ryazanskaya",
            "description": blank_table_doc()
        })),
    )
    .await;
    assert_eq!(blank_province.status, StatusCode::OK);
    assert_eq!(blank_province.body["description"], Value::Null);
    assert_eq!(blank_province.body["images"][0]["id"], "img-table-province");
}

#[tokio::test]
async fn settlement_reference_table_rename_content_and_gallery_are_atomic_and_durable() {
    let env = TestEnv::new();
    let app = env.app.clone();
    let cookie = admin_cookie(&app).await;
    let now_ms = dimasik_backend::util::now_unix() * 1_000;
    insert_pending_image(
        &env.config.database_path,
        "img-table-ref-first",
        640,
        480,
        now_ms,
    );
    insert_pending_image(
        &env.config.database_path,
        "img-table-ref-second",
        1024,
        768,
        now_ms,
    );

    let created = api(
        &app,
        Method::POST,
        "/api/gubernias/ryazan/settlements",
        Some(&cookie),
        Some(json!({
            "name": "Табличное село",
            "uyezdId": "uyezd-1897-2",
            "coordinates": "22, 22",
            "url": "/naselennyy-punkt/table-reference",
            "type": "Село"
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let settlement_id = created.body["id"]
        .as_str()
        .expect("settlement id")
        .to_owned();

    // A blank grid has no text, but is valid for an image-backed reference.
    // Existing blank-body semantics store the canonical empty paragraph.
    let blank = api(
        &app,
        Method::PATCH,
        "/api/naselennyy-punkt/table-reference/reference",
        Some(&cookie),
        Some(json!({
            "body": blank_table_doc(),
            "imageIds": ["img-table-ref-first"]
        })),
    )
    .await;
    assert_eq!(blank.status, StatusCode::OK);
    assert_eq!(blank.body["body"], canonical_empty_doc());
    assert_eq!(blank.body["images"][0]["id"], "img-table-ref-first");

    // Name-only editing preserves both the document and gallery.
    let name_only = api(
        &app,
        Method::PATCH,
        "/api/naselennyy-punkt/table-reference/reference",
        Some(&cookie),
        Some(json!({ "name": "  Село после правки  " })),
    )
    .await;
    assert_eq!(name_only.status, StatusCode::OK);
    assert_eq!(name_only.body["name"], "Село после правки");
    assert_eq!(name_only.body["body"], canonical_empty_doc());
    assert_eq!(name_only.body["images"][0]["id"], "img-table-ref-first");
    let after_name_only = api(
        &app,
        Method::GET,
        "/internal/settlement/table-reference",
        None,
        None,
    )
    .await;
    let renamed_settlement = &after_name_only.body["settlement"];
    assert_eq!(renamed_settlement["id"], created.body["id"]);
    assert_eq!(
        renamed_settlement["url"],
        "/naselennyy-punkt/table-reference"
    );
    assert_eq!(renamed_settlement["guberniaId"], created.body["guberniaId"]);
    assert_eq!(renamed_settlement["uyezdId"], created.body["uyezdId"]);
    assert_eq!(renamed_settlement["latitude"], created.body["latitude"]);
    assert_eq!(renamed_settlement["longitude"], created.body["longitude"]);
    assert_eq!(renamed_settlement["createdAt"], created.body["createdAt"]);
    assert_eq!(renamed_settlement["type"], created.body["type"]);

    // Rename, rich table replacement and ordered gallery replacement commit as
    // one operation.
    let body = rich_table_doc("Сведения о селе");
    let combined = api(
        &app,
        Method::PATCH,
        "/api/naselennyy-punkt/table-reference/reference",
        Some(&cookie),
        Some(json!({
            "name": "Окончательное название",
            "body": body,
            "imageIds": ["img-table-ref-second", "img-table-ref-first"]
        })),
    )
    .await;
    assert_eq!(combined.status, StatusCode::OK);
    assert_eq!(combined.body["name"], "Окончательное название");
    assert_eq!(combined.body["body"], body);
    assert_eq!(combined.body["images"][0]["id"], "img-table-ref-second");
    assert_eq!(combined.body["images"][1]["id"], "img-table-ref-first");

    let reference = api(
        &app,
        Method::GET,
        &format!("/internal/settlement-reference/{settlement_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(reference.body["body"], body);
    assert_eq!(reference.body["images"][0]["id"], "img-table-ref-second");
    let settlement = api(
        &app,
        Method::GET,
        "/internal/settlement/table-reference",
        None,
        None,
    )
    .await;
    assert_eq!(
        settlement.body["settlement"]["name"],
        "Окончательное название"
    );

    // Bad geometry rolls back the attempted rename, body and gallery removal.
    let rejected = api(
        &app,
        Method::PATCH,
        "/api/naselennyy-punkt/table-reference/reference",
        Some(&cookie),
        Some(json!({
            "name": "Не сохранять",
            "body": ragged_table_doc(),
            "imageIds": []
        })),
    )
    .await;
    assert_eq!(rejected.status, StatusCode::BAD_REQUEST);
    let after_rejection = api(
        &app,
        Method::GET,
        &format!("/internal/settlement-reference/{settlement_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(after_rejection.body["body"], body);
    assert_eq!(after_rejection.body["images"][0]["id"], "img-table-ref-second");
    assert_eq!(after_rejection.body["images"][1]["id"], "img-table-ref-first");
    let settlement_after = api(
        &app,
        Method::GET,
        "/internal/settlement/table-reference",
        None,
        None,
    )
    .await;
    assert_eq!(
        settlement_after.body["settlement"]["name"],
        "Окончательное название"
    );

    env.prepare_again().expect("table reference database reopens");
    let reopened = TestEnv::router_for_config(&env.config);
    let persisted = api(
        &reopened,
        Method::GET,
        &format!("/internal/settlement-reference/{settlement_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(persisted.body["body"], body);
    assert_eq!(persisted.body["images"][0]["id"], "img-table-ref-second");
    assert_eq!(persisted.body["images"][1]["id"], "img-table-ref-first");
    let persisted_settlement = api(
        &reopened,
        Method::GET,
        "/internal/settlement/table-reference",
        None,
        None,
    )
    .await;
    assert_eq!(
        persisted_settlement.body["settlement"]["name"],
        "Окончательное название"
    );
}
