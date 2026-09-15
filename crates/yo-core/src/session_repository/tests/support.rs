#[cfg(test)]
use std::env;
#[cfg(test)]
use std::path::Path;
#[cfg(test)]
use std::process;
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use super::super::{DurableRecord, RecordDiscovery};
use crate::SessionId;

pub(super) struct TestDirectory(PathBuf);

impl TestDirectory {
    pub(super) fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the system clock should be after the Unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "yo-session-repository-{}-{name}-{nonce}",
            process::id()
        ));
        fs::create_dir_all(&path).expect("the test directory should be created");
        Self(path)
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn session(value: u64) -> SessionId {
    crate::fixture_session(value)
}

pub(super) fn log_path(root: &Path, session_id: SessionId) -> PathBuf {
    root.join(format!("{session_id}.jsonl"))
}

pub(super) fn discovered(session_id: SessionId, record: DurableRecord) -> DurableRecord {
    record.with_discovery(RecordDiscovery::new(crate::fixture_descriptor(session_id)))
}
