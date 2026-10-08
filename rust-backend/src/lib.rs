pub mod config;
pub mod content;
pub mod db;
pub mod error;
pub mod geo;
pub mod http;
pub mod import;
pub mod routes;
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
use crate::session::AuthConfig;

/// Shared state: one serialized SQLite connection (the analogue of the
/// TypeScript in-process queues), the auth configuration, the cached canonical
/// atlas and the district geometry cache.
#[derive(Clone)]
pub struct AppState {
    pub connection: Arc<Mutex<Connection>>,
    pub auth: AuthConfig,
    pub frontend_origin: Option<String>,
    pub canonical: Arc<CanonicalProvinces>,
    pub geo: Arc<GeoRuntime>,
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
    Ok(AppState {
        connection: Arc::new(Mutex::new(connection)),
        auth,
        frontend_origin: config.frontend_origin.clone(),
        canonical,
        geo,
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

pub async fn run(config: Config) -> Result<(), String> {
    let state = prepare(&config)?;
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
