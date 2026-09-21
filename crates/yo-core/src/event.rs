use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputEnvelope<T> {
    schema: String,
    output: T,
}

mod activity;
mod interaction;
mod outcome;
mod plan;
mod summary;
mod tool;

pub use activity::{ActivityKind, ActivityUpdate, AgentEvent};
pub use interaction::{
    ActivityApproval, ActivityNotice, ActivityQuestion, ApprovalChoice, NoticeLevel,
    QuestionChoice, SecretStorageOffer, SecretStorageRecommendation,
};
pub use outcome::{ActivityOutcome, Failure, TurnOutcome};
pub use plan::{ActivityPlan, PlanStep, PlanStepStatus};
pub use summary::{ActivityDocument, ActivityReasoning, ActivitySummary, SummaryKind};
pub use tool::{MessageContent, ToolOutput};
