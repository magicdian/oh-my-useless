use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use crate::error::AppResult;

#[derive(Debug, Clone, Default)]
pub struct SensorSample {
    pub load_percent: u8,
    pub mem_available_percent: u8,
    pub interactive_session: bool,
    pub tty_sessions: usize,
    pub ssh_sessions: usize,
    pub net_rx_bytes_per_sec: u64,
    pub net_tx_bytes_per_sec: u64,
}

pub trait SensorBackend {
    fn sample(&mut self, active_tty_idle_secs: u64) -> AppResult<SensorSample>;
    fn backend_name(&self) -> &'static str;
}

pub struct ProcfsSensor {
    last_net: Option<(Instant, u64, u64)>,
}

impl ProcfsSensor {
    pub fn new() -> Self {
        Self { last_net: None }
    }
}

impl SensorBackend for ProcfsSensor {
    fn sample(&mut self, active_tty_idle_secs: u64) -> AppResult<SensorSample> {
        let cores = std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(1)
            .max(1);
        let load_one = read_load_one()?;
        let load_percent = ((load_one / cores as f64) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8;
        let mem_available_percent = read_mem_available_percent()?;
        let (tty_sessions, ssh_sessions) = read_interactive_counts(active_tty_idle_secs);
        let interactive_session = tty_sessions > 0 || ssh_sessions > 0;
        let (net_rx_bytes_per_sec, net_tx_bytes_per_sec) = self.read_network_rates()?;

        Ok(SensorSample {
            load_percent,
            mem_available_percent,
            interactive_session,
            tty_sessions,
            ssh_sessions,
            net_rx_bytes_per_sec,
            net_tx_bytes_per_sec,
        })
    }

    fn backend_name(&self) -> &'static str {
        "procfs"
    }
}

impl ProcfsSensor {
    fn read_network_rates(&mut self) -> AppResult<(u64, u64)> {
        let now = Instant::now();
        let (rx_total, tx_total) = read_network_totals()?;

        let rates = if let Some((previous_time, previous_rx, previous_tx)) = self.last_net {
            let elapsed = now.duration_since(previous_time).as_secs_f64().max(1.0);
            (
                ((rx_total.saturating_sub(previous_rx)) as f64 / elapsed) as u64,
                ((tx_total.saturating_sub(previous_tx)) as f64 / elapsed) as u64,
            )
        } else {
            (0, 0)
        };

        self.last_net = Some((now, rx_total, tx_total));
        Ok(rates)
    }
}

fn read_load_one() -> AppResult<f64> {
    let content = fs::read_to_string("/proc/loadavg")?;
    let value = content
        .split_whitespace()
        .next()
        .ok_or_else(|| std::io::Error::other("missing loadavg field"))?;
    Ok(value.parse::<f64>()?)
}

fn read_mem_available_percent() -> AppResult<u8> {
    let content = fs::read_to_string("/proc/meminfo")?;
    let mut total = 0u64;
    let mut available = 0u64;

    for line in content.lines() {
        if let Some(value) = line.strip_prefix("MemTotal:") {
            total = value
                .split_whitespace()
                .next()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("MemAvailable:") {
            available = value
                .split_whitespace()
                .next()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
        }
    }

    if total == 0 {
        return Ok(0);
    }

    Ok(((available as f64 / total as f64) * 100.0)
        .round()
        .clamp(0.0, 100.0) as u8)
}

fn read_interactive_counts(active_tty_idle_secs: u64) -> (usize, usize) {
    let active_sessions = Command::new("who")
        .args(["-u", "--ips"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|stdout| parse_who_users(&stdout, active_tty_idle_secs))
        .unwrap_or_default();

    let ssh_ttys = Command::new("ps")
        .args(["-eo", "args"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|stdout| parse_sshd_ttys(&stdout))
        .unwrap_or_default();

    counts_from_active_sessions(&active_sessions, &ssh_ttys)
}

fn read_network_totals() -> AppResult<(u64, u64)> {
    let mut rx_total = 0u64;
    let mut tx_total = 0u64;

    for entry in fs::read_dir("/sys/class/net")? {
        let entry = entry?;
        let interface = entry.file_name();
        let name = interface.to_string_lossy();
        if name == "lo" {
            continue;
        }

        let path = entry.path();
        if !interface_is_up(&path) {
            continue;
        }

        rx_total += read_u64(path.join("statistics/rx_bytes")).unwrap_or(0);
        tx_total += read_u64(path.join("statistics/tx_bytes")).unwrap_or(0);
    }

    Ok((rx_total, tx_total))
}

fn interface_is_up(path: &Path) -> bool {
    fs::read_to_string(path.join("operstate"))
        .map(|value| value.trim() == "up")
        .unwrap_or(false)
}

fn read_u64(path: impl AsRef<Path>) -> AppResult<u64> {
    Ok(fs::read_to_string(path)?.trim().parse::<u64>()?)
}

fn parse_who_users(stdout: &str, active_tty_idle_secs: u64) -> BTreeMap<String, bool> {
    let mut sessions_by_tty: BTreeMap<String, bool> = BTreeMap::new();

    for line in stdout.lines() {
        let Some(session) = parse_who_user_line(line, active_tty_idle_secs) else {
            continue;
        };
        if !session.active {
            continue;
        }

        sessions_by_tty
            .entry(session.tty)
            .and_modify(|remote| *remote = *remote || session.remote)
            .or_insert(session.remote);
    }

    sessions_by_tty
}

fn counts_from_active_sessions(
    sessions_by_tty: &BTreeMap<String, bool>,
    ssh_ttys: &BTreeSet<String>,
) -> (usize, usize) {
    let tty_sessions = sessions_by_tty.len();
    let fallback_ssh_sessions = sessions_by_tty.values().filter(|remote| **remote).count();

    let ssh_sessions = if ssh_ttys.is_empty() {
        fallback_ssh_sessions
    } else {
        let correlated_ssh_sessions = sessions_by_tty
            .iter()
            .filter(|(tty, remote)| **remote && ssh_ttys.contains(*tty))
            .count();

        if correlated_ssh_sessions > 0 {
            correlated_ssh_sessions
        } else {
            fallback_ssh_sessions
        }
    };

    (tty_sessions, ssh_sessions)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WhoSession {
    tty: String,
    active: bool,
    remote: bool,
}

fn parse_who_user_line(line: &str, active_tty_idle_secs: u64) -> Option<WhoSession> {
    let parts: Vec<_> = line.split_whitespace().collect();
    if parts.len() < 5 {
        return None;
    }

    let tty = parts.get(1)?.to_string();
    if !(tty.starts_with("pts/") || tty.starts_with("tty")) {
        return None;
    }

    let idle = *parts.get(4)?;
    let idle_secs = parse_idle_token(idle)?;
    let remote = is_remote_session(&tty, &parts);

    Some(WhoSession {
        tty,
        active: idle_secs <= active_tty_idle_secs,
        remote,
    })
}

fn parse_idle_token(token: &str) -> Option<u64> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }

    match token {
        "." => Some(0),
        "old" => Some(u64::MAX),
        _ if token.ends_with('s') => parse_idle_seconds_suffix(token),
        _ if token.ends_with('m') => parse_idle_minutes_suffix(token),
        _ if token.contains(':') => parse_idle_colon(token),
        _ => token.parse::<u64>().ok().map(|value| value * 60),
    }
}

fn parse_idle_seconds_suffix(token: &str) -> Option<u64> {
    let value = token.strip_suffix('s')?;
    if let Some((whole, frac)) = value.split_once('.') {
        let whole = whole.parse::<u64>().ok()?;
        let frac = frac.parse::<u64>().ok().unwrap_or(0);
        Some(whole + u64::from(frac > 0))
    } else {
        value.parse::<u64>().ok()
    }
}

fn parse_idle_minutes_suffix(token: &str) -> Option<u64> {
    let value = token.strip_suffix('m')?;
    let (minutes, seconds) = value.split_once(':')?;
    let minutes = minutes.parse::<u64>().ok()?;
    let seconds = seconds.parse::<u64>().ok()?;
    Some(minutes * 60 + seconds)
}

fn parse_idle_colon(token: &str) -> Option<u64> {
    let segments: Vec<_> = token.split(':').collect();
    match segments.as_slice() {
        [minutes, seconds] => {
            let minutes = minutes.parse::<u64>().ok()?;
            let seconds = seconds.parse::<u64>().ok()?;
            Some(minutes * 60 + seconds)
        }
        [hours, minutes, seconds] => {
            let hours = hours.parse::<u64>().ok()?;
            let minutes = minutes.parse::<u64>().ok()?;
            let seconds = seconds.parse::<u64>().ok()?;
            Some(hours * 3600 + minutes * 60 + seconds)
        }
        _ => None,
    }
}

fn is_remote_session(tty: &str, parts: &[&str]) -> bool {
    if !tty.starts_with("pts/") {
        return false;
    }

    let Some(last) = parts.last().copied() else {
        return false;
    };

    if last.starts_with('(') && last.ends_with(')') {
        return true;
    }

    if parts.len() < 7 {
        return false;
    }

    looks_like_remote_host(last)
}

fn looks_like_remote_host(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }

    value.contains('.') || value.contains(':') || value.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn parse_sshd_ttys(stdout: &str) -> BTreeSet<String> {
    stdout.lines().filter_map(parse_sshd_tty).collect()
}

fn parse_sshd_tty(line: &str) -> Option<String> {
    if !line.contains("sshd:") {
        return None;
    }

    let marker = "@pts/";
    let start = line.find(marker)?;
    let suffix = &line[start + 1..];
    let tty = suffix
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches(|ch: char| matches!(ch, ')' | ',' | ':' | ']'));

    if tty.starts_with("pts/") {
        Some(tty.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{counts_from_active_sessions, parse_idle_token, parse_sshd_ttys, parse_who_users};

    #[test]
    fn counts_only_recent_remote_pts_sessions() {
        let stdout = "\
root pts/0 2026-04-15 16:16 . 1000 (183.240.41.22)\n\
root pts/1 2026-04-15 15:00 old 1001 (183.240.41.22)\n\
root pts/2 2026-04-15 16:15 00:03 1002 (14.145.215.6)\n\
root tty1 2026-04-15 16:10 00:10 1003\n";

        let sessions = parse_who_users(stdout, 300);
        let ssh_ttys = parse_sshd_ttys("sshd: root@pts/0\nsshd: root [priv]\nsshd: root@pts/2\n");
        let (tty_sessions, ssh_sessions) = counts_from_active_sessions(&sessions, &ssh_ttys);
        assert_eq!(tty_sessions, 3);
        assert_eq!(ssh_sessions, 2);
    }

    #[test]
    fn counts_remote_sessions_without_parentheses() {
        let stdout = "\
root pts/6 2026-04-15 16:30 00:02 1115311 183.240.41.22\n\
root pts/7 2026-04-15 16:31 . 1116000 14.145.215.6\n";

        let sessions = parse_who_users(stdout, 300);
        let ssh_ttys = parse_sshd_ttys("sshd: root@pts/6\nsshd: root@pts/7\n");
        let (tty_sessions, ssh_sessions) = counts_from_active_sessions(&sessions, &ssh_ttys);
        assert_eq!(tty_sessions, 2);
        assert_eq!(ssh_sessions, 2);
    }

    #[test]
    fn deduplicates_active_tty_rows() {
        let stdout = "\
ubuntu pts/1 2026-04-15 09:11 . 1234 (98.98.112.219)\n\
root pts/1 2026-04-15 09:11 . 5678 (98.98.112.219)\n";

        let sessions = parse_who_users(stdout, 300);
        let ssh_ttys = parse_sshd_ttys("sshd: ubuntu@pts/1\n");
        let (tty_sessions, ssh_sessions) = counts_from_active_sessions(&sessions, &ssh_ttys);
        assert_eq!(tty_sessions, 1);
        assert_eq!(ssh_sessions, 1);
    }

    #[test]
    fn falls_back_to_remote_pts_when_ps_cannot_confirm_ssh() {
        let stdout = "\
root pts/5 2026-04-15 16:16 . 1110247 (183.240.41.22)\n\
root tty1 2026-04-15 16:10 00:10 1003\n";

        let sessions = parse_who_users(stdout, 300);
        let ssh_ttys = parse_sshd_ttys("");
        let (tty_sessions, ssh_sessions) = counts_from_active_sessions(&sessions, &ssh_ttys);
        assert_eq!(tty_sessions, 2);
        assert_eq!(ssh_sessions, 1);
    }

    #[test]
    fn falls_back_to_remote_pts_when_ps_mismatches_active_ttys() {
        let stdout = "\
root pts/5 2026-04-15 16:16 . 1110247 (183.240.41.22)\n\
root pts/6 2026-04-15 16:17 . 1110248 (183.240.41.22)\n";

        let sessions = parse_who_users(stdout, 300);
        let ssh_ttys = parse_sshd_ttys("sshd: root@pts/9\n");
        let (tty_sessions, ssh_sessions) = counts_from_active_sessions(&sessions, &ssh_ttys);
        assert_eq!(tty_sessions, 2);
        assert_eq!(ssh_sessions, 2);
    }

    #[test]
    fn parses_sshd_pts_targets() {
        let ssh_ttys = parse_sshd_ttys(
            "\
sshd: ubuntu [priv]\n\
sshd: ubuntu@pts/0\n\
sshd: root@pts/1,\n\
not-sshd: ignored\n",
        );

        assert!(ssh_ttys.contains("pts/0"));
        assert!(ssh_ttys.contains("pts/1"));
        assert_eq!(ssh_ttys.len(), 2);
    }

    #[test]
    fn parses_idle_tokens() {
        assert_eq!(parse_idle_token("."), Some(0));
        assert_eq!(parse_idle_token("old"), Some(u64::MAX));
        assert_eq!(parse_idle_token("00:03"), Some(3));
        assert_eq!(parse_idle_token("1:02:03"), Some(3723));
        assert_eq!(parse_idle_token("2.00s"), Some(2));
        assert_eq!(parse_idle_token("05:10m"), Some(310));
    }
}
