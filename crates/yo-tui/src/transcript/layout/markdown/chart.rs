//! Validated numeric series and responsive chart layout, independent of theme/transport.
use std::num::NonZeroU16;

use super::{Block, Decoration, Role, media_text};
use crate::{
    meter::{MeterGlyphs, MeterShape, MeterSpec},
    surface::cell_width,
};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum ChartKind {
    Bars,
    Sparkline,
    Line,
    Step,
    Histogram,
    Scatter,
}

#[derive(Clone, Copy)]
struct Sample<'a> {
    label: &'a str,
    literal: &'a str,
    value: f64,
    x: f64,
}

struct Series<'a> {
    samples: Vec<Sample<'a>>,
    minimum: f64,
    maximum: f64,
}

impl<'a> Series<'a> {
    fn parse(source: &'a str, kind: ChartKind) -> Option<Self> {
        let samples: Option<Vec<_>> = if kind != ChartKind::Bars {
            source
                .split_whitespace()
                .take(65)
                .enumerate()
                .map(|(index, text)| {
                    let (x, value) = if kind == ChartKind::Scatter {
                        let (x, y) = text.split_once(',')?;
                        (x.parse::<f64>().ok()?, y.parse::<f64>().ok()?)
                    } else {
                        (index as f64, text.parse::<f64>().ok()?)
                    };
                    (x.is_finite() && value.is_finite()).then_some(Sample {
                        label: text,
                        literal: text,
                        value,
                        x,
                    })
                })
                .collect()
        } else {
            source
                .lines()
                .filter(|line| !line.trim().is_empty())
                .take(65)
                .map(|line| {
                    let (label, value) = line.rsplit_once(':')?;
                    let literal = value.trim();
                    let value = literal.parse::<f64>().ok()?;
                    (!label.trim().is_empty() && value.is_finite()).then_some(Sample {
                        label: label.trim(),
                        literal,
                        value,
                        x: 0.0,
                    })
                })
                .collect()
        };
        let samples = samples.filter(|values| !values.is_empty() && values.len() <= 64)?;
        let minimum = samples
            .iter()
            .map(|sample| sample.value)
            .fold(f64::INFINITY, f64::min);
        let maximum = samples
            .iter()
            .map(|sample| sample.value)
            .fold(f64::NEG_INFINITY, f64::max);
        Some(Self {
            samples,
            minimum,
            maximum,
        })
    }

    fn normalized(&self, value: f64) -> f64 {
        normalize(value, self.minimum, self.maximum)
    }

    fn bars(&self, context: &Block, width: usize) -> Vec<Block> {
        let negative = self.minimum < 0.0;
        let magnitude = self.minimum.abs().max(self.maximum.abs());
        let label_width = self
            .samples
            .iter()
            .map(|sample| cell_width(sample.label).unwrap_or(0))
            .max()
            .unwrap_or(0);
        let value_width = self
            .samples
            .iter()
            .map(|sample| sample.literal.len())
            .max()
            .unwrap_or(1);
        let compact = width >= label_width + value_width + 16;
        let plot_width = if compact {
            width - label_width - value_width - 4
        } else {
            width.saturating_sub(1)
        }
        .max(1);
        let half = if negative {
            plot_width.saturating_sub(1) / 2
        } else {
            plot_width.saturating_sub(1)
        };
        let mut rows = Vec::new();
        let scale = if negative {
            format!("Scale −{} ← 0 → {}", short(magnitude), short(magnitude))
        } else {
            format!("Scale 0 → {}", short(magnitude))
        };
        rows.push(media_text(scale, context, Role::Quote));
        for &Sample {
            label,
            literal: number,
            value,
            ..
        } in &self.samples
        {
            let percent = if magnitude == 0.0 {
                0
            } else {
                ((value.abs() / magnitude) * 10_000.0).round() as u16
            };
            let meter = if half == 0 {
                String::new()
            } else {
                MeterSpec::raw(
                    MeterShape::HorizontalBar { width: half },
                    MeterGlyphs::new("█", " ", &[]),
                )
                .render_glyph(percent)
                .expect("bounded chart meter")
            };
            let bar = if negative {
                if value < 0.0 {
                    format!(
                        "{}│{}",
                        meter.chars().rev().collect::<String>(),
                        " ".repeat(half)
                    )
                } else {
                    format!("{}│{}", " ".repeat(half), meter)
                }
            } else {
                format!("│{meter}")
            };
            if compact {
                let padding = label_width - cell_width(label).unwrap_or(0);
                let text = format!(
                    "{label}{}  {bar}  {number:>value_width$}",
                    " ".repeat(padding)
                );
                let mut row = media_text(text, context, Role::Body);
                row.format = super::BlockFormat::TableRow;
                let start = label.len() + padding + 2;
                row.spans.push((start, Decoration::role(Role::Bar)));
                row.spans
                    .push((start + bar.len(), Decoration::role(Role::Body)));
                rows.push(row);
            } else {
                rows.push(media_text(
                    format!("{label}  {number}"),
                    context,
                    Role::Body,
                ));
                rows.push(media_text(bar, context, Role::Bar));
            }
        }
        rows
    }

    fn histogram(&self, context: &Block, width: usize, requested_bins: usize) -> Vec<Block> {
        let mut bins = if self.minimum == self.maximum {
            1
        } else {
            requested_bins
        };
        let edges = loop {
            let range = self.maximum - self.minimum;
            let edges: Vec<_> = (0..=bins)
                .map(|index| {
                    if index == 0 {
                        self.minimum
                    } else if index == bins {
                        self.maximum
                    } else {
                        let fraction = index as f64 / bins as f64;
                        if range.is_finite() {
                            self.minimum + range * fraction
                        } else {
                            self.minimum * (1.0 - fraction) + self.maximum * fraction
                        }
                    }
                })
                .collect();
            if bins == 1 || edges.windows(2).all(|pair| pair[0] < pair[1]) {
                break edges;
            }
            bins -= 1;
        };
        let mut counts = vec![0_usize; bins];
        for sample in &self.samples {
            let index = edges
                .partition_point(|edge| *edge <= sample.value)
                .saturating_sub(1)
                .min(bins - 1);
            counts[index] += 1;
        }
        let labels: Vec<_> = edges
            .windows(2)
            .enumerate()
            .map(|(index, pair)| {
                format!(
                    "[{}, {}{}",
                    exact_label(pair[0]),
                    exact_label(pair[1]),
                    if index + 1 == bins { "]" } else { ")" }
                )
            })
            .collect();
        let literals: Vec<_> = counts.iter().map(usize::to_string).collect();
        let histogram = Series {
            samples: labels
                .iter()
                .zip(&literals)
                .zip(&counts)
                .map(|((label, literal), count)| Sample {
                    label,
                    literal,
                    value: *count as f64,
                    x: 0.0,
                })
                .collect(),
            minimum: 0.0,
            maximum: counts.iter().copied().max().unwrap_or(0) as f64,
        };
        let mut rows = vec![media_text(
            format!(
                "{} sample{} · {} bin{} · count",
                self.samples.len(),
                if self.samples.len() == 1 { "" } else { "s" },
                bins,
                if bins == 1 { "" } else { "s" }
            ),
            context,
            Role::Quote,
        )];
        rows.extend(histogram.bars(context, width));
        rows.push(media_text(
            format!(
                "Values: {}",
                self.samples
                    .iter()
                    .map(|sample| sample.literal)
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            context,
            Role::Body,
        ));
        rows
    }

    fn points(
        &self,
        columns: usize,
        height: usize,
        kind: ChartKind,
        x_min: f64,
        x_max: f64,
    ) -> Vec<(i32, i32)> {
        self.samples
            .iter()
            .enumerate()
            .map(|(i, sample)| {
                (
                    if kind == ChartKind::Scatter {
                        (normalize(sample.x, x_min, x_max) * (columns * 2 - 1) as f64).round()
                            as i32
                    } else {
                        (i * (columns * 2 - 1) / self.samples.len().saturating_sub(1).max(1)) as i32
                    },
                    ((1.0 - self.normalized(sample.value)) * (height * 4 - 1) as f64).round()
                        as i32,
                )
            })
            .collect()
    }

    fn pixels(
        &self,
        columns: usize,
        height: usize,
        kind: ChartKind,
        x_min: f64,
        x_max: f64,
    ) -> Vec<Vec<bool>> {
        let points = self.points(columns, height, kind, x_min, x_max);
        let mut pixels = vec![vec![false; columns * 2]; height * 4];
        if kind == ChartKind::Scatter || points.len() == 1 {
            for &point in &points {
                draw_line(&mut pixels, point, point);
            }
        }
        if kind != ChartKind::Scatter {
            for pair in points.windows(2) {
                if kind == ChartKind::Step {
                    let corner = (pair[1].0, pair[0].1);
                    draw_line(&mut pixels, pair[0], corner);
                    draw_line(&mut pixels, corner, pair[1]);
                } else {
                    draw_line(&mut pixels, pair[0], pair[1]);
                }
            }
        }
        pixels
    }

    fn trend(&self, context: &Block, width: usize, kind: ChartKind, height: usize) -> Vec<Block> {
        let mut rows = Vec::new();
        let scatter = kind == ChartKind::Scatter;
        let x_min = self
            .samples
            .iter()
            .map(|sample| sample.x)
            .fold(f64::INFINITY, f64::min);
        let x_max = self
            .samples
            .iter()
            .map(|sample| sample.x)
            .fold(f64::NEG_INFINITY, f64::max);
        let line = matches!(kind, ChartKind::Line | ChartKind::Step | ChartKind::Scatter);
        let labels: Vec<_> = if line {
            (0..height)
                .map(|row| {
                    if row == 0 || row == height / 2 || row == height - 1 {
                        short(
                            self.maximum * (1.0 - row as f64 / (height - 1) as f64)
                                + self.minimum * (row as f64 / (height - 1) as f64),
                        )
                    } else {
                        String::new()
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        let axis_width = labels.iter().map(String::len).max().unwrap_or(0).max(7);
        if line && width >= (axis_width + 6).max(16) && (self.samples.len() > 1 || scatter) {
            let columns = (width - axis_width - 2).clamp(4, 60);
            let pixels = self.pixels(columns, height, kind, x_min, x_max);
            for (row, label) in labels.iter().enumerate() {
                let mut text = format!("{label:>axis_width$} │");
                for column in 0..columns {
                    let mask = braille_mask(&pixels, row, column);
                    text.push(char::from_u32(0x2800 + mask).expect("Braille cell"));
                }
                rows.push(media_text(text, context, Role::Chart));
            }
            rows.push(media_text(
                format!("{}└{}", " ".repeat(axis_width + 1), "─".repeat(columns)),
                context,
                Role::Chart,
            ));
            rows.push(media_text(
                if scatter {
                    format!("X {} → {}", exact_label(x_min), exact_label(x_max))
                } else {
                    format!(
                        "{}1 → {} samples",
                        " ".repeat(axis_width + 3),
                        self.samples.len()
                    )
                },
                context,
                Role::Quote,
            ));
        } else if scatter {
            rows.push(media_text(
                "Scatter plot · showing coordinates".into(),
                context,
                Role::Quote,
            ));
            rows.push(media_text(
                format!("X {} → {}", exact_label(x_min), exact_label(x_max)),
                context,
                Role::Quote,
            ));
        } else {
            let levels = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
            let text = self
                .samples
                .iter()
                .map(|sample| levels[(self.normalized(sample.value) * 7.0).round() as usize])
                .collect();
            rows.push(media_text(text, context, Role::Chart));
        }
        rows.push(media_text(
            if scatter {
                format!(
                    "Y {} → {} · {} point{}",
                    exact_label(self.minimum),
                    exact_label(self.maximum),
                    self.samples.len(),
                    if self.samples.len() == 1 { "" } else { "s" }
                )
            } else {
                format!(
                    "Range {}–{} · last {}",
                    short(self.minimum),
                    short(self.maximum),
                    self.samples.last().expect("nonempty").literal
                )
            },
            context,
            Role::Quote,
        ));
        rows.push(media_text(
            format!(
                "{}: {}",
                if scatter { "Points" } else { "Values" },
                self.samples
                    .iter()
                    .map(|sample| sample.literal)
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            context,
            Role::Body,
        ));
        rows
    }
}

fn braille_mask(pixels: &[Vec<bool>], row: usize, column: usize) -> u32 {
    let mut mask = 0;
    for (dy, bits) in [[0, 3], [1, 4], [2, 5], [6, 7]].iter().enumerate() {
        for dx in 0..2 {
            if pixels[row * 4 + dy][column * 2 + dx] {
                mask |= 1 << bits[dx];
            }
        }
    }
    mask
}

struct NamedSeries<'a>(Vec<(&'a str, Series<'a>)>);

impl<'a> NamedSeries<'a> {
    fn parse(source: &'a str, kind: ChartKind) -> Option<Self> {
        let mut series = Vec::new();
        for line in source
            .lines()
            .filter(|line| !line.trim().is_empty())
            .take(5)
        {
            let (name, values) = line.split_once(':')?;
            let name = name.trim();
            if name.is_empty()
                || name.len() > 64
                || name.chars().any(char::is_control)
                || series.iter().any(|(existing, _)| *existing == name)
            {
                return None;
            }
            series.push((name, Series::parse(values, kind)?));
        }
        if series.is_empty() || series.len() > 4 {
            return None;
        }
        if kind != ChartKind::Scatter
            && series
                .iter()
                .any(|(_, value)| value.samples.len() != series[0].1.samples.len())
        {
            return None;
        }
        let minimum = series
            .iter()
            .map(|(_, s)| s.minimum)
            .fold(f64::INFINITY, f64::min);
        let maximum = series
            .iter()
            .map(|(_, s)| s.maximum)
            .fold(f64::NEG_INFINITY, f64::max);
        for (_, value) in &mut series {
            value.minimum = minimum;
            value.maximum = maximum;
        }
        Some(Self(series))
    }

    fn render(&self, context: &Block, width: usize, kind: ChartKind, height: usize) -> Vec<Block> {
        let minimum = self.0[0].1.minimum;
        let maximum = self.0[0].1.maximum;
        let x_min = self
            .0
            .iter()
            .flat_map(|(_, s)| s.samples.iter())
            .map(|s| s.x)
            .fold(f64::INFINITY, f64::min);
        let x_max = self
            .0
            .iter()
            .flat_map(|(_, s)| s.samples.iter())
            .map(|s| s.x)
            .fold(f64::NEG_INFINITY, f64::max);
        let mut rows = self
            .0
            .iter()
            .enumerate()
            .map(|(i, (name, _))| {
                media_text(
                    format!("[{}] {name}", i + 1),
                    context,
                    Role::ChartSeries(i as u8),
                )
            })
            .collect::<Vec<_>>();
        let labels = (0..height)
            .map(|row| {
                if row == 0 || row == height / 2 || row == height - 1 {
                    short(
                        maximum * (1.0 - row as f64 / (height - 1) as f64)
                            + minimum * row as f64 / (height - 1) as f64,
                    )
                } else {
                    String::new()
                }
            })
            .collect::<Vec<_>>();
        let axis_width = labels.iter().map(String::len).max().unwrap_or(0).max(7);
        if width >= (axis_width + 6).max(16) {
            let columns = (width - axis_width - 2).clamp(4, 60);
            let pixels = self
                .0
                .iter()
                .map(|(_, s)| s.pixels(columns, height, kind, x_min, x_max))
                .collect::<Vec<_>>();
            let points = self
                .0
                .iter()
                .map(|(_, s)| s.points(columns, height, kind, x_min, x_max))
                .collect::<Vec<_>>();
            let mut overlap = false;
            let mut mixed = false;
            for (row, label) in labels.iter().enumerate() {
                let mut text = format!("{label:>axis_width$} │");
                let mut spans = vec![(0, Decoration::role(Role::Chart))];
                for column in 0..columns {
                    let owners = pixels
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| braille_mask(p, row, column) != 0)
                        .map(|(i, _)| i)
                        .collect::<Vec<_>>();
                    let (glyph, role) = match owners.as_slice() {
                        [] => (' ', Role::Chart),
                        [index] => {
                            let point = points[*index]
                                .iter()
                                .any(|&(x, y)| x as usize / 2 == column && y as usize / 4 == row);
                            (
                                if point {
                                    char::from(b'1' + *index as u8)
                                } else {
                                    char::from_u32(
                                        0x2800 + braille_mask(&pixels[*index], row, column),
                                    )
                                    .expect("Braille cell")
                                },
                                Role::ChartSeries(*index as u8),
                            )
                        },
                        _ => {
                            let mut combined = 0;
                            let mut intersects = false;
                            for index in owners {
                                let mask = braille_mask(&pixels[index], row, column);
                                intersects |= combined & mask != 0;
                                combined |= mask;
                            }
                            if intersects {
                                overlap = true;
                                ('×', Role::ChartOverlap)
                            } else {
                                mixed = true;
                                (
                                    char::from_u32(0x2800 + combined).expect("Braille cell"),
                                    Role::ChartOverlap,
                                )
                            }
                        },
                    };
                    spans.push((text.len(), Decoration::role(role)));
                    text.push(glyph);
                }
                let mut block = media_text(text, context, Role::Chart);
                block.spans = spans;
                rows.push(block);
            }
            rows.push(media_text(
                format!("{}└{}", " ".repeat(axis_width + 1), "─".repeat(columns)),
                context,
                Role::Chart,
            ));
            if overlap {
                rows.push(media_text(
                    "× plotted overlap".into(),
                    context,
                    Role::ChartOverlap,
                ));
            }
            if mixed {
                rows.push(media_text(
                    "Mixed cells shown muted".into(),
                    context,
                    Role::ChartOverlap,
                ));
            }
        } else {
            rows.push(media_text(
                "Chart narrowed · showing series values".into(),
                context,
                Role::Quote,
            ));
        }
        rows.push(media_text(
            format!(
                "X {} → {} · Y {} → {}",
                exact_label(x_min),
                exact_label(x_max),
                exact_label(minimum),
                exact_label(maximum)
            ),
            context,
            Role::Quote,
        ));
        for (i, (name, series)) in self.0.iter().enumerate() {
            rows.push(media_text(
                format!(
                    "[{}] {name}: {}",
                    i + 1,
                    series
                        .samples
                        .iter()
                        .map(|s| s.literal)
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
                context,
                Role::ChartSeries(i as u8),
            ));
        }
        rows
    }
}

fn normalize(value: f64, minimum: f64, maximum: f64) -> f64 {
    if maximum == minimum {
        return 0.5;
    }
    let range = maximum - minimum;
    if range.is_finite() {
        (value - minimum) / range
    } else {
        (value / 2.0 - minimum / 2.0) / (maximum / 2.0 - minimum / 2.0)
    }
    .clamp(0.0, 1.0)
}

fn exact_label(value: f64) -> String {
    let decimal = value.to_string();
    let scientific = format!("{value:e}");
    if scientific.len() < decimal.len() {
        scientific
    } else {
        decimal
    }
}

fn short(value: f64) -> String {
    let plain = value.to_string();
    if plain.len() <= 7 {
        plain
    } else {
        format!("{value:.1e}")
    }
}

fn draw_line(pixels: &mut [Vec<bool>], (mut x, mut y): (i32, i32), (end_x, end_y): (i32, i32)) {
    let dx = (end_x - x).abs();
    let dy = -(end_y - y).abs();
    let sx = if x < end_x { 1 } else { -1 };
    let sy = if y < end_y { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        if let Some(row) = pixels.get_mut(y as usize)
            && let Some(pixel) = row.get_mut(x as usize)
        {
            *pixel = true;
        }
        if x == end_x && y == end_y {
            break;
        }
        let twice = error * 2;
        if twice >= dy {
            error += dy;
            x += sx;
        }
        if twice <= dx {
            error += dx;
            y += sy;
        }
    }
}

pub(super) fn chart_blocks(block: Block, width: NonZeroU16, kind: ChartKind) -> Vec<Block> {
    let source = block.text.trim();
    let (option, default, range) = match kind {
        ChartKind::Histogram => ("bins:", 8, 1..=32),
        ChartKind::Line | ChartKind::Step | ChartKind::Scatter => ("height:", 6, 2..=16),
        _ => ("", 6, 2..=16),
    };
    let data = if !option.is_empty() && source.starts_with(option) {
        source.split_once('\n').and_then(|(header, values)| {
            header
                .strip_prefix(option)?
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|value| range.contains(value))
                .map(|value| (values, value))
        })
    } else {
        Some((source, default))
    };
    if let Some((source, extent)) = data
        && matches!(kind, ChartKind::Line | ChartKind::Step | ChartKind::Scatter)
        && source.contains(':')
        && let Some(series) = NamedSeries::parse(source, kind)
    {
        let available = usize::from(width.get())
            .saturating_sub(block.prefix.chars().count())
            .max(1);
        let mut rows = series.render(&block, available, kind, extent);
        if let Some(first) = rows.first_mut() {
            first.gap = block.gap;
        }
        return rows;
    }
    let Some((series, extent)) = data
        .and_then(|(source, extent)| Series::parse(source, kind).map(|series| (series, extent)))
    else {
        let mut source = block.clone();
        source.spans = vec![(0, Decoration::role(Role::Code))];
        return vec![
            media_text(
                "Chart data incomplete · showing source".into(),
                &block,
                Role::Quote,
            ),
            source,
        ];
    };
    let width = usize::from(width.get())
        .saturating_sub(block.prefix.chars().count())
        .max(1);
    let mut rows = if kind == ChartKind::Bars {
        series.bars(&block, width)
    } else if kind == ChartKind::Histogram {
        series.histogram(&block, width, extent)
    } else {
        series.trend(&block, width, kind, extent)
    };
    if let Some(first) = rows.first_mut() {
        first.gap = block.gap;
    }
    rows
}
