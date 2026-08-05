use std::{f64::consts::PI, time::Duration};

use unicode_segmentation::UnicodeSegmentation;

use super::AppearanceCandidateError;
use crate::surface::{Attributes, Color, Grapheme, Style};

pub(super) const BUILT_IN_REPAINT_INTERVAL: Duration = Duration::from_millis(16);
const MINIMUM_REPAINT_INTERVAL: Duration = Duration::from_millis(16);
const BUILT_IN_SWEEP_PERIOD: Duration = Duration::from_secs(2);
const SWEEP_PADDING: f64 = 10.0;
const BAND_HALF_WIDTH: f64 = 5.0;

/// Color depth classified by the process host before appearance publication.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum ColorCapability {
    /// The host can faithfully emit 24-bit RGB foreground colors.
    TrueColor,
    /// The host supports color or attributes, but not the configured RGB ramp.
    Limited,
    /// The host has no stable evidence for a particular color depth.
    #[default]
    Unknown,
}

/// Host-selected motion behavior retained by the committed appearance.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum MotionPreference {
    /// Render the configured activity animation.
    #[default]
    Standard,
    /// Keep activity indicators static and do not schedule motion repaints.
    Reduced,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ActivityRgb {
    red: u8,
    green: u8,
    blue: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ActivityStyles {
    pub(crate) marker: Style,
    pub(crate) muted: Style,
    pub(crate) trail: Style,
    pub(crate) peak: Style,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ActivityMotionProfile {
    marker: String,
    repaint_interval: Duration,
    sweep_period: Duration,
    color_capability: ColorCapability,
    base_rgb: ActivityRgb,
    highlight_rgb: ActivityRgb,
    reduced_motion: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ActivityMotionFrame<'frame> {
    marker: &'frame str,
    repaint_interval: Duration,
    sweep_period: Duration,
    color_capability: ColorCapability,
    base_rgb: ActivityRgb,
    highlight_rgb: ActivityRgb,
    reduced_motion: bool,
    elapsed: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ActivitySheen {
    position: f64,
    color_capability: ColorCapability,
    base_rgb: ActivityRgb,
    highlight_rgb: ActivityRgb,
}

impl ActivityRgb {
    const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    fn blend(self, highlight: Self, intensity: f64) -> Color {
        let amount = 0.9 * intensity.clamp(0.0, 1.0);
        Color::Rgb {
            red: blend_channel(self.red, highlight.red, amount),
            green: blend_channel(self.green, highlight.green, amount),
            blue: blend_channel(self.blue, highlight.blue, amount),
        }
    }
}

impl ActivityStyles {
    pub(crate) const fn built_in() -> Self {
        Self {
            marker: Style::new(Color::Default, Color::Default, Attributes::empty()),
            muted: Style::new(Color::Default, Color::Default, Attributes::DIM),
            trail: Style::new(Color::Default, Color::Default, Attributes::empty()),
            peak: Style::new(Color::Default, Color::Default, Attributes::BOLD),
        }
    }
}

impl ActivityMotionProfile {
    pub(super) fn built_in(
        marker: &str,
        color_capability: ColorCapability,
        motion_preference: MotionPreference,
    ) -> Self {
        Self {
            marker: marker.to_owned(),
            repaint_interval: BUILT_IN_REPAINT_INTERVAL,
            sweep_period: BUILT_IN_SWEEP_PERIOD,
            color_capability,
            base_rgb: ActivityRgb::new(128, 128, 128),
            highlight_rgb: ActivityRgb::new(255, 255, 255),
            reduced_motion: motion_preference == MotionPreference::Reduced,
        }
    }

    pub(super) fn validate(&self) -> Result<(), AppearanceCandidateError> {
        validate_activity_marker(&self.marker)?;
        if self.repaint_interval < MINIMUM_REPAINT_INTERVAL {
            return Err(AppearanceCandidateError::ActivityRepaintIntervalTooFast {
                minimum: MINIMUM_REPAINT_INTERVAL,
                actual: self.repaint_interval,
            });
        }
        if self.sweep_period.is_zero() {
            return Err(AppearanceCandidateError::ZeroActivitySweepPeriod);
        }
        Ok(())
    }

    pub(super) fn frame_at(&self, elapsed: Duration) -> ActivityMotionFrame<'_> {
        ActivityMotionFrame {
            marker: &self.marker,
            repaint_interval: self.repaint_interval,
            sweep_period: self.sweep_period,
            color_capability: self.color_capability,
            base_rgb: self.base_rgb,
            highlight_rgb: self.highlight_rgb,
            reduced_motion: self.reduced_motion,
            elapsed,
        }
    }

    #[cfg(test)]
    pub(super) fn with_test_motion(mut self, repaint_interval: Duration, marker: &str) -> Self {
        self.repaint_interval = repaint_interval;
        self.marker = marker.to_owned();
        self
    }

    #[cfg(test)]
    pub(super) const fn with_test_sweep_period(mut self, sweep_period: Duration) -> Self {
        self.sweep_period = sweep_period;
        self
    }
}

impl<'frame> ActivityMotionFrame<'frame> {
    pub(crate) const fn still(marker: &'frame str) -> Self {
        Self {
            marker,
            repaint_interval: BUILT_IN_REPAINT_INTERVAL,
            sweep_period: BUILT_IN_SWEEP_PERIOD,
            color_capability: ColorCapability::Unknown,
            base_rgb: ActivityRgb::new(128, 128, 128),
            highlight_rgb: ActivityRgb::new(255, 255, 255),
            reduced_motion: true,
            elapsed: Duration::ZERO,
        }
    }

    pub(crate) const fn marker(self) -> &'frame str {
        self.marker
    }

    pub(crate) const fn period(self) -> Option<Duration> {
        if self.reduced_motion {
            None
        } else {
            Some(self.repaint_interval)
        }
    }

    pub(crate) fn sheen(self, visible_graphemes: usize) -> Option<ActivitySheen> {
        self.period()?;
        if visible_graphemes == 0 {
            return None;
        }
        Some(ActivitySheen {
            position: sweep_position(self.elapsed, self.sweep_period, visible_graphemes),
            color_capability: self.color_capability,
            base_rgb: self.base_rgb,
            highlight_rgb: self.highlight_rgb,
        })
    }

    pub(crate) fn marker_style(self, styles: ActivityStyles) -> Style {
        let intensity = self.sheen(1).map_or(0.0, |sheen| sheen.intensity_at(0));
        resolve_style(
            styles,
            styles.marker,
            self.color_capability,
            self.base_rgb,
            self.highlight_rgb,
            intensity,
        )
    }

    pub(crate) fn static_style(self, styles: ActivityStyles) -> Style {
        if self.reduced_motion {
            styles.marker
        } else {
            resolve_style(
                styles,
                styles.trail,
                self.color_capability,
                self.base_rgb,
                self.highlight_rgb,
                0.0,
            )
        }
    }
}

impl ActivitySheen {
    pub(crate) fn style_at(self, index: usize, styles: ActivityStyles) -> Style {
        resolve_style(
            styles,
            styles.trail,
            self.color_capability,
            self.base_rgb,
            self.highlight_rgb,
            self.intensity_at(index),
        )
    }

    fn intensity_at(self, index: usize) -> f64 {
        let distance = (index as f64 - self.position).abs();
        if distance > BAND_HALF_WIDTH {
            0.0
        } else {
            0.5 * (1.0 + (PI * distance / BAND_HALF_WIDTH).cos())
        }
    }
}

fn sweep_position(elapsed: Duration, sweep_period: Duration, visible_graphemes: usize) -> f64 {
    let period = sweep_period.as_nanos();
    let phase = (elapsed.as_nanos() % period) as f64 / period as f64;
    -SWEEP_PADDING + phase * (visible_graphemes as f64 + 2.0 * SWEEP_PADDING)
}

fn blend_channel(base: u8, highlight: u8, amount: f64) -> u8 {
    (f64::from(base) + (f64::from(highlight) - f64::from(base)) * amount)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn resolve_style(
    styles: ActivityStyles,
    base_style: Style,
    capability: ColorCapability,
    base_rgb: ActivityRgb,
    highlight_rgb: ActivityRgb,
    intensity: f64,
) -> Style {
    if capability == ColorCapability::TrueColor {
        return Style::new(
            base_rgb.blend(highlight_rgb, intensity),
            base_style.background,
            base_style.attributes,
        );
    }
    if intensity < 0.2 {
        styles.muted
    } else if intensity < 0.6 {
        styles.trail
    } else {
        styles.peak
    }
}

impl Default for ActivityStyles {
    fn default() -> Self {
        Self::built_in()
    }
}

fn validate_activity_marker(marker: &str) -> Result<(), AppearanceCandidateError> {
    if marker.is_empty() {
        return Err(AppearanceCandidateError::EmptyActivityMarker);
    }
    if marker.chars().any(char::is_control) {
        return Err(AppearanceCandidateError::ActivityMarkerContainsControl);
    }
    let mut graphemes = marker.graphemes(true);
    let text = graphemes
        .next()
        .ok_or(AppearanceCandidateError::EmptyActivityMarker)?;
    if graphemes.next().is_some() {
        return Err(AppearanceCandidateError::ActivityMarkerMustBeOneGrapheme);
    }
    let grapheme = Grapheme::try_from(text)
        .map_err(|cause| AppearanceCandidateError::InvalidActivityMarker { cause })?;
    if grapheme.width().get() > 1 {
        return Err(AppearanceCandidateError::ActivityMarkerMustBeOneCell {
            actual: grapheme.width().get(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
