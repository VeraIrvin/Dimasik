pub mod config;
pub mod content;
pub mod db;
pub mod error;
pub mod geo;
pub mod http;
pub mod import;
pub mod routes;
pub mod s3;
pub mod session;
pub mod store;
pub mod util;

use std::sync::{Arc, Mutex};

use axum::routing::{delete, get, patch, post};
use axum::Router;
use rusqlite::Connection;

use crate::config::Config;
use crate::error::{ApiError, ApiResult};
use crate::geo::{CanonicalProvinces, GeoRuntime};
use crate::s3::S3Storage;
use crate::session::AuthConfig;
use crate::util::now_unix;

/// Shared state: one serialized SQLite connection (the analogue of the
/// TypeScript in-process queues), the auth configuration, the cached canonical
/// atlas, the district geometry cache and the optional image storage.
#[derive(Clone)]
pub struct AppState {
    pub connection: Arc<Mutex<Connection>>,
    pub auth: AuthConfig,
    pub frontend_origin: Option<String>,
    pub canonical: Arc<CanonicalProvinces>,
    pub geo: Arc<GeoRuntime>,
    /// `None` when the S3_* variables are unset: uploads answer 503 and the
    /// rest of the API keeps working.
    pub image_storage: Option<Arc<S3Storage>>,
}

impl AppState {
    /// Runs one blocking store operation on the shared connection behind
    /// `spawn_blocking`, so async handlers never block the runtime.
    pub async fn call<T, F>(&self, operation: F) -> ApiResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> ApiResult<T> + Send + 'static,
    {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = connection
                .lock()
                .map_err(|_| ApiError::internal("database lock poisoned"))?;
            operation(&mut guard)
        })
        .await
        .map_err(ApiError::internal)?
    }
}

/// Loads the canonical atlas, opens/migrates the database, runs the first
/// bootstrap import when needed and verifies the province set. Any failure is
/// fatal for `serve`: the service never runs against a half-imported database.
pub fn prepare(config: &Config) -> Result<AppState, String> {
    let canonical_path = config.public_data_dir.join("gubernias.geojson");
    let canonical = Arc::new(CanonicalProvinces::load(&canonical_path)?);
    let geo = Arc::new(GeoRuntime::new(&config.public_data_dir));

    let mut connection = db::open_database(&config.database_path)?;
    db::migrate(&mut connection)?;
    if db::imported_at(&connection)?.is_none() {
        match import::import_bootstrap(&mut connection, config, &canonical, &geo)? {
            Some(summary) => println!(
                "Imported {} published provinces, {} districts, {} posts, {} settlements, {} references from {}.",
                summary.published,
                summary.districts,
                summary.posts,
                summary.settlements,
                summary.references,
                config.import_dir.display()
            ),
            None => println!("Database was imported by another process; continuing."),
        }
    } else {
        // Databases created before migration 2 get the district registry
        // backfilled once from the same canonical district files.
        let backfilled = import::ensure_districts(&mut connection, &canonical, &geo)?;
        if backfilled > 0 {
            println!(
                "Backfilled {backfilled} district rows from {}.",
                config.public_data_dir.display()
            );
        }
    }
    db::verify_provinces(&connection, &canonical)?;

    let auth = AuthConfig {
        username: config.admin_username.clone(),
        password: config.admin_password.clone(),
        secret: config.admin_session_secret.clone(),
    };
    // Unconfigured storage is not an error (empty posts must keep working);
    // partial or invalid S3_* configuration logs the reason and disables it.
    let image_storage = S3Storage::setup(config).map(Arc::new);
    Ok(AppState {
        connection: Arc::new(Mutex::new(connection)),
        auth,
        frontend_origin: config.frontend_origin.clone(),
        canonical,
        geo,
        image_storage,
    })
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route(
            "/api/gubernias",
            get(routes::gubernias_get).post(routes::gubernias_post),
        )
        .route(
            "/api/gubernias/{id}",
            patch(routes::gubernia_patch).delete(routes::gubernia_delete),
        )
        .route("/api/gubernias/{id}/posts", post(routes::post_create))
        .route(
            "/api/gubernias/{id}/posts/{post_id}",
            patch(routes::post_update).delete(routes::post_delete),
        )
        .route(
            "/api/post-images",
            post(routes::post_image_upload)
                .layer(axum::extract::DefaultBodyLimit::max(routes::MAX_UPLOAD_REQUEST_BYTES)),
        )
        .route(
            "/api/post-images/{id}/{variant}",
            get(routes::post_image_get),
        )
        .route("/api/post-images/{id}", delete(routes::post_image_delete))
        .route(
            "/api/gubernias/{id}/settlements",
            post(routes::settlement_create),
        )
        .route(
            "/api/gubernias/{id}/settlements/{settlement_id}",
            delete(routes::settlement_delete),
        )
        .route(
            "/api/naselennyy-punkt/{slug}/reference",
            patch(routes::reference_patch),
        )
        .route("/api/about-content", patch(routes::about_patch))
        .route(
            "/api/nastroyki",
            post(routes::settings_post)
                .patch(routes::settings_patch)
                .delete(routes::settings_delete),
        )
        .route(
            "/api/admin/session",
            post(routes::session_post).delete(routes::session_delete),
        )
        .route("/internal/session", get(routes::internal_session))
        .route("/internal/geo", get(routes::internal_geo))
        .route("/internal/gubernia/{slug}", get(routes::internal_gubernia))
        .route(
            "/internal/settlement/{slug}",
            get(routes::internal_settlement),
        )
        .route(
            "/internal/settlement-reference/{id}",
            get(routes::internal_settlement_reference),
        )
        .route("/internal/about", get(routes::internal_about))
        .route("/internal/settings", get(routes::internal_settings))
        .route("/internal/metrics", get(routes::internal_metrics))
        .layer(axum::middleware::from_fn(http::no_store_middleware))
        .with_state(state)
}

/// Deletes storage objects that no row needs any more — pending uploads past
/// their TTL and rows detached from deleted posts — and drops each row only
/// after both objects are gone. It runs once at startup and opportunistically
/// after uploads and deletions.
///
/// Every candidate is fenced in the database before its objects are touched,
/// so an upload that finishes its PUTs after the listing can no longer
/// complete a row whose objects this pass deletes (and a post can no longer
/// claim it). A pass walks the queue behind a `(sort_key, id)` cursor: a row
/// whose deletion fails stays fenced with its persisted keys for a later pass,
/// but it never stops the current pass from cleaning the rows behind it.
pub async fn sweep_image_objects(state: &AppState) {
    const BATCH: u32 = 32;
    let Some(storage) = state.image_storage.clone() else {
        return;
    };
    let mut cursor: Option<store::ImageCleanupCursor> = None;
    loop {
        let now_ms = now_unix() * 1_000;
        let after = cursor.clone();
        let batch = match state
            .call(move |conn| store::list_image_cleanup_batch(conn, now_ms, after.as_ref(), BATCH))
            .await
        {
            Ok(batch) => batch,
            Err(error) => {
                eprintln!(
                    "Backend failure: image cleanup scan failed: {}",
                    error.message
                );
                return;
            }
        };
        let Some(last) = batch.last() else {
            return;
        };
        cursor = Some(store::ImageCleanupCursor::after(last));
        for image in batch {
            let id = image.id.clone();
            let claimed = match state
                .call(move |conn| store::claim_image_cleanup(conn, &id, now_ms))
                .await
            {
                Ok(claimed) => claimed,
                Err(error) => {
                    eprintln!(
                        "Backend failure: cannot fence the image {} for cleanup: {}",
                        image.id, error.message
                    );
                    continue;
                }
            };
            let Some(fenced) = claimed else {
                // The row stopped being a cleanup candidate: an upload
                // completed, a post claimed it or another pass finished it.
                continue;
            };
            if let Err(error) = storage
                .delete_image(&fenced.original_key, &fenced.thumbnail_key)
                .await
            {
                eprintln!(
                    "Backend failure: cannot delete image objects for {}: {}",
                    fenced.id, error.message
                );
                // The row stays fenced with its keys: a later pass retries it.
                continue;
            }
            let id = fenced.id.clone();
            match state
                .call(move |conn| store::finish_image_cleanup(conn, &id))
                .await
            {
                Ok(_) => {}
                Err(error) => eprintln!(
                    "Backend failure: cannot finish image cleanup for {}: {}",
                    fenced.id, error.message
                ),
            }
        }
    }
}

pub async fn run(config: Config) -> Result<(), String> {
    let state = prepare(&config)?;
    // Abandoned uploads and detached objects are retried at every start.
    sweep_image_objects(&state).await;
    let listener = tokio::net::TcpListener::bind(&config.bind_addr)
        .await
        .map_err(|error| format!("Cannot bind {}: {error}", config.bind_addr))?;
    println!("dimasik-backend listening on http://{}", config.bind_addr);
    axum::serve(listener, build_router(state))
        .await
        .map_err(|error| format!("Server error: {error}"))
}

/// `import` command: migrations plus the one-time bootstrap import (`--force`
/// wipes the mutable state first, which is only meant for fixture verification).
pub fn run_import(config: &Config, force: bool) -> Result<(), String> {
    let canonical_path = config.public_data_dir.join("gubernias.geojson");
    let canonical = CanonicalProvinces::load(&canonical_path)?;
    let geo = GeoRuntime::new(&config.public_data_dir);
    let mut connection = db::open_database(&config.database_path)?;
    db::migrate(&mut connection)?;
    import::run_import_command(&mut connection, config, &canonical, &geo, force)?;
    db::verify_provinces(&connection, &canonical)?;
    println!("Database {} is ready.", config.database_path.display());
    Ok(())
}
