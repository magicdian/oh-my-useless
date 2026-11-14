use std::path::PathBuf;

use crate::config::DEFAULT_CONFIG_PATH;
use crate::error::{AppResult, boxed_error};

#[derive(Debug, Clone)]
pub enum Command {
    Install {
        config_path: PathBuf,
    },
    Uninstall {
        purge_config: bool,
    },
    Reload,
    Status {
        config_path: PathBuf,
    },
    Daemon {
        config_path: PathBuf,
    },
    Update {
        check: bool,
        version: Option<String>,
        no_restart: bool,
    },
    Version,
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
            for flag in args.by_ref() {
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
        "update" => take_update_options(&mut args),
        "version" | "--version" | "-V" => Ok(Command::Version),
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

fn take_update_options(
    args: &mut std::iter::Peekable<impl Iterator<Item = String>>,
) -> AppResult<Command> {
    let mut check = false;
    let mut version = None;
    let mut no_restart = false;

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--check" => check = true,
            "--no-restart" => no_restart = true,
            "--version" => {
                let Some(value) = args.next() else {
                    return Err(boxed_error("missing value after --version"));
                };
                if value.starts_with('-') {
                    return Err(boxed_error(format!(
                        "expected a version after --version, got {value}"
                    )));
                }
                version = Some(value);
            }
            other => return Err(boxed_error(format!("unknown update flag: {other}"))),
        }
    }

    Ok(Command::Update {
        check,
        version,
        no_restart,
    })
}

pub fn usage() -> &'static str {
    "useless <command> [options]

Commands:
  install [--config <path>]      Install the binary and service files
  uninstall [--purge]            Remove service files and optionally /etc/useless
  reload                         Reload the running service configuration
  status [--config <path>]       Show service and config status
  daemon [--config <path>]       Run the foreground daemon (used by service managers)
  update [--check] [--version <semver>] [--no-restart]
                                 Update the installed binary from GitHub Releases
  version                        Print the current version
  print-default-config           Print the default config template
  help                           Show this help
"
}

#[cfg(test)]
mod tests {
    use super::{Command, parse_args};

    fn parse(args: &[&str]) -> Command {
        parse_args(args.iter().map(|arg| arg.to_string())).expect("parse should succeed")
    }

    #[test]
    fn parses_update_with_defaults() {
        match parse(&["update"]) {
            Command::Update {
                check,
                version,
                no_restart,
            } => {
                assert!(!check);
                assert_eq!(version, None);
                assert!(!no_restart);
            }
            other => panic!("expected update, got {other:?}"),
        }
    }

    #[test]
    fn parses_update_flags_in_any_order() {
        match parse(&["update", "--no-restart", "--version", "v0.2.0", "--check"]) {
            Command::Update {
                check,
                version,
                no_restart,
            } => {
                assert!(check);
                assert_eq!(version.as_deref(), Some("v0.2.0"));
                assert!(no_restart);
            }
            other => panic!("expected update, got {other:?}"),
        }
    }

    #[test]
    fn rejects_update_flags_that_are_not_versions() {
        assert!(
            parse_args(
                ["update", "--version", "--check"]
                    .into_iter()
                    .map(String::from)
            )
            .is_err()
        );
    }

    #[test]
    fn parses_version_aliases() {
        assert!(matches!(parse(&["version"]), Command::Version));
        assert!(matches!(parse(&["--version"]), Command::Version));
        assert!(matches!(parse(&["-V"]), Command::Version));
    }
}
