use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::state::{Mode, ReasonCode};
use crate::ports::JournalSink;

const MAX_JOURNAL_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub at: SystemTime,
    pub signal: String,
    pub from: Mode,
    pub to: Mode,
    pub reasons: Vec<ReasonCode>,
    pub budget_bytes: u64,
    pub mem_available_bytes: u64,
}

/// Bounded in-memory transition history with an optional append-only file.
pub struct Journal {
    capacity: usize,
    entries: VecDeque<Transition>,
    file: Option<PathBuf>,
    warned: bool,
}

impl Journal {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: VecDeque::new(),
            file: None,
            warned: false,
        }
    }

    pub fn with_file(capacity: usize, path: PathBuf) -> Self {
        let mut journal = Self::new(capacity);
        journal.file = Some(path);
        journal
    }

    pub fn record(&mut self, transition: Transition) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(transition.clone());

        if let Some(path) = self.file.clone() {
            if let Err(error) = append_line(&path, &transition) {
                if !self.warned {
                    eprintln!("[useless] journal file disabled: {error}");
                    self.warned = true;
                }
                self.file = None;
            } else {
                self.trim_file();
            }
        }
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[allow(dead_code)]
    pub fn recent(&self) -> impl Iterator<Item = &Transition> {
        self.entries.iter()
    }

    fn trim_file(&mut self) {
        let Some(path) = self.file.clone() else {
            return;
        };
        let Ok(metadata) = fs::metadata(&path) else {
            return;
        };
        if metadata.len() <= MAX_JOURNAL_BYTES {
            return;
        }

        let keep = (self.capacity / 2).max(1);
        let lines: Vec<String> = self
            .entries
            .iter()
            .rev()
            .take(keep)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(json_line)
            .collect();
        let content = format!("{}\n", lines.join("\n"));

        if let Err(error) = write_atomic(&path, &content) {
            if !self.warned {
                eprintln!("[useless] journal trim failed: {error}");
                self.warned = true;
            }
            self.file = None;
        }
    }
}

impl JournalSink for Journal {
    fn record(&mut self, transition: Transition) {
        Journal::record(self, transition);
    }
}

/// Human-readable daemon state. Line-based on purpose: `status` must not need
/// a JSON parser in a std-only binary.
#[derive(Debug, Clone)]
pub struct StateSnapshot {
    pub mode: &'static str,
    pub reasons: String,
    pub scope: String,
    pub summary: String,
    pub budget_mb: u64,
    pub reserve_mb: u64,
    pub available_mb: u64,
    pub capabilities: String,
    pub last_transition: String,
}

pub fn write_state(path: &Path, snapshot: &StateSnapshot) -> io::Result<()> {
    let content = format!(
        "mode={}\nreasons={}\nscope={}\nbudget_mb={}\nreserve_mb={}\navailable_mb={}\ncapabilities={}\nlast_transition={}\nsummary={}\n",
        snapshot.mode,
        snapshot.reasons,
        snapshot.scope,
        snapshot.budget_mb,
        snapshot.reserve_mb,
        snapshot.available_mb,
        snapshot.capabilities,
        snapshot.last_transition,
        snapshot.summary
    );
    write_atomic(path, &content)
}

fn append_line(path: &Path, transition: &Transition) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", json_line(transition))
}

fn write_atomic(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, content)?;
    fs::rename(&temporary, path)
}

fn json_line(transition: &Transition) -> String {
    let ts_ms = transition
        .at
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let reasons = transition
        .reasons
        .iter()
        .map(|reason| format!("\"{}\"", reason.label()))
        .collect::<Vec<_>>()
        .join(",");

    format!(
        "{{\"ts_ms\":{},\"signal\":\"{}\",\"from\":\"{}\",\"to\":\"{}\",\"reasons\":[{}],\"budget_mb\":{},\"available_mb\":{}}}",
        ts_ms,
        escape(&transition.signal),
        transition.from.key(),
        transition.to.key(),
        reasons,
        transition.budget_bytes / (1024 * 1024),
        transition.mem_available_bytes / (1024 * 1024)
    )
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use crate::domain::state::{Mode, ReasonCode};

    use super::{Journal, StateSnapshot, Transition, write_state};

    fn transition(to: Mode) -> Transition {
        Transition {
            at: SystemTime::now(),
            signal: "sample".to_string(),
            from: Mode::Running,
            to,
            reasons: vec![ReasonCode::LoadPressure],
            budget_bytes: 0,
            mem_available_bytes: 0,
        }
    }

    #[test]
    fn keeps_only_the_configured_capacity() {
        let mut journal = Journal::new(2);
        journal.record(transition(Mode::Cooling));
        journal.record(transition(Mode::Suspended));
        journal.record(transition(Mode::Running));

        assert_eq!(journal.len(), 2);
        let recent: Vec<_> = journal.recent().collect();
        assert_eq!(recent[0].to, Mode::Suspended);
        assert_eq!(recent[1].to, Mode::Running);
    }

    #[test]
    fn appends_json_lines_to_the_file() {
        let path =
            std::env::temp_dir().join(format!("useless-journal-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut journal = Journal::with_file(2, path.clone());
        journal.record(transition(Mode::Cooling));

        let text = std::fs::read_to_string(&path).expect("journal file");
        assert!(text.contains("\"to\":\"cooling\""));
        assert!(text.contains("\"reasons\":[\"load_pressure\"]"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn trims_the_file_when_it_exceeds_the_cap() {
        let path =
            std::env::temp_dir().join(format!("useless-journal-trim-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut journal = Journal::with_file(8, path.clone());
        for _ in 0..4000 {
            journal.record(transition(Mode::Running));
        }

        let len = std::fs::metadata(&path).expect("journal metadata").len();
        assert!(
            len <= super::MAX_JOURNAL_BYTES + 4096,
            "journal stayed unbounded: {len} bytes"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unwritable_journal_path_does_not_panic() {
        let blocker =
            std::env::temp_dir().join(format!("useless-journal-block-{}", std::process::id()));
        let _ = std::fs::write(&blocker, b"not a directory");

        let mut journal = Journal::with_file(2, blocker.join("journal.jsonl"));
        journal.record(transition(Mode::Running));

        assert_eq!(journal.len(), 1);
        let _ = std::fs::remove_file(&blocker);
    }

    #[test]
    fn writes_a_readable_state_snapshot() {
        let path = std::env::temp_dir().join(format!("useless-state-{}", std::process::id()));
        write_state(
            &path,
            &StateSnapshot {
                mode: "running",
                reasons: "load_pressure".to_string(),
                scope: String::new(),
                summary: "pressure=Idle load=10".to_string(),
                budget_mb: 12,
                reserve_mb: 4,
                available_mb: 600,
                capabilities: "psi=ok".to_string(),
                last_transition: "0".to_string(),
            },
        )
        .expect("state file");

        let text = std::fs::read_to_string(&path).expect("state text");
        assert!(text.contains("mode=running"));
        assert!(text.contains("available_mb=600"));

        let _ = std::fs::remove_file(&path);
    }
}
