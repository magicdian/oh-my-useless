use std::env;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::{
    Config, DEFAULT_CONFIG_DIR, INITD_SCRIPT_PATH, INSTALL_BIN_PATH, PID_FILE_PATH,
    STATE_FILE_PATH, SYSTEMD_UNIT_PATH, ensure_default_config, load_config,
};
use crate::daemon;
use crate::error::{AppResult, boxed_error};

const SERVICE_NAME: &str = "useless";

unsafe extern "C" {
    fn geteuid() -> u32;
}

pub fn install(config_path: &Path) -> AppResult<()> {
    ensure_root()?;
    ensure_default_config(config_path)?;

    let config = load_config(config_path)?;
    let current_exe = env::current_exe()?;
    let install_path = Path::new(INSTALL_BIN_PATH);
    if current_exe != install_path {
        if let Some(parent) = install_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&current_exe, install_path)?;
        let mut perms = fs::metadata(install_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(install_path, perms)?;
    }

    match detect_service_manager() {
        ServiceManager::Systemd => install_systemd(config_path, &config)?,
        ServiceManager::Initd => install_initd(config_path)?,
        ServiceManager::None => {
            return Err(boxed_error(
                "no supported service manager found (systemd or init.d required)",
            ));
        }
    }

    println!(
        "installed {} with config {}",
        INSTALL_BIN_PATH,
        config_path.display()
    );
    Ok(())
}

pub fn uninstall(purge_config: bool) -> AppResult<()> {
    ensure_root()?;
    match detect_service_manager() {
        ServiceManager::Systemd => uninstall_systemd()?,
        ServiceManager::Initd => uninstall_initd()?,
        ServiceManager::None => {}
    }

    let _ = fs::remove_file(INSTALL_BIN_PATH);
    if purge_config {
        let _ = fs::remove_dir_all(DEFAULT_CONFIG_DIR);
    }

    println!("uninstalled useless");
    Ok(())
}

pub fn reload() -> AppResult<()> {
    match detect_service_manager() {
        ServiceManager::Systemd => run_command("systemctl", ["reload", SERVICE_NAME]),
        ServiceManager::Initd => {
            let script = Path::new(INITD_SCRIPT_PATH);
            if script.exists() {
                run_command(INITD_SCRIPT_PATH, ["reload"])
            } else {
                daemon::send_reload_from_pidfile()
            }
        }
        ServiceManager::None => daemon::send_reload_from_pidfile(),
    }
}

/// Restart the installed service when it is currently running.
///
/// Returns `false` when no service manager or no active service exists, which is
/// not an error: the binary may have been updated before `install` was ever run.
pub(crate) fn restart_service_if_active() -> AppResult<bool> {
    match detect_service_manager() {
        ServiceManager::Systemd => {
            if !command_success("systemctl", ["is-active", "--quiet", SERVICE_NAME]) {
                return Ok(false);
            }
            run_command("systemctl", ["restart", SERVICE_NAME])?;
            Ok(true)
        }
        ServiceManager::Initd => {
            let script = Path::new(INITD_SCRIPT_PATH);
            if !script.exists() || !command_success(INITD_SCRIPT_PATH, ["status"]) {
                return Ok(false);
            }
            run_command(INITD_SCRIPT_PATH, ["restart"])?;
            Ok(true)
        }
        ServiceManager::None => Ok(false),
    }
}

pub fn status(config_path: &Path) -> AppResult<()> {
    let config = load_config(config_path)?;
    let manager = detect_service_manager();
    println!("binary: {}", INSTALL_BIN_PATH);
    println!("config: {}", config_path.display());
    println!("preset: {}", config.preset);
    println!("service_manager: {}", manager.as_str());
    println!("pid_file: {}", PID_FILE_PATH);
    println!(
        "cpu: enabled={} max_percent={}",
        config.cpu.enabled, config.cpu.max_percent
    );
    println!(
        "memory: enabled={} max_percent={}",
        config.memory.enabled, config.memory.max_percent
    );
    println!(
        "network: enabled={} max_mbps={} targets={}",
        config.network.enabled,
        config.network.max_mbps,
        config.network.targets.len()
    );

    match manager {
        ServiceManager::Systemd => {
            println!(
                "systemd_active: {}",
                command_success("systemctl", ["is-active", "--quiet", SERVICE_NAME])
            );
            println!(
                "systemd_enabled: {}",
                command_success("systemctl", ["is-enabled", "--quiet", SERVICE_NAME])
            );
        }
        ServiceManager::Initd => {
            let active = Path::new(PID_FILE_PATH).exists();
            println!("initd_script: {}", Path::new(INITD_SCRIPT_PATH).exists());
            println!("pid_present: {active}");
        }
        ServiceManager::None => {
            println!("pid_present: {}", Path::new(PID_FILE_PATH).exists());
        }
    }

    match fs::read_to_string(STATE_FILE_PATH) {
        Ok(text) => {
            println!("daemon_state:");
            for line in text.lines() {
                println!("  {line}");
            }
        }
        Err(_) => println!("daemon_state: unavailable"),
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum ServiceManager {
    Systemd,
    Initd,
    None,
}

impl ServiceManager {
    fn as_str(self) -> &'static str {
        match self {
            ServiceManager::Systemd => "systemd",
            ServiceManager::Initd => "init.d",
            ServiceManager::None => "none",
        }
    }
}

fn detect_service_manager() -> ServiceManager {
    if Path::new("/run/systemd/system").exists() && command_exists("systemctl") {
        ServiceManager::Systemd
    } else if Path::new("/etc/init.d").exists() {
        ServiceManager::Initd
    } else {
        ServiceManager::None
    }
}

fn install_systemd(config_path: &Path, config: &Config) -> AppResult<()> {
    let unit = systemd_unit(config_path, config, total_memory_mb().unwrap_or(2048));
    fs::write(SYSTEMD_UNIT_PATH, unit)?;
    run_command("systemctl", ["daemon-reload"])?;
    run_command("systemctl", ["enable", "--now", SERVICE_NAME])?;
    Ok(())
}

fn systemd_unit(config_path: &Path, config: &Config, total_mb: u64) -> String {
    let ceiling = memory_ceiling_mb(config, total_mb);
    format!(
        "[Unit]\n\
         Description=useless synthetic resource daemon\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         User=root\n\
         Nice=19\n\
         OOMScoreAdjust=1000\n\
         IOSchedulingClass=idle\n\
         CPUWeight=10\n\
         MemoryHigh={}M\n\
         MemoryMax={}M\n\
         ExecStart={} daemon --config {}\n\
         ExecReload=/bin/kill -HUP $MAINPID\n\
         Restart=on-failure\n\
         RestartSec=10\n\
         StartLimitIntervalSec=300\n\
         StartLimitBurst=5\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        ceiling + 64,
        ceiling + 128,
        INSTALL_BIN_PATH,
        config_path.display()
    )
}

fn memory_ceiling_mb(config: &Config, total_mb: u64) -> u64 {
    if !config.memory.enabled {
        return 64;
    }
    let ratio = total_mb.saturating_mul(config.memory.max_percent as u64) / 100;
    let absolute = if config.memory.max_absolute_mb == 0 {
        u64::MAX
    } else {
        config.memory.max_absolute_mb
    };
    ratio.min(absolute).max(64)
}

fn total_memory_mb() -> Option<u64> {
    let content = fs::read_to_string("/proc/meminfo").ok()?;
    let line = content.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kib / 1024)
}

fn uninstall_systemd() -> AppResult<()> {
    let _ = run_command("systemctl", ["disable", "--now", SERVICE_NAME]);
    let _ = fs::remove_file(SYSTEMD_UNIT_PATH);
    let _ = run_command("systemctl", ["daemon-reload"]);
    Ok(())
}

fn install_initd(config_path: &Path) -> AppResult<()> {
    let script = format!(
        "#!/bin/sh\n\
         ### BEGIN INIT INFO\n\
         # Provides:          useless\n\
         # Required-Start:    $remote_fs $syslog $network\n\
         # Required-Stop:     $remote_fs $syslog $network\n\
         # Default-Start:     2 3 4 5\n\
         # Default-Stop:      0 1 6\n\
         # Short-Description: useless synthetic resource daemon\n\
         ### END INIT INFO\n\
         \n\
         PIDFILE={pid}\n\
         BIN={bin}\n\
         CONFIG={config}\n\
         \n\
         start() {{\n\
           if [ -f \"$PIDFILE\" ] && kill -0 $(cat \"$PIDFILE\") 2>/dev/null; then\n\
             echo \"useless already running\"\n\
             return 0\n\
           fi\n\
           if command -v start-stop-daemon >/dev/null 2>&1; then\n\
             start-stop-daemon --start --background --make-pidfile --pidfile \"$PIDFILE\" --nicelevel 19 --exec \"$BIN\" -- daemon --config \"$CONFIG\"\n\
           else\n\
             nice -n 19 \"$BIN\" daemon --config \"$CONFIG\" >/var/log/useless.log 2>&1 &\n\
             echo $! > \"$PIDFILE\"\n\
           fi\n\
         }}\n\
         \n\
         stop() {{\n\
           if [ -f \"$PIDFILE\" ]; then\n\
             kill $(cat \"$PIDFILE\") 2>/dev/null || true\n\
             rm -f \"$PIDFILE\"\n\
           fi\n\
         }}\n\
         \n\
         reload() {{\n\
           if [ -f \"$PIDFILE\" ]; then\n\
             kill -HUP $(cat \"$PIDFILE\") 2>/dev/null || true\n\
           fi\n\
         }}\n\
         \n\
         status() {{\n\
           if [ -f \"$PIDFILE\" ] && kill -0 $(cat \"$PIDFILE\") 2>/dev/null; then\n\
             echo \"useless is running\"\n\
             exit 0\n\
           fi\n\
           echo \"useless is stopped\"\n\
           exit 1\n\
         }}\n\
         \n\
         case \"$1\" in\n\
           start) start ;;\n\
           stop) stop ;;\n\
           restart) stop; start ;;\n\
           reload) reload ;;\n\
           status) status ;;\n\
           *) echo \"Usage: $0 {{start|stop|restart|reload|status}}\"; exit 1 ;;\n\
         esac\n",
        pid = PID_FILE_PATH,
        bin = INSTALL_BIN_PATH,
        config = config_path.display()
    );
    fs::write(INITD_SCRIPT_PATH, script)?;
    let mut perms = fs::metadata(INITD_SCRIPT_PATH)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(INITD_SCRIPT_PATH, perms)?;

    if command_exists("update-rc.d") {
        let _ = run_command("update-rc.d", ["useless", "defaults"]);
    } else if command_exists("chkconfig") {
        let _ = run_command("chkconfig", ["--add", "useless"]);
    }
    let _ = run_command(INITD_SCRIPT_PATH, ["restart"]);
    Ok(())
}

fn uninstall_initd() -> AppResult<()> {
    if Path::new(INITD_SCRIPT_PATH).exists() {
        let _ = run_command(INITD_SCRIPT_PATH, ["stop"]);
        if command_exists("update-rc.d") {
            let _ = run_command("update-rc.d", ["-f", "useless", "remove"]);
        } else if command_exists("chkconfig") {
            let _ = run_command("chkconfig", ["--del", "useless"]);
        }
        let _ = fs::remove_file(INITD_SCRIPT_PATH);
    }
    Ok(())
}

pub(crate) fn ensure_root() -> AppResult<()> {
    let euid = unsafe { geteuid() };
    if euid == 0 {
        Ok(())
    } else {
        Err(boxed_error("this command must be run as root"))
    }
}

fn command_exists(name: &str) -> bool {
    let path = env::var_os("PATH").unwrap_or_default();
    env::split_paths(&path).any(|dir| dir.join(name).exists())
}

fn run_command<const N: usize>(program: &str, args: [&str; N]) -> AppResult<()> {
    let output = Command::new(program).args(args).output()?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(boxed_error(format!(
        "command failed: {} {:?}\nstdout: {}\nstderr: {}",
        program, args, stdout, stderr
    )))
}

fn command_success<const N: usize>(program: &str, args: [&str; N]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[allow(dead_code)]
fn write_file(path: impl Into<PathBuf>, content: impl AsRef<[u8]>) -> AppResult<()> {
    let path = path.into();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(path)?;
    file.write_all(content.as_ref())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::config::{Config, Preset};

    use super::{memory_ceiling_mb, systemd_unit};

    #[test]
    fn derives_self_containment_limits() {
        let config = Config::for_preset(Preset::Balanced);
        assert_eq!(memory_ceiling_mb(&config, 2048), 307);

        let unit = systemd_unit(Path::new("/etc/useless/config.toml"), &config, 2048);
        assert!(unit.contains("OOMScoreAdjust=1000"));
        assert!(unit.contains("MemoryHigh=371M"));
        assert!(unit.contains("MemoryMax=435M"));
        assert!(unit.contains("IOSchedulingClass=idle"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("Nice=19"));
    }

    #[test]
    fn absolute_memory_cap_wins() {
        let mut config = Config::for_preset(Preset::Balanced);
        config.memory.max_absolute_mb = 128;
        assert_eq!(memory_ceiling_mb(&config, 2048), 128);
    }
}
