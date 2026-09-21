use yo_core::{
    ActivityRequestRef, SessionId,
    interview::{InterviewCatalog, InterviewError},
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
}
