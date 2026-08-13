//! Preserves YAML scalar style where structured model profiles need lexical number identity.
//!
//! `serde_norway` correctly owns configuration decoding, but its visitor boundary cannot
//! distinguish an out-of-range plain number that fell back to a string from an authored string.
//! The pinned event parser supplies that missing style and alias context without reimplementing
//! quoting, comments, continuation lines, or YAML collection structure.

use std::collections::{HashMap, HashSet};

use saphyr_parser::{Event, EventReceiver, Parser, ScalarStyle};

use super::ModelServiceError;

const STRUCTURED_FIELDS: [&str; 2] = ["reasoning_parameters", "optional_request_parameters"];

pub fn validate_profile_yaml_number_spellings(contents: &str) -> Result<(), ModelServiceError> {
    validate_plain_number_spellings(contents).map_err(ModelServiceError::new)
}

fn validate_plain_number_spellings(contents: &str) -> Result<(), String> {
    let root = parse_document(contents)?;
    let mut anchors = HashMap::new();
    collect_anchors(&root, &mut anchors)?;
    validate_structured_fields(&root, &anchors)
}

enum Node {
    Scalar {
        value: String,
        style: ScalarStyle,
        explicit_tag: Option<ExplicitScalarTag>,
        anchor: usize,
    },
    Sequence {
        values: Vec<Self>,
        anchor: usize,
    },
    Mapping {
        entries: Vec<(Self, Self)>,
        anchor: usize,
    },
    Alias(usize),
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ExplicitScalarTag {
    String,
    Integer,
    Float,
    Other,
}

impl Node {
    const fn anchor(&self) -> usize {
        match self {
            Self::Scalar { anchor, .. }
            | Self::Sequence { anchor, .. }
            | Self::Mapping { anchor, .. } => *anchor,
            Self::Alias(_) => 0,
        }
    }
}

struct EventSink<'input>(Vec<Event<'input>>);

impl<'input> EventReceiver<'input> for EventSink<'input> {
    fn on_event(&mut self, event: Event<'input>) {
        self.0.push(event);
    }
}

fn parse_document(contents: &str) -> Result<Node, String> {
    let mut sink = EventSink(Vec::new());
    Parser::new_from_str(contents)
        .load(&mut sink, false)
        .map_err(|error| format!("profile scalar scan failed: {error}"))?;
    let first = sink
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                Event::Scalar(..)
                    | Event::SequenceStart(..)
                    | Event::MappingStart(..)
                    | Event::Alias(..)
            )
        })
        .ok_or_else(|| "profile scalar scan found no YAML document".to_owned())?;
    let mut cursor = first;
    parse_node(&sink.0, &mut cursor)
}

fn parse_node(events: &[Event<'_>], cursor: &mut usize) -> Result<Node, String> {
    let event = events
        .get(*cursor)
        .ok_or_else(|| "profile scalar scan reached an incomplete YAML node".to_owned())?;
    *cursor += 1;
    match event {
        Event::Scalar(value, style, anchor, tag) => Ok(Node::Scalar {
            value: value.to_string(),
            style: *style,
            explicit_tag: tag.as_deref().and_then(|tag| {
                if !tag.is_yaml_core_schema() {
                    return None;
                }
                Some(match tag.suffix.as_ref() {
                    "str" => ExplicitScalarTag::String,
                    "int" => ExplicitScalarTag::Integer,
                    "float" => ExplicitScalarTag::Float,
                    _ => ExplicitScalarTag::Other,
                })
            }),
            anchor: *anchor,
        }),
        Event::Alias(anchor) => Ok(Node::Alias(*anchor)),
        Event::SequenceStart(anchor, _) => {
            let mut values = Vec::new();
            while !matches!(events.get(*cursor), Some(Event::SequenceEnd)) {
                values.push(parse_node(events, cursor)?);
            }
            *cursor += 1;
            Ok(Node::Sequence {
                values,
                anchor: *anchor,
            })
        },
        Event::MappingStart(anchor, _) => {
            let mut entries = Vec::new();
            while !matches!(events.get(*cursor), Some(Event::MappingEnd)) {
                let key = parse_node(events, cursor)?;
                let value = parse_node(events, cursor)?;
                entries.push((key, value));
            }
            *cursor += 1;
            Ok(Node::Mapping {
                entries,
                anchor: *anchor,
            })
        },
        _ => Err("profile scalar scan encountered an unexpected YAML event".to_owned()),
    }
}

fn collect_anchors<'a>(
    node: &'a Node,
    anchors: &mut HashMap<usize, &'a Node>,
) -> Result<(), String> {
    if node.anchor() != 0 && anchors.insert(node.anchor(), node).is_some() {
        return Err("profile scalar scan found a duplicate YAML anchor".to_owned());
    }
    match node {
        Node::Sequence { values, .. } => {
            for value in values {
                collect_anchors(value, anchors)?;
            }
        },
        Node::Mapping { entries, .. } => {
            for (key, value) in entries {
                collect_anchors(key, anchors)?;
                collect_anchors(value, anchors)?;
            }
        },
        Node::Scalar { .. } | Node::Alias(_) => {},
    }
    Ok(())
}

fn validate_structured_fields(node: &Node, anchors: &HashMap<usize, &Node>) -> Result<(), String> {
    match node {
        Node::Sequence { values, .. } => {
            for value in values {
                validate_structured_fields(value, anchors)?;
            }
        },
        Node::Mapping { entries, .. } => {
            for (key, value) in entries {
                if scalar_key(key, anchors, &mut HashSet::new())
                    .is_some_and(|key| STRUCTURED_FIELDS.contains(&key))
                {
                    validate_profile_value(value, anchors, &mut HashSet::new())?;
                }
                validate_structured_fields(value, anchors)?;
            }
        },
        Node::Scalar { .. } | Node::Alias(_) => {},
    }
    Ok(())
}

fn scalar_key<'a>(
    node: &'a Node,
    anchors: &HashMap<usize, &'a Node>,
    visiting: &mut HashSet<usize>,
) -> Option<&'a str> {
    match node {
        Node::Scalar { value, .. } => Some(value),
        Node::Alias(anchor) => {
            if !visiting.insert(*anchor) {
                return None;
            }
            let value = scalar_key(anchors.get(anchor)?, anchors, visiting);
            visiting.remove(anchor);
            value
        },
        _ => None,
    }
}

fn validate_profile_value(
    node: &Node,
    anchors: &HashMap<usize, &Node>,
    visiting: &mut HashSet<usize>,
) -> Result<(), String> {
    match node {
        Node::Alias(anchor) => {
            if !visiting.insert(*anchor) {
                return Err("structured profile value contains a cyclic YAML alias".to_owned());
            }
            let target = anchors
                .get(anchor)
                .ok_or_else(|| "structured profile value uses an unknown YAML alias".to_owned())?;
            validate_profile_value(target, anchors, visiting)?;
            visiting.remove(anchor);
        },
        Node::Scalar {
            value,
            style,
            explicit_tag,
            ..
        } if *style == ScalarStyle::Plain && *explicit_tag != Some(ExplicitScalarTag::String) => {
            validate_plain_scalar(value, *explicit_tag)?;
        },
        Node::Sequence { values, .. } => {
            for value in values {
                validate_profile_value(value, anchors, visiting)?;
            }
        },
        Node::Mapping { entries, .. } => {
            for (key, value) in entries {
                validate_profile_value(key, anchors, visiting)?;
                validate_profile_value(value, anchors, visiting)?;
            }
        },
        Node::Scalar { .. } => {},
    }
    Ok(())
}

fn validate_plain_scalar(
    value: &str,
    explicit_tag: Option<ExplicitScalarTag>,
) -> Result<(), String> {
    let spelling = if integer_spelling(value).is_some() {
        NumericSpelling::Integer
    } else if decimal_or_exponent_spelling(value) {
        NumericSpelling::Float
    } else {
        NumericSpelling::Other
    };
    let tag_matches = match explicit_tag {
        Some(ExplicitScalarTag::Integer) => spelling == NumericSpelling::Integer,
        Some(ExplicitScalarTag::Float) => spelling == NumericSpelling::Float,
        _ => true,
    };
    if !tag_matches {
        return Err(format!(
            "explicit YAML numeric tag does not match profile number spelling {value:?}"
        ));
    }
    if let Some((negative, radix, digits)) = integer_spelling(value) {
        let magnitude = u64::from_str_radix(digits, radix).ok();
        let valid = if negative {
            magnitude.is_some_and(|value| value <= i64::MIN.unsigned_abs())
        } else {
            magnitude.is_some()
        };
        if !valid {
            return Err(format!(
                "plain profile integer {value:?} is outside its signed or unsigned 64-bit range; quote it to author a string"
            ));
        }
    } else if decimal_or_exponent_spelling(value) && !value.parse::<f64>().is_ok_and(f64::is_finite)
    {
        return Err(format!(
            "plain profile number {value:?} must be a finite f64; quote it to author a string"
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum NumericSpelling {
    Integer,
    Float,
    Other,
}

fn integer_spelling(value: &str) -> Option<(bool, u32, &str)> {
    let value = if let Some(value) = value.strip_prefix('+') {
        if value.starts_with(['+', '-']) {
            return None;
        }
        value
    } else {
        value
    };
    let (negative, unsigned) = value
        .strip_prefix('-')
        .map_or((false, value), |value| (true, value));
    let (radix, digits) = if let Some(digits) = unsigned.strip_prefix("0x") {
        (16, digits)
    } else if let Some(digits) = unsigned.strip_prefix("0o") {
        (8, digits)
    } else if let Some(digits) = unsigned.strip_prefix("0b") {
        (2, digits)
    } else {
        if unsigned.len() > 1
            && unsigned.starts_with('0')
            && unsigned.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        (10, unsigned)
    };
    (!digits.is_empty() && digits.chars().all(|value| value.is_digit(radix)))
        .then_some((negative, radix, digits))
}

fn decimal_or_exponent_spelling(value: &str) -> bool {
    let unsigned = value.strip_prefix(['+', '-']).unwrap_or(value);
    let Some(exponent) = unsigned.find(['e', 'E']) else {
        return decimal_mantissa(unsigned);
    };
    let (mantissa, exponent) = unsigned.split_at(exponent);
    let exponent = exponent[1..]
        .strip_prefix(['+', '-'])
        .unwrap_or(&exponent[1..]);
    float_mantissa(mantissa)
        && !exponent.is_empty()
        && exponent.bytes().all(|byte| byte.is_ascii_digit())
}

fn decimal_mantissa(value: &str) -> bool {
    let Some(dot) = value.find('.') else {
        return false;
    };
    value[dot + 1..].bytes().all(|byte| byte.is_ascii_digit())
        && value[..dot].bytes().all(|byte| byte.is_ascii_digit())
        && (dot > 0 || value.len() > 1)
}

fn float_mantissa(value: &str) -> bool {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        || decimal_mantissa(value)
}
