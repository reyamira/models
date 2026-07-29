use std::io::IsTerminal;

// ── Color constants (crossterm) ──────────────────────────────────────
pub const CODE_BG: crossterm::style::Color = crossterm::style::Color::Rgb {
    r: 50,
    g: 40,
    b: 25,
};
pub const INPUT_BG: crossterm::style::Color = crossterm::style::Color::Rgb {
    r: 60,
    g: 60,
    b: 60,
};

// ── TTY detection ────────────────────────────────────────────────────
pub fn is_tty() -> bool {
    static IS_TTY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *IS_TTY.get_or_init(|| std::io::stdout().is_terminal())
}

// ── Table arrangement (comfy-table) ──────────────────────────────────
/// On a TTY, arrange columns dynamically to the terminal width so wide tables
/// wrap cell content instead of overflowing (the models list is ~330 cols
/// content-sized). Piped/non-TTY output is left untouched — content-sized,
/// byte-identical to previous releases — so text parsers are unaffected.
pub fn arrange_table(table: &mut comfy_table::Table) {
    if is_tty() {
        table.set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
    }
}

// ── Cell helpers (comfy-table) ───────────────────────────────────────
pub fn header_cell(text: &str) -> comfy_table::Cell {
    if is_tty() {
        comfy_table::Cell::new(text)
            .fg(comfy_table::Color::Cyan)
            .add_attribute(comfy_table::Attribute::Bold)
    } else {
        comfy_table::Cell::new(text)
    }
}

pub fn bold_cell(text: &str) -> comfy_table::Cell {
    if is_tty() {
        comfy_table::Cell::new(text).add_attribute(comfy_table::Attribute::Bold)
    } else {
        comfy_table::Cell::new(text)
    }
}

pub fn green_cell(text: &str) -> comfy_table::Cell {
    if is_tty() {
        comfy_table::Cell::new(text).fg(comfy_table::Color::Green)
    } else {
        comfy_table::Cell::new(text)
    }
}

pub fn yellow_cell(text: &str) -> comfy_table::Cell {
    if is_tty() {
        comfy_table::Cell::new(text).fg(comfy_table::Color::Yellow)
    } else {
        comfy_table::Cell::new(text)
    }
}

pub fn dim_cell(text: &str) -> comfy_table::Cell {
    if is_tty() {
        comfy_table::Cell::new(text).fg(comfy_table::Color::DarkGrey)
    } else {
        comfy_table::Cell::new(text)
    }
}

// ── Inline text helpers (crossterm) ──────────────────────────────────
pub fn agent_name(text: &str) -> String {
    if is_tty() {
        use crossterm::style::Stylize;
        text.cyan().bold().to_string()
    } else {
        text.to_string()
    }
}

pub fn code_ref(text: &str) -> String {
    if is_tty() {
        use crossterm::style::Stylize;
        format!(" {} ", text)
            .as_str()
            .yellow()
            .on(CODE_BG)
            .to_string()
    } else {
        format!("`{}`", text)
    }
}

pub fn input_badge(text: &str) -> String {
    if is_tty() {
        use crossterm::style::Stylize;
        format!(" {} ", text)
            .as_str()
            .yellow()
            .on(INPUT_BG)
            .to_string()
    } else {
        format!("\"{}\"", text)
    }
}

pub fn url(text: &str) -> String {
    if is_tty() {
        use crossterm::style::{Attribute, Color, ContentStyle, StyledContent};
        let style = ContentStyle {
            foreground_color: Some(Color::Cyan),
            attributes: Attribute::Underlined.into(),
            ..Default::default()
        };
        StyledContent::new(style, text).to_string()
    } else {
        text.to_string()
    }
}

pub fn dim(text: &str) -> String {
    if is_tty() {
        use crossterm::style::Stylize;
        text.dark_grey().to_string()
    } else {
        text.to_string()
    }
}

pub fn key_value(text: &str) -> String {
    if is_tty() {
        use crossterm::style::Stylize;
        text.bold().to_string()
    } else {
        text.to_string()
    }
}

pub fn error_prefix() -> String {
    if is_tty() {
        use crossterm::style::Stylize;
        "error:".red().bold().to_string()
    } else {
        "error:".to_string()
    }
}

pub fn separator(width: usize) -> String {
    let line = "\u{2500}".repeat(width);
    if is_tty() {
        use crossterm::style::Stylize;
        line.as_str().dark_grey().to_string()
    } else {
        line
    }
}

// ── Termimad skin ────────────────────────────────────────────────────
pub fn changelog_skin() -> termimad::MadSkin {
    let mut skin = termimad::MadSkin::default();
    use termimad::crossterm::style::{Attribute, Color as TColor};
    for h in &mut skin.headers {
        h.set_fg(TColor::Magenta);
        h.add_attr(Attribute::Bold);
        h.compound_style.remove_attr(Attribute::Underlined);
    }
    skin.bullet =
        termimad::StyledChar::from_fg_char(termimad::crossterm::style::Color::Magenta, '•');
    skin.inline_code
        .set_fg(termimad::crossterm::style::Color::Yellow);
    // Cannot use CODE_BG here: the project depends on crossterm 0.28 while
    // termimad 0.30 re-exports crossterm 0.29, so the Color types are distinct.
    skin.inline_code
        .set_bg(termimad::crossterm::style::Color::Rgb {
            r: 50,
            g: 40,
            b: 25,
        });
    skin
}

/// Post-process rendered text to apply cyan+underline to bare URLs.
/// Termimad has no link style field, so we regex-replace after rendering.
pub fn style_urls(text: &str) -> String {
    use crossterm::style::{Attribute, Color, ContentStyle, StyledContent};
    let url_re = regex::Regex::new(r"https?://[^\s)\]>]+").unwrap();
    url_re
        .replace_all(text, |caps: &regex::Captures| {
            let url_text = &caps[0];
            let style = ContentStyle {
                foreground_color: Some(Color::Cyan),
                attributes: Attribute::Underlined.into(),
                ..Default::default()
            };
            StyledContent::new(style, url_text).to_string()
        })
        .into_owned()
}
