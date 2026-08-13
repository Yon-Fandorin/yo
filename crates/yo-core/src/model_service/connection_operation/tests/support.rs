use std::{fs, path::PathBuf, time::SystemTime};

use crate::model_service::{
    AccountId, ApiCredential, ConnectionOperationJournalEntry, LocalConnectionOperationGuard,
    LocalConnectionOperationJournal, LocalConnectionRepository, LocalCredentialRepository,
    ProviderId, StartupTarget,
};

pub(super) const CANDIDATE_SECRET: &str = "candidate-secret-must-not-enter-operation-journal";

pub(super) struct Fixture {
    root: PathBuf,
    pub(super) connections: LocalConnectionRepository,
    pub(super) credentials: LocalCredentialRepository,
    pub(super) journal: LocalConnectionOperationJournal,
}

impl Fixture {
    pub(super) fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("the fixture clock must be after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "yo-connection-operation-{}-{name}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("the fixture root must be creatable");
        Self {
            connections: LocalConnectionRepository::new(root.join("state/connections.yaml")),
            credentials: LocalCredentialRepository::new(root.join("state/credentials.yaml")),
            journal: LocalConnectionOperationJournal::new(
                root.join("state/connection-operation.yaml"),
            ),
            root,
        }
    }

    pub(super) fn connect_entry(&self) -> ConnectionOperationJournalEntry {
        let connection = self
            .connections
            .capture()
            .expect("the connection snapshot must be capturable")
            .prepare_preference(Some(StartupTarget::HostCodex))
            .expect("the public mutation must be preparable")
            .expect("the unset preference must produce a mutation");
        let credential = self
            .credentials
            .prepare_set(&provider(), &account())
            .expect("the credential mutation must be preparable");
        ConnectionOperationJournalEntry::connect_credential_change(
            digest('a'),
            vec![digest('b')],
            connection,
            credential,
        )
        .expect("the connect intent must be valid")
    }

    pub(super) fn operation_guard(&self) -> LocalConnectionOperationGuard {
        self.connections
            .acquire_operation()
            .expect("the fixture operation lock must be available")
    }

    pub(super) fn seed_disconnect_remove(&self) -> ConnectionOperationJournalEntry {
        let credential = self
            .credentials
            .prepare_set(&provider(), &account())
            .expect("the initial credential must be preparable");
        self.credentials
            .commit(
                &credential,
                Some(
                    &ApiCredential::new(CANDIDATE_SECRET)
                        .expect("the fixture credential must be valid"),
                ),
            )
            .expect("the initial credential must commit");
        let set = self
            .connections
            .capture()
            .expect("the initial public snapshot must be capturable")
            .prepare_preference(Some(StartupTarget::HostCodex))
            .expect("the initial public mutation must be preparable")
            .expect("the unset preference must produce a mutation");
        self.connections
            .commit(&set)
            .expect("the initial public mutation must commit");

        let connection = self
            .connections
            .capture()
            .expect("the disconnect public snapshot must be capturable")
            .prepare_preference(None)
            .expect("the disconnect public mutation must be preparable")
            .expect("the set preference must produce a removal mutation");
        let credential = self
            .credentials
            .prepare_remove(&provider(), &account())
            .expect("the credential removal must be preparable")
            .expect("the existing credential must produce a removal");
        ConnectionOperationJournalEntry::disconnect_remove(
            digest('c'),
            vec![digest('d')],
            connection,
            credential,
        )
        .expect("the disconnect intent must be valid")
    }

    pub(super) fn seed_disconnect_preserve(&self) -> ConnectionOperationJournalEntry {
        let credential = self
            .credentials
            .prepare_set(&provider(), &account())
            .expect("the preserved credential must be preparable");
        self.credentials
            .commit(
                &credential,
                Some(
                    &ApiCredential::new(CANDIDATE_SECRET)
                        .expect("the fixture credential must be valid"),
                ),
            )
            .expect("the preserved credential must commit");
        let set = self
            .connections
            .capture()
            .expect("the initial public snapshot must be capturable")
            .prepare_preference(Some(StartupTarget::HostCodex))
            .expect("the initial public mutation must be preparable")
            .expect("the unset preference must produce a mutation");
        self.connections
            .commit(&set)
            .expect("the initial public mutation must commit");
        let connection = self
            .connections
            .capture()
            .expect("the disconnect public snapshot must be capturable")
            .prepare_preference(None)
            .expect("the disconnect public mutation must be preparable")
            .expect("the set preference must produce a removal mutation");
        let expected_credential_revision = self
            .credentials
            .capture()
            .expect("the credential snapshot must be capturable")
            .revision()
            .clone();
        ConnectionOperationJournalEntry::disconnect_preserve(
            digest('e'),
            vec![],
            connection,
            expected_credential_revision,
        )
        .expect("the preserve intent must be valid")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) fn provider() -> ProviderId {
    ProviderId::new("qwencloud").expect("the fixture ProviderId must be valid")
}

pub(super) fn account() -> AccountId {
    AccountId::new("default").expect("the fixture AccountId must be valid")
}

pub(super) fn candidate() -> ApiCredential {
    ApiCredential::new(CANDIDATE_SECRET).expect("the fixture credential must be valid")
}

pub(super) fn digest(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}
