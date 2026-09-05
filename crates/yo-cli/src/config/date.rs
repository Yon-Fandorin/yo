use jiff::{Timestamp, fmt::strtime, tz::TimeZone};

use super::ConfigError;

pub(super) const DEFAULT_DATE_FORMAT: &str = "%Y-%m-%d %H:%M %:z";
const MAX_DATE_FORMAT_BYTES: usize = 128;

#[derive(Clone, Debug)]
pub(crate) struct DateFormatter {
    format: String,
    timezone: TimeZone,
}

impl DateFormatter {
    pub(super) fn new(format: &str) -> Result<Self, ConfigError> {
        if format.is_empty() || format.len() > MAX_DATE_FORMAT_BYTES {
            return Err(ConfigError::InvalidDateFormat(
                "session.list.date_format must contain 1 to 128 bytes".to_owned(),
            ));
        }
        if format.chars().any(char::is_control) {
            return Err(ConfigError::InvalidDateFormat(
                "session.list.date_format must not contain control characters".to_owned(),
            ));
        }
        let timezone = TimeZone::system();
        let probe = Timestamp::UNIX_EPOCH.to_zoned(timezone.clone());
        strtime::format(format, &probe).map_err(|error| {
            ConfigError::InvalidDateFormat(format!(
                "session.list.date_format `{format}` is invalid: {error}"
            ))
        })?;
        Ok(Self {
            format: format.to_owned(),
            timezone,
        })
    }

    pub(crate) fn format_unix_millis(&self, millis: u64) -> Result<String, ConfigError> {
        let millis = i64::try_from(millis).map_err(|_| ConfigError::TimestampOutOfRange(millis))?;
        let timestamp = Timestamp::from_millisecond(millis)
            .map_err(|_| ConfigError::TimestampOutOfRange(millis.cast_unsigned()))?;
        strtime::format(&self.format, &timestamp.to_zoned(self.timezone.clone()))
            .map_err(|error| ConfigError::InvalidDateFormat(error.to_string()))
    }
}
