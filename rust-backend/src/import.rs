use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use rusqlite::{params, Connection, Transaction, TransactionBehavior};
use serde_json::Value;

use crate::config::Config;
use crate::content::{legacy_description_json, normalize_post_document};
use crate::db;
use crate::geo::{CanonicalProvinces, DistrictCollection, GeoRuntime};
use crate::store::{
    default_settings_lists, SettingsLists, MAX_ARCHIVE_REFERENCE_LENGTH, MAX_ID_LENGTH,
    MAX_SETTLEMENT_NAME_LENGTH, MAX_SETTINGS_ITEMS, MAX_SETTINGS_NAME_CHARACTERS,
    MAX_STORED_SETTING_NAME_LENGTH, MAX_TITLE_LENGTH, MAX_YEAR_LENGTH,
};
use crate::util::{
    effective_settlement_url, is_valid_settlement_url, is_valid_slug, iso_now,
    js_collapse_whitespace, js_trim, parse_js_date_ms, utf16_len,
};

/// The previous string API capped descriptions at 20 000 UTF-16 units; the
/// legacy publication fixture keeps that bound while the converted document
/// follows the shared rich-text resource limits.
const MAX_LEGACY_DESCRIPTION_LENGTH: usize = 20_000;

#[derive(Debug, Clone, Copy)]
pub struct ImportSummary {
    pub published: usize,
    pub posts: usize,
    pub settlements: usize,
    pub references: usize,
    pub districts: usize,
}

struct ImportSettlement {
    id: String,
    name: String,
    uyezd_id: String,
    latitude: f64,
    longitude: f64,
    created_at: String,
    url: Option<String>,
    type_name: Option<String>,
}

struct ImportPost {
    id: String,
    title: String,
    body_json: String,
    created_at: String,
    updated_at: String,
    created_ms: i64,
    uyezd_id: Option<String>,
    settlement_id: Option<String>,
    year: String,
    archive_reference: String,
    category: Option<String>,
}

struct ImportEntry {
    published: bool,
    slug: Option<String>,
    /// Canonical rich document JSON; `None` is the "no description" state.
    description_json: Option<String>,
    posts: Vec<ImportPost>,
    settlements: Vec<ImportSettlement>,
}

struct ImportState {
    entries: Vec<(String, ImportEntry)>,
}

/// Everything an import needs, fully parsed and validated before any write.
struct LoadedSources {
    state: ImportState,
    settings: SettingsLists,
    about: Value,
    references: Vec<(String, String)>,
}

struct PreparedImport {
    sources: LoadedSources,
    districts: HashMap<String, Arc<DistrictCollection>>,
    summary: ImportSummary,
    source_dir: String,
}

/// Reads, validates and preloads every source document and district file.
/// Nothing here touches the database, so a rejected fixture can never damage
/// live state; the import only starts once everything parsed successfully.
fn prepare_import(
    config: &Config,
    canonical: &CanonicalProvinces,
    geo: &GeoRuntime,
) -> Result<PreparedImport, String> {
    let required = [
        "gubernia-publications.json",
        "site-settings.json",
        "about-content.json",
        "settlement-references.json",
    ];
    if !config.fresh_install {
        let missing: Vec<&str> = required
            .iter()
            .copied()
            .filter(|name| !config.import_dir.join(name).is_file())
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "Import source directory {} is missing required documents: {}. Provide all four JSON documents, point IMPORT_DIR at them, or pass --fresh for an intentional empty install.",
                config.import_dir.display(),
                missing.join(", ")
            ));
        }
    }

    let sources = LoadedSources {
        state: load_publication_state(&config.import_dir, canonical)?,
        settings: load_settings(&config.import_dir)?,
        about: load_about(&config.import_dir)?,
        references: load_references(&config.import_dir)?,
    };

    let mut districts: HashMap<String, Arc<DistrictCollection>> = HashMap::new();
    let mut district_count = 0usize;
    for feature in &canonical.features {
        // Strict: a canonical province without its district file is a partial
        // import, so the whole bootstrap is rejected before any write.
        let collection = geo.load_required(&feature.id)?;
        district_count += collection.districts().len();
        districts.insert(feature.id.clone(), collection);
    }

    let summary = ImportSummary {
        published: sources
            .state
            .entries
            .iter()
            .filter(|(_, entry)| entry.published)
            .count(),
        posts: sources
            .state
            .entries
            .iter()
            .map(|(_, entry)| entry.posts.len())
            .sum(),
        settlements: sources
            .state
            .entries
            .iter()
            .map(|(_, entry)| entry.settlements.len())
            .sum(),
        references: sources.references.len(),
        districts: district_count,
    };

    Ok(PreparedImport {
        sources,
        districts,
        summary,
        source_dir: config.import_dir.display().to_string(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportOutcome {
    Imported,
    AlreadyImported,
}

/// One transaction: optionally clears every mutable table, writes the prepared
/// sources and sets the import marker. Any failure — including a bad fixture on
/// `import --force` — rolls back to the previous state. The bootstrap marker is
/// re-checked under the write lock, so two fresh processes starting together
/// can never both seed (the BEGIN IMMEDIATE check is authoritative).
fn import_prepared(
    connection: &mut Connection,
    prepared: &PreparedImport,
    replace: bool,
) -> Result<ImportOutcome, String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if !replace
        && db::meta_get(&transaction, "bootstrap_imported_at")
            .map_err(|error| error.to_string())?
            .is_some()
    {
        // Another process committed while this one was still validating.
        drop(transaction);
        return Ok(ImportOutcome::AlreadyImported);
    }
    if replace {
        clear_mutable_state(&transaction)?;
    }
    write_sources(&transaction, &prepared.sources, &prepared.districts)?;
    db::meta_set(&transaction, "bootstrap_imported_at", &iso_now())
        .map_err(|error| error.to_string())?;
    db::meta_set(&transaction, "bootstrap_import_source", &prepared.source_dir)
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(ImportOutcome::Imported)
}

fn clear_mutable_state(transaction: &Transaction) -> Result<(), String> {
    transaction
        .execute_batch(
            "DELETE FROM posts;\n\
             DELETE FROM settlements;\n\
             DELETE FROM settlement_references;\n\
             DELETE FROM site_settings;\n\
             DELETE FROM about_content;\n\
             DELETE FROM districts;\n\
             DELETE FROM provinces;\n\
             DELETE FROM app_meta;",
        )
        .map_err(|error| error.to_string())
}

/// First bootstrap import. Returns `Some(summary)` when this process imported
/// the database and `None` when it was already imported (by a previous start or
/// by another process that won the race), so `serve` can call it
/// unconditionally on startup.
pub fn import_bootstrap(
    connection: &mut Connection,
    config: &Config,
    canonical: &CanonicalProvinces,
    geo: &GeoRuntime,
) -> Result<Option<ImportSummary>, String> {
    if db::imported_at(connection)?.is_some() {
        return Ok(None);
    }
    let prepared = prepare_import(config, canonical, geo)?;
    match import_prepared(connection, &prepared, false)? {
        ImportOutcome::Imported => Ok(Some(prepared.summary)),
        ImportOutcome::AlreadyImported => Ok(None),
    }
}

/// Backfills the district registry for databases that were imported before
/// migration 2 existed. Idempotent: the rows are seeded only while the table is
/// empty, in one transaction, from the same canonical files.
pub fn ensure_districts(
    connection: &mut Connection,
    canonical: &CanonicalProvinces,
    geo: &GeoRuntime,
) -> Result<usize, String> {
    let existing: i64 = connection
        .query_row("SELECT COUNT(*) FROM districts", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if existing > 0 {
        return Ok(0);
    }

    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    // Re-check under the write lock: a concurrent backfill may have committed
    // while this process was deciding to seed.
    let existing: i64 = transaction
        .query_row("SELECT COUNT(*) FROM districts", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if existing > 0 {
        drop(transaction);
        return Ok(0);
    }
    let inserted = insert_districts(&transaction, canonical, geo)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(inserted)
}

fn insert_districts(
    transaction: &Transaction,
    canonical: &CanonicalProvinces,
    geo: &GeoRuntime,
) -> Result<usize, String> {
    let mut inserted = 0usize;
    for feature in &canonical.features {
        let collection = geo.load_required(&feature.id)?;
        for (position, district) in collection.districts().iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO districts (id, province_id, name, position) VALUES (?1, ?2, ?3, ?4)",
                    params![district.id, feature.id, district.name, position as i64],
                )
                .map_err(|error| error.to_string())?;
            inserted += 1;
        }
    }
    Ok(inserted)
}

/// `import` command: validates the sources first, then imports once. With
/// `--force` the whole replacement is a single transaction, so an invalid
/// fixture leaves the existing database exactly as it was.
pub fn run_import_command(
    connection: &mut Connection,
    config: &Config,
    canonical: &CanonicalProvinces,
    geo: &GeoRuntime,
    force: bool,
) -> Result<(), String> {
    let already_imported = db::imported_at(connection)?.is_some();
    if already_imported && !force {
        println!(
            "Database {} was already imported; nothing to do (use --force to re-import).",
            config.database_path.display()
        );
        return Ok(());
    }
    let prepared = prepare_import(config, canonical, geo)?;
    match import_prepared(connection, &prepared, already_imported)? {
        ImportOutcome::Imported => {
            println!(
                "Imported {} published provinces, {} districts, {} posts, {} settlements, {} references from {}.",
                prepared.summary.published,
                prepared.summary.districts,
                prepared.summary.posts,
                prepared.summary.settlements,
                prepared.summary.references,
                config.import_dir.display()
            );
        }
        ImportOutcome::AlreadyImported => {
            println!(
                "Database {} was imported by another process; nothing to do.",
                config.database_path.display()
            );
        }
    }
    Ok(())
}

fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(source) => Ok(Some(source)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Cannot read {}: {error}", path.display())),
    }
}

fn load_publication_state(
    import_dir: &Path,
    canonical: &CanonicalProvinces,
) -> Result<ImportState, String> {
    let path = import_dir.join("gubernia-publications.json");
    match read_optional(&path)? {
        Some(source) => parse_publication_state(&source, canonical),
        None => Ok(seed_from_canonical(canonical)),
    }
}

fn seed_from_canonical(canonical: &CanonicalProvinces) -> ImportState {
    let entries = canonical
        .features
        .iter()
        .map(|feature| {
            let published = feature.properties.get("published") == Some(&Value::Bool(true));
            let slug = if published {
                feature
                    .properties
                    .get("slug")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            } else {
                None
            };
            (
                feature.id.clone(),
                ImportEntry {
                    published,
                    slug,
                    description_json: None,
                    posts: Vec::new(),
                    settlements: Vec::new(),
                },
            )
        })
        .collect();
    ImportState { entries }
}

fn parse_publication_state(
    source: &str,
    canonical: &CanonicalProvinces,
) -> Result<ImportState, String> {
    let parsed: Value = serde_json::from_str(source)
        .map_err(|error| format!("Gubernia publication state contains invalid JSON: {error}"))?;
    let Value::Object(candidate) = parsed else {
        return Err("Gubernia publication state is not an object.".to_string());
    };
    if candidate.get("version") != Some(&Value::from(1)) {
        return Err("Gubernia publication state has an unsupported format.".to_string());
    }
    let Some(Value::Object(entries)) = candidate.get("gubernias") else {
        return Err("Gubernia publication state has an unsupported format.".to_string());
    };

    let canonical_ids: Vec<&str> = canonical
        .features
        .iter()
        .map(|feature| feature.id.as_str())
        .collect();
    if entries.len() != canonical_ids.len()
        || entries
            .keys()
            .any(|id| !canonical_ids.contains(&id.as_str()))
    {
        return Err("Gubernia publication state does not match the canonical feature set.".to_string());
    }

    let mut state_entries: Vec<(String, ImportEntry)> = Vec::with_capacity(canonical_ids.len());
    let mut slugs: Vec<String> = Vec::new();
    let mut settlement_urls: Vec<String> = Vec::new();
    let mut all_post_ids: Vec<String> = Vec::new();
    let mut all_settlement_ids: Vec<String> = Vec::new();

    for feature in &canonical.features {
        let id = feature.id.as_str();
        let Some(Value::Object(entry)) = entries.get(id) else {
            return Err(format!(
                "Gubernia publication state has an invalid entry for {id}."
            ));
        };
        let Some(published) = entry.get("published").and_then(Value::as_bool) else {
            return Err(format!(
                "Gubernia publication state has invalid fields for {id}."
            ));
        };
        let Some(legacy_description) = entry.get("description").and_then(Value::as_str) else {
            return Err(format!(
                "Gubernia publication state has invalid fields for {id}."
            ));
        };
        if utf16_len(legacy_description) > MAX_LEGACY_DESCRIPTION_LENGTH {
            return Err(format!(
                "Gubernia publication state has invalid fields for {id}."
            ));
        }

        if published {
            let description_json = legacy_description_json(legacy_description).map_err(|error| {
                format!("Gubernia publication state has an invalid description for {id}: {error}")
            })?;
            let slug = match entry.get("slug") {
                Some(Value::String(slug)) if is_valid_slug(slug) => slug.clone(),
                _ => {
                    return Err(format!(
                        "Gubernia publication state has an invalid slug for {id}."
                    ))
                }
            };
            if slugs.contains(&slug) {
                return Err(format!(
                    "Gubernia publication state contains duplicate slug {slug}."
                ));
            }
            slugs.push(slug.clone());

            let posts = parse_stored_posts(entry.get("posts"), id)?;
            let settlements = parse_stored_settlements(entry.get("settlements"), id)?;
            for settlement in &settlements {
                let effective =
                    effective_settlement_url(settlement.url.as_deref(), &settlement.id);
                if settlement_urls.contains(&effective) {
                    return Err(format!(
                        "Gubernia publication state contains duplicate settlement url {effective}."
                    ));
                }
                settlement_urls.push(effective);
            }
            for post in &posts {
                if let Some(settlement_id) = &post.settlement_id {
                    let settlement = settlements
                        .iter()
                        .find(|candidate| &candidate.id == settlement_id);
                    let consistent = settlement.is_some_and(|settlement| {
                        post.uyezd_id.is_none()
                            || post.uyezd_id.as_deref() == Some(settlement.uyezd_id.as_str())
                    });
                    if !consistent {
                        return Err(format!(
                            "Gubernia publication state has a post with a foreign settlement for {id}."
                        ));
                    }
                }
            }
            for post in &posts {
                if all_post_ids.contains(&post.id) {
                    return Err(format!(
                        "Gubernia publication state contains duplicate post {}.",
                        post.id
                    ));
                }
                all_post_ids.push(post.id.clone());
            }
            for settlement in &settlements {
                if all_settlement_ids.contains(&settlement.id) {
                    return Err(format!(
                        "Gubernia publication state contains duplicate settlement {}.",
                        settlement.id
                    ));
                }
                all_settlement_ids.push(settlement.id.clone());
            }

            state_entries.push((
                id.to_string(),
                ImportEntry {
                    published: true,
                    slug: Some(slug),
                    description_json,
                    posts,
                    settlements,
                },
            ));
        } else {
            let empty = |value: Option<&Value>| {
                value.is_none_or(|item| matches!(item, Value::Array(items) if items.is_empty()))
            };
            if entry.get("slug") != Some(&Value::Null)
                || !legacy_description.is_empty()
                || !empty(entry.get("posts"))
                || !empty(entry.get("settlements"))
            {
                return Err(format!(
                    "Unpublished gubernia {id} must not retain publication content."
                ));
            }
            state_entries.push((
                id.to_string(),
                ImportEntry {
                    published: false,
                    slug: None,
                    description_json: None,
                    posts: Vec::new(),
                    settlements: Vec::new(),
                },
            ));
        }
    }

    Ok(ImportState {
        entries: state_entries,
    })
}

fn parse_stored_posts(value: Option<&Value>, gubernia_id: &str) -> Result<Vec<ImportPost>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Value::Array(items) = value else {
        return Err(format!(
            "Gubernia publication state has invalid posts for {gubernia_id}."
        ));
    };
    let mut posts = Vec::with_capacity(items.len());
    let mut ids: Vec<String> = Vec::new();
    for item in items {
        let post = parse_stored_post(item, gubernia_id)?;
        if ids.contains(&post.id) {
            return Err(format!(
                "Gubernia publication state contains duplicate post {}.",
                post.id
            ));
        }
        ids.push(post.id.clone());
        posts.push(post);
    }
    Ok(posts)
}

fn parse_stored_post(value: &Value, gubernia_id: &str) -> Result<ImportPost, String> {
    const STORED_POST_KEYS: [&str; 10] = [
        "id",
        "title",
        "body",
        "createdAt",
        "updatedAt",
        "uyezdId",
        "settlementId",
        "year",
        "archiveReference",
        "category",
    ];
    let Value::Object(post) = value else {
        return Err(format!(
            "Gubernia publication state has an invalid post for {gubernia_id}."
        ));
    };
    if !post.keys().all(|key| STORED_POST_KEYS.contains(&key.as_str())) {
        return Err(format!(
            "Gubernia publication state has unrecognised post fields for {gubernia_id}."
        ));
    }
    let invalid_fields = || {
        format!("Gubernia publication state has invalid post fields for {gubernia_id}.")
    };

    let id = match post.get("id") {
        Some(Value::String(id)) if !id.is_empty() && utf16_len(id) <= MAX_ID_LENGTH => id.clone(),
        _ => return Err(invalid_fields()),
    };
    let title = match post.get("title") {
        Some(Value::String(title))
            if !js_trim(title).is_empty() && utf16_len(title) <= MAX_TITLE_LENGTH =>
        {
            title.clone()
        }
        _ => return Err(invalid_fields()),
    };
    let created_at = match post.get("createdAt") {
        Some(Value::String(value)) if parse_js_date_ms(value).is_some() => value.clone(),
        _ => return Err(invalid_fields()),
    };
    let updated_at = match post.get("updatedAt") {
        Some(Value::String(value)) if parse_js_date_ms(value).is_some() => value.clone(),
        _ => return Err(invalid_fields()),
    };
    let created_ms = parse_js_date_ms(&created_at).unwrap_or_default();

    let body = post.get("body").ok_or_else(invalid_fields)?;
    let body = normalize_post_document(body).map_err(|_| {
        format!("Gubernia publication state has an invalid post body for {gubernia_id}.")
    })?;

    let uyezd_id = parse_stored_optional_id(post.get("uyezdId"), gubernia_id, "uyezd")?;
    let settlement_id =
        parse_stored_optional_id(post.get("settlementId"), gubernia_id, "settlement")?;
    let year = parse_stored_optional_text(post.get("year"), MAX_YEAR_LENGTH, gubernia_id, "year")?;
    let archive_reference = parse_stored_optional_text(
        post.get("archiveReference"),
        MAX_ARCHIVE_REFERENCE_LENGTH,
        gubernia_id,
        "archive reference",
    )?;
    let category = match post.get("category") {
        None | Some(Value::Null) => None,
        Some(Value::String(category))
            if !js_trim(category).is_empty()
                && utf16_len(category) <= MAX_STORED_SETTING_NAME_LENGTH =>
        {
            Some(category.clone())
        }
        _ => {
            return Err(format!(
                "Gubernia publication state has an invalid post category for {gubernia_id}."
            ))
        }
    };

    Ok(ImportPost {
        id,
        title,
        body_json: serde_json::to_string(&body).map_err(|error| error.to_string())?,
        created_at,
        updated_at,
        created_ms,
        uyezd_id,
        settlement_id,
        year,
        archive_reference,
        category,
    })
}

fn parse_stored_optional_id(
    value: Option<&Value>,
    gubernia_id: &str,
    label: &str,
) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) if !id.is_empty() && utf16_len(id) <= MAX_ID_LENGTH => {
            Ok(Some(id.clone()))
        }
        _ => Err(format!(
            "Gubernia publication state has an invalid post {label} for {gubernia_id}."
        )),
    }
}

fn parse_stored_optional_text(
    value: Option<&Value>,
    max_length: usize,
    gubernia_id: &str,
    label: &str,
) -> Result<String, String> {
    match value {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(text)) if utf16_len(text) <= max_length => Ok(text.clone()),
        _ => Err(format!(
            "Gubernia publication state has an invalid post {label} for {gubernia_id}."
        )),
    }
}

fn parse_stored_settlements(
    value: Option<&Value>,
    gubernia_id: &str,
) -> Result<Vec<ImportSettlement>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Value::Array(items) = value else {
        return Err(format!(
            "Gubernia publication state has invalid settlements for {gubernia_id}."
        ));
    };
    let mut settlements = Vec::with_capacity(items.len());
    let mut ids: Vec<String> = Vec::new();
    for item in items {
        let settlement = parse_stored_settlement(item, gubernia_id)?;
        if ids.contains(&settlement.id) {
            return Err(format!(
                "Gubernia publication state contains duplicate settlement {}.",
                settlement.id
            ));
        }
        ids.push(settlement.id.clone());
        settlements.push(settlement);
    }
    Ok(settlements)
}

fn parse_stored_settlement(value: &Value, gubernia_id: &str) -> Result<ImportSettlement, String> {
    const STORED_SETTLEMENT_KEYS: [&str; 9] = [
        "id",
        "name",
        "guberniaId",
        "uyezdId",
        "latitude",
        "longitude",
        "createdAt",
        "url",
        "type",
    ];
    let Value::Object(settlement) = value else {
        return Err(format!(
            "Gubernia publication state has an invalid settlement for {gubernia_id}."
        ));
    };
    if !settlement
        .keys()
        .all(|key| STORED_SETTLEMENT_KEYS.contains(&key.as_str()))
    {
        return Err(format!(
            "Gubernia publication state has unrecognised settlement fields for {gubernia_id}."
        ));
    }
    let invalid_fields = || {
        format!("Gubernia publication state has invalid settlement fields for {gubernia_id}.")
    };

    let id = match settlement.get("id") {
        Some(Value::String(id)) if !id.is_empty() && utf16_len(id) <= MAX_ID_LENGTH => id.clone(),
        _ => return Err(invalid_fields()),
    };
    let name = match settlement.get("name") {
        Some(Value::String(name))
            if !js_trim(name).is_empty() && utf16_len(name) <= MAX_SETTLEMENT_NAME_LENGTH =>
        {
            name.clone()
        }
        _ => return Err(invalid_fields()),
    };
    if settlement.get("guberniaId").and_then(Value::as_str) != Some(gubernia_id) {
        return Err(invalid_fields());
    }
    let uyezd_id = match settlement.get("uyezdId") {
        Some(Value::String(uyezd_id))
            if !uyezd_id.is_empty() && utf16_len(uyezd_id) <= MAX_ID_LENGTH =>
        {
            uyezd_id.clone()
        }
        _ => return Err(invalid_fields()),
    };
    let latitude = match settlement.get("latitude").and_then(Value::as_f64) {
        Some(value) if value.is_finite() && (-90.0..=90.0).contains(&value) => value,
        _ => return Err(invalid_fields()),
    };
    let longitude = match settlement.get("longitude").and_then(Value::as_f64) {
        Some(value) if value.is_finite() && (-180.0..=180.0).contains(&value) => value,
        _ => return Err(invalid_fields()),
    };
    let created_at = match settlement.get("createdAt") {
        Some(Value::String(value)) if parse_js_date_ms(value).is_some() => value.clone(),
        _ => return Err(invalid_fields()),
    };
    let url = match settlement.get("url") {
        None | Some(Value::Null) => None,
        Some(Value::String(url)) if is_valid_settlement_url(url) => Some(url.clone()),
        _ => {
            return Err(format!(
                "Gubernia publication state has an invalid settlement url for {gubernia_id}."
            ))
        }
    };
    let type_name = match settlement.get("type") {
        None | Some(Value::Null) => None,
        Some(Value::String(settlement_type))
            if !js_trim(settlement_type).is_empty()
                && utf16_len(settlement_type) <= MAX_STORED_SETTING_NAME_LENGTH =>
        {
            Some(settlement_type.clone())
        }
        _ => {
            return Err(format!(
                "Gubernia publication state has an invalid settlement type for {gubernia_id}."
            ))
        }
    };

    Ok(ImportSettlement {
        id,
        name,
        uyezd_id,
        latitude,
        longitude,
        created_at,
        url,
        type_name,
    })
}

fn load_settings(import_dir: &Path) -> Result<SettingsLists, String> {
    let path = import_dir.join("site-settings.json");
    match read_optional(&path)? {
        Some(source) => parse_settings(&source),
        None => Ok(default_settings_lists()),
    }
}

fn parse_settings(source: &str) -> Result<SettingsLists, String> {
    let parsed: Value = serde_json::from_str(source)
        .map_err(|error| format!("Site settings storage contains invalid JSON: {error}"))?;
    let Value::Object(candidate) = parsed else {
        return Err("Site settings storage is not an object.".to_string());
    };
    let keys_ok = candidate.len() == 3
        && candidate.keys().all(|key| {
            matches!(key.as_str(), "version" | "categories" | "settlementTypes")
        });
    if candidate.get("version") != Some(&Value::from(1)) || !keys_ok {
        return Err("Site settings storage has an unsupported format.".to_string());
    }
    let categories = parse_settings_list(
        candidate.get("categories").unwrap_or(&Value::Null),
        "categories",
    )?;
    let settlement_types = parse_settings_list(
        candidate.get("settlementTypes").unwrap_or(&Value::Null),
        "settlement types",
    )?;
    Ok(SettingsLists {
        categories,
        settlement_types,
    })
}

fn parse_settings_list(value: &Value, label: &str) -> Result<Vec<String>, String> {
    let Value::Array(items) = value else {
        return Err(format!(
            "Site settings {label} list has an unsupported format."
        ));
    };
    if items.len() > MAX_SETTINGS_ITEMS {
        return Err(format!(
            "Site settings {label} list has an unsupported format."
        ));
    }
    let mut result: Vec<String> = Vec::new();
    for item in items {
        let name = match item.as_str() {
            Some(raw) => {
                let name = js_collapse_whitespace(raw);
                if name.is_empty() || utf16_len(&name) > MAX_SETTINGS_NAME_CHARACTERS {
                    return Err(format!(
                        "Site settings {label} list contains an invalid value."
                    ));
                }
                name
            }
            None => {
                return Err(format!(
                    "Site settings {label} list contains an invalid value."
                ))
            }
        };
        if result.iter().any(|existing| existing == &name) {
            return Err(format!(
                "Site settings {label} list contains a duplicate value."
            ));
        }
        result.push(name);
    }
    Ok(result)
}

fn load_about(import_dir: &Path) -> Result<Value, String> {
    let path = import_dir.join("about-content.json");
    match read_optional(&path)? {
        Some(source) => parse_about(&source),
        None => {
            let content = crate::store::DEFAULT_ABOUT_PARAGRAPHS
                .iter()
                .map(|text| {
                    serde_json::json!({
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": text }],
                    })
                })
                .collect::<Vec<_>>();
            Ok(serde_json::json!({ "type": "doc", "content": content }))
        }
    }
}

fn parse_about(source: &str) -> Result<Value, String> {
    let parsed: Value = serde_json::from_str(source)
        .map_err(|error| format!("About content storage contains invalid JSON: {error}"))?;
    let Value::Object(candidate) = parsed else {
        return Err("About content storage is not an object.".to_string());
    };
    let keys_ok = candidate.len() == 2
        && candidate
            .keys()
            .all(|key| matches!(key.as_str(), "version" | "body"));
    if candidate.get("version") != Some(&Value::from(1)) || !keys_ok {
        return Err("About content storage has an unsupported format.".to_string());
    }
    let Some(body) = candidate.get("body") else {
        return Err("About content storage has an unsupported format.".to_string());
    };
    normalize_post_document(body)
        .map_err(|_| "About content storage contains an invalid document.".to_string())
}

fn load_references(import_dir: &Path) -> Result<Vec<(String, String)>, String> {
    let path = import_dir.join("settlement-references.json");
    match read_optional(&path)? {
        Some(source) => parse_references(&source),
        None => Ok(Vec::new()),
    }
}

fn parse_references(source: &str) -> Result<Vec<(String, String)>, String> {
    let parsed: Value = serde_json::from_str(source).map_err(|error| {
        format!("Settlement reference storage contains invalid JSON: {error}")
    })?;
    let Value::Object(candidate) = parsed else {
        return Err("Settlement reference storage is not an object.".to_string());
    };
    let keys_ok = candidate.len() == 2
        && candidate
            .keys()
            .all(|key| matches!(key.as_str(), "version" | "references"));
    if candidate.get("version") != Some(&Value::from(1)) || !keys_ok {
        return Err("Settlement reference storage has an unsupported format.".to_string());
    }
    let Some(Value::Object(references)) = candidate.get("references") else {
        return Err("Settlement reference storage has an invalid reference map.".to_string());
    };
    let mut out = Vec::with_capacity(references.len());
    for (settlement_id, body) in references {
        if settlement_id.is_empty() || utf16_len(settlement_id) > MAX_ID_LENGTH {
            return Err("Settlement reference storage contains an invalid settlement id.".to_string());
        }
        if !body.is_object() {
            return Err("Settlement reference storage contains an invalid document.".to_string());
        }
        out.push((
            settlement_id.clone(),
            serde_json::to_string(body).map_err(|error| error.to_string())?,
        ));
    }
    Ok(out)
}

fn write_sources(
    transaction: &Transaction,
    sources: &LoadedSources,
    districts: &HashMap<String, Arc<DistrictCollection>>,
) -> Result<(), String> {
    for (id, entry) in &sources.state.entries {
        transaction
            .execute(
                "INSERT INTO provinces (id, published, slug, description_json) VALUES (?1, ?2, ?3, ?4)",
                params![
                    id,
                    if entry.published { 1i64 } else { 0i64 },
                    entry.slug.as_deref(),
                    entry.description_json.as_deref().unwrap_or_default(),
                ],
            )
            .map_err(|error| error.to_string())?;
        if let Some(collection) = districts.get(id) {
            for (position, district) in collection.districts().iter().enumerate() {
                transaction
                    .execute(
                        "INSERT INTO districts (id, province_id, name, position) VALUES (?1, ?2, ?3, ?4)",
                        params![district.id, id, district.name, position as i64],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        for (position, settlement) in entry.settlements.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO settlements (id, province_id, name, uyezd_id, latitude, longitude, created_at, position, url, type)\n\
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        settlement.id,
                        id,
                        settlement.name,
                        settlement.uyezd_id,
                        settlement.latitude,
                        settlement.longitude,
                        settlement.created_at,
                        position as i64,
                        settlement.url.as_deref(),
                        settlement.type_name.as_deref(),
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
        for (position, post) in entry.posts.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO posts (id, province_id, title, body_json, created_at, updated_at, created_ms, position, uyezd_id, settlement_id, year, archive_reference, category)\n\
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    params![
                        post.id,
                        id,
                        post.title,
                        post.body_json,
                        post.created_at,
                        post.updated_at,
                        post.created_ms,
                        position as i64,
                        post.uyezd_id.as_deref(),
                        post.settlement_id.as_deref(),
                        post.year,
                        post.archive_reference,
                        post.category.as_deref(),
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
    }

    transaction
        .execute(
            "INSERT INTO site_settings (id, categories_json, settlement_types_json) VALUES (1, ?1, ?2)",
            params![
                serde_json::to_string(&sources.settings.categories).map_err(|error| error.to_string())?,
                serde_json::to_string(&sources.settings.settlement_types).map_err(|error| error.to_string())?,
            ],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "INSERT INTO about_content (id, body_json) VALUES (1, ?1)",
            params![serde_json::to_string(&sources.about).map_err(|error| error.to_string())?],
        )
        .map_err(|error| error.to_string())?;
    for (settlement_id, body_json) in &sources.references {
        transaction
            .execute(
                "INSERT INTO settlement_references (settlement_id, body_json) VALUES (?1, ?2)",
                params![settlement_id, body_json],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
