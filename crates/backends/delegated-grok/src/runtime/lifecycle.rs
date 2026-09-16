use serde_json::{Value, json};
use yo_core::{
    BackendBindingEvidence, BackendFailure, BackendFailureKind, BackendIdentity,
    BackendResumeTarget, ContinuationStrategy, SessionId,
};

use super::state::Backend;
use crate::{
    BACKEND_KIND, READ_ONLY_BINDING_SCHEMA, READ_ONLY_REVIEW_PROFILE, STANDARD_BINDING_SCHEMA,
    client::initialize_and_authenticate, protocol, transport::JsonPeer,
};

impl<P: JsonPeer> Backend<P> {
    pub(super) fn initialize(&mut self) -> Result<(), BackendFailure> {
        if self.initialized {
            return Ok(());
        }
        let authenticated = initialize_and_authenticate(&mut self.client)?;
        let initialized = authenticated.initialized;
        self.backend_version = Some(format!(
            "{}/{}",
            initialized.agent_name, initialized.agent_version
        ));
        self.load_session = initialized.load_session;
        self.initialized = true;
        Ok(())
    }

    pub(super) fn verify(&mut self) -> Result<(), BackendFailure> {
        self.initialize()
    }

    pub(super) fn shutdown(&mut self) -> Result<(), BackendFailure> {
        self.client.shutdown()
    }

    pub(super) fn create_session(
        &mut self,
        session_id: SessionId,
    ) -> Result<yo_core::BackendCommandEvidence, BackendFailure> {
        self.initialize()?;
        let result = self
            .client
            .call("session/new", json!({ "cwd": self.cwd, "mcpServers": [] }))?
            .result;
        let grok_session = protocol::string_at(&result, &["sessionId"])?;
        validate_session_id(grok_session)?;
        let evidence = self.binding_evidence(grok_session)?;
        self.session = Some(super::state::SessionBinding {
            yo: session_id,
            grok: grok_session.to_owned(),
        });
        Ok(yo_core::BackendCommandEvidence::BindingOpened(evidence))
    }

    pub(super) fn resume_session(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.resume_binding(target.session_id(), target.binding())
    }

    fn resume_binding(
        &mut self,
        session_id: SessionId,
        binding: &BackendBindingEvidence,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        if binding.backend_kind() != BACKEND_KIND {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                format!(
                    "Grok cannot resume backend kind `{}`",
                    binding.backend_kind()
                ),
            ));
        }
        self.validate_execution_binding(binding)?;
        let locator = binding.session_locator();
        if locator.schema() != "grok.acp/session-locator/v1" {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                format!("unsupported Grok Session locator `{}`", locator.schema()),
            ));
        }
        validate_session_id(locator.value())?;
        self.initialize()?;
        if !self.load_session {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "this Grok ACP agent does not advertise session/load",
            ));
        }
        self.client.call(
            "session/load",
            json!({
                "sessionId": locator.value(),
                "cwd": self.cwd,
                "mcpServers": [],
            }),
        )?;
        self.client.discard_session_updates(locator.value());
        let evidence = self.binding_evidence(locator.value())?;
        if !binding.same_resume_identity(&evidence) {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Grok loaded a Session whose durable identity differs from its Continuation Anchor",
            ));
        }
        self.session = Some(super::state::SessionBinding {
            yo: session_id,
            grok: locator.value().to_owned(),
        });
        Ok(evidence)
    }

    pub(super) fn binding_evidence(
        &self,
        grok_session: &str,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        let backend_version = self.backend_version.clone().ok_or_else(|| {
            protocol::protocol_failure("Grok backend version was not retained after initialize")
        })?;
        Ok(BackendBindingEvidence::new(
            BACKEND_KIND,
            backend_version,
            self.binding_identity(grok_session),
            BackendIdentity::new("grok.build/model-selection/v1", "backend-managed"),
            BackendIdentity::new("grok.acp/session-locator/v1", grok_session),
            ContinuationStrategy::BackendManagedState,
        ))
    }

    fn binding_identity(&self, grok_session: &str) -> BackendIdentity {
        if self.read_only_review {
            BackendIdentity::new(
                READ_ONLY_BINDING_SCHEMA,
                json!({
                    "executionProfile": READ_ONLY_REVIEW_PROFILE,
                    "sessionId": grok_session,
                })
                .to_string(),
            )
        } else {
            BackendIdentity::new(
                STANDARD_BINDING_SCHEMA,
                json!({ "sessionId": grok_session }).to_string(),
            )
        }
    }

    pub(super) fn validate_execution_binding(
        &self,
        binding: &BackendBindingEvidence,
    ) -> Result<(), BackendFailure> {
        let identity = binding.binding_identity();
        let expected_schema = if self.read_only_review {
            READ_ONLY_BINDING_SCHEMA
        } else {
            STANDARD_BINDING_SCHEMA
        };
        if identity.schema() != expected_schema {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Grok durable execution profile differs from the requested resume profile",
            ));
        }
        if self.read_only_review {
            let value: Value = serde_json::from_str(identity.value()).map_err(|_| {
                protocol::protocol_failure("Grok read-only review binding is malformed")
            })?;
            if value.get("executionProfile").and_then(Value::as_str)
                != Some(READ_ONLY_REVIEW_PROFILE)
            {
                return Err(protocol::protocol_failure(
                    "Grok read-only review binding has a different execution profile",
                ));
            }
        }
        Ok(())
    }
}

fn validate_session_id(session_id: &str) -> Result<(), BackendFailure> {
    if session_id.is_empty() || session_id.len() > 2048 {
        return Err(protocol::protocol_failure(
            "Grok ACP returned an invalid Session id",
        ));
    }
    Ok(())
}
