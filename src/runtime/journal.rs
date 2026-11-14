use std::collections::VecDeque;
use std::time::SystemTime;

use crate::domain::state::{Mode, ReasonCode};
use crate::ports::JournalSink;

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

/// Bounded in-memory transition history. Child 2 adds a file-backed sink.
pub struct Journal {
    capacity: usize,
    entries: VecDeque<Transition>,
}

impl Journal {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: VecDeque::new(),
        }
    }

    pub fn record(&mut self, transition: Transition) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(transition);
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[allow(dead_code)]
    pub fn recent(&self) -> impl Iterator<Item = &Transition> {
        self.entries.iter()
    }
}

impl JournalSink for Journal {
    fn record(&mut self, transition: Transition) {
        Journal::record(self, transition);
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use crate::domain::state::{Mode, ReasonCode};

    use super::{Journal, Transition};

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
}
