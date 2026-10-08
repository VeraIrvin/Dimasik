use std::path::PathBuf;
use std::process::ExitCode;

use dimasik_backend::config::{self, Command, USAGE};

#[tokio::main]
async fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|arg| arg == "-h" || arg == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let file_env = config::load_env_files(&cwd);
    let args = match config::parse_args(&argv) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let config = config::Config::resolve(&file_env, &args);

    match args.command {
        Command::Serve => match dimasik_backend::run(config).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Startup failed: {error}");
                ExitCode::from(1)
            }
        },
        Command::Import { force } => match dimasik_backend::run_import(&config, force) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Import failed: {error}");
                ExitCode::from(1)
            }
        },
    }
}
