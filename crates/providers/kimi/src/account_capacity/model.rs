use yo_core::AccountCapacityWindow;

pub(super) const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
pub(super) const MAX_LIMIT_ROWS: usize = 32;
pub(super) const MAX_NAME_BYTES: usize = 256;
pub(super) const MINUTES_PER_WEEK: u64 = 7 * 24 * 60;
pub(super) const FIXED_POINT_PER_CENT: u64 = 1_000_000;

pub(super) struct DecodedUsageRow {
    name: Option<String>,
    window: AccountCapacityWindow,
    exhausted: bool,
}

impl DecodedUsageRow {
    pub(super) fn new(
        name: Option<String>,
        window: AccountCapacityWindow,
        exhausted: bool,
    ) -> Self {
        Self {
            name,
            window,
            exhausted,
        }
    }

    pub(super) const fn window(&self) -> AccountCapacityWindow {
        self.window
    }

    pub(super) fn into_parts(self) -> (Option<String>, AccountCapacityWindow, bool) {
        (self.name, self.window, self.exhausted)
    }

    pub(super) const fn exhausted(&self) -> bool {
        self.exhausted
    }
}
