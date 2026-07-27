//! Safe visible projections for non-executing terminal text.

const TAB_STOP: u16 = 4;

pub(super) const fn tab_spaces(column: u16) -> u16 {
    TAB_STOP - (column % TAB_STOP)
}

pub(super) fn control_notation(text: &str) -> Option<String> {
    let mut characters = text.chars();
    let character = characters.next()?;
    if characters.next().is_some() || !character.is_control() {
        return None;
    }

    Some(match character {
        '\0'..='\u{1f}' => {
            let notation = char::from_u32(u32::from(character) + 0x40)
                .expect("C0 caret notation is valid ASCII");
            format!("^{notation}")
        },
        '\u{7f}' => "^?".to_owned(),
        _ => format!("\\u{{{:04X}}}", u32::from(character)),
    })
}

#[cfg(test)]
mod tests;
