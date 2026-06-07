use axum::{
    body::Bytes,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use resvg::usvg;
use std::sync::{Arc, LazyLock};
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

// ── Layout constants ──────────────────────────────────────────────────────────
const IMAGE_WIDTH: u32 = 1200;
const OUTER_PADDING: f32 = 24.0;
const CARD_PADDING: f32 = 60.0;
const PADDING: f32 = OUTER_PADDING + CARD_PADDING;
const HEADER_TOP: f32 = 60.0;
const HEADER_BOTTOM: f32 = 20.0;
const BODY_FONT_SIZE: f32 = 34.0;
const LINE_HEIGHT: f32 = 64.0;
const H1_SIZE: f32 = 74.0;
const H1_LH: f32 = 104.0;
const H2_SIZE: f32 = 54.0;
const H2_LH: f32 = 74.0;
const H3_SIZE: f32 = 44.0;
const H3_LH: f32 = 62.0;
const CODE_FONT_SIZE: f32 = 26.0;
const CODE_LH: f32 = 38.0;

// ── Theme constants ───────────────────────────────────────────────────────────
const COLOR_SURFACE: &str = "#f5f5f7";
const COLOR_CARD: &str = "#ffffff";
const COLOR_TEXT: &str = "#475569";
const COLOR_TEXT_MUTED: &str = "#86868b";
const COLOR_BORDER: &str = "#d2d2d7";
const COLOR_SEED: &str = "#ff9500";
const COLOR_CODE_BG: &str = "#333333";
const COLOR_CODE_BORDER: &str = "#444444";

// ── Lazy-loaded global resources ──────────────────────────────────────────────
static SS: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static TS: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

/// Build usvg options once (loads system fonts for CJK support).
static USVG_OPTS: LazyLock<usvg::Options<'static>> = LazyLock::new(|| {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut opt = usvg::Options::default();
    opt.fontdb = Arc::new(db);
    opt
});

// ── Entry point ───────────────────────────────────────────────────────────────
#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/", get(|| async { "Markdown-to-PNG Service is running" }))
        .route("/generate", post(generate_handler));

    let port = std::env::var("PORT").unwrap_or_else(|_| "3001".to_string());
    let addr = format!("0.0.0.0:{port}");
    println!("Server listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

// ── HTTP handler ──────────────────────────────────────────────────────────────
async fn generate_handler(body: Bytes) -> Response {
    let markdown = String::from_utf8_lossy(&body).to_string();
    let svg = markdown_to_svg(&markdown);

    match svg_to_png(&svg) {
        Ok(png) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "image/png")
            .body(axum::body::Body::from(png))
            .unwrap()
            .into_response(),
        Err(e) => {
            eprintln!("[ERROR] render failed: {e}");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(axum::body::Body::from(format!("render error: {e}")))
                .unwrap()
                .into_response()
        }
    }
}

// ── Markdown → SVG ────────────────────────────────────────────────────────────
fn markdown_to_svg(markdown: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_SMART_PUNCTUATION);

    let parser = Parser::new_ext(markdown, opts);

    let word_count = markdown.chars().filter(|c| !c.is_whitespace()).count();
    let mut b = SvgBuilder::new(IMAGE_WIDTH, word_count);

    // State machine for block-level elements
    let mut in_code = false;
    let mut code_lang = String::new();
    let mut code_buf = String::new();

    let mut in_heading = false;
    let mut heading_lvl: u8 = 0;

    let mut in_para = false;
    let mut text_buf = String::new();

    let mut in_list = false;
    let mut ordered = false;
    let mut list_idx: u64 = 0;

    let mut in_quote = false;
    let mut quote_buf = String::new();

    for event in parser {
        match event {
            // ── Code blocks ───────────────────────────────────────────
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code = true;
                code_lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.trim().to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                code_buf.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                b.add_code_block(&code_buf, &code_lang);
                code_buf.clear();
                code_lang.clear();
            }

            // ── Headings ─────────────────────────────────────────────
            Event::Start(Tag::Heading { level, .. }) => {
                in_heading = true;
                heading_lvl = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                text_buf.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                if in_heading {
                    let (fs, lh) = match heading_lvl {
                        1 => (H1_SIZE, H1_LH),
                        2 => (H2_SIZE, H2_LH),
                        _ => (H3_SIZE, H3_LH),
                    };
                    b.add_heading(&text_buf, fs, lh, heading_lvl);
                    text_buf.clear();
                    in_heading = false;
                }
            }

            // ── Block quotes ────────────────────────────────────────────
            Event::Start(Tag::BlockQuote(_)) => {
                in_quote = true;
                quote_buf.clear();
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                if in_quote {
                    b.add_blockquote(quote_buf.trim());
                    quote_buf.clear();
                    in_quote = false;
                }
            }

            // ── Paragraphs ────────────────────────────────────────────
            Event::Start(Tag::Paragraph) => {
                in_para = true;
                if !in_quote {
                    text_buf.clear();
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if in_quote {
                    if !quote_buf.ends_with('\n') {
                        quote_buf.push('\n');
                    }
                    text_buf.clear();
                    in_para = false;
                } else if in_para {
                    b.add_paragraph(&text_buf);
                    text_buf.clear();
                    in_para = false;
                }
            }

            // ── Lists ─────────────────────────────────────────────────
            Event::Start(Tag::List(start)) => {
                in_list = true;
                ordered = start.is_some();
                list_idx = start.unwrap_or(0);
            }
            Event::End(TagEnd::List(_)) => {
                in_list = false;
            }
            Event::Start(Tag::Item) => {
                text_buf.clear();
            }
            Event::End(TagEnd::Item) => {
                if in_list {
                    let prefix = if ordered {
                        let s = format!("{list_idx}. ");
                        list_idx += 1;
                        s
                    } else {
                        "• ".to_string()
                    };
                    b.add_list_item(&text_buf, &prefix);
                    text_buf.clear();
                }
            }

            // ── Leaf events ───────────────────────────────────────────
            Event::Text(t) => {
                if in_code {
                    code_buf.push_str(&t);
                } else if in_quote {
                    quote_buf.push_str(&t);
                } else {
                    text_buf.push_str(&t);
                }
            }
            Event::Code(inline) => {
                if in_quote {
                    quote_buf.push('`');
                    quote_buf.push_str(&inline);
                    quote_buf.push('`');
                } else {
                    text_buf.push('`');
                    text_buf.push_str(&inline);
                    text_buf.push('`');
                }
            }
            Event::SoftBreak => {
                if in_code {
                    code_buf.push('\n');
                } else if in_quote {
                    quote_buf.push(' ');
                } else {
                    text_buf.push(' ');
                }
            }
            Event::HardBreak => {
                if in_code {
                    code_buf.push('\n');
                } else if in_quote {
                    quote_buf.push('\n');
                } else {
                    text_buf.push('\n');
                }
            }
            Event::TaskListMarker(checked) => {
                text_buf.push_str(if checked { "☑ " } else { "☐ " });
            }
            Event::Rule => {
                b.add_rule();
            }
            _ => {}
        }
    }

    b.build()
}

// ── SVG → PNG (resvg pipeline) ────────────────────────────────────────────────
fn svg_to_png(svg: &str) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let tree = usvg::Tree::from_str(svg, &USVG_OPTS)?;

    let size = tree.size().to_int_size();
    let mut pixmap =
        tiny_skia::Pixmap::new(size.width(), size.height()).ok_or("pixmap too large")?;

    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());

    Ok(pixmap.encode_png()?)
}

// ═══════════════════════════════════════════════════════════════════════════════
//  SVG Builder
// ═══════════════════════════════════════════════════════════════════════════════

struct SvgBuilder {
    elems: Vec<String>,
    y: f32,
    w: u32,
    word_count: usize,
}

impl SvgBuilder {
    fn new(width: u32, word_count: usize) -> Self {
        Self {
            elems: Vec::new(),
            y: OUTER_PADDING + HEADER_TOP + HEADER_BOTTOM + 58.0,
            w: width,
            word_count,
        }
    }

    fn text_area_width(&self) -> f32 {
        self.w as f32 - 2.0 * PADDING
    }

    // ── Heading ───────────────────────────────────────────────────────
    fn add_heading(&mut self, text: &str, font_size: f32, line_height: f32, level: u8) {
        let fill = match level {
            1 => "#000000",
            2 => "#111111",
            _ => "#333333",
        };
        let max_chars = self.max_chars_proportional(font_size);
        let lines = wrap_text(text, max_chars);

        if self.y > OUTER_PADDING + HEADER_TOP + HEADER_BOTTOM + 58.0 {
            self.y += if level == 1 { 18.0 } else { 42.0 };
        }

        if level == 2 {
            self.elems.push(format!(
                "<rect x=\"{}\" y=\"{}\" width=\"6\" height=\"{}\" rx=\"4\" fill=\"{COLOR_SEED}\"/>",
                PADDING - 24.0,
                self.y - font_size * 0.75,
                font_size * 0.9,
            ));
        }

        for line in &lines {
            self.elems.push(format!(
                "<text x=\"{PADDING}\" y=\"{}\" font-size=\"{font_size}\" fill=\"{fill}\" font-weight=\"700\" letter-spacing=\"1\">{}</text>",
                self.y,
                esc(line),
            ));
            self.y += line_height;
        }

        self.y += match level {
            1 => 12.0,
            2 => 8.0,
            _ => 4.0,
        };
    }

    // ── Paragraph ─────────────────────────────────────────────────────
    fn add_paragraph(&mut self, text: &str) {
        let max_chars = self.max_chars_proportional(BODY_FONT_SIZE);
        let lines = wrap_text(text, max_chars);

        self.y += 6.0;
        for (i, line) in lines.iter().enumerate() {
            let justify_attrs = if i + 1 < lines.len() && !line.is_empty() {
                format!(
                    " textLength=\"{}\" lengthAdjust=\"spacing\"",
                    self.text_area_width()
                )
            } else {
                String::new()
            };
            self.elems.push(format!(
                "<text x=\"{PADDING}\" y=\"{}\" font-size=\"{BODY_FONT_SIZE}\" fill=\"{COLOR_TEXT}\" letter-spacing=\"0.7\"{justify_attrs}>{}</text>",
                self.y,
                esc(line),
            ));
            self.y += LINE_HEIGHT;
        }
        self.y += 6.0;
    }

    // ── List item ─────────────────────────────────────────────────────
    fn add_list_item(&mut self, text: &str, prefix: &str) {
        let max_chars = self.max_chars_proportional(BODY_FONT_SIZE);
        let text = if ordered_prefix(prefix) {
            format!("{prefix}{text}")
        } else {
            text.to_string()
        };
        let lines = wrap_text(&text, max_chars);

        for (i, line) in lines.iter().enumerate() {
            let x = if i == 0 {
                PADDING + 24.0
            } else {
                PADDING + 44.0
            };
            if i == 0 && !ordered_prefix(prefix) {
                self.elems.push(format!(
                    "<circle cx=\"{}\" cy=\"{}\" r=\"5\" fill=\"{COLOR_SEED}\" opacity=\"0.85\"/>",
                    PADDING + 5.0,
                    self.y - BODY_FONT_SIZE * 0.35,
                ));
            }
            self.elems.push(format!(
                "<text x=\"{x}\" y=\"{}\" font-size=\"{BODY_FONT_SIZE}\" fill=\"{COLOR_TEXT}\" letter-spacing=\"0.7\">{}</text>",
                self.y,
                esc(line),
            ));
            self.y += LINE_HEIGHT;
        }
    }

    // ── Block quote ───────────────────────────────────────────────────
    fn add_blockquote(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        self.y += 28.0;

        let inner_x = PADDING + 44.0;
        let quote_pad_y = 36.0;
        let quote_w = self.text_area_width();
        let max_chars = ((quote_w - 88.0) / (BODY_FONT_SIZE * 0.55)) as usize;
        let lines = wrap_text(text, max_chars);
        let block_h = lines.len() as f32 * LINE_HEIGHT + quote_pad_y * 2.0;
        let block_y = self.y;

        self.elems.push(format!(
            "<rect x=\"{PADDING}\" y=\"{block_y}\" width=\"{quote_w}\" height=\"{block_h}\" rx=\"24\" fill=\"#f8fafc\" stroke=\"#ffffff\" stroke-width=\"1\"/>"
        ));
        self.elems.push(format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"96\" font-weight=\"700\" fill=\"{COLOR_SEED}\" opacity=\"0.35\">&quot;</text>",
            PADDING + 22.0,
            block_y + 70.0,
        ));

        self.y += quote_pad_y + BODY_FONT_SIZE;
        for line in &lines {
            self.elems.push(format!(
                "<text x=\"{inner_x}\" y=\"{}\" font-size=\"{BODY_FONT_SIZE}\" fill=\"#374151\" font-style=\"italic\" letter-spacing=\"0.7\">{}</text>",
                self.y,
                esc(line),
            ));
            self.y += LINE_HEIGHT;
        }

        self.y = block_y + block_h + 40.0;
    }

    // ── Code block (syntect-highlighted) ──────────────────────────────
    fn add_code_block(&mut self, code: &str, language: &str) {
        self.y += 26.0;

        let highlighted = highlight_code(code, language);
        let pad_x = 30.0;
        let chrome_h = 50.0;
        let pad_bottom = 34.0;
        let block_h = highlighted.len() as f32 * CODE_LH + chrome_h + pad_bottom;

        // Background
        self.elems.push(format!(
            "<rect x=\"{PADDING}\" y=\"{}\" width=\"{}\" height=\"{block_h}\" rx=\"12\" fill=\"{COLOR_CODE_BG}\" stroke=\"{COLOR_CODE_BORDER}\" stroke-width=\"1\"/>",
            self.y,
            self.text_area_width(),
        ));

        // Window control dots. Keep syntax highlighting, but do not show the language label.
        self.elems.push(format!(
            "<circle cx=\"{}\" cy=\"{}\" r=\"6\" fill=\"#ff5f56\"/><circle cx=\"{}\" cy=\"{}\" r=\"6\" fill=\"#ffbd2e\"/><circle cx=\"{}\" cy=\"{}\" r=\"6\" fill=\"#27c93f\"/>",
            PADDING + 24.0,
            self.y + 22.0,
            PADDING + 44.0,
            self.y + 22.0,
            PADDING + 64.0,
            self.y + 22.0,
        ));

        // Highlighted lines
        let code_top = self.y + chrome_h + CODE_FONT_SIZE * 0.72;
        for (i, tokens) in highlighted.iter().enumerate() {
            let y = code_top + i as f32 * CODE_LH;
            if tokens.is_empty() {
                self.elems.push(format!(
                    "<text x=\"{}\" y=\"{y}\" font-size=\"{CODE_FONT_SIZE}\" font-family=\"'0xProto Nerd Font Mono',Consolas,Courier New,monospace\"> </text>",
                    PADDING + pad_x,
                ));
            } else {
                let tspans: String = tokens
                    .iter()
                    .map(|(c, t)| format!("<tspan fill=\"{c}\">{}</tspan>", esc(t)))
                    .collect();
                self.elems.push(format!(
                    "<text x=\"{}\" y=\"{y}\" font-size=\"{CODE_FONT_SIZE}\" font-family=\"'0xProto Nerd Font Mono',Consolas,Courier New,monospace\">{tspans}</text>",
                    PADDING + pad_x,
                ));
            }
        }

        self.y += block_h + 72.0;
    }

    // ── Horizontal rule ───────────────────────────────────────────────
    fn add_rule(&mut self) {
        self.y += 42.0;
        self.elems.push(format!(
            "<line x1=\"{PADDING}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{COLOR_SEED}\" stroke-width=\"2\" opacity=\"0.6\"/>",
            self.y,
            self.w as f32 - PADDING,
            self.y,
        ));
        self.y += 70.0;
    }

    // ── Finalise SVG document ─────────────────────────────────────────
    fn build(&self) -> String {
        let footer_top = self.y + 30.0;
        let h = (footer_top + 80.0 + OUTER_PADDING).max(260.0) as u32;
        let card_w = self.w as f32 - OUTER_PADDING * 2.0;
        let card_h = h as f32 - OUTER_PADDING * 2.0;
        let mut svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{h}\" viewBox=\"0 0 {} {h}\">",
            self.w, self.w,
        );
        svg.push_str(&format!(
            "<rect width=\"100%\" height=\"100%\" fill=\"{COLOR_SURFACE}\"/>"
        ));
        svg.push_str(&format!(
            "<rect x=\"{OUTER_PADDING}\" y=\"{OUTER_PADDING}\" width=\"{card_w}\" height=\"{card_h}\" rx=\"24\" fill=\"{COLOR_CARD}\"/>"
        ));
        // Font stack covers Windows / Linux / macOS CJK fonts
        svg.push_str(
            "<style>text{font-family:'0xProto Nerd Font Mono',Microsoft YaHei,SimHei,Noto Sans CJK SC,WenQuanYi Micro Hei,PingFang SC,Hiragino Sans GB,monospace,sans-serif;}</style>",
        );
        svg.push_str(&self.header_svg());
        for e in &self.elems {
            svg.push_str(e);
        }
        svg.push_str(&self.footer_svg(footer_top));
        svg.push_str("</svg>");
        svg
    }

    fn header_svg(&self) -> String {
        let words = if self.word_count >= 1000 {
            format!("{:.1}k WORDS", self.word_count as f32 / 1000.0)
        } else {
            format!("{} WORDS", self.word_count)
        };
        format!(
            "<circle cx=\"{}\" cy=\"{}\" r=\"4\" fill=\"{COLOR_SEED}\"/><text x=\"{}\" y=\"{}\" font-size=\"25\" font-weight=\"600\" fill=\"{COLOR_TEXT_MUTED}\" letter-spacing=\"1.2\">MARKDOWN</text><text x=\"{}\" y=\"{}\" font-size=\"25\" font-weight=\"600\" fill=\"{COLOR_TEXT_MUTED}\" letter-spacing=\"1.2\" text-anchor=\"end\">{}</text>",
            PADDING,
            OUTER_PADDING + HEADER_TOP - 7.0,
            PADDING + 20.0,
            OUTER_PADDING + HEADER_TOP,
            self.w as f32 - PADDING,
            OUTER_PADDING + HEADER_TOP,
            esc(&words),
        )
    }

    fn footer_svg(&self, y: f32) -> String {
        let center = self.w as f32 / 2.0;
        format!(
            "<line x1=\"{PADDING}\" y1=\"{y}\" x2=\"{}\" y2=\"{y}\" stroke=\"{COLOR_BORDER}\" stroke-width=\"2\" opacity=\"0.5\"/><circle cx=\"{}\" cy=\"{}\" r=\"5\" fill=\"{COLOR_SEED}\" opacity=\"0.85\"/><circle cx=\"{}\" cy=\"{}\" r=\"5\" fill=\"{COLOR_SEED}\" opacity=\"0.85\"/><circle cx=\"{}\" cy=\"{}\" r=\"5\" fill=\"{COLOR_SEED}\" opacity=\"0.85\"/>",
            self.w as f32 - PADDING,
            center - 18.0,
            y + 40.0,
            center,
            y + 40.0,
            center + 18.0,
            y + 40.0,
        )
    }

    // Helper: approximate max chars per line for a proportional font
    fn max_chars_proportional(&self, font_size: f32) -> usize {
        (self.text_area_width() / (font_size * 0.55)) as usize
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Syntect code highlighting → Vec<Vec<(hex_color, text)>>
// ═══════════════════════════════════════════════════════════════════════════════

fn highlight_code(code: &str, language: &str) -> Vec<Vec<(String, String)>> {
    let syntax = SS
        .find_syntax_by_token(language)
        .unwrap_or_else(|| SS.find_syntax_plain_text());

    let theme = TS
        .themes
        .get("base16-ocean.dark")
        .or_else(|| TS.themes.values().next())
        .unwrap();

    let mut h = HighlightLines::new(syntax, theme);

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

// ═══════════════════════════════════════════════════════════════════════════════
//  Text helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// Break `text` into lines that fit the estimated visual width.
fn wrap_text(text: &str, max: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }

    let max_units = max as f32;
    let mut out = Vec::new();

    for para in text.split('\n') {
        if para.is_empty() {
            out.push(String::new());
            continue;
        }

        let mut line = String::new();
        let mut units = 0.0f32;

        for ch in para.chars() {
            let ch_units = visual_units(ch);

            if ch.is_whitespace() {
                if !line.ends_with(' ') && !line.is_empty() {
                    line.push(' ');
                    units += ch_units;
                }
                continue;
            }

            if units + ch_units > max_units && !line.is_empty() && !is_no_line_start_punctuation(ch)
            {
                out.push(line.trim_end().to_string());
                line.clear();
                units = 0.0;
            }

            line.push(ch);
            units += ch_units;
        }

        if !line.is_empty() {
            out.push(line.trim_end().to_string());
        }
    }

    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn visual_units(ch: char) -> f32 {
    if is_cjk(ch) {
        1.85
    } else if ch.is_ascii_punctuation() {
        0.7
    } else {
        1.0
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x4E00..=0x9FFF   // CJK Unified Ideographs
            | 0x3400..=0x4DBF // CJK Extension A
            | 0x3000..=0x303F // CJK Symbols and Punctuation
            | 0xFF00..=0xFFEF // Fullwidth forms
    )
}

fn is_no_line_start_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '，' | '。'
            | '、'
            | '；'
            | '：'
            | '！'
            | '？'
            | '）'
            | '】'
            | '》'
            | '」'
            | '』'
            | '”'
            | '’'
            | ','
            | '.'
            | ';'
            | ':'
            | '!'
            | '?'
            | ')'
            | ']'
            | '}'
    )
}

/// Returns true for ordered list prefixes such as "1. ".
fn ordered_prefix(prefix: &str) -> bool {
    prefix
        .trim_end()
        .strip_suffix('.')
        .is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()))
}

/// Escape XML special characters for safe SVG embedding.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
