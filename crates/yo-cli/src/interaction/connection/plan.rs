use crate::interaction::TextStyle;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlanAction {
    Add,
    Change,
    Remove,
    Keep,
    Attention,
    Success,
}

impl PlanAction {
    pub(crate) const fn marker(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Change => "~",
            Self::Remove => "−",
            Self::Keep => "=",
            Self::Attention => "!",
            Self::Success => "✓",
        }
    }

    pub(crate) const fn text_style(self) -> TextStyle {
        match self {
            Self::Add | Self::Success => TextStyle::Positive,
            Self::Change | Self::Attention => TextStyle::Warning,
            Self::Remove => TextStyle::Danger,
            Self::Keep => TextStyle::Muted,
        }
    }
}

#[derive(Default)]
pub(crate) struct PlanCounts {
    add: usize,
    change: usize,
    remove: usize,
    keep: usize,
}

impl PlanCounts {
    pub(crate) fn record(&mut self, action: PlanAction) {
        match action {
            PlanAction::Add => self.add += 1,
            PlanAction::Change => self.change += 1,
            PlanAction::Remove => self.remove += 1,
            PlanAction::Keep => self.keep += 1,
            PlanAction::Attention | PlanAction::Success => {},
        }
    }

    pub(crate) fn sentence(&self) -> String {
        let mut parts = Vec::new();
        for (count, verb) in [
            (self.add, "add"),
            (self.remove, "remove"),
            (self.change, "change"),
            (self.keep, "keep"),
        ] {
            if count > 0 {
                parts.push(format!("{count} to {verb}"));
            }
        }
        format!("Plan: {}.", parts.join(", "))
    }
}
