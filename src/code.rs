use crate::globals::{SS, TS};

pub(crate) fn highlight_code(code: &str, language: &str) -> Vec<Vec<(String, String)>> {
    let syntax = SS
        .find_syntax_by_token(language)
        .unwrap_or_else(|| SS.find_syntax_plain_text());

    let theme = TS
        .themes
        .get("base16-ocean.dark")
        .or_else(|| TS.themes.values().next())
        .unwrap();

    let mut h = syntect::easy::HighlightLines::new(syntax, theme);

    code.lines()
        .map(|line| {
            let ranges = h
                .highlight_line(line, &SS)
                .unwrap_or_else(|_| vec![(syntect::highlighting::Style::default(), line)]);
            ranges
                .into_iter()
                .map(|(style, text)| {
                    let c = style.foreground;
                    (
                        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b),
                        text.to_string(),
                    )
                })
                .collect()
        })
        .collect()
}

pub(crate) fn wrap_highlighted_code_lines(
    highlighted: Vec<Vec<(String, String)>>,
    max_pixel_width: f32,
    font_size: f32,
) -> Vec<Vec<(String, String)>> {
    let mut out = Vec::new();

    for tokens in highlighted {
        let plain_line: String = tokens.iter().map(|(_, text)| text.as_str()).collect();
        let indent: String = plain_line
            .replace('\t', "    ")
            .chars()
            .take_while(|ch| ch.is_whitespace())
            .collect();
        let continuation_width = code_text_width(&indent, font_size);

        let mut line_tokens: Vec<(String, String)> = Vec::new();
        let mut line_plain = String::new();
        let mut width = 0.0f32;
        let mut current_text = String::new();

        for (color, text) in tokens {
            for ch in text.chars() {
                let mut buf = [0; 4];
                let (ch_str, text_width) = if ch == '\t' {
                    ("    ", code_char_width(' ', font_size) * 4.0)
                } else {
                    (ch.encode_utf8(&mut buf) as &str, code_char_width(ch, font_size))
                };

                if width + text_width > max_pixel_width && !line_plain.trim().is_empty() {
                    if !current_text.is_empty() {
                        push_code_token(&mut line_tokens, color.clone(), std::mem::take(&mut current_text));
                    }
                    trim_code_line_end(&mut line_tokens, &mut line_plain);
                    out.push(line_tokens);

                    line_tokens = Vec::new();
                    line_plain = String::new();
                    width = 0.0;

                    if !indent.is_empty() {
                        push_code_token(&mut line_tokens, color.clone(), indent.clone());
                        line_plain.push_str(&indent);
                        width = continuation_width;
                    }
                }

                current_text.push_str(ch_str);
                line_plain.push_str(ch_str);
                width += text_width;
            }

            if !current_text.is_empty() {
                push_code_token(&mut line_tokens, color, std::mem::take(&mut current_text));
            }
        }

        trim_code_line_end(&mut line_tokens, &mut line_plain);
        out.push(line_tokens);
    }

    out
}

fn push_code_token(tokens: &mut Vec<(String, String)>, color: String, text: String) {
    if text.is_empty() {
        return;
    }

    if let Some((last_color, last_text)) = tokens.last_mut() {
        if *last_color == color {
            last_text.push_str(&text);
            return;
        }
    }

    tokens.push((color, text));
}

fn trim_code_line_end(tokens: &mut Vec<(String, String)>, plain: &mut String) {
    while plain.ends_with(' ') {
        plain.pop();
        if let Some((_, text)) = tokens.last_mut() {
            text.pop();
            if text.is_empty() {
                tokens.pop();
            }
        } else {
            break;
        }
    }
}

pub(crate) fn code_text_width(text: &str, font_size: f32) -> f32 {
    text.chars().map(|ch| code_char_width(ch, font_size)).sum()
}

fn code_char_width(ch: char, font_size: f32) -> f32 {
    let unit = if is_ascii_printable(ch) {
        0.54
    } else if is_cjk_punctuation(ch) || is_cjk(ch) {
        1.0
    } else {
        0.9
    };
    unit * font_size
}

fn is_ascii_printable(ch: char) -> bool {
    matches!(ch as u32, 0x0020..=0x007E)
}

fn is_cjk_punctuation(ch: char) -> bool {
    matches!(ch as u32, 0x3000..=0x303F | 0xFF00..=0xFFEF)
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF)
}
