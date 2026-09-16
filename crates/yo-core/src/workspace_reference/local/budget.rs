use std::{
    path::Path,
    time::{Duration, Instant},
};

/// Shared bounds for filesystem and tracked-path discovery in one inventory.
pub(super) struct DiscoveryBudget {
    pub(super) entries_left: usize,
    pub(super) bytes_left: usize,
    pub(super) deadline: Instant,
    pub(super) exhausted: bool,
}

impl Default for DiscoveryBudget {
    fn default() -> Self {
        Self {
            entries_left: 32_768,
            bytes_left: 16 * 1024 * 1024,
            deadline: Instant::now() + Duration::from_secs(30),
            exhausted: false,
        }
    }
}

impl DiscoveryBudget {
    pub(super) fn admit(&mut self, path: &Path) -> bool {
        if self.exhausted
            || self.entries_left == 0
            || path.as_os_str().len() > self.bytes_left
            || Instant::now() >= self.deadline
        {
            self.exhausted = true;
            return false;
        }
        self.entries_left -= 1;
        self.bytes_left -= path.as_os_str().len();
        true
    }
}
