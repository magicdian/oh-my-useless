use std::path::PathBuf;

use crate::config::DEFAULT_CONFIG_PATH;
use crate::error::{AppResult, boxed_error};

#[derive(Debug, Clone)]
pub enum Command {
    Install { config_path: PathBuf },
    Uninstall { purge_config: bool },
    Reload,
    Status { config_path: PathBuf },
    Daemon { config_path: PathBuf },
    PrintDefaultConfig,
    Help,
}

pub fn parse_args(args: impl Iterator<Item = String>) -> AppResult<Command> {
    let mut args = args.peekable();
    let Some(command) = args.next() else {
        return Ok(Command::Help);
    };

    match command.as_str() {
        "install" => Ok(Command::Install {
            config_path: take_config_path(&mut args)?,
        }),
        "uninstall" => {
            let mut purge_config = false;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--purge" | "--purge-config" => purge_config = true,
                    other => return Err(boxed_error(format!("unknown uninstall flag: {other}"))),
                }
            }
            Ok(Command::Uninstall { purge_config })
        }
        "reload" => Ok(Command::Reload),
        "status" => Ok(Command::Status {
            config_path: take_config_path(&mut args)?,
        }),
        "daemon" => Ok(Command::Daemon {
            config_path: take_config_path(&mut args)?,
        }),
        "print-default-config" => Ok(Command::PrintDefaultConfig),
        "help" | "--help" | "-h" => Ok(Command::Help),
        other => Err(boxed_error(format!("unknown command: {other}"))),
    }
}

fn take_config_path(
    args: &mut std::iter::Peekable<impl Iterator<Item = String>>,
) -> AppResult<PathBuf> {
    let mut config_path = PathBuf::from(DEFAULT_CONFIG_PATH);

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--config" => {
                let Some(value) = args.next() else {
                    return Err(boxed_error("missing value after --config"));
                };
                config_path = PathBuf::from(value);
            }
            other => return Err(boxed_error(format!("unknown flag: {other}"))),
        }
    }

    Ok(config_path)
}

pub fn usage() -> &'static str {
    "useless <command> [options]

Commands:
  install [--config <path>]      Install the binary and service files
  uninstall [--purge]            Remove service files and optionally /etc/useless
  reload                         Reload the running service configuration
  status [--config <path>]       Show service and config status
  daemon [--config <path>]       Run the foreground daemon (used by service managers)
  print-default-config           Print the default config template
  help                           Show this help
"
}
