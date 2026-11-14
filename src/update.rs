use std::env;
use std::fs;
use std::io::IsTerminal;
use std::path::Path;

use self_update::backends::github;

use crate::config::INSTALL_BIN_PATH;
use crate::error::{AppResult, boxed_error};
use crate::install;

const REPO_OWNER: &str = "magicdian";
const REPO_NAME: &str = "oh-my-useless";
const BIN_NAME: &str = "useless";
const TOKEN_ENV: &str = "USELESS_GITHUB_TOKEN";

pub struct UpdateOptions {
    pub check: bool,
    pub version: Option<String>,
    pub no_restart: bool,
}

pub fn run(options: UpdateOptions) -> AppResult<()> {
    let current = env!("CARGO_PKG_VERSION");

    if options.check {
        return check(current, options.version.as_deref());
    }

    install::ensure_root()?;

    let install_path = Path::new(INSTALL_BIN_PATH);
    if let Some(parent) = install_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let updater = configured_updater(
        current,
        options.version.as_deref(),
        std::io::stdout().is_terminal(),
    )?;
    let status = updater
        .update()
        .map_err(|error| boxed_error(format!("update failed: {error}")))?;
    let installed = status.version().to_string();

    if status.is_up_to_date() {
        println!("useless {installed} is up to date");
        return Ok(());
    }

    println!(
        "updated useless {current} -> {installed} ({})",
        install_path.display()
    );

    if options.no_restart {
        println!("skipped service restart (--no-restart)");
        return Ok(());
    }

    match install::restart_service_if_active() {
        Ok(true) => {
            println!("restarted the useless service");
            Ok(())
        }
        Ok(false) => {
            println!("no active service found; start it manually or run `useless install`");
            Ok(())
        }
        Err(error) => Err(boxed_error(format!(
            "updated to {installed}, but the service restart failed: {error}"
        ))),
    }
}

fn check(current: &str, version: Option<&str>) -> AppResult<()> {
    let updater = configured_updater(current, version, false)?;

    match version {
        Some(version) => {
            let tag = normalize_tag(version)?;
            let release = updater
                .get_release_version(&tag)
                .map_err(|error| boxed_error(format!("could not fetch {tag}: {error}")))?;
            if release.version() == current {
                println!("useless {current} is already at {tag}");
            } else {
                println!("requested {tag}; installed {current}");
            }
        }
        None => match updater.get_latest_release() {
            Ok(releases) => {
                if releases.is_empty() {
                    println!("no releases published yet");
                } else if releases
                    .is_update_available()
                    .map_err(|error| boxed_error(format!("update check failed: {error}")))?
                {
                    if let Some(release) = releases.into_vec().into_iter().next() {
                        println!("update available: {current} -> {}", release.version());
                    }
                } else {
                    println!("useless {current} is up to date");
                }
            }
            Err(self_update::Error::NotFound { .. }) => println!("no releases published yet"),
            Err(error) => return Err(boxed_error(format!("update check failed: {error}"))),
        },
    }

    Ok(())
}

fn configured_updater(
    current: &str,
    version: Option<&str>,
    show_progress: bool,
) -> AppResult<github::Update> {
    let mut builder = github::Update::configure();
    builder
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(BIN_NAME)
        .bin_install_path(Path::new(INSTALL_BIN_PATH))
        .bin_path_in_archive(BIN_NAME)
        .check_install_path_writable(true)
        .current_version(current)
        .no_confirm(true)
        .show_output(false)
        .show_download_progress(show_progress);

    if let Some(version) = version {
        builder.release_tag(normalize_tag(version)?);
    }

    if let Ok(token) = env::var(TOKEN_ENV)
        && !token.trim().is_empty()
    {
        builder.auth_token(token);
    }

    builder
        .build()
        .map_err(|error| boxed_error(format!("could not configure the updater: {error}")))
}

fn normalize_tag(version: &str) -> AppResult<String> {
    let trimmed = version.trim();
    let bare = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);

    if bare.is_empty() || bare.starts_with('-') || bare.contains('/') {
        return Err(boxed_error(format!("invalid version: {version}")));
    }

    Ok(format!("v{bare}"))
}

#[cfg(test)]
mod tests {
    use super::normalize_tag;

    #[test]
    fn normalizes_plain_and_v_prefixed_versions() {
        assert_eq!(normalize_tag("0.2.0").expect("tag"), "v0.2.0");
        assert_eq!(normalize_tag("v0.2.0").expect("tag"), "v0.2.0");
        assert_eq!(normalize_tag("V0.2.0").expect("tag"), "v0.2.0");
        assert_eq!(normalize_tag(" v0.2.0 ").expect("tag"), "v0.2.0");
    }

    #[test]
    fn rejects_empty_and_path_like_versions() {
        assert!(normalize_tag("").is_err());
        assert!(normalize_tag("   ").is_err());
        assert!(normalize_tag("--check").is_err());
        assert!(normalize_tag("../../etc/passwd").is_err());
    }

    #[test]
    fn configures_the_install_target_and_archive_path() {
        use self_update::UpdateConfig;

        let install_path = std::path::Path::new(crate::config::INSTALL_BIN_PATH);
        let updater = super::configured_updater("0.1.0", Some("0.2.0"), false)
            .expect("updater should configure");

        assert_eq!(updater.bin_name(), "useless");
        assert_eq!(updater.bin_install_path(), install_path);
        assert_eq!(updater.bin_path_in_archive(), "useless");
        assert_eq!(updater.current_version(), "0.1.0");
        assert_eq!(updater.release_tag(), Some("v0.2.0"));
        assert!(updater.check_install_path_writable());
        assert!(updater.no_confirm());
        assert!(!updater.show_output());
    }
}
