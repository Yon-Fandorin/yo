use crate::model_connector::ResponsesEvent;

pub(super) fn dummy_event(id: &str) -> ResponsesEvent {
    ResponsesEvent::ResponseCreated {
        response_id: id.to_owned(),
    }
}
