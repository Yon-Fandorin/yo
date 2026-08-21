use yo_core::ModelConnectorEvent;

pub(super) fn dummy_event(id: &str) -> ModelConnectorEvent {
    ModelConnectorEvent::ResponseCreated {
        response_id: id.to_owned(),
    }
}
