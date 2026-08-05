use std::{f64::consts::PI, time::Duration};

use unicode_segmentation::UnicodeSegmentation;

use super::AppearanceCandidateError;
use crate::surface::{Attributes, Color, Grapheme, Style};

pub(super) const BUILT_IN_REPAINT_INTERVAL: Duration = Duration::from_millis(16);
const MINIMUM_REPAINT_INTERVAL: Duration = Duration::from_millis(16);
const BUILT_IN_MARKER_INTERVAL: Duration = Duration::from_millis(80);
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
    marker_frames: Vec<ActivityMarkerFrame>,
    marker_interval: Duration,
    reserved_marker_width: u16,
    repaint_interval: Duration,
    sweep_period: Duration,
    color_capability: ColorCapability,
    base_rgb: ActivityRgb,
    highlight_rgb: ActivityRgb,
    reduced_motion: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActivityMarkerFrame {
    text: String,
    width: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ActivityMotionFrame<'frame> {
    marker: &'frame str,
    marker_width: u16,
    reserved_marker_width: u16,
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
        marker_frames: &[&str],
        color_capability: ColorCapability,
        motion_preference: MotionPreference,
    ) -> Self {
        let (marker_frames, reserved_marker_width) =
            resolve_marker_frames(marker_frames).expect("built-in activity frames must be valid");
        Self {
            marker_frames,
            marker_interval: BUILT_IN_MARKER_INTERVAL,
            reserved_marker_width,
            repaint_interval: BUILT_IN_REPAINT_INTERVAL,
            sweep_period: BUILT_IN_SWEEP_PERIOD,
            color_capability,
            base_rgb: ActivityRgb::new(128, 128, 128),
            highlight_rgb: ActivityRgb::new(255, 255, 255),
            reduced_motion: motion_preference == MotionPreference::Reduced,
        }
    }

    pub(super) fn validate(&self) -> Result<(), AppearanceCandidateError> {
        if self.repaint_interval < MINIMUM_REPAINT_INTERVAL {
            return Err(AppearanceCandidateError::ActivityRepaintIntervalTooFast {
                minimum: MINIMUM_REPAINT_INTERVAL,
                actual: self.repaint_interval,
            });
        }
        if self.marker_interval.is_zero() {
            return Err(AppearanceCandidateError::ZeroActivityMarkerInterval);
        }
        if self.marker_interval < self.repaint_interval {
            return Err(AppearanceCandidateError::ActivityMarkerIntervalTooFast {
                minimum: self.repaint_interval,
                actual: self.marker_interval,
            });
        }
        if self.sweep_period.is_zero() {
            return Err(AppearanceCandidateError::ZeroActivitySweepPeriod);
        }
        Ok(())
    }

    pub(super) fn frame_at(&self, elapsed: Duration) -> ActivityMotionFrame<'_> {
        let index = if self.reduced_motion {
            0
        } else {
            marker_frame_index(elapsed, self.marker_interval, self.marker_frames.len())
        };
        let marker = &self.marker_frames[index];
        ActivityMotionFrame {
            marker: &marker.text,
            marker_width: marker.width,
            reserved_marker_width: self.reserved_marker_width,
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
    pub(super) fn with_test_motion(
        mut self,
        repaint_interval: Duration,
        marker_interval: Duration,
        marker_frames: &[&str],
    ) -> Result<Self, AppearanceCandidateError> {
        let (marker_frames, reserved_marker_width) = resolve_marker_frames(marker_frames)?;
        self.repaint_interval = repaint_interval;
        self.marker_interval = marker_interval;
        self.marker_frames = marker_frames;
        self.reserved_marker_width = reserved_marker_width;
        self.validate()?;
        Ok(self)
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
            marker_width: 1,
            reserved_marker_width: 1,
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

    pub(crate) const fn marker_width(self) -> u16 {
        self.marker_width
    }

    pub(crate) const fn reserved_marker_width(self) -> u16 {
        self.reserved_marker_width
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

fn marker_frame_index(elapsed: Duration, interval: Duration, frame_count: usize) -> usize {
    let elapsed_intervals = elapsed.as_nanos() / interval.as_nanos();
    (elapsed_intervals % frame_count as u128) as usize
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

fn resolve_marker_frames(
    frames: &[&str],
) -> Result<(Vec<ActivityMarkerFrame>, u16), AppearanceCandidateError> {
    if frames.is_empty() {
        return Err(AppearanceCandidateError::EmptyActivityMarkerFrames);
    }
    let mut resolved = Vec::with_capacity(frames.len());
    let mut reserved_width = 0;
    for (frame_index, frame) in frames.iter().copied().enumerate() {
        if frame.is_empty() {
            return Err(AppearanceCandidateError::EmptyActivityMarkerFrame { frame_index });
        }
        if frame.chars().any(char::is_control) {
            return Err(
                AppearanceCandidateError::ActivityMarkerFrameContainsControl { frame_index },
            );
        }
        let mut width = 0_u16;
        for (grapheme_index, text) in frame.graphemes(true).enumerate() {
            let grapheme = Grapheme::try_from(text).map_err(|cause| {
                AppearanceCandidateError::InvalidActivityMarkerGrapheme {
                    frame_index,
                    grapheme_index,
                    cause,
                }
            })?;
            if grapheme.width().get() > 2 {
                return Err(AppearanceCandidateError::ActivityMarkerGraphemeTooWide {
                    frame_index,
                    grapheme_index,
                    actual: grapheme.width().get(),
                });
            }
            width = width
                .checked_add(grapheme.width().get())
                .ok_or(AppearanceCandidateError::ActivityMarkerWidthOverflow { frame_index })?;
        }
        reserved_width = reserved_width.max(width);
        resolved.push(ActivityMarkerFrame {
            text: frame.to_owned(),
            width,
        });
    }
    Ok((resolved, reserved_width))
}

#[cfg(test)]
mod tests;
