use crate::surface::{Attributes, Color, Style};

pub(super) fn data_color(color: Color) -> String {
    match color {
        Color::Default => "default".to_owned(),
        Color::Indexed(index) => format!("indexed-{index}"),
        Color::Rgb { red, green, blue } => format!("rgb-{red}-{green}-{blue}"),
    }
}

pub(super) fn data_attributes(attributes: Attributes) -> String {
    let mut names = Vec::new();
    for (attribute, name) in [
        (Attributes::BOLD, "bold"),
        (Attributes::DIM, "dim"),
        (Attributes::ITALIC, "italic"),
        (Attributes::UNDERLINE, "underline"),
        (Attributes::BLINK, "blink"),
        (Attributes::REVERSE, "reverse"),
        (Attributes::HIDDEN, "hidden"),
        (Attributes::STRIKETHROUGH, "strikethrough"),
    ] {
        if attributes.contains(attribute) {
            names.push(name);
        }
    }
    names.join(" ")
}

pub(super) fn cell_style(style: Style, column: u16, span: u16) -> String {
    let (_, background) = displayed_colors(style);
    format!(
        "grid-column:{}/span {span};min-width:0;overflow:hidden;background-color:{};",
        u32::from(column) + 1,
        css_color(background.0, background.1)
    )
}

pub(super) fn glyph_style(style: Style) -> String {
    let (foreground, background) = displayed_colors(style);
    let foreground = css_color(foreground.0, foreground.1);
    let background = css_color(background.0, background.1);
    let mut output = "display:block;inline-size:100%;block-size:100%;overflow:hidden;".to_owned();

    if style.attributes.contains(Attributes::DIM) {
        output.push_str("color:color-mix(in srgb,");
        output.push_str(&foreground);
        output.push_str(" 50%,");
        output.push_str(&background);
        output.push_str(");");
    } else {
        output.push_str("color:");
        output.push_str(&foreground);
        output.push(';');
    }

    if style.attributes.contains(Attributes::BOLD) {
        output.push_str("font-weight:700;");
    }
    if style.attributes.contains(Attributes::ITALIC) {
        output.push_str("font-style:italic;");
    }

    let underline = style.attributes.contains(Attributes::UNDERLINE);
    let strikethrough = style.attributes.contains(Attributes::STRIKETHROUGH);
    match (underline, strikethrough) {
        (true, true) => output.push_str("text-decoration-line:underline line-through;"),
        (true, false) => output.push_str("text-decoration-line:underline;"),
        (false, true) => output.push_str("text-decoration-line:line-through;"),
        (false, false) => {},
    }

    if style.attributes.contains(Attributes::BLINK) {
        output.push_str("animation:yo-surface-blink 1s step-end infinite;");
    }
    if style.attributes.contains(Attributes::HIDDEN) {
        output.push_str("visibility:hidden;");
    }
    output
}

fn displayed_colors(style: Style) -> ((Color, DefaultRole), (Color, DefaultRole)) {
    if style.attributes.contains(Attributes::REVERSE) {
        (
            (style.background, DefaultRole::Background),
            (style.foreground, DefaultRole::Foreground),
        )
    } else {
        (
            (style.foreground, DefaultRole::Foreground),
            (style.background, DefaultRole::Background),
        )
    }
}

#[derive(Clone, Copy)]
enum DefaultRole {
    Foreground,
    Background,
}

fn css_color(color: Color, default_role: DefaultRole) -> String {
    match color {
        Color::Default => match default_role {
            DefaultRole::Foreground => "var(--yo-default-foreground,#d0d0d0)".to_owned(),
            DefaultRole::Background => "var(--yo-default-background,#000000)".to_owned(),
        },
        Color::Indexed(index) => {
            format!("var(--yo-color-{index},{})", indexed_fallback(index))
        },
        Color::Rgb { red, green, blue } => format!("rgb({red} {green} {blue})"),
    }
}

fn indexed_fallback(index: u8) -> String {
    let (red, green, blue) = match index {
        0 => (0, 0, 0),
        1 => (128, 0, 0),
        2 => (0, 128, 0),
        3 => (128, 128, 0),
        4 => (0, 0, 128),
        5 => (128, 0, 128),
        6 => (0, 128, 128),
        7 => (192, 192, 192),
        8 => (128, 128, 128),
        9 => (255, 0, 0),
        10 => (0, 255, 0),
        11 => (255, 255, 0),
        12 => (0, 0, 255),
        13 => (255, 0, 255),
        14 => (0, 255, 255),
        15 => (255, 255, 255),
        16..=231 => {
            let cube = index - 16;
            let levels = [0, 95, 135, 175, 215, 255];
            (
                levels[usize::from(cube / 36)],
                levels[usize::from((cube % 36) / 6)],
                levels[usize::from(cube % 6)],
            )
        },
        232..=255 => {
            let level = 8 + (index - 232) * 10;
            (level, level, level)
        },
    };
    format!("#{red:02x}{green:02x}{blue:02x}")
}
