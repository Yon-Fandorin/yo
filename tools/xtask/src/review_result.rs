mod bounds;
mod correction;
mod model;
mod parse;

pub(crate) use correction::correction_preflight;
#[allow(unused_imports)]
pub(crate) use model::{Finding, Verdict, VerifiedResult};
pub(crate) use parse::verify;

#[cfg(test)]
mod tests;

pub(crate) const SCHEMA: &str = "yo.slice-review-result/v1alpha1";

pub(crate) const OUTPUT_INSTRUCTION: &str = r#"Finish the response with exactly one terminal structured result after any explanation. Copy the current packet's exact ReviewId (or ReviewDeltaId), candidate commit, and every requested lens exactly once. Use verdict `clear` or `findings`; list every material finding with a unique finding_id, bounded summary, and its affected lenses. Findings must be empty exactly when every lens is clear. Write nothing after the end marker:
<<<YO-SLICE-REVIEW-RESULT>>>
{"schema":"yo.slice-review-result/v1alpha1","review_id":"<current review id>","candidate_commit":"<current candidate>","verdicts":[{"lens":"<requested lens>","verdict":"clear"}],"findings":[]}
<<<YO-SLICE-REVIEW-RESULT-END>>>"#;
