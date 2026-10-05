//! SessionJournal semantic correlation facade.

mod context;
mod records;

#[cfg(test)]
mod tests;

pub(crate) use context::ContextActiveSource;
// Writer와 recovery는 같은 durable ordinary 질문 결과 검증을 사용합니다.
pub(super) use context::question_results_match;
