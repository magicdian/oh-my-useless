use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{AppResult, boxed_error};

pub const DEFAULT_CONFIG_DIR: &str = "/etc/useless";
pub const DEFAULT_CONFIG_PATH: &str = "/etc/useless/config.toml";
pub const INSTALL_BIN_PATH: &str = "/usr/local/bin/useless";
pub const SYSTEMD_UNIT_PATH: &str = "/etc/systemd/system/useless.service";
pub const INITD_SCRIPT_PATH: &str = "/etc/init.d/useless";
pub const PID_FILE_PATH: &str = "/run/useless.pid";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Low,
    Balanced,
    Aggressive,
}

impl Preset {
    pub fn parse(value: &str) -> AppResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "low" => Ok(Self::Low),
            "balanced" => Ok(Self::Balanced),
            "aggressive" => Ok(Self::Aggressive),
            other => Err(boxed_error(format!("unsupported preset: {other}"))),
        }
    }
}

impl fmt::Display for Preset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Preset::Low => write!(f, "low"),
            Preset::Balanced => write!(f, "balanced"),
            Preset::Aggressive => write!(f, "aggressive"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BurstWindow {
    pub burst_secs_min: u64,
    pub burst_secs_max: u64,
    pub rest_secs_min: u64,
    pub rest_secs_max: u64,
}

#[derive(Debug, Clone)]
pub struct GlobalConfig {
    pub check_interval_secs: u64,
    pub interactive_grace_secs: u64,
    pub active_tty_idle_secs: u64,
    pub caution_duty_percent: u8,
    pub caution_load_percent: u8,
    pub pause_load_percent: u8,
    pub caution_mem_available_percent: u8,
    pub pause_mem_available_percent: u8,
}

#[derive(Debug, Clone)]
pub struct CpuConfig {
    pub enabled: bool,
    pub max_percent: u8,
    pub window: BurstWindow,
}

#[derive(Debug, Clone)]
pub struct MemoryConfig {
    pub enabled: bool,
    pub max_percent: u8,
    pub window: BurstWindow,
}

#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub enabled: bool,
    pub max_mbps: u32,
    pub connect_timeout_secs: u64,
    pub window: BurstWindow,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub preset: Preset,
    pub global: GlobalConfig,
    pub cpu: CpuConfig,
    pub memory: MemoryConfig,
    pub network: NetworkConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self::for_preset(Preset::Balanced)
    }
}

impl Config {
    pub fn for_preset(preset: Preset) -> Self {
        match preset {
            Preset::Low => Self {
                preset,
                global: GlobalConfig {
                    check_interval_secs: 5,
                    interactive_grace_secs: 120,
                    active_tty_idle_secs: 300,
                    caution_duty_percent: 35,
                    caution_load_percent: 45,
                    pause_load_percent: 70,
                    caution_mem_available_percent: 25,
                    pause_mem_available_percent: 12,
                },
                cpu: CpuConfig {
                    enabled: true,
                    max_percent: 8,
                    window: BurstWindow {
                        burst_secs_min: 10,
                        burst_secs_max: 35,
                        rest_secs_min: 45,
                        rest_secs_max: 180,
                    },
                },
                memory: MemoryConfig {
                    enabled: true,
                    max_percent: 8,
                    window: BurstWindow {
                        burst_secs_min: 30,
                        burst_secs_max: 90,
                        rest_secs_min: 120,
                        rest_secs_max: 360,
                    },
                },
                network: NetworkConfig {
                    enabled: true,
                    max_mbps: 5,
                    connect_timeout_secs: 3,
                    window: BurstWindow {
                        burst_secs_min: 15,
                        burst_secs_max: 45,
                        rest_secs_min: 120,
                        rest_secs_max: 420,
                    },
                    targets: default_targets(),
                },
            },
            Preset::Balanced => Self {
                preset,
                global: GlobalConfig {
                    check_interval_secs: 5,
                    interactive_grace_secs: 180,
                    active_tty_idle_secs: 300,
                    caution_duty_percent: 45,
                    caution_load_percent: 55,
                    pause_load_percent: 82,
                    caution_mem_available_percent: 20,
                    pause_mem_available_percent: 10,
                },
                cpu: CpuConfig {
                    enabled: true,
                    max_percent: 15,
                    window: BurstWindow {
                        burst_secs_min: 15,
                        burst_secs_max: 50,
                        rest_secs_min: 40,
                        rest_secs_max: 180,
                    },
                },
                memory: MemoryConfig {
                    enabled: true,
                    max_percent: 15,
                    window: BurstWindow {
                        burst_secs_min: 40,
                        burst_secs_max: 120,
                        rest_secs_min: 90,
                        rest_secs_max: 360,
                    },
                },
                network: NetworkConfig {
                    enabled: true,
                    max_mbps: 10,
                    connect_timeout_secs: 3,
                    window: BurstWindow {
                        burst_secs_min: 20,
                        burst_secs_max: 60,
                        rest_secs_min: 90,
                        rest_secs_max: 360,
                    },
                    targets: default_targets(),
                },
            },
            Preset::Aggressive => Self {
                preset,
                global: GlobalConfig {
                    check_interval_secs: 4,
                    interactive_grace_secs: 180,
                    active_tty_idle_secs: 300,
                    caution_duty_percent: 55,
                    caution_load_percent: 60,
                    pause_load_percent: 90,
                    caution_mem_available_percent: 18,
                    pause_mem_available_percent: 8,
                },
                cpu: CpuConfig {
                    enabled: true,
                    max_percent: 22,
                    window: BurstWindow {
                        burst_secs_min: 20,
                        burst_secs_max: 70,
                        rest_secs_min: 25,
                        rest_secs_max: 120,
                    },
                },
                memory: MemoryConfig {
                    enabled: true,
                    max_percent: 22,
                    window: BurstWindow {
                        burst_secs_min: 60,
                        burst_secs_max: 180,
                        rest_secs_min: 60,
                        rest_secs_max: 300,
                    },
                },
                network: NetworkConfig {
                    enabled: true,
                    max_mbps: 20,
                    connect_timeout_secs: 3,
                    window: BurstWindow {
                        burst_secs_min: 30,
                        burst_secs_max: 90,
                        rest_secs_min: 45,
                        rest_secs_max: 240,
                    },
                    targets: default_targets(),
                },
            },
        }
    }
}

pub fn load_config(path: &Path) -> AppResult<Config> {
    let content = fs::read_to_string(path)?;
    parse_config(&content)
}

pub fn parse_config(content: &str) -> AppResult<Config> {
    #[derive(Clone)]
    struct Entry {
        section: Option<String>,
        key: String,
        value: String,
        line_number: usize,
    }

    let mut entries = Vec::new();
    let mut section = None::<String>;

    for (index, raw_line) in content.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = Some(line[1..line.len() - 1].trim().to_string());
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            return Err(boxed_error(format!(
                "invalid config line {line_number}: {line}"
            )));
        };

        entries.push(Entry {
            section: section.clone(),
            key: key.trim().to_string(),
            value: value.trim().to_string(),
            line_number,
        });
    }

    let preset = entries
        .iter()
        .find(|entry| entry.section.is_none() && entry.key == "preset")
        .map(|entry| Preset::parse(&parse_string(&entry.value)))
        .transpose()?
        .unwrap_or(Preset::Balanced);

    let mut config = Config::for_preset(preset);

    for entry in entries {
        let line = entry.line_number;
        match (entry.section.as_deref(), entry.key.as_str()) {
            (None, "preset") => {}
            (None, "check_interval_secs") => {
                config.global.check_interval_secs = parse_u64(&entry.value, line)?
            }
            (None, "interactive_grace_secs") => {
                config.global.interactive_grace_secs = parse_u64(&entry.value, line)?
            }
            (None, "active_tty_idle_secs") => {
                config.global.active_tty_idle_secs = parse_u64(&entry.value, line)?
            }
            (None, "caution_duty_percent") => {
                config.global.caution_duty_percent = parse_u8(&entry.value, line)?
            }
            (None, "caution_load_percent") => {
                config.global.caution_load_percent = parse_u8(&entry.value, line)?
            }
            (None, "pause_load_percent") => {
                config.global.pause_load_percent = parse_u8(&entry.value, line)?
            }
            (None, "caution_mem_available_percent") => {
                config.global.caution_mem_available_percent = parse_u8(&entry.value, line)?
            }
            (None, "pause_mem_available_percent") => {
                config.global.pause_mem_available_percent = parse_u8(&entry.value, line)?
            }
            (Some("cpu"), "enabled") => config.cpu.enabled = parse_bool(&entry.value, line)?,
            (Some("cpu"), "max_percent") => config.cpu.max_percent = parse_u8(&entry.value, line)?,
            (Some("cpu"), "burst_secs_min") => {
                config.cpu.window.burst_secs_min = parse_u64(&entry.value, line)?
            }
            (Some("cpu"), "burst_secs_max") => {
                config.cpu.window.burst_secs_max = parse_u64(&entry.value, line)?
            }
            (Some("cpu"), "rest_secs_min") => {
                config.cpu.window.rest_secs_min = parse_u64(&entry.value, line)?
            }
            (Some("cpu"), "rest_secs_max") => {
                config.cpu.window.rest_secs_max = parse_u64(&entry.value, line)?
            }
            (Some("memory"), "enabled") => config.memory.enabled = parse_bool(&entry.value, line)?,
            (Some("memory"), "max_percent") => {
                config.memory.max_percent = parse_u8(&entry.value, line)?
            }
            (Some("memory"), "burst_secs_min") => {
                config.memory.window.burst_secs_min = parse_u64(&entry.value, line)?
            }
            (Some("memory"), "burst_secs_max") => {
                config.memory.window.burst_secs_max = parse_u64(&entry.value, line)?
            }
            (Some("memory"), "rest_secs_min") => {
                config.memory.window.rest_secs_min = parse_u64(&entry.value, line)?
            }
            (Some("memory"), "rest_secs_max") => {
                config.memory.window.rest_secs_max = parse_u64(&entry.value, line)?
            }
            (Some("network"), "enabled") => {
                config.network.enabled = parse_bool(&entry.value, line)?
            }
            (Some("network"), "max_mbps") => {
                config.network.max_mbps = parse_u32(&entry.value, line)?
            }
            (Some("network"), "connect_timeout_secs") => {
                config.network.connect_timeout_secs = parse_u64(&entry.value, line)?
            }
            (Some("network"), "burst_secs_min") => {
                config.network.window.burst_secs_min = parse_u64(&entry.value, line)?
            }
            (Some("network"), "burst_secs_max") => {
                config.network.window.burst_secs_max = parse_u64(&entry.value, line)?
            }
            (Some("network"), "rest_secs_min") => {
                config.network.window.rest_secs_min = parse_u64(&entry.value, line)?
            }
            (Some("network"), "rest_secs_max") => {
                config.network.window.rest_secs_max = parse_u64(&entry.value, line)?
            }
            (Some("network"), "targets") => {
                config.network.targets = parse_string_list(&entry.value, line)?
            }
            _ => {
                return Err(boxed_error(format!(
                    "unknown config key on line {line}: section={:?} key={}",
                    entry.section, entry.key
                )));
            }
        }
    }

    validate_config(&config)?;
    Ok(config)
}

pub fn default_config_text() -> String {
    let config = Config::default();
    format!(
        "# useless configuration\n\
         # Edit the values below and run `useless reload` to apply changes.\n\
         preset = \"{preset}\"\n\
         check_interval_secs = {check_interval}\n\
         interactive_grace_secs = {interactive_grace}\n\
         active_tty_idle_secs = {active_tty_idle}\n\
         caution_duty_percent = {caution_duty}\n\
         caution_load_percent = {caution_load}\n\
         pause_load_percent = {pause_load}\n\
         caution_mem_available_percent = {caution_mem}\n\
         pause_mem_available_percent = {pause_mem}\n\
         \n\
         [cpu]\n\
         enabled = {cpu_enabled}\n\
         max_percent = {cpu_max}\n\
         burst_secs_min = {cpu_burst_min}\n\
         burst_secs_max = {cpu_burst_max}\n\
         rest_secs_min = {cpu_rest_min}\n\
         rest_secs_max = {cpu_rest_max}\n\
         \n\
         [memory]\n\
         enabled = {memory_enabled}\n\
         max_percent = {memory_max}\n\
         burst_secs_min = {memory_burst_min}\n\
         burst_secs_max = {memory_burst_max}\n\
         rest_secs_min = {memory_rest_min}\n\
         rest_secs_max = {memory_rest_max}\n\
         \n\
         [network]\n\
         enabled = {network_enabled}\n\
         max_mbps = {network_max}\n\
         connect_timeout_secs = {connect_timeout}\n\
         burst_secs_min = {network_burst_min}\n\
         burst_secs_max = {network_burst_max}\n\
         rest_secs_min = {network_rest_min}\n\
         rest_secs_max = {network_rest_max}\n\
         targets = [{targets}]\n",
        preset = config.preset,
        check_interval = config.global.check_interval_secs,
        interactive_grace = config.global.interactive_grace_secs,
        active_tty_idle = config.global.active_tty_idle_secs,
        caution_duty = config.global.caution_duty_percent,
        caution_load = config.global.caution_load_percent,
        pause_load = config.global.pause_load_percent,
        caution_mem = config.global.caution_mem_available_percent,
        pause_mem = config.global.pause_mem_available_percent,
        cpu_enabled = config.cpu.enabled,
        cpu_max = config.cpu.max_percent,
        cpu_burst_min = config.cpu.window.burst_secs_min,
        cpu_burst_max = config.cpu.window.burst_secs_max,
        cpu_rest_min = config.cpu.window.rest_secs_min,
        cpu_rest_max = config.cpu.window.rest_secs_max,
        memory_enabled = config.memory.enabled,
        memory_max = config.memory.max_percent,
        memory_burst_min = config.memory.window.burst_secs_min,
        memory_burst_max = config.memory.window.burst_secs_max,
        memory_rest_min = config.memory.window.rest_secs_min,
        memory_rest_max = config.memory.window.rest_secs_max,
        network_enabled = config.network.enabled,
        network_max = config.network.max_mbps,
        connect_timeout = config.network.connect_timeout_secs,
        network_burst_min = config.network.window.burst_secs_min,
        network_burst_max = config.network.window.burst_secs_max,
        network_rest_min = config.network.window.rest_secs_min,
        network_rest_max = config.network.window.rest_secs_max,
        targets = config
            .network
            .targets
            .iter()
            .map(|target| format!("\"{target}\""))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

pub fn ensure_default_config(path: &Path) -> AppResult<()> {
    if path.exists() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, default_config_text())?;
    Ok(())
}

pub fn default_config_path() -> PathBuf {
    PathBuf::from(DEFAULT_CONFIG_PATH)
}

fn default_targets() -> Vec<String> {
    vec![
        "http://mirror.nl.leaseweb.net/speedtest/10000mb.bin".to_string(),
        "http://mirror.dal10.us.leaseweb.net/speedtest/10000mb.bin".to_string(),
        "http://mirror.hk.leaseweb.net/speedtest/10000mb.bin".to_string(),
        "http://mirror.de.leaseweb.net/speedtest/10000mb.bin".to_string(),
        "http://proof.ovh.net/files/10Gio.dat".to_string(),
    ]
}

fn parse_string(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

fn parse_string_list(value: &str, line_number: usize) -> AppResult<Vec<String>> {
    let trimmed = value.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return Err(boxed_error(format!(
            "expected array syntax on line {line_number}"
        )));
    }
    let inner = trimmed[1..trimmed.len() - 1].trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    Ok(inner
        .split(',')
        .map(parse_string)
        .filter(|item| !item.is_empty())
        .collect())
}

fn parse_bool(value: &str, line_number: usize) -> AppResult<bool> {
    match value.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(boxed_error(format!(
            "expected boolean on line {line_number}"
        ))),
    }
}

fn parse_u8(value: &str, line_number: usize) -> AppResult<u8> {
    value
        .trim()
        .parse::<u8>()
        .map_err(|_| boxed_error(format!("expected u8 on line {line_number}")))
}

fn parse_u32(value: &str, line_number: usize) -> AppResult<u32> {
    value
        .trim()
        .parse::<u32>()
        .map_err(|_| boxed_error(format!("expected u32 on line {line_number}")))
}

fn parse_u64(value: &str, line_number: usize) -> AppResult<u64> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|_| boxed_error(format!("expected u64 on line {line_number}")))
}

fn validate_config(config: &Config) -> AppResult<()> {
    for (name, window) in [
        ("cpu", &config.cpu.window),
        ("memory", &config.memory.window),
        ("network", &config.network.window),
    ] {
        if window.burst_secs_min == 0 || window.rest_secs_min == 0 {
            return Err(boxed_error(format!(
                "{name} burst/rest minimums must be positive"
            )));
        }
        if window.burst_secs_min > window.burst_secs_max
            || window.rest_secs_min > window.rest_secs_max
        {
            return Err(boxed_error(format!(
                "{name} burst/rest minimums cannot exceed maximums"
            )));
        }
    }

    if config.global.caution_duty_percent == 0 || config.global.caution_duty_percent > 100 {
        return Err(boxed_error(
            "caution_duty_percent must be between 1 and 100",
        ));
    }
    if config.global.active_tty_idle_secs == 0 {
        return Err(boxed_error("active_tty_idle_secs must be positive"));
    }
    if config.global.caution_load_percent > config.global.pause_load_percent {
        return Err(boxed_error(
            "caution_load_percent cannot exceed pause_load_percent",
        ));
    }
    if config.global.pause_mem_available_percent > config.global.caution_mem_available_percent {
        return Err(boxed_error(
            "pause_mem_available_percent cannot exceed caution_mem_available_percent",
        ));
    }
    if config.network.enabled && config.network.targets.is_empty() {
        return Err(boxed_error(
            "network.targets cannot be empty when network is enabled",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Preset, parse_config};

    #[test]
    fn parses_overrides_from_text() {
        let config = parse_config(
            r#"
            preset = "low"
            caution_duty_percent = 20

            [cpu]
            max_percent = 11

            [network]
            targets = ["http://example.com/file.bin"]
            "#,
        )
        .expect("config parses");

        assert!(matches!(config.preset, Preset::Low));
        assert_eq!(config.global.caution_duty_percent, 20);
        assert_eq!(config.global.active_tty_idle_secs, 300);
        assert_eq!(config.cpu.max_percent, 11);
        assert_eq!(config.network.targets.len(), 1);
    }
}
