use icu_properties::{
    CodePointMapData, CodePointSetData,
    props::{DefaultIgnorableCodePoint, EastAsianWidth, EmojiPresentation, GeneralCategory},
};

use super::GraphemeError;

pub const WIDTH_PROFILE: &str = "yo-unicode-17.0-narrow/v1";

const TEXT_VARIATION_SELECTOR: char = '\u{fe0e}';
const EMOJI_VARIATION_SELECTOR: char = '\u{fe0f}';

pub(super) fn display_width(text: &str) -> Result<u16, GraphemeError> {
    if is_standardized_text_sequence(text) {
        return non_emoji_width(text);
    }

    if is_exact_rgi_emoji(text)
        || text.chars().any(is_emoji_presentation)
        || is_standardized_emoji_sequence(text)
    {
        return Ok(2);
    }

    non_emoji_width(text)
}

fn non_emoji_width(text: &str) -> Result<u16, GraphemeError> {
    let width = text.chars().map(scalar_width).max().unwrap_or(0);
    if width == 0 {
        Err(GraphemeError::ZeroWidth)
    } else {
        Ok(width)
    }
}

fn scalar_width(character: char) -> u16 {
    let general_category = CodePointMapData::<GeneralCategory>::new().get(character);
    if matches!(
        general_category,
        GeneralCategory::NonspacingMark
            | GeneralCategory::SpacingMark
            | GeneralCategory::EnclosingMark
    ) || CodePointSetData::new::<DefaultIgnorableCodePoint>().contains(character)
    {
        return 0;
    }

    match CodePointMapData::<EastAsianWidth>::new().get(character) {
        EastAsianWidth::Wide | EastAsianWidth::Fullwidth => 2,
        _ => 1,
    }
}

fn is_emoji_presentation(character: char) -> bool {
    CodePointSetData::new::<EmojiPresentation>().contains(character)
}

fn is_exact_rgi_emoji(text: &str) -> bool {
    emojis::get(text).is_some_and(|emoji| emoji.as_str() == text)
}

fn is_standardized_text_sequence(text: &str) -> bool {
    contains_standardized_variation_sequence(text, TEXT_VARIATION_SELECTOR)
}

fn is_standardized_emoji_sequence(text: &str) -> bool {
    contains_standardized_variation_sequence(text, EMOJI_VARIATION_SELECTOR)
}

fn contains_standardized_variation_sequence(text: &str, selector: char) -> bool {
    let mut characters = text.chars().peekable();
    while let Some(base) = characters.next() {
        if characters.peek() == Some(&selector) && has_canonical_emoji_variant(base) {
            return true;
        }
    }
    false
}

fn has_canonical_emoji_variant(base: char) -> bool {
    let candidate = format!("{base}{EMOJI_VARIATION_SELECTOR}");
    emojis::get(&candidate).is_some()
}
