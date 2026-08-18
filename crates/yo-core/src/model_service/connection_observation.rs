use super::{
    CompleteModelBinding, ConnectionCommit, ConnectionOperationExecutionError, CredentialRevision,
    LocalConnectionOperationRepositories, ModelLastFailure, ModelRequestFailureKind,
    ModelSelection,
};

/// One typed terminal outcome from an actual model request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelRequestOutcome {
    Succeeded,
    Failed(ModelRequestFailureKind),
}

/// Secret-free result of conditionally publishing one request observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelObservationWriteOutcome {
    Committed,
    AlreadyCommitted,
    Unchanged,
    Stale,
}

/// Owns one request's exact binding and private credential revision until its outcome is known.
#[derive(Clone, Debug)]
pub struct LocalModelRequestObservation {
    repositories: LocalConnectionOperationRepositories,
    selection: ModelSelection,
    complete_binding: CompleteModelBinding,
    credential_revision: CredentialRevision,
}

impl LocalModelRequestObservation {
    #[must_use]
    pub fn new(
        repositories: LocalConnectionOperationRepositories,
        complete_binding: CompleteModelBinding,
        credential_revision: CredentialRevision,
    ) -> Self {
        let binding = complete_binding.binding();
        Self {
            repositories,
            selection: ModelSelection::new(
                binding.provider_id().clone(),
                binding.account_id().clone(),
                binding.model_id().clone(),
            ),
            complete_binding,
            credential_revision,
        }
    }

    pub fn record(
        &self,
        outcome: ModelRequestOutcome,
    ) -> Result<ModelObservationWriteOutcome, ConnectionOperationExecutionError> {
        let now = jiff::Timestamp::now();
        let observed_at = jiff::Timestamp::from_second(now.as_second())
            .expect("the current timestamp rounded to seconds remains in range")
            .to_string();
        let last_failure = match outcome {
            ModelRequestOutcome::Succeeded => None,
            ModelRequestOutcome::Failed(kind) => Some(
                ModelLastFailure::new(kind, observed_at)
                    .expect("the generated UTC whole-second timestamp is canonical"),
            ),
        };
        self.record_last_failure(last_failure)
    }

    fn record_last_failure(
        &self,
        last_failure: Option<ModelLastFailure>,
    ) -> Result<ModelObservationWriteOutcome, ConnectionOperationExecutionError> {
        let mut session = self.repositories.acquire()?;
        session.recover_pending_operation()?;
        let credentials = session.capture_credentials()?;
        if credentials.revision() != &self.credential_revision {
            return Ok(ModelObservationWriteOutcome::Stale);
        }
        let connections = session.capture_connections()?;
        let Some(current) = connections
            .models()
            .iter()
            .find(|binding| binding.selection() == self.selection)
        else {
            return Ok(ModelObservationWriteOutcome::Stale);
        };
        if current.complete() != &self.complete_binding {
            return Ok(ModelObservationWriteOutcome::Stale);
        }
        let Some(mutation) = connections
            .prepare_model_observation(&self.selection, &self.complete_binding, last_failure)
            .map_err(ConnectionOperationExecutionError::PublicPreparation)?
        else {
            return Ok(ModelObservationWriteOutcome::Unchanged);
        };
        session
            .commit_connection_mutation(&mutation)
            .map(|outcome| match outcome {
                ConnectionCommit::Committed => ModelObservationWriteOutcome::Committed,
                ConnectionCommit::AlreadyCommitted => {
                    ModelObservationWriteOutcome::AlreadyCommitted
                },
            })
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, time::SystemTime};

    use super::*;
    use crate::{AccountId, ApiCredential, ConnectionAccount, ProviderId, StoredModelBinding};

    struct Fixture {
        root: PathBuf,
        repositories: LocalConnectionOperationRepositories,
        complete: CompleteModelBinding,
        credential_revision: CredentialRevision,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let temp_dir = fs::canonicalize(std::env::temp_dir())
                .expect("the observation fixture temp directory must resolve physically");
            let root = temp_dir.join(format!(
                "yo-model-observation-{}-{name}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&root).unwrap();
            let repositories =
                LocalConnectionOperationRepositories::in_directory(root.clone()).unwrap();
            let complete = complete("medium");
            let account = ConnectionAccount::new(
                ProviderId::new("qwencloud").unwrap(),
                AccountId::new("default").unwrap(),
                None,
                None,
            )
            .unwrap();
            let binding = StoredModelBinding::new(complete.clone(), None).unwrap();
            let mutation = repositories
                .connections()
                .capture()
                .unwrap()
                .prepare_model_upsert(account, binding)
                .unwrap()
                .unwrap();
            repositories.connections().commit(&mutation).unwrap();

            let credentials = repositories.credentials();
            let mutation = credentials
                .prepare_set(
                    complete.binding().provider_id(),
                    complete.binding().account_id(),
                )
                .unwrap();
            credentials
                .commit(
                    &mutation,
                    Some(&ApiCredential::new("fixture-secret").unwrap()),
                )
                .unwrap();
            let credential_revision = credentials.capture().unwrap().revision().clone();
            Self {
                root,
                repositories,
                complete,
                credential_revision,
            }
        }

        fn observation(&self) -> LocalModelRequestObservation {
            LocalModelRequestObservation::new(
                self.repositories.clone(),
                self.complete.clone(),
                self.credential_revision.clone(),
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn complete(effort: &str) -> CompleteModelBinding {
        CompleteModelBinding::from_durable_json(&format!(
            r#"{{"provider":"qwencloud","account":"default","model":"model-a","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{"effort":"{effort}"}},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1"}}"#
        ))
        .unwrap()
    }

    // 실제 failure는 exact binding과 credential revision이 모두 현재일 때만 기록되고,
    // 다음 성공은 그 값만 제거해 preference와 complete binding을 건드리지 않습니다.
    #[test]
    fn exact_current_failure_is_recorded_and_next_success_clears_it() {
        let fixture = Fixture::new("record-clear");
        let observation = fixture.observation();
        assert_eq!(
            observation
                .record(ModelRequestOutcome::Failed(
                    ModelRequestFailureKind::Authentication,
                ))
                .unwrap(),
            ModelObservationWriteOutcome::Committed
        );
        let failed = fixture.repositories.connections().capture().unwrap();
        let stored = failed.models()[0].last_failure().unwrap();
        assert_eq!(stored.kind(), ModelRequestFailureKind::Authentication);
        assert_eq!(
            stored
                .observed_at()
                .parse::<jiff::Timestamp>()
                .unwrap()
                .subsec_nanosecond(),
            0
        );

        assert_eq!(
            observation.record(ModelRequestOutcome::Succeeded).unwrap(),
            ModelObservationWriteOutcome::Committed
        );
        let cleared = fixture.repositories.connections().capture().unwrap();
        assert!(cleared.models()[0].last_failure().is_none());
        assert_eq!(failed.preference(), cleared.preference());
        assert_eq!(
            failed.models()[0].complete(),
            cleared.models()[0].complete()
        );
        assert_eq!(
            observation.record(ModelRequestOutcome::Succeeded).unwrap(),
            ModelObservationWriteOutcome::Unchanged
        );
    }

    // 요청 뒤 credential rotation이 일어나면 이전 key로 얻은 결과는 새 key의 모델 상태를
    // annotate하거나 성공으로 지우지 않고 stale outcome으로 폐기합니다.
    #[test]
    fn credential_rotation_discards_the_stale_request_outcome() {
        let fixture = Fixture::new("credential-stale");
        let observation = fixture.observation();
        let credentials = fixture.repositories.credentials();
        let mutation = credentials
            .prepare_set(
                fixture.complete.binding().provider_id(),
                fixture.complete.binding().account_id(),
            )
            .unwrap();
        credentials
            .commit(
                &mutation,
                Some(&ApiCredential::new("rotated-secret").unwrap()),
            )
            .unwrap();

        assert_eq!(
            observation
                .record(ModelRequestOutcome::Failed(
                    ModelRequestFailureKind::AccessDenied,
                ))
                .unwrap(),
            ModelObservationWriteOutcome::Stale
        );
        assert!(
            fixture
                .repositories
                .connections()
                .capture()
                .unwrap()
                .models()[0]
                .last_failure()
                .is_none()
        );
    }

    // 같은 좌표라도 complete binding이 교체되면 이전 endpoint/profile 요청 결과는 새 epoch에
    // 붙지 않으며, operation lock contention도 요청 결과와 분리된 persistence 오류로 남습니다.
    #[test]
    fn binding_replacement_is_stale_and_operation_contention_is_separate() {
        let fixture = Fixture::new("binding-stale");
        let observation = fixture.observation();
        let replacement = StoredModelBinding::new(complete("high"), None).unwrap();
        let account = ConnectionAccount::new(
            ProviderId::new("qwencloud").unwrap(),
            AccountId::new("default").unwrap(),
            None,
            None,
        )
        .unwrap();
        let mutation = fixture
            .repositories
            .connections()
            .capture()
            .unwrap()
            .prepare_model_upsert(account, replacement)
            .unwrap()
            .unwrap();
        fixture
            .repositories
            .connections()
            .commit(&mutation)
            .unwrap();
        assert_eq!(
            observation
                .record(ModelRequestOutcome::Failed(
                    ModelRequestFailureKind::Protocol
                ))
                .unwrap(),
            ModelObservationWriteOutcome::Stale
        );

        let current_credentials = fixture.repositories.credentials().capture().unwrap();
        let current_complete = fixture
            .repositories
            .connections()
            .capture()
            .unwrap()
            .models()[0]
            .complete()
            .clone();
        let current = LocalModelRequestObservation::new(
            fixture.repositories.clone(),
            current_complete,
            current_credentials.revision().clone(),
        );
        let _held = fixture.repositories.acquire().unwrap();
        let error = current
            .record(ModelRequestOutcome::Failed(
                ModelRequestFailureKind::Timeout,
            ))
            .unwrap_err();
        assert!(matches!(
            error,
            ConnectionOperationExecutionError::OperationLock(_)
        ));
        assert!(!error.to_string().contains("rotated-secret"));
        assert!(!error.to_string().contains("fixture-secret"));
    }
}
