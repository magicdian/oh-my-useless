mod cli;
mod config;
mod daemon;
mod error;
mod install;
mod policy;
mod runtime;
mod sensor;
mod workers;

use cli::{Command, parse_args, usage};
use config::{default_config_path, default_config_text};
use error::AppResult;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let command = parse_args(std::env::args().skip(1))?;
    match command {
        Command::Install { config_path } => install::install(&config_path),
        Command::Uninstall { purge_config } => install::uninstall(purge_config),
        Command::Reload => install::reload(),
        Command::Status { config_path } => install::status(&config_path),
        Command::Daemon { config_path } => daemon::run(&config_path),
        Command::PrintDefaultConfig => {
            print!("{}", default_config_text());
            Ok(())
        }
        Command::Help => {
            println!("{}", usage());
            println!("default config path: {}", default_config_path().display());
            Ok(())
        }
    }
}
