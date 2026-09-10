//! Bounded Mermaid-to-terminal projection; source survives every fallback.
use std::num::NonZeroU16;

use mermaid_text::render;

use super::{Block, BlockFormat, Decoration, Role, media_text};
use crate::surface::cell_width;

pub(super) fn diagram_blocks(source: Block, width: NonZeroU16, enabled: bool) -> Vec<Block> {
    if !enabled {
        return vec![source];
    }
    let failure = |reason: &str| {
        vec![
            media_text(
                format!("Diagram shown as source: {reason}"),
                &source,
                Role::DiffMeta,
            ),
            source.clone(),
        ]
    };
    // Gantt's dependency parser supplies today's date for implicit starts; keep projection
    // deterministic.
    if source.text.trim_start().starts_with("gantt") {
        return failure("schedule rendering requires explicit date handling");
    }
    if source.text.split(['\n', ';']).any(|statement| {
        matches!(
            statement.split_whitespace().next(),
            Some("style" | "classDef" | "class" | "click" | "linkStyle")
        )
    }) {
        return failure("diagram directives require source view");
    }
    // Bound work before entering the layout engine, including compact one-line graphs.
    if source.text.len() > 8192
        || source.text.split(['\n', ';']).count() > 64
        || source.text.split_whitespace().count() > 512
        || source.text.lines().any(|line| line.len() > 512)
    {
        return failure("diagram exceeds preview limits");
    }
    let Ok(rendered) = render(&source.text) else {
        return failure("unsupported or incomplete diagram");
    };
    let available = usize::from(width.get())
        .saturating_sub(source.prefix.chars().count())
        .saturating_sub(1);
    if rendered.is_empty() || rendered.lines().count() > 256 || rendered.len() > 65536 {
        return failure("diagram exceeds preview limits");
    }
    if rendered.lines().any(|line| {
        line.chars().any(|c| c.is_control())
            || cell_width(line).map_or(true, |cells| cells > available)
    }) {
        return failure("diagram does not fit this width");
    }
    rendered
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let mut block = source.clone();
            block.text = line.to_owned();
            block.format = BlockFormat::Code;
            block.gap = index == 0 && source.gap;
            block.spans = vec![(0, Decoration::role(Role::Code))];
            block
        })
        .collect()
}
