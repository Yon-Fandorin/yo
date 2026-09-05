//! Deterministic top-to-bottom allocation for a small component stack.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the vertical solver lands before the agent shell consumes it"
    )
)]

use std::num::NonZeroU16;

use crate::surface::{Point, Rect, Size};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VerticalTrack(TrackSizing);

impl VerticalTrack {
    pub(crate) const fn exact(rows: u16) -> Self {
        Self(TrackSizing::Exact(rows))
    }

    pub(crate) const fn preferred(rows: NonZeroU16) -> Self {
        Self(TrackSizing::Preferred(rows))
    }

    pub(crate) const fn flexible() -> Self {
        Self(TrackSizing::Flexible)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrackSizing {
    Exact(u16),
    Preferred(NonZeroU16),
    Flexible,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerticalLayout {
    areas: Vec<Rect>,
}

impl VerticalLayout {
    pub(crate) fn areas(&self) -> &[Rect] {
        &self.areas
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VerticalLayoutError {
    MultiplePreferred,
    MultipleFlexible,
    InsufficientHeight { required: u16, available: u16 },
    Overflow,
}

pub(crate) fn solve_vertical(
    area: Rect,
    tracks: &[VerticalTrack],
) -> Result<VerticalLayout, VerticalLayoutError> {
    area.end_x().map_err(|_| VerticalLayoutError::Overflow)?;
    area.end_y().map_err(|_| VerticalLayoutError::Overflow)?;

    let inventory = inventory(tracks)?;
    if inventory.minimum_height > area.size.height {
        return Err(VerticalLayoutError::InsufficientHeight {
            required: inventory.minimum_height,
            available: area.size.height,
        });
    }

    let preferred_height = inventory.preferred_height.map_or(0, |desired| {
        desired.min(area.size.height - inventory.exact_height)
    });
    let flexible_height = area
        .size
        .height
        .checked_sub(inventory.exact_height)
        .and_then(|remaining| remaining.checked_sub(preferred_height))
        .expect("the minimum-height check guarantees available rows");

    let mut next_y = area.origin.y;
    let mut areas = Vec::with_capacity(tracks.len());
    for track in tracks {
        let height = match track.0 {
            TrackSizing::Exact(rows) => rows,
            TrackSizing::Preferred(_) => preferred_height,
            TrackSizing::Flexible => flexible_height,
        };
        areas.push(Rect::new(
            Point::new(area.origin.x, next_y),
            Size::new(area.size.width, height),
        ));
        next_y = next_y
            .checked_add(height)
            .ok_or(VerticalLayoutError::Overflow)?;
    }

    Ok(VerticalLayout { areas })
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct TrackInventory {
    exact_height: u16,
    minimum_height: u16,
    preferred_height: Option<u16>,
    has_flexible: bool,
}

fn inventory(tracks: &[VerticalTrack]) -> Result<TrackInventory, VerticalLayoutError> {
    let mut inventory = TrackInventory::default();

    for track in tracks {
        match track.0 {
            TrackSizing::Exact(rows) => {
                inventory.exact_height = inventory
                    .exact_height
                    .checked_add(rows)
                    .ok_or(VerticalLayoutError::Overflow)?;
                inventory.minimum_height = inventory
                    .minimum_height
                    .checked_add(rows)
                    .ok_or(VerticalLayoutError::Overflow)?;
            },
            TrackSizing::Preferred(rows) => {
                if inventory.preferred_height.replace(rows.get()).is_some() {
                    return Err(VerticalLayoutError::MultiplePreferred);
                }
                inventory.minimum_height = inventory
                    .minimum_height
                    .checked_add(1)
                    .ok_or(VerticalLayoutError::Overflow)?;
            },
            TrackSizing::Flexible => {
                if inventory.has_flexible {
                    return Err(VerticalLayoutError::MultipleFlexible);
                }
                inventory.has_flexible = true;
            },
        }
    }

    Ok(inventory)
}

#[cfg(test)]
mod tests;
