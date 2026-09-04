//! Safe placeholder expansion for meter labels and layouts.

use super::{MAX_METER_BYTES, MAX_METER_CELLS, error::MeterTemplateError, format_percent};

/// A layout template for a rendered meter.
///
/// The supported placeholders are {label}, {meter}, and {percent}.
/// {value} is accepted as an alias for {percent}, and {bar} as an alias
/// for {meter}. Double braces escape literal braces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MeterTemplate<'a> {
    pattern: &'a str,
}

impl<'a> MeterTemplate<'a> {
    /// Creates a template. Syntax is validated when a value is rendered.
    #[must_use]
    pub const fn new(pattern: &'a str) -> Self {
        Self { pattern }
    }

    /// Returns the source pattern.
    #[must_use]
    pub const fn pattern(self) -> &'a str {
        self.pattern
    }

    /// Expands this template with a pre-rendered meter and a percentage value.
    pub fn render(
        self,
        label: &str,
        meter: &str,
        percent_basis_points: u16,
    ) -> Result<String, MeterTemplateError> {
        if label.chars().any(char::is_control) {
            return Err(MeterTemplateError::LabelContainsControl);
        }
        if let Some(character) = meter
            .chars()
            .find(|character| character.is_control() && *character != '\n')
        {
            return Err(MeterTemplateError::ControlCharacter(character));
        }
        let percent = format_percent(percent_basis_points);
        let values = [
            ("label", label),
            ("meter", meter),
            ("bar", meter),
            ("percent", percent.as_str()),
            ("value", percent.as_str()),
        ];
        expand_template(self.pattern, &values)
    }
}

pub(super) fn expand_template(
    pattern: &str,
    values: &[(&str, &str)],
) -> Result<String, MeterTemplateError> {
    if pattern.len() > MAX_METER_BYTES {
        return Err(MeterTemplateError::OutputTooLarge {
            cells: usize::MAX,
            bytes: pattern.len(),
        });
    }
    let mut output = String::new();
    output
        .try_reserve(pattern.len())
        .map_err(|_| MeterTemplateError::OutputTooLarge {
            cells: usize::MAX,
            bytes: pattern.len(),
        })?;
    let mut output_cells = 0;
    let mut characters = pattern.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '{' => {
                if characters.peek() == Some(&'{') {
                    characters.next();
                    push_char(&mut output, &mut output_cells, '{')?;
                    continue;
                }
                let mut name = String::new();
                loop {
                    match characters.next() {
                        Some('}') => break,
                        Some('{') => return Err(MeterTemplateError::NestedPlaceholder),
                        Some(character) if character.is_control() => {
                            return Err(MeterTemplateError::ControlCharacter(character));
                        },
                        Some(character) => push_name_char(&mut name, character)?,
                        None => return Err(MeterTemplateError::UnterminatedPlaceholder),
                    }
                }
                if name.is_empty() {
                    return Err(MeterTemplateError::EmptyPlaceholder);
                }
                let Some((_, value)) = values.iter().find(|(key, _)| *key == name) else {
                    return Err(MeterTemplateError::UnknownPlaceholder(name));
                };
                push_str(&mut output, &mut output_cells, value)?;
            },
            '}' => {
                if characters.peek() == Some(&'}') {
                    characters.next();
                    push_char(&mut output, &mut output_cells, '}')?;
                } else {
                    return Err(MeterTemplateError::UnmatchedClosingBrace);
                }
            },
            character if character.is_control() && character != '\n' => {
                return Err(MeterTemplateError::ControlCharacter(character));
            },
            character => push_char(&mut output, &mut output_cells, character)?,
        }
    }
    Ok(output)
}

fn push_str(
    output: &mut String,
    output_cells: &mut usize,
    value: &str,
) -> Result<(), MeterTemplateError> {
    let bytes =
        output
            .len()
            .checked_add(value.len())
            .ok_or(MeterTemplateError::OutputTooLarge {
                cells: usize::MAX,
                bytes: usize::MAX,
            })?;
    if bytes > MAX_METER_BYTES {
        return Err(MeterTemplateError::OutputTooLarge {
            cells: usize::MAX,
            bytes,
        });
    }
    let value_cells = terminal_cells(value)?;
    let cells =
        output_cells
            .checked_add(value_cells)
            .ok_or(MeterTemplateError::OutputTooLarge {
                cells: usize::MAX,
                bytes,
            })?;
    if cells > MAX_METER_CELLS {
        return Err(MeterTemplateError::OutputTooLarge { cells, bytes });
    }
    output
        .try_reserve(value.len())
        .map_err(|_| MeterTemplateError::OutputTooLarge {
            cells: usize::MAX,
            bytes,
        })?;
    output.push_str(value);
    *output_cells = cells;
    Ok(())
}

fn push_char(
    output: &mut String,
    output_cells: &mut usize,
    character: char,
) -> Result<(), MeterTemplateError> {
    let mut encoded = [0_u8; 4];
    let value = character.encode_utf8(&mut encoded);
    push_str(output, output_cells, value)
}

fn push_name_char(name: &mut String, character: char) -> Result<(), MeterTemplateError> {
    let bytes =
        name.len()
            .checked_add(character.len_utf8())
            .ok_or(MeterTemplateError::OutputTooLarge {
                cells: usize::MAX,
                bytes: usize::MAX,
            })?;
    if bytes > MAX_METER_BYTES {
        return Err(MeterTemplateError::OutputTooLarge {
            cells: usize::MAX,
            bytes,
        });
    }
    name.try_reserve(character.len_utf8())
        .map_err(|_| MeterTemplateError::OutputTooLarge {
            cells: usize::MAX,
            bytes,
        })?;
    name.push(character);
    Ok(())
}

fn terminal_cells(value: &str) -> Result<usize, MeterTemplateError> {
    value.split('\n').try_fold(0_usize, |cells, line| {
        let line_cells =
            crate::surface::cell_width(line).map_err(MeterTemplateError::InvalidGrapheme)?;
        cells
            .checked_add(line_cells)
            .ok_or(MeterTemplateError::OutputTooLarge {
                cells: usize::MAX,
                bytes: value.len(),
            })
    })
}
