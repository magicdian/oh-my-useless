use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::runtime::SharedRuntime;

pub fn run(shared: SharedRuntime) {
    let mut cursor = 0usize;

    while !should_stop(&shared) {
        let (allowed, max_mbps, duty_percent, timeout_secs, targets, burst_seconds) = {
            let state = shared.read().expect("network worker read lock");
            let targets = state.config.network.targets.clone();
            let burst = state.config.network.window.burst_secs_max.max(1);
            (
                state.policy.network.allowed,
                state.config.network.max_mbps,
                state.policy.network.duty_percent,
                state.config.network.connect_timeout_secs,
                targets,
                burst,
            )
        };

        if !allowed || targets.is_empty() || max_mbps == 0 {
            thread::sleep(Duration::from_secs(1));
            continue;
        }

        let target = pick_target(&targets, &mut cursor);
        let max_bytes_per_sec = max_mbps as u64 * 125_000 * duty_percent as u64 / 100;
        if max_bytes_per_sec == 0 {
            thread::sleep(Duration::from_secs(1));
            continue;
        }

        let _ = download_until_paused(
            &shared,
            &target,
            max_bytes_per_sec,
            Duration::from_secs(burst_seconds),
            Duration::from_secs(timeout_secs.max(1)),
        );
        thread::sleep(Duration::from_millis(250));
    }
}

fn download_until_paused(
    shared: &SharedRuntime,
    target: &str,
    max_bytes_per_sec: u64,
    max_duration: Duration,
    connect_timeout: Duration,
) -> std::io::Result<()> {
    let parsed = ParsedUrl::parse(target)?;
    let mut addresses = (parsed.host.as_str(), parsed.port).to_socket_addrs()?;
    let address = addresses
        .next()
        .ok_or_else(|| std::io::Error::other("no address resolved for target"))?;
    let mut stream = TcpStream::connect_timeout(&address, connect_timeout)?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: useless/0.1.0\r\nConnection: close\r\n\r\n",
        parsed.path, parsed.host
    );
    stream.write_all(request.as_bytes())?;

    let started = Instant::now();
    let mut buffer = [0u8; 32 * 1024];
    let mut window_started = Instant::now();
    let mut window_bytes = 0u64;

    loop {
        if should_stop(shared) || !network_allowed(shared) || started.elapsed() >= max_duration {
            break;
        }

        if window_started.elapsed() >= Duration::from_secs(1) {
            window_started = Instant::now();
            window_bytes = 0;
        }
        if window_bytes >= max_bytes_per_sec {
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        let remaining = max_bytes_per_sec.saturating_sub(window_bytes).max(1);
        let read_cap = remaining.min(buffer.len() as u64) as usize;
        match stream.read(&mut buffer[..read_cap]) {
            Ok(0) => break,
            Ok(bytes_read) => {
                window_bytes += bytes_read as u64;
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
    }

    Ok(())
}

fn pick_target(targets: &[String], cursor: &mut usize) -> String {
    if targets.is_empty() {
        return String::new();
    }
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos() as usize)
        .unwrap_or(0);
    let index = (*cursor + jitter) % targets.len();
    *cursor = (*cursor + 1) % targets.len();
    targets[index].clone()
}

fn should_stop(shared: &SharedRuntime) -> bool {
    shared.read().expect("network worker read lock").shutdown
}

fn network_allowed(shared: &SharedRuntime) -> bool {
    shared
        .read()
        .expect("network worker read lock")
        .policy
        .network
        .allowed
}

struct ParsedUrl {
    host: String,
    port: u16,
    path: String,
}

impl ParsedUrl {
    fn parse(url: &str) -> std::io::Result<Self> {
        let without_scheme = url
            .strip_prefix("http://")
            .ok_or_else(|| std::io::Error::other("only plain http:// targets are supported"))?;
        let (host_port, path) = match without_scheme.split_once('/') {
            Some((host_port, path)) => (host_port, format!("/{}", path)),
            None => (without_scheme, "/".to_string()),
        };
        let (host, port) = match host_port.split_once(':') {
            Some((host, port)) => (host.to_string(), port.parse::<u16>().unwrap_or(80)),
            None => (host_port.to_string(), 80),
        };
        Ok(Self { host, port, path })
    }
}
