use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8787";
pub const DEFAULT_FRONTEND_ORIGIN: &str = "http://localhost:3000";

pub const USAGE: &str = "\
dimasik-backend — Rust + SQLite backend for the Dimasik portal

USAGE:
    dimasik-backend [serve] [OPTIONS]
    dimasik-backend import [--force] [OPTIONS]

COMMANDS:
    serve    Run the HTTP server (default). Runs migrations and the first
             bootstrap import when the database was never imported.
    import   Apply migrations and run the bootstrap import once. Skips with a
             notice when the database was already imported; --force replaces
             all mutable state in a single transaction, so an invalid fixture
             leaves the existing database untouched.

OPTIONS:
    --data-dir <DIR>        Override DATA_DIR (default: data).
    --import-dir <DIR>      Override IMPORT_DIR (default: DATA_DIR).
    --database-path <PATH>  Override DATABASE_PATH (default: data/dimasik.sqlite).
    --fresh                 Allow an intentional empty install: missing source
                            documents fall back to the canonical seed and the
                            bundled defaults. Without it, all four documents
                            must exist in IMPORT_DIR.
    --force                 With import: replace existing state atomically.
    -h, --help              Print this help.

ENVIRONMENT:
    DATABASE_PATH, DATA_DIR, IMPORT_DIR, PUBLIC_DATA_DIR, BIND_ADDR,
    FRONTEND_ORIGIN, ADMIN_USERNAME, ADMIN_PASSWORD, ADMIN_SESSION_SECRET,
    S3_ENDPOINT, S3_REGION, S3_BUCKET, S3_ACCESS_KEY_ID, S3_SECRET_ACCESS_KEY
    (SESSION_SECRET is accepted as a fallback). Values are read from the process
    environment first, then from ./.env and ./.env.local relative to the current
    working directory.
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Serve,
    Import { force: bool },
}

#[derive(Debug, Clone)]
pub struct Args {
    pub command: Command,
    pub data_dir: Option<String>,
    pub import_dir: Option<String>,
    pub database_path: Option<String>,
    pub fresh: bool,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_path: PathBuf,
    pub data_dir: PathBuf,
    pub import_dir: PathBuf,
    pub public_data_dir: PathBuf,
    pub bind_addr: String,
    pub frontend_origin: Option<String>,
    pub admin_username: Option<String>,
    pub admin_password: Option<String>,
    pub admin_session_secret: Option<String>,
    pub s3_endpoint: Option<String>,
    pub s3_region: Option<String>,
    pub s3_bucket: Option<String>,
    pub s3_access_key_id: Option<String>,
    pub s3_secret_access_key: Option<String>,
    pub fresh_install: bool,
}

impl Config {
    pub fn resolve(file_env: &HashMap<String, String>, args: &Args) -> Self {
        let lookup = |key: &str| -> Option<String> {
            std::env::var(key)
                .ok()
                .filter(|value| !value.is_empty())
                .or_else(|| file_env.get(key).cloned().filter(|value| !value.is_empty()))
        };

        let data_dir = args
            .data_dir
            .clone()
            .or_else(|| lookup("DATA_DIR"))
            .unwrap_or_else(|| "data".to_string());
        let import_dir = args
            .import_dir
            .clone()
            .or_else(|| lookup("IMPORT_DIR"))
            .unwrap_or_else(|| data_dir.clone());
        let database_path = args
            .database_path
            .clone()
            .or_else(|| lookup("DATABASE_PATH"))
            .unwrap_or_else(|| "data/dimasik.sqlite".to_string());
        let public_data_dir = lookup("PUBLIC_DATA_DIR").unwrap_or_else(|| "public/data".to_string());
        let bind_addr = lookup("BIND_ADDR").unwrap_or_else(|| DEFAULT_BIND_ADDR.to_string());
        let frontend_origin =
            lookup("FRONTEND_ORIGIN").or_else(|| Some(DEFAULT_FRONTEND_ORIGIN.to_string()));
        let admin_username = lookup("ADMIN_USERNAME");
        let admin_password = lookup("ADMIN_PASSWORD");
        let admin_session_secret =
            lookup("ADMIN_SESSION_SECRET").or_else(|| lookup("SESSION_SECRET"));
        let s3_endpoint = lookup("S3_ENDPOINT");
        let s3_region = lookup("S3_REGION");
        let s3_bucket = lookup("S3_BUCKET");
        let s3_access_key_id = lookup("S3_ACCESS_KEY_ID");
        let s3_secret_access_key = lookup("S3_SECRET_ACCESS_KEY");

        Self {
            database_path: PathBuf::from(database_path),
            data_dir: PathBuf::from(data_dir),
            import_dir: PathBuf::from(import_dir),
            public_data_dir: PathBuf::from(public_data_dir),
            bind_addr,
            frontend_origin,
            admin_username,
            admin_password,
            admin_session_secret,
            s3_endpoint,
            s3_region,
            s3_bucket,
            s3_access_key_id,
            s3_secret_access_key,
            fresh_install: args.fresh,
        }
    }
}

pub fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut command = Command::Serve;
    let mut command_seen = false;
    let mut data_dir = None;
    let mut import_dir = None;
    let mut database_path = None;
    let mut force = false;
    let mut fresh = false;

    let mut index = 0;
    while index < argv.len() {
        let arg = argv[index].as_str();
        match arg {
            "serve" => {
                if command_seen {
                    return Err("Only one command may be given.".to_string());
                }
                command = Command::Serve;
                command_seen = true;
            }
            "import" => {
                if command_seen {
                    return Err("Only one command may be given.".to_string());
                }
                command = Command::Import { force: false };
                command_seen = true;
            }
            "--force" => force = true,
            "--fresh" => fresh = true,
            "--data-dir" | "--import-dir" | "--database-path" => {
                index += 1;
                let value = argv
                    .get(index)
                    .ok_or_else(|| format!("{arg} requires a value."))?
                    .clone();
                match arg {
                    "--data-dir" => data_dir = Some(value),
                    "--import-dir" => import_dir = Some(value),
                    _ => database_path = Some(value),
                }
            }
            other => return Err(format!("Unknown argument: {other}")),
        }
        index += 1;
    }

    if force {
        match command {
            Command::Import { .. } => command = Command::Import { force: true },
            Command::Serve => return Err("--force is only valid with the import command.".to_string()),
        }
    }

    Ok(Args {
        command,
        data_dir,
        import_dir,
        database_path,
        fresh,
    })
}

/// Loads `./.env` then `./.env.local` (the later file wins inside this map).
/// Process environment always takes precedence when `Config::resolve` reads
/// the values, so real deployments never depend on these files.
pub fn load_env_files(cwd: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for name in [".env", ".env.local"] {
        if let Ok(contents) = std::fs::read_to_string(cwd.join(name)) {
            parse_env_contents(&contents, &mut map);
        }
    }
    map
}

fn parse_env_contents(contents: &str, map: &mut HashMap<String, String>) {
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        map.insert(key.to_string(), parse_env_value(raw_value.trim()));
    }
}

fn parse_env_value(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix('"') {
        let mut value = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') => value.push('\n'),
                    Some('t') => value.push('\t'),
                    Some('r') => value.push('\r'),
                    Some(other) => value.push(other),
                    None => break,
                },
                other => value.push(other),
            }
        }
        return value;
    }
    if let Some(rest) = raw.strip_prefix('\'') {
        return rest.split('\'').next().unwrap_or_default().to_string();
    }
    let trimmed = match raw.find(" #") {
        Some(position) => &raw[..position],
        None => raw,
    };
    trimmed.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands_and_flags() {
        let args = parse_args(&[]).unwrap();
        assert_eq!(args.command, Command::Serve);
        let args = parse_args(&["serve".to_string()]).unwrap();
        assert_eq!(args.command, Command::Serve);
        let args = parse_args(&["import".to_string(), "--force".to_string()]).unwrap();
        assert_eq!(args.command, Command::Import { force: true });
        let args = parse_args(&[
            "import".to_string(),
            "--data-dir".to_string(),
            "/tmp/x".to_string(),
        ])
        .unwrap();
        assert_eq!(args.data_dir.as_deref(), Some("/tmp/x"));
        let args = parse_args(&["serve".to_string(), "--fresh".to_string()]).unwrap();
        assert!(args.fresh);
        assert!(!parse_args(&["serve".to_string()]).unwrap().fresh);
        assert!(parse_args(&["--force".to_string()]).is_err());
        assert!(parse_args(&["nope".to_string()]).is_err());
    }

    #[test]
    fn parses_env_values() {
        let mut map = HashMap::new();
        parse_env_contents(
            "# comment\nADMIN_USERNAME=admin\nADMIN_PASSWORD=\"p a#ss\"\nexport SESSION_SECRET='s3cret'\nEMPTY=\n",
            &mut map,
        );
        assert_eq!(map.get("ADMIN_USERNAME").unwrap(), "admin");
        assert_eq!(map.get("ADMIN_PASSWORD").unwrap(), "p a#ss");
        assert_eq!(map.get("SESSION_SECRET").unwrap(), "s3cret");
        assert_eq!(map.get("EMPTY").unwrap(), "");
    }

    #[test]
    fn resolves_file_values_unless_the_process_environment_wins() {
        let mut file_env = HashMap::new();
        file_env.insert("FRONTEND_ORIGIN".to_string(), "http://example.test".to_string());
        file_env.insert("SESSION_SECRET".to_string(), "x".repeat(40));

        let args = parse_args(&[]).unwrap();
        let config = Config::resolve(&file_env, &args);
        if std::env::var("FRONTEND_ORIGIN").is_err() {
            assert_eq!(
                config.frontend_origin.as_deref(),
                Some("http://example.test")
            );
        }
        if std::env::var("ADMIN_SESSION_SECRET").is_err()
            && std::env::var("SESSION_SECRET").is_err()
        {
            assert_eq!(
                config.admin_session_secret.as_deref(),
                Some("x".repeat(40).as_str())
            );
        }

        // CLI flags beat both files and the process environment.
        let args = parse_args(&["--data-dir".to_string(), "/tmp/flag-data".to_string()]).unwrap();
        let config = Config::resolve(&file_env, &args);
        assert_eq!(config.data_dir.to_string_lossy(), "/tmp/flag-data");
        assert_eq!(config.import_dir.to_string_lossy(), "/tmp/flag-data");
    }
}
