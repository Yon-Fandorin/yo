mod confirmation;
mod details;
mod error;
mod layout;
mod plan;
mod result;

#[cfg(test)]
mod tests;

pub(crate) use confirmation::ConfirmationView;
pub(crate) use details::{BindingDetails, group_profiles};
#[cfg(test)]
pub(crate) use details::{ProfileDetails, ProfileGroup};
pub(crate) use error::PresentationError;
pub(crate) use layout::{
    default_width, display_model_item, escape_remote_text, plural, push_bullet, push_change,
    push_detail_field, push_model_list_field, push_plan_summary, push_section_heading, push_title,
    trim_trailing_newline, wrap,
};
#[cfg(test)]
pub(crate) use layout::{safe_width, widest_grapheme};
pub(crate) use plan::{PlanAction, PlanCounts};
pub(crate) use result::{SuccessPresentation, render_success};
