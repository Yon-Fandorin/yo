//! Typed activity presentation retained independently of literal log content.

use std::{fmt, num::NonZeroU16, sync::Arc};

use yo_core::{ActivityDocument, ActivityKind, ToolOutput};

use super::{
    TranscriptBody, TranscriptItem, TranscriptItemId, TranscriptMessage, TranscriptPhase,
    TranscriptState, TranscriptStateError,
};
use crate::surface::Hyperlink;

/// Host resolution of explicit Markdown link destinations into validated terminal links.
/// Return None to retain the default web-only behavior. A host-authorized file destination
/// must be constructed explicitly with Hyperlink::from_file_path; raw file URLs are never
/// automatically trusted.
/// Callbacks must be deterministic, bounded and free of I/O. Resolve files and ownership
/// before constructing the handle; replace the handle when that mapping changes to invalidate
/// layout caches. Fenced code, standalone code spans, literal logs, images, source exports
/// and disabled links do not call it.
#[derive(Clone)]
pub struct LinkResolver(Arc<ResolveLink>);

type ResolveLink = dyn Fn(&str) -> Option<Hyperlink> + Send + Sync;

impl LinkResolver {
    /// Wraps a resolver for control-free explicit destinations no longer than 8 KiB.
    pub fn new(resolver: impl Fn(&str) -> Option<Hyperlink> + Send + Sync + 'static) -> Self {
        Self(Arc::new(resolver))
    }

    pub(super) fn resolve(&self, destination: &str) -> Option<Hyperlink> {
        if destination.len() > 8 * 1024 || destination.chars().any(char::is_control) {
            return None;
        }
        (self.0)(destination)
    }
}

impl fmt::Debug for LinkResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LinkResolver(..)")
    }
}

impl PartialEq for LinkResolver {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for LinkResolver {}

/// Read-only assistant answer supplied to a session-local presentation callback.
#[derive(Clone, Copy, Debug)]
pub struct AssistantRenderInput<'a> {
    /// Original answer body, excluding TUI-owned failure and interruption text.
    pub source: &'a str,
    /// Available body columns after output preferences.
    pub columns: NonZeroU16,
    /// Whether this transcript item has been finalized; this does not imply success.
    pub finalized: bool,
}

/// Assistant-only Markdown presentation. User text, tools, approvals and documents are excluded.
/// Return None for the original body. Source export and terminal outcome text stay unchanged.
/// Callbacks must be deterministic, bounded and free of I/O; replace this handle to invalidate
/// cached layouts. Oversized or unrenderable results fall back to the original answer.
#[derive(Clone)]
pub struct AssistantRenderer(Arc<RenderAssistant>);

type RenderAssistant = dyn for<'a> Fn(AssistantRenderInput<'a>) -> Option<String> + Send + Sync;

impl AssistantRenderer {
    /// Maximum replacement Markdown bytes; the first excess byte falls back to the original.
    pub const MAX_RENDERED_BYTES: usize = 256 * 1024;

    /// Wraps a host-owned assistant presentation callback.
    pub fn new(
        renderer: impl for<'a> Fn(AssistantRenderInput<'a>) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(renderer))
    }

    pub(super) fn render(&self, input: AssistantRenderInput<'_>) -> Option<String> {
        (self.0)(input).filter(|text| text.len() <= Self::MAX_RENDERED_BYTES)
    }
}

impl fmt::Debug for AssistantRenderer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AssistantRenderer(..)")
    }
}

impl PartialEq for AssistantRenderer {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for AssistantRenderer {}

/// Read-only tool body supplied to a session's presentation callback.
/// Versioned output profiles additionally expose original tool identity and JSON data.
#[derive(Clone, Copy, Debug)]
pub struct ToolRenderInput<'a> {
    /// ToolCall or ToolResult; approvals, file diffs and user messages never enter this callback.
    pub kind: ActivityKind,
    /// Observed terminal activity outcome; None means no terminal outcome was observed.
    /// Payload text and structured error fields do not determine this value.
    pub outcome: Option<TranscriptActivityOutcome>,
    /// Readable body without the TUI-owned heading/footer; structured profiles use plain_text.
    pub source: &'a str,
    /// Validated structured output, when the adapter supplied the explicit profile.
    pub output: Option<&'a ToolOutput>,
    /// Session requests expanded activity bodies (for example, after Ctrl+O).
    /// False permits a compact summary; ordinary host folding still applies to the result.
    pub expanded: bool,
    /// Available body columns after the session's width preference is applied.
    pub columns: NonZeroU16,
}

/// Session-local tool-body presentation extension.
///
/// Return Markdown to use the built-in code/table/chart/image renderers, or `None`
/// to retain the default projection (literal text or structured output).
/// The callback must be deterministic, bounded and free
/// of I/O: measurement and rendering may call it repeatedly. Replace this handle
/// to change behavior; mutating captured state does not invalidate layout caches.
/// Retained records, plain output, status headings and failure footers are unchanged.
#[derive(Clone)]
pub struct ToolRenderer(Arc<RenderTool>);

type RenderTool = dyn for<'a> Fn(ToolRenderInput<'a>) -> Option<String> + Send + Sync;

impl ToolRenderer {
    /// Wraps a host renderer. Unrenderable Markdown falls back to literal output.
    pub fn new(
        renderer: impl for<'a> Fn(ToolRenderInput<'a>) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(renderer))
    }

    pub(super) fn render(&self, input: ToolRenderInput<'_>) -> Option<String> {
        (self.0)(input)
    }
}

impl fmt::Debug for ToolRenderer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ToolRenderer(..)")
    }
}

impl PartialEq for ToolRenderer {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ToolRenderer {}

/// Read-only explicit document supplied to the session's presentation callback.
#[derive(Clone, Copy, Debug)]
pub struct DocumentRenderInput<'a> {
    /// Original document or lossless document projection of provider reasoning.
    pub document: &'a ActivityDocument,
    /// Observed terminal outcome, independent of the document body.
    pub outcome: Option<TranscriptActivityOutcome>,
    /// Whether the session requests expanded activity bodies.
    pub expanded: bool,
    /// Available body columns after width preferences.
    pub columns: NonZeroU16,
}

/// Session-local presentation of explicit documents and provider reasoning bodies.
/// Return Markdown or None for the original document. Titles, status footers and source
/// export remain owned by the TUI. Summaries, tools and approval requests are excluded.
/// Callbacks must be deterministic, bounded and free of I/O; replace the handle to
/// invalidate cached layouts. Document images remain placeholders, as in the default view.
#[derive(Clone)]
pub struct DocumentRenderer(Arc<RenderDocument>);

type RenderDocument = dyn for<'a> Fn(DocumentRenderInput<'a>) -> Option<String> + Send + Sync;

impl DocumentRenderer {
    /// Wraps a document renderer; oversized or unrenderable results use the original body.
    pub fn new(
        renderer: impl for<'a> Fn(DocumentRenderInput<'a>) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(renderer))
    }

    pub(super) fn render(&self, input: DocumentRenderInput<'_>) -> Option<String> {
        (self.0)(input)
    }
}

impl fmt::Debug for DocumentRenderer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DocumentRenderer(..)")
    }
}

impl PartialEq for DocumentRenderer {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for DocumentRenderer {}

#[derive(Clone, Copy)]
pub(crate) struct FileChangeView<'a> {
    pub(crate) heading: &'a str,
    pub(crate) body: &'a str,
    pub(crate) footer: Option<&'a str>,
    pub(crate) outcome: Option<TranscriptActivityOutcome>,
}

impl TranscriptItem {
    pub(crate) fn is_activity(&self) -> bool {
        let TranscriptBody::Message(message) = self.body();
        message.activity.is_some()
    }
}

impl TranscriptMessage {
    pub(crate) fn tool_source(&self) -> Option<&str> {
        let activity = self.activity?;
        if !matches!(
            activity.kind,
            Some(ActivityKind::ToolCall | ActivityKind::ToolResult)
        ) {
            return None;
        }
        let body = &self.text[..activity.footer_start.unwrap_or(self.text.len())];
        let start = body.find('\n').map_or(body.len(), |index| index + 1);
        Some(&body[start..])
    }

    pub(crate) fn file_change(&self) -> Option<FileChangeView<'_>> {
        let activity = self.activity.filter(|activity| activity.diff)?;
        let header_end = self.text.find('\n').unwrap_or(self.text.len());
        let footer_start = activity.footer_start.unwrap_or(self.text.len());
        Some(FileChangeView {
            heading: &self.text[..header_end],
            body: self.text[header_end..footer_start]
                .strip_prefix('\n')
                .unwrap_or(&self.text[header_end..footer_start]),
            footer: activity.footer_start.map(|start| &self.text[start..]),
            outcome: activity.outcome,
        })
    }
}

/// Observed terminal outcome used by activity presentation and custom tool bodies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranscriptActivityOutcome {
    /// Activity completed normally.
    Completed,
    /// Activity was interrupted.
    Interrupted,
    /// Activity failed; the TUI retains its failure footer independently.
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ActivityPresentation {
    pub(super) outcome: Option<TranscriptActivityOutcome>,
    pub(super) footer_start: Option<usize>,
    pub(super) diff: bool,
    pub(super) usage: bool,
    pub(super) kind: Option<ActivityKind>,
}

impl TranscriptState {
    pub(crate) fn start_activity_message(
        &mut self,
        id: TranscriptItemId,
    ) -> Result<(), TranscriptStateError> {
        self.start_activity(id, false, None)
    }

    pub(crate) fn start_file_change_message(
        &mut self,
        id: TranscriptItemId,
    ) -> Result<(), TranscriptStateError> {
        self.start_activity(id, true, None)
    }

    pub(crate) fn start_typed_activity_message(
        &mut self,
        id: TranscriptItemId,
        kind: ActivityKind,
    ) -> Result<(), TranscriptStateError> {
        self.start_activity(id, false, Some(kind))
    }

    fn start_activity(
        &mut self,
        id: TranscriptItemId,
        diff: bool,
        kind: Option<ActivityKind>,
    ) -> Result<(), TranscriptStateError> {
        let mut item = TranscriptItem::streaming_assistant(id);
        let TranscriptBody::Message(message) = &mut item.body;
        message.activity = Some(ActivityPresentation {
            diff,
            kind,
            ..ActivityPresentation::default()
        });
        self.push(item)
    }

    // Publish terminal status, optional outcome text, and Final phase as one revision.
    // The footer boundary comes from owned text insertion, never from matching logs.
    pub(crate) fn finish_activity_message(
        &mut self,
        id: TranscriptItemId,
        outcome: TranscriptActivityOutcome,
        footer: Option<&str>,
    ) -> Result<(), TranscriptStateError> {
        self.finish_activity_presentation(id, outcome, footer, false)
    }

    pub(crate) fn finish_usage_message(
        &mut self,
        id: TranscriptItemId,
    ) -> Result<(), TranscriptStateError> {
        self.finish_activity_presentation(id, TranscriptActivityOutcome::Completed, None, true)
    }

    fn finish_activity_presentation(
        &mut self,
        id: TranscriptItemId,
        outcome: TranscriptActivityOutcome,
        footer: Option<&str>,
        usage: bool,
    ) -> Result<(), TranscriptStateError> {
        let item = self.item_mut(id)?;
        if item.phase == TranscriptPhase::Final {
            return Err(TranscriptStateError::FinalItem(id));
        }
        let TranscriptBody::Message(message) = &mut item.body;
        let activity = message
            .activity
            .as_mut()
            .ok_or(TranscriptStateError::NotActivity(id))?;
        let revision = item
            .revision
            .checked_add(1)
            .ok_or(TranscriptStateError::RevisionOverflow(id))?;
        if let Some(footer) = footer.filter(|footer| !footer.is_empty()) {
            activity.footer_start = Some(message.text.len());
            message.text.push_str(footer);
        }
        activity.outcome = Some(outcome);
        activity.usage = usage;
        item.phase = TranscriptPhase::Final;
        item.revision = revision;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 답변 변환 결과의 정확한 byte 상한은 허용하고 첫 초과 byte는 원문 fallback을 선택한다.
    #[test]
    fn assistant_replacement_byte_limit_is_exact() {
        for length in [
            AssistantRenderer::MAX_RENDERED_BYTES,
            AssistantRenderer::MAX_RENDERED_BYTES + 1,
        ] {
            let renderer = AssistantRenderer::new(move |_| Some("x".repeat(length)));
            let input = AssistantRenderInput {
                source: "original",
                columns: NonZeroU16::new(80).unwrap(),
                finalized: true,
            };
            assert_eq!(
                renderer.render(input).is_some(),
                length == AssistantRenderer::MAX_RENDERED_BYTES
            );
        }
    }

    // 완료 상태·footer·phase는 하나의 revision으로 확정되고 이후 변경은 거부한다.
    #[test]
    fn terminal_activity_publishes_one_immutable_revision() {
        let mut state = TranscriptState::new();
        let id = TranscriptItemId::new(1);
        state.start_activity_message(id).unwrap();
        state
            .append_text(id, "Tool failed\nlog says Failed: fake")
            .unwrap();
        let before = state.items()[0].revision();
        state
            .finish_activity_message(
                id,
                TranscriptActivityOutcome::Failed,
                Some("\nFailed: actual"),
            )
            .unwrap();
        let item = &state.items()[0];
        assert_eq!(item.revision(), before + 1);
        assert_eq!(item.phase(), TranscriptPhase::Final);
        let TranscriptBody::Message(message) = item.body();
        let activity = message.activity.unwrap();
        assert_eq!(activity.outcome, Some(TranscriptActivityOutcome::Failed));
        assert_eq!(
            &message.text[activity.footer_start.unwrap()..],
            "\nFailed: actual"
        );
        assert_eq!(
            state.append_text(id, "later"),
            Err(TranscriptStateError::FinalItem(id))
        );
    }

    // revision overflow가 생기면 footer와 상태를 포함한 항목 전체를 그대로 보존한다.
    #[test]
    fn failed_activity_publication_is_atomic() {
        let mut state = TranscriptState::new();
        let id = TranscriptItemId::new(1);
        state.start_activity_message(id).unwrap();
        state.item_mut(id).unwrap().revision = u64::MAX;
        let before = state.items().to_vec();
        assert_eq!(
            state.finish_activity_message(id, TranscriptActivityOutcome::Failed, Some("\nFailure")),
            Err(TranscriptStateError::RevisionOverflow(id))
        );
        assert_eq!(state.items(), before);
    }
    // 호스트 callback은 유효한 길이의 목적지만 받으며 제어 문자·첫 초과 바이트에서는 호출하지
    // 않는다.
    #[test]
    fn link_resolver_bounds_candidates_before_host_callback() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let resolver = LinkResolver::new(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            Hyperlink::new("https://example.com/resolved")
        });
        assert!(resolver.resolve(&"x".repeat(8 * 1024)).is_some());
        for destination in [
            "x".repeat(8 * 1024 + 1),
            "bad\nlink".into(),
            "bad\u{1b}link".into(),
        ] {
            assert!(resolver.resolve(&destination).is_none());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
