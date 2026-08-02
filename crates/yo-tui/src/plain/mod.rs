//! Terminal-aware plain-text presentation without terminal ownership.

mod list;

pub use list::{
    Column, ColumnBehavior, ContinuationLayout, HeadingStyle, ListError, ListSpec, OutputWidth,
    render_list,
};
