/// A host observation that does not depend on interactive sessions.
///
/// Byte fields are carried alongside percentages so later budget work can use
/// absolute headroom without re-reading `/proc`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SampleSnapshot {
    pub load_percent: u8,
    pub mem_available_percent: u8,
    pub mem_available_bytes: u64,
    pub mem_total_bytes: u64,
    pub swap_used_bytes: u64,
    pub net_rx_bytes_per_sec: u64,
    pub net_tx_bytes_per_sec: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub tty: String,
    pub remote: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionSet {
    sessions: Vec<Session>,
}

impl SessionSet {
    pub fn new(sessions: Vec<Session>) -> Self {
        Self { sessions }
    }

    #[allow(dead_code)]
    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn tty_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn ssh_count(&self) -> usize {
        self.sessions
            .iter()
            .filter(|session| session.remote)
            .count()
    }
}

/// A fact observed by an adapter. Signals never encode decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    Sample(SampleSnapshot),
    Sessions(SessionSet),
    SourceDegraded { source: String, message: String },
}
