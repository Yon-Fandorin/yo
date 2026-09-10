use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ActivityRef, RequestId, SessionId, TurnRef};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentEvent {
    SessionCreated {
        session_id: SessionId,
    },
    TurnStarted {
        turn: TurnRef,
    },
    ActivityStarted {
        activity: ActivityRef,
        kind: ActivityKind,
    },
    ActivityUpdated {
        activity: ActivityRef,
        update: ActivityUpdate,
    },
    ActivityFinished {
        activity: ActivityRef,
        outcome: ActivityOutcome,
    },
    TurnFinished {
        turn: TurnRef,
        outcome: TurnOutcome,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivityKind {
    ModelWork,
    AgentMessage,
    ToolCall,
    ToolResult,
    FileChange,
    ApprovalRequest { request_id: RequestId },
    ApprovalResponse { request_id: RequestId },
    UserInputRequest { request_id: RequestId },
    UserInputResponse { request_id: RequestId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActivityUpdate {
    /// Appends one ordered fragment to the Activity's current text.
    TextDelta(String),
    /// Replaces the Activity's current text with an authoritative snapshot.
    TextSnapshot(String),
}

/// Backend-neutral tool output carried as an explicitly versioned text snapshot.
/// Original JSON fields remain available to hosts; `plain_text` is the readable fallback.
/// This adds no journal record variant and grants no execution or attachment authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolOutput {
    /// Tool identity reported by the adapter.
    pub tool: String,
    /// Optional tool server identity.
    pub server: Option<String>,
    /// Original proposed arguments, without schema-specific interpretation.
    pub arguments: Option<Value>,
    /// Original result object, including content, structured output and extension metadata.
    pub result: Option<Value>,
    /// Original dynamic-tool content array, when the adapter uses that result shape.
    pub content_items: Option<Value>,
    /// Original error details.
    pub error: Option<Value>,
    /// Adapter-produced human-readable projection, independent of rich presentation.
    pub plain_text: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputEnvelope<T> {
    schema: String,
    output: T,
}

/// Backend-neutral non-text answer block, carried without new journal record kinds.
/// Unknown fields remain available to renderers and source export; no URI is fetched.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageContent {
    /// Original content block, including provider extension metadata.
    pub block: Value,
}

impl MessageContent {
    /// Exact discriminator for this presentation-only profile.
    pub const SCHEMA: &str = "yo.message-content/v1";

    /// Encodes a bounded snapshot; excessive input uses the caller's literal fallback.
    pub fn to_snapshot(&self) -> Option<String> {
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// Admits only this exact bounded profile; all other input stays ordinary text.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA).then_some(envelope.output)
    }
}

impl ToolOutput {
    /// Exact discriminator for this presentation-only output profile.
    pub const SCHEMA: &str = "yo.tool-output/v1";
    /// Maximum encoded bytes admitted by this profile.
    pub const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

    /// Encodes a complete snapshot; invalid identity or excessive size uses the caller's fallback.
    pub fn to_snapshot(&self) -> Option<String> {
        if self.tool.is_empty() {
            return None;
        }
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= Self::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// Admits only this exact profile. Unrecognized or malformed input stays ordinary text.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > Self::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA && !envelope.output.tool.is_empty())
            .then_some(envelope.output)
    }

    /// Original content blocks in source order, retaining unknown types and fields.
    pub fn content_blocks(&self) -> impl Iterator<Item = &Value> {
        self.result
            .as_ref()
            .and_then(|result| result.get("content"))
            .or(self.content_items.as_ref())
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    }
}

/// Presentation severity; it does not change an activity or turn outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    /// Informational host observation.
    Info,
    /// Warning or retry observation requiring user visibility.
    Warning,
}

/// Explicit host notice carried in a ModelWork text snapshot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityNotice {
    /// Host-provided notice heading.
    pub title: String,
    /// Literal explanatory text, never interpreted as commands or Markdown.
    pub message: String,
    /// Semantic palette role for the heading.
    pub level: NoticeLevel,
}

impl ActivityNotice {
    /// Exact presentation profile discriminator.
    pub const SCHEMA: &str = "yo.activity-notice/v1";

    /// Encodes a bounded notice without changing journal record types.
    pub fn to_snapshot(&self) -> Option<String> {
        if self.title.is_empty() {
            return None;
        }
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// Accepts only this exact bounded profile and a nonempty heading.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA && !envelope.output.title.is_empty())
            .then_some(envelope.output)
    }
}

/// Selectable choice belonging to one active user-input request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionChoice {
    /// Provider option label, preserved verbatim.
    pub label: String,
    /// Explanatory text shown with the choice.
    pub description: String,
}

/// Optional structured presentation for a user-input request; request IDs retain authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityQuestion {
    /// Complete readable prompt, including any question progress and input hints.
    pub plain_text: String,
    /// Ordered choices; selecting one submits its one-based ordinal.
    pub choices: Vec<QuestionChoice>,
    /// Host supports typed choice-plus-notes responses; absent profiles keep ordinal/text input.
    #[serde(default)]
    pub allow_notes: bool,
    /// Host can revisit the preceding question without submitting an answer.
    #[serde(default)]
    pub previous_question: bool,
    /// Previously retained plain-text draft; absence never replaces frontend input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<String>,
    /// One-based choice associated with the draft; requires notes support.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_choice: Option<u32>,
}

impl ActivityQuestion {
    fn valid(&self) -> bool {
        !self.plain_text.is_empty()
            && self.choices.len() <= 64
            && self.draft_choice.is_none_or(|choice| {
                self.allow_notes
                    && self.draft.is_some()
                    && choice > 0
                    && choice as usize <= self.choices.len()
            })
    }

    /// Exact presentation profile discriminator.
    pub const SCHEMA: &str = "yo.activity-question/v1";

    /// Encodes a bounded question; larger option sets retain their plain-text path.
    pub fn to_snapshot(&self) -> Option<String> {
        if !self.valid() {
            return None;
        }
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// Accepts this exact bounded profile without granting response authority.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA && envelope.output.valid()).then_some(envelope.output)
    }
}

/// One approval decision offered by the host, retaining its original ordinal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalChoice {
    /// Visible action, including its approval scope.
    pub label: String,
    /// Literal scope explanation; never executable input.
    pub description: String,
    /// False for decisions that the adapter cannot safely interpret.
    pub enabled: bool,
}

/// Presentation of choices bound to an approval request; the backend owns decision authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityApproval {
    /// Observed file-change activity ID in the request's own Turn; presentation only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_change: Option<u64>,
    /// Complete readable request and reported permissions.
    pub plain_text: String,
    /// Original offered order; responses use one-based ordinals.
    pub choices: Vec<ApprovalChoice>,
    /// Non-granting decline/cancel choice preferred for selection and Escape.
    pub decline_choice: Option<u32>,
}

impl ActivityApproval {
    /// Exact approval presentation discriminator.
    pub const SCHEMA: &str = "yo.activity-approval/v1";

    fn valid(&self) -> bool {
        !self.plain_text.is_empty()
            && self.related_change.is_none_or(|id| id != 0)
            && self.choices.len() <= 64
            && self.choices.iter().all(|choice| !choice.label.is_empty())
            && self.decline_choice.is_none_or(|choice| {
                choice
                    .checked_sub(1)
                    .and_then(|index| self.choices.get(index as usize))
                    .is_some_and(|choice| choice.enabled)
            })
    }

    /// Encodes a bounded profile; never falls back to granting a generic approval.
    pub fn to_snapshot(&self) -> Option<String> {
        if !self.valid() {
            return None;
        }
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// Accepts an exact valid profile without authorizing a decision.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA && envelope.output.valid()).then_some(envelope.output)
    }
}

/// Origin of an explicitly supplied conversation summary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryKind {
    /// Context compacted by the host.
    Compaction,
    /// A branch summarized by the host.
    Branch,
    /// Public reasoning summary explicitly exposed by the host.
    Reasoning,
}

/// Host-supplied summary; presentation does not execute compaction or create a branch.
/// Carried in a ModelWork text snapshot using the existing journal record shape.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivitySummary {
    /// Semantic heading selected by the host's operation.
    pub kind: SummaryKind,
    /// Complete Markdown summary, retained even when the view is collapsed.
    pub summary: String,
    /// Observed token count before compaction, when supplied by the host.
    pub tokens_before: Option<u64>,
}

impl ActivitySummary {
    /// Exact presentation profile discriminator.
    pub const SCHEMA: &str = "yo.activity-summary/v1";

    /// Encodes the bounded profile without changing journal record variants.
    pub fn to_snapshot(&self) -> Option<String> {
        if self.kind != SummaryKind::Compaction && self.tokens_before.is_some() {
            return None;
        }
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// Accepts the exact bounded profile; unknown fields or kinds remain literal text.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA
            && (envelope.output.kind == SummaryKind::Compaction
                || envelope.output.tokens_before.is_none()))
        .then_some(envelope.output)
    }
}

/// Reasoning explicitly delivered by a provider, distinct from a public summary.
/// Presentation visibility never removes the retained source.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityReasoning {
    /// Text stream snapshot or original non-text content block.
    pub content: Value,
}

impl ActivityReasoning {
    /// Bounded ModelWork profile using existing journal records.
    pub const SCHEMA: &str = "yo.activity-reasoning/v1";

    /// Encodes supplied content without reclassifying it as a public summary.
    pub fn to_snapshot(&self) -> Option<String> {
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// Accepts only the exact bounded profile and known fields.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA).then_some(envelope.output)
    }
}

/// Explicit host-authored Markdown document, such as a proposed plan.
/// Presentation grants no execution, approval, or attachment authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityDocument {
    /// Literal heading, independent of Markdown body formatting.
    pub title: String,
    /// Complete Markdown source, retained across streaming replacement and resize.
    pub markdown: String,
}

impl ActivityDocument {
    /// Exact bounded ModelWork text profile, using existing journal record shapes.
    pub const SCHEMA: &str = "yo.activity-document/v1";

    /// Encodes a bounded document with a nonempty heading.
    pub fn to_snapshot(&self) -> Option<String> {
        if self.title.is_empty() {
            return None;
        }
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// Admits only this exact bounded profile and known fields.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA && !envelope.output.title.is_empty())
            .then_some(envelope.output)
    }
}

/// Observed status of one host-owned plan step.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    /// Not yet started.
    Pending,
    /// Currently being worked on.
    InProgress,
    /// Reported complete by the host.
    Completed,
}

/// One literal plan step; text is never interpreted as commands or Markdown.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanStep {
    /// Host-provided step text.
    pub text: String,
    /// Host-reported progress.
    pub status: PlanStepStatus,
}

/// Mutable host plan carried in an existing ModelWork text snapshot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityPlan {
    /// Optional literal explanation of the plan update.
    pub explanation: Option<String>,
    /// Steps in authoritative source order; an empty plan remains explicit.
    pub steps: Vec<PlanStep>,
}

impl ActivityPlan {
    /// Exact presentation profile discriminator; no journal record variants change.
    pub const SCHEMA: &str = "yo.activity-plan/v1";

    /// Encodes a bounded snapshot without inferring status from activity completion.
    pub fn to_snapshot(&self) -> Option<String> {
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// Admits only the exact bounded profile, known statuses and known fields.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA).then_some(envelope.output)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActivityOutcome {
    Completed,
    Interrupted,
    Failed(Failure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TurnOutcome {
    Completed,
    Interrupted,
    Failed(Failure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Failure {
    code: Option<String>,
    message: String,
}

impl Failure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Result<Self, &'static str> {
        let code = code.into();
        if code.is_empty()
            || code.len() > 128
            || !code.is_ascii()
            || code
                .chars()
                .any(|character| character.is_ascii_whitespace() || character.is_ascii_control())
        {
            return Err("failure code must be a non-empty bounded ASCII identifier");
        }
        self.code = Some(code);
        Ok(self)
    }

    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}
