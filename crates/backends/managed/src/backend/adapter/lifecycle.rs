//! 중지 핸들, 기능 투영, 종료 처리.

use std::sync::{Arc, atomic::Ordering};

use yo_core::{
    BackendCapabilities, BackendFailure, BackendStopHandle, ImageInputCapability,
    InputImageSnapshot,
};

use super::super::{
    IdleCompactionState, NativeModelBackend, map_connector_cleanup, map_tool_cleanup,
};

pub(super) fn stop_handle(backend: &NativeModelBackend) -> BackendStopHandle {
    let shared = Arc::clone(&backend.shared_stop);
    BackendStopHandle::new(move || {
        shared.requested.store(true, Ordering::Release);
        if let Ok(guard) = shared.response.lock()
            && let Some(cancellation) = guard.as_ref()
        {
            cancellation.cancel();
        }
    })
}

pub(super) fn capabilities(backend: &NativeModelBackend) -> BackendCapabilities {
    BackendCapabilities::none().with_image_input(if backend.image_accounting.is_some() {
        ImageInputCapability::Supported {
            maximum_occurrences: 16,
            maximum_image_bytes: InputImageSnapshot::MAX_BYTES as u64,
            maximum_input_bytes: InputImageSnapshot::MAX_BYTES as u64,
        }
    } else {
        ImageInputCapability::Unsupported
    })
}

pub(super) fn shutdown(backend: &mut NativeModelBackend) -> Result<(), BackendFailure> {
    if let Some(result) = &backend.shutdown_result {
        return result.clone();
    }
    let mut result = Ok(());
    if let Some(mut state) = backend.turn.take() {
        if let Some(mut stream) = state.stream.take() {
            stream.cancel();
            if let Err(error) = stream.shutdown() {
                result = Err(map_connector_cleanup(error));
            }
        }
        if let Some(mut active) = state.active_tool.take() {
            active.execution.cancel();
            if let Err(error) = active.execution.shutdown()
                && result.is_ok()
            {
                result = Err(map_tool_cleanup(error));
            }
        }
    }
    if let Some(IdleCompactionState::Summarizing { mut stream, .. }) =
        backend.idle_compaction.take()
    {
        stream.cancel();
        if let Err(error) = stream.shutdown()
            && result.is_ok()
        {
            result = Err(map_connector_cleanup(error));
        }
    }
    if let Err(error) = backend.tool_host.shutdown()
        && result.is_ok()
    {
        result = Err(map_tool_cleanup(error));
    }
    backend.events.clear();
    backend.open_activities.clear();
    backend.closed = true;
    backend.shutdown_result = Some(result.clone());
    result
}
