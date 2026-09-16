use std::num::NonZeroU16;

use super::error::PresentationError;
use crate::interaction::PresentationStyle;

pub(super) trait ConfirmationView {
    fn render_styled(
        &self,
        width: NonZeroU16,
        style: PresentationStyle,
    ) -> Result<String, PresentationError>;
    fn prompt(&self) -> &'static str;
}
