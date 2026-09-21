use yo_core::{
    ActivityRequestRef, SessionId, SubmissionId,
    interview::{InterviewCatalog, InterviewError, Submission, WorkingCopy},
    session_repository::read_stored_session,
};

use crate::state::storage;

pub(super) struct HistoryHost;
fn invalid(message: impl Into<String>) -> InterviewError {
    InterviewError::Invalid(message.into())
}
fn historical_catalog(session: SessionId) -> Result<InterviewCatalog, InterviewError> {
    let storage = storage::open_default_reader().map_err(|error| invalid(error.to_string()))?;
    let reader = storage
        .reader()
        .ok_or_else(|| invalid("original interview Journal is unavailable"))?;
    let history =
        read_stored_session(reader, session).map_err(|error| invalid(error.to_string()))?;
    if !history.discovery_consistent() {
        return Err(invalid("original interview discovery is inconsistent"));
    }
    Ok(history.interviews())
}
impl yo_tui::InterviewHistoryHost for HistoryHost {
    fn resolve(&mut self, source: ActivityRequestRef) -> Result<InterviewCatalog, InterviewError> {
        historical_catalog(source.activity().turn().session_id())
    }

    fn historical_interviews(
        &mut self,
        session: SessionId,
    ) -> Result<InterviewCatalog, InterviewError> {
        historical_catalog(session)
    }
    fn validate_submission(&mut self, copy: &WorkingCopy) -> Result<(), InterviewError> {
        let Some(Submission::NewConversation {
            turn,
            submission_id,
            accepted_request_sequence,
        }) = &copy.submission
        else {
            return Ok(());
        };
        if turn.session_id() == copy.source().0.activity().turn().session_id() {
            return Err(invalid(
                "interview new conversation must have a new Session",
            ));
        }
        let id = submission_id
            .parse::<SubmissionId>()
            .map_err(|error| invalid(error.to_string()))?;
        let storage = storage::open_default_reader().map_err(|error| invalid(error.to_string()))?;
        let reader = storage
            .reader()
            .ok_or_else(|| invalid("new conversation Journal is unavailable"))?;
        let history = read_stored_session(reader, turn.session_id())
            .map_err(|error| invalid(error.to_string()))?;
        let valid = history.discovery_consistent()
            && history
                .accepted_initial_submission(id)
                .is_some_and(|(actual, seq)| {
                    actual == *turn && seq.get() == *accepted_request_sequence
                });
        if valid {
            Ok(())
        } else {
            Err(invalid(
                "new conversation submission is unconfirmed; stored copy preserved",
            ))
        }
    }
}
