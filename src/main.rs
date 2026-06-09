use axum::{
    body::Bytes,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use resvg::usvg;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};
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

const CODE_FONT_SIZE: f32 = 32.0;
const CODE_LH: f32 = 52.0;

// ── Theme constants ───────────────────────────────────────────────────────────
const COLOR_SURFACE: &str = "#f5f5f7";
const COLOR_CARD: &str = "#ffffff";
const COLOR_TEXT: &str = "#475569";
const COLOR_TEXT_MUTED: &str = "#86868b";
const COLOR_BORDER: &str = "#d2d2d7";
const COLOR_SEED: &str = "#ff9500"; // Orange

const COLOR_CODE_BG: &str = "#333333";
const COLOR_CODE_BORDER: &str = "#444444";

// ── Lazy-loaded global resources ──────────────────────────────────────────────
static SS: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static TS: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);
static MATH_SVG_CACHE: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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
    opts.insert(Options::ENABLE_MATH);

    let parser = Parser::new_ext(markdown, opts);
    let word_count = markdown.chars().filter(|c| !c.is_whitespace()).count();
    let mut b = SvgBuilder::new(IMAGE_WIDTH, word_count);

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
                    _ => 3,
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
            Event::Start(Tag::Emphasis) => {
                push_inline_svg(
                    in_quote,
                    &mut quote_buf,
                    &mut text_buf,
                    "<tspan font-style=\"italic\">",
                );
            }
            Event::End(TagEnd::Emphasis) => {
                push_inline_svg(in_quote, &mut quote_buf, &mut text_buf, "</tspan>");
            }
            Event::Start(Tag::Strong) => {
                push_inline_svg(
                    in_quote,
                    &mut quote_buf,
                    &mut text_buf,
                    "<tspan font-weight=\"700\">",
                );
            }
            Event::End(TagEnd::Strong) => {
                push_inline_svg(in_quote, &mut quote_buf, &mut text_buf, "</tspan>");
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
                } else {
                    push_inline_svg(in_quote, &mut quote_buf, &mut text_buf, &esc(&t));
                }
            }
            Event::Code(inline) => {
                let formatted = format!("<tspan fill=\"{COLOR_SEED}\"> {}</tspan>", esc(&inline));
                push_inline_svg(in_quote, &mut quote_buf, &mut text_buf, &formatted);
            }
            Event::InlineMath(math) => {
                let marker = format!("\u{E000}{}\u{E001}", hex_encode(math.as_bytes()));
                push_inline_svg(in_quote, &mut quote_buf, &mut text_buf, &marker);
            }
            Event::DisplayMath(math) => {
                if in_quote {
                    let formatted =
                        format!("\n<tspan fill=\"#0f766e\">$$ {} $$</tspan>\n", esc(&math));
                    quote_buf.push_str(&formatted);
                } else {
                    if has_visible_text(&text_buf) {
                        b.add_paragraph(&text_buf);
                        text_buf.clear();
                    }
                    b.add_math_block(&math);
                    in_para = false;
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_code {
                    code_buf.push('\n');
                } else if in_quote {
                    quote_buf.push('\n');
                } else {
                    text_buf.push('\n');
                }
            }
            Event::TaskListMarker(checked) => {
                let marker = if checked { "☑ " } else { "☐ " };
                push_inline_svg(in_quote, &mut quote_buf, &mut text_buf, marker);
            }
            Event::Rule => {
                b.add_rule();
            }
            _ => {}
        }
    }

    b.build()
}

fn push_inline_svg(in_quote: bool, quote_buf: &mut String, text_buf: &mut String, svg: &str) {
    if in_quote {
        quote_buf.push_str(svg);
    } else {
        text_buf.push_str(svg);
    }
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
            y: OUTER_PADDING + HEADER_TOP + HEADER_BOTTOM + 88.0,
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
        let available_w = self.text_area_width();
        let lines = wrap_text(text, available_w, font_size);

        if self.y > OUTER_PADDING + HEADER_TOP + HEADER_BOTTOM + 88.0 {
            self.y += if level == 1 { 18.0 } else { 42.0 };
        }

        // Avoid skew transforms on tall SVGs because skewX uses the global coordinate
        // system and can shift headings outside the left boundary as y grows.
        let skew = "";

        // Vertical bar for H1 and H2
        if level <= 2 {
            self.elems.push(format!(
                "<rect x=\"{}\" y=\"{}\" width=\"6\" height=\"{}\" rx=\"3\" fill=\"{COLOR_SEED}\"/>",
                PADDING - 24.0,
                self.y - font_size * 0.85,
                font_size * 1.1,
            ));
        }

        for line in &lines {
            self.elems.push(format!(
                "<text x=\"{PADDING}\" y=\"{}\" font-size=\"{font_size}\" fill=\"{fill}\" font-weight=\"700\" letter-spacing=\"0.02em\" {skew}>{}</text>",
                self.y,
                line,
            ));
            self.y += line_height;
        }
        self.y += 12.0;
    }

    // ── Paragraph ─────────────────────────────────────────────────────
    fn add_paragraph(&mut self, text: &str) {
        let available_w = self.text_area_width();
        let lines = wrap_text(text, available_w, BODY_FONT_SIZE);

        self.y += 6.0;
        for line in &lines {
            self.render_inline_line(
                PADDING,
                self.y,
                BODY_FONT_SIZE,
                COLOR_TEXT,
                "letter-spacing=\"0.7\"",
                line,
            );
            self.y += LINE_HEIGHT;
        }
        self.y += 6.0;
    }

    // ── List item ─────────────────────────────────────────────────────
    fn add_list_item(&mut self, text: &str, prefix: &str) {
        let available_w = self.text_area_width() - 44.0;
        let is_ord = ordered_prefix(prefix);
        let lines = wrap_text(text, available_w, BODY_FONT_SIZE);

        self.y += 8.0;
        for (i, line) in lines.iter().enumerate() {
            let text_x = PADDING + 44.0;
            if i == 0 {
                if is_ord {
                    self.elems.push(format!(
                        "<text x=\"{PADDING}\" y=\"{}\" font-size=\"{BODY_FONT_SIZE}\" fill=\"{COLOR_SEED}\" font-weight=\"700\">{}</text>",
                        self.y,
                        esc(prefix),
                    ));
                } else {
                    // "Ink dot" bullet: irregular circle
                    self.elems.push(format!(
                        "<ellipse cx=\"{}\" cy=\"{}\" rx=\"6\" ry=\"5\" fill=\"{COLOR_SEED}\" opacity=\"0.8\" transform=\"rotate(-15, {}, {})\"/>",
                        PADDING + 16.0,
                        self.y - BODY_FONT_SIZE * 0.35,
                        PADDING + 16.0,
                        self.y - BODY_FONT_SIZE * 0.35,
                    ));
                }
            }
            self.render_inline_line(
                text_x,
                self.y,
                BODY_FONT_SIZE,
                COLOR_TEXT,
                "letter-spacing=\"0.7\"",
                line,
            );
            self.y += LINE_HEIGHT;
        }
        self.y += 8.0;
    }

    // ── Block quote ───────────────────────────────────────────────────
    fn add_blockquote(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        self.y += 28.0; // 块前纵向留白

        let inner_x = PADDING + 44.0;
        let quote_pad_y = 36.0;
        let quote_w = self.text_area_width();
        let raw_lines = wrap_text(text, quote_w - 88.0, BODY_FONT_SIZE);
        let lines: Vec<String> = raw_lines
            .into_iter()
            .filter(|l| !l.trim().is_empty())
            .collect();

        if lines.is_empty() {
            return;
        }

        let block_h = lines.len() as f32 * LINE_HEIGHT + quote_pad_y * 2.0;
        let block_y = self.y;

        // 1. 绘制容器背景
        self.elems.push(format!(
            "<rect x=\"{PADDING}\" y=\"{block_y}\" width=\"{quote_w}\" height=\"{block_h}\" rx=\"24\" fill=\"#f8fafc\" stroke=\"#ffffff\" stroke-width=\"1\"/>"
        ));

        // 2. 绘制左上角引号。使用普通文本基线，避免 dominant-baseline 在 resvg 中产生视觉偏移。
        self.elems.push(format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"96\" font-weight=\"700\" fill=\"{COLOR_SEED}\" opacity=\"0.35\">&quot;</text>",
            PADDING + 22.0,
            block_y + 70.0,
        ));

        // 3. 绘制文字
        self.y += quote_pad_y + BODY_FONT_SIZE;
        for line in &lines {
            self.render_inline_line(
                inner_x,
                self.y,
                BODY_FONT_SIZE,
                "#374151",
                "font-style=\"italic\" letter-spacing=\"0.7\"",
                line,
            );
            self.y += LINE_HEIGHT;
        }

        // 4. 更新主 Y 轴指针
        self.y = block_y + block_h + 40.0;
    }

    // ── Inline rich text line ──────────────────────────────────────────
    fn render_inline_line(
        &mut self,
        x: f32,
        baseline_y: f32,
        font_size: f32,
        fill: &str,
        attrs: &str,
        line: &str,
    ) {
        let mut current_x = x;
        let mut rest = line;
        let start_marker = '\u{E000}';
        let end_marker = '\u{E001}';

        while let Some(start) = rest.find(start_marker) {
            let text = &rest[..start];
            current_x += self.render_text_run(current_x, baseline_y, font_size, fill, attrs, text);

            let after_start = &rest[start + start_marker.len_utf8()..];
            let Some(end) = after_start.find(end_marker) else {
                self.render_text_run(current_x, baseline_y, font_size, fill, attrs, after_start);
                return;
            };

            let hex = &after_start[..end];
            if let Some(latex) = hex_decode_string(hex) {
                if let Some((svg, w, h)) = inline_math_svg(&latex, font_size) {
                    let y = baseline_y - h * 0.78;
                    self.elems.push(format!(
                        "<svg x=\"{current_x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" viewBox=\"{}\" color=\"#0f766e\">{}</svg>",
                        svg.view_box, svg.inner
                    ));
                    current_x += w + font_size * 0.18;
                } else {
                    let fallback = format!("<tspan fill=\"#0f766e\">${}$</tspan>", esc(&latex));
                    current_x += self
                        .render_text_run(current_x, baseline_y, font_size, fill, attrs, &fallback);
                }
            }

            rest = &after_start[end + end_marker.len_utf8()..];
        }

        self.render_text_run(current_x, baseline_y, font_size, fill, attrs, rest);
    }

    fn render_text_run(
        &mut self,
        x: f32,
        baseline_y: f32,
        font_size: f32,
        fill: &str,
        attrs: &str,
        text: &str,
    ) -> f32 {
        if !has_visible_text(text) {
            return 0.0;
        }

        self.elems.push(format!(
            "<text x=\"{x}\" y=\"{baseline_y}\" font-size=\"{font_size}\" fill=\"{fill}\" {attrs}>{text}</text>"
        ));

        visible_units(text) * font_size
    }

    // ── Math block (MathJax-rendered SVG) ──────────────────────────────
    fn add_math_block(&mut self, latex: &str) {
        if latex.trim().is_empty() {
            return;
        }

        self.y += 28.0;
        let area_w = self.text_area_width();
        let max_w = area_w - 48.0;
        let fallback = format!(
            "<tspan fill=\"#0f766e\">$$ {} $$</tspan>",
            esc(latex.trim())
        );

        let options = mathjax_svg_rs::Options {
            font_size: 24.0,
            horizontal_align: mathjax_svg_rs::HorizontalAlign::Center,
        };
        let Ok(math_svg) = render_math_svg_cached(latex.trim(), &options) else {
            self.add_paragraph(&fallback);
            return;
        };

        let Some((vb_x, vb_y, vb_w, vb_h)) = svg_view_box(&math_svg) else {
            self.add_paragraph(&fallback);
            return;
        };

        let target_h = 54.0f32.max((vb_h / 1000.0) * 42.0);
        let natural_w = target_h * vb_w / vb_h;
        let target_w = natural_w.min(max_w);
        let target_h = target_w * vb_h / vb_w;
        let x = PADDING + (area_w - target_w) / 2.0;
        let y = self.y;
        let inner = svg_inner_content(&math_svg).unwrap_or(math_svg.as_str());

        self.elems.push(format!(
            "<svg x=\"{x}\" y=\"{y}\" width=\"{target_w}\" height=\"{target_h}\" viewBox=\"{vb_x} {vb_y} {vb_w} {vb_h}\" color=\"#0f766e\">{inner}</svg>"
        ));

        self.y += target_h + 36.0;
    }

    // ── Code block ────────────────────────────────────────────────────
    fn add_code_block(&mut self, code: &str, language: &str) {
        self.y += 26.0;

        let pad_x = 30.0;
        let chrome_h = 50.0;
        let pad_bottom = 20.0;
        let content_y_offset = 6.0;
        let block_w = self.text_area_width();
        let code_area_w = block_w - pad_x * 2.0;
        let max_code_units = code_area_w / (CODE_FONT_SIZE * 0.58);
        let wrapped_code = wrap_code_lines(code, max_code_units);
        let highlighted = highlight_code(&wrapped_code, language);
        let block_h = highlighted.len() as f32 * CODE_LH + chrome_h + pad_bottom + content_y_offset;

        // 1. Code block container
        self.elems.push(format!(
            "<rect x=\"{PADDING}\" y=\"{}\" width=\"{block_w}\" height=\"{block_h}\" rx=\"12\" fill=\"{COLOR_CODE_BG}\" stroke=\"{COLOR_CODE_BORDER}\" stroke-width=\"1\"/>",
            self.y,
        ));

        // Mac style three dots
        let dot_y = self.y + 22.0;
        self.elems.push(format!(
            "<circle cx=\"{}\" cy=\"{dot_y}\" r=\"6\" fill=\"#ff5f56\"/><circle cx=\"{}\" cy=\"{dot_y}\" r=\"6\" fill=\"#ffbd2e\"/><circle cx=\"{}\" cy=\"{dot_y}\" r=\"6\" fill=\"#27c93f\"/>",
            PADDING + 24.0,
            PADDING + 44.0,
            PADDING + 64.0,
        ));

        if !language.is_empty() {
            self.elems.push(format!(
                "<text x=\"{}\" y=\"{}\" font-size=\"14\" fill=\"#8b949e\" text-anchor=\"end\" font-weight=\"600\" letter-spacing=\"1\">{}</text>",
                PADDING + block_w - 30.0,
                self.y + 35.0,
                esc(&language.to_uppercase()),
            ));
        }

        // 2. Render highlighted code lines
        let mut current_code_y = self.y + chrome_h + CODE_FONT_SIZE * 0.72 + content_y_offset;

        for tokens in &highlighted {
            let tspans: String = tokens
                .iter()
                .map(|(c, t)| format!("<tspan fill=\"{c}\">{}</tspan>", esc(t)))
                .collect();

            self.elems.push(format!(
                "<text x=\"{}\" y=\"{current_code_y}\" font-size=\"{CODE_FONT_SIZE}\" xml:space=\"preserve\">{tspans}</text>",
                PADDING + pad_x,
            ));

            current_code_y += CODE_LH;
        }

        self.y += block_h + 72.0;
    }

    // ── Horizontal rule ───────────────────────────────────────────────
    fn add_rule(&mut self) {
        self.y += 40.0;
        self.elems.push(format!(
            "<line x1=\"{PADDING}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{COLOR_BORDER}\" stroke-width=\"2\" stroke-dasharray=\"8 8\"/>",
            self.y,
            self.w as f32 - PADDING,
            self.y,
        ));
        self.y += 48.0;
    }

    // ── Finalise SVG document ─────────────────────────────────────────
    fn build(&self) -> String {
        let footer_top = self.y + 40.0;
        let h = (footer_top + 100.0 + OUTER_PADDING).max(220.0) as u32;
        let card_w = self.w as f32 - OUTER_PADDING * 2.0;
        let card_h = h as f32 - OUTER_PADDING * 2.0;

        let mut svg =
            String::with_capacity(self.elems.iter().map(String::len).sum::<usize>() + 4096);
        svg.push_str(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{}\" height=\"{h}\" viewBox=\"0 0 {} {h}\">",
            self.w, self.w,
        ));

        // Definitions for gradients and shadows
        svg.push_str(&format!(
            r#"
    <defs>
    <filter id="shadow" x="-20%" y="-20%" width="140%" height="140%">
        <feGaussianBlur in="SourceAlpha" stdDeviation="10" result="blur"/>
        <feOffset in="blur" dx="0" dy="20" result="offsetBlur"/>
        <feComponentTransfer>
            <feFuncA type="linear" slope="0.08"/>
        </feComponentTransfer>
        <feMerge>
            <feMergeNode/>
            <feMergeNode in="SourceGraphic"/>
        </feMerge>
    </filter>
    </defs>
    "#
        ));

        svg.push_str(&format!(
            "<rect width=\"100%\" height=\"100%\" fill=\"{COLOR_SURFACE}\"/>"
        ));

        let card_filter = if h > 6000 {
            ""
        } else {
            " filter=\"url(#shadow)\""
        };

        // Card with shadow. Very tall images skip the blur filter because it is expensive over a huge area.
        svg.push_str(&format!(
            "<rect x=\"{OUTER_PADDING}\" y=\"{OUTER_PADDING}\" width=\"{card_w}\" height=\"{card_h}\" rx=\"24\" fill=\"{COLOR_CARD}\"{card_filter}/>"
        ));

        svg.push_str(
            "<style>text{font-family:'LXGW WenKai','Microsoft YaHei','SimHei','Noto Sans CJK SC',sans-serif;}</style>",
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
        let now = chrono::Local::now();
        let date_str = now.format("%Y.%m.%d").to_string();
        let header_y = OUTER_PADDING + HEADER_TOP + 10.0;
        let font_stack =
            "font-family:'LXGW WenKai Mono','LXGWWenKaiMono','Microsoft YaHei','SimHei','Noto Sans CJK SC',sans-serif";
        format!(
            "<circle cx=\"{}\" cy=\"{}\" r=\"4\" fill=\"{COLOR_SEED}\"/><text x=\"{}\" y=\"{header_y}\" font-size=\"25\" font-weight=\"700\" fill=\"{COLOR_TEXT_MUTED}\" stroke=\"{COLOR_TEXT_MUTED}\" stroke-width=\"0.8\" style=\"{font_stack}\" letter-spacing=\"1.2\">{}</text><text x=\"{}\" y=\"{header_y}\" font-size=\"25\" font-weight=\"700\" fill=\"{COLOR_TEXT_MUTED}\" stroke=\"{COLOR_TEXT_MUTED}\" stroke-width=\"0.8\" style=\"{font_stack}\" letter-spacing=\"1.2\" text-anchor=\"end\">{}</text>",
            PADDING,
            header_y - 7.0,
            PADDING + 20.0,
            date_str,
            self.w as f32 - PADDING,
            esc(&words),
        )
    }

    fn footer_svg(&self, y: f32) -> String {
        let center = self.w as f32 / 2.0;
        format!(
            "<line x1=\"{PADDING}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{COLOR_BORDER}\" stroke-width=\"1\" opacity=\"0.45\"/><circle cx=\"{}\" cy=\"{}\" r=\"5\" fill=\"{COLOR_SEED}\" opacity=\"0.8\" transform=\"rotate(0, {}, {})\"/><circle cx=\"{}\" cy=\"{}\" r=\"5\" fill=\"{COLOR_SEED}\" opacity=\"0.8\" transform=\"rotate(120, {}, {})\"/><circle cx=\"{}\" cy=\"{}\" r=\"5\" fill=\"{COLOR_SEED}\" opacity=\"0.8\" transform=\"rotate(240, {}, {})\"/>",
            y,
            self.w as f32 - PADDING,
            y,
            center - 24.0,
            y + 40.0,
            center - 24.0, y + 40.0,
            center,
            y + 40.0,
            center, y + 40.0,
            center + 24.0,
            y + 40.0,
            center + 24.0, y + 40.0,
        )
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
///
/// The input may contain trusted inline SVG `<tspan>` fragments generated by the
/// Markdown parser. Tags are ignored for width calculation and are kept balanced
/// when a visual line wraps.
fn wrap_text(text: &str, max_pixel_width: f32, font_size: f32) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }

    let mut out = Vec::new();

    for para in text.split('\n') {
        if para.is_empty() {
            out.push(String::new());
            continue;
        }

        let mut line = String::new();
        let mut current_line_width = 0.0f32;
        let mut open_tags: Vec<String> = Vec::new();
        let chars: Vec<char> = para.chars().collect();
        let mut i = 0;
        let start_marker = '\u{E000}';
        let end_marker = '\u{E001}';

        while i < chars.len() {
            if chars[i] == start_marker {
                if let Some(end) = chars[i + 1..].iter().position(|&ch| ch == end_marker) {
                    let marker: String = chars[i..=i + end + 1].iter().collect();
                    let hex: String = chars[i + 1..i + 1 + end].iter().collect();
                    let marker_width = hex_decode_string(&hex)
                        .map(|latex| {
                            latex.chars().map(char_visual_width).sum::<f32>() * font_size * 0.9
                        })
                        .unwrap_or(4.0 * font_size);

                    if current_line_width + marker_width > max_pixel_width
                        && has_visible_text(&line)
                    {
                        let mut finished = line.trim_end().to_string();
                        for _ in open_tags.iter().rev() {
                            finished.push_str("</tspan>");
                        }
                        out.push(finished);

                        line.clear();
                        for tag in &open_tags {
                            line.push_str(tag);
                        }
                        current_line_width = 0.0;
                    }

                    line.push_str(&marker);
                    current_line_width += marker_width;
                    i += end + 2;
                    continue;
                }
            }

            if chars[i] == '<' {
                if let Some(end) = chars[i..].iter().position(|&ch| ch == '>') {
                    let tag: String = chars[i..=i + end].iter().collect();
                    if tag.starts_with("<tspan") && !tag.ends_with("/>") {
                        open_tags.push(tag.clone());
                    } else if tag == "</tspan>" {
                        open_tags.pop();
                    }
                    line.push_str(&tag);
                    i += end + 1;
                    continue;
                }
            }

            let ch = chars[i];
            let ch_width = char_visual_width(ch) * font_size;

            if ch.is_whitespace() {
                if !line.ends_with(' ') && has_visible_text(&line) {
                    line.push(' ');
                    current_line_width += ch_width;
                }
                i += 1;
                continue;
            }

            if current_line_width + ch_width > max_pixel_width
                && has_visible_text(&line)
                && !is_no_line_start_punctuation(ch)
            {
                let mut finished = line.trim_end().to_string();
                for _ in open_tags.iter().rev() {
                    finished.push_str("</tspan>");
                }
                out.push(finished);

                line.clear();
                for tag in &open_tags {
                    line.push_str(tag);
                }
                current_line_width = 0.0;
            }

            line.push(ch);
            current_line_width += ch_width;
            i += 1;
        }

        if has_visible_text(&line) {
            out.push(line.trim_end().to_string());
        }
    }

    if out.is_empty() {
        out.push(String::new());
    }
    out
}

struct InlineMathSvg {
    view_box: String,
    inner: String,
}

fn render_math_svg_cached(
    latex: &str,
    options: &mathjax_svg_rs::Options,
) -> Result<String, String> {
    let align = match options.horizontal_align {
        mathjax_svg_rs::HorizontalAlign::Left => "left",
        mathjax_svg_rs::HorizontalAlign::Center => "center",
        mathjax_svg_rs::HorizontalAlign::Right => "right",
    };
    let key = format!("{}|{align}|{latex}", options.font_size);

    if let Some(svg) = MATH_SVG_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).cloned())
    {
        return Ok(svg);
    }

    let svg = mathjax_svg_rs::render_tex(latex, options)?;
    if let Ok(mut cache) = MATH_SVG_CACHE.lock() {
        if cache.len() > 512 {
            cache.clear();
        }
        cache.insert(key, svg.clone());
    }
    Ok(svg)
}

fn inline_math_svg(latex: &str, font_size: f32) -> Option<(InlineMathSvg, f32, f32)> {
    let options = mathjax_svg_rs::Options {
        font_size: font_size as f64,
        horizontal_align: mathjax_svg_rs::HorizontalAlign::Left,
    };
    let math_svg = render_math_svg_cached(latex.trim(), &options).ok()?;
    let (vb_x, vb_y, vb_w, vb_h) = svg_view_box(&math_svg)?;
    let inner = svg_inner_content(&math_svg)?.to_string();

    // MathJax uses a large internal viewBox. Scale it to align naturally with
    // surrounding body text.
    let target_h = font_size * 1.15;
    let target_w = (target_h * vb_w / vb_h).max(font_size * 0.8);

    Some((
        InlineMathSvg {
            view_box: format!("{vb_x} {vb_y} {vb_w} {vb_h}"),
            inner,
        },
        target_w,
        target_h,
    ))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode_string(hex: &str) -> Option<String> {
    if hex.len() % 2 != 0 {
        return None;
    }

    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let mut chars = hex.bytes();
    while let (Some(hi), Some(lo)) = (chars.next(), chars.next()) {
        let hi = hex_value(hi)?;
        let lo = hex_value(lo)?;
        bytes.push((hi << 4) | lo);
    }

    String::from_utf8(bytes).ok()
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn svg_view_box(svg: &str) -> Option<(f32, f32, f32, f32)> {
    let marker = "viewBox=\"";
    let start = svg.find(marker)? + marker.len();
    let end = svg[start..].find('"')? + start;
    let nums: Vec<f32> = svg[start..end]
        .split_whitespace()
        .filter_map(|n| n.parse::<f32>().ok())
        .collect();

    match nums.as_slice() {
        [x, y, w, h] if *w > 0.0 && *h > 0.0 => Some((*x, *y, *w, *h)),
        _ => None,
    }
}

fn svg_inner_content(svg: &str) -> Option<&str> {
    let start = svg.find('>')? + 1;
    let end = svg.rfind("</svg>")?;
    if start <= end {
        Some(&svg[start..end])
    } else {
        None
    }
}

fn visible_units(s: &str) -> f32 {
    let mut units = 0.0;
    let mut in_tag = false;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            '<' => in_tag = true,
            '>' => in_tag = false,
            '\u{E000}' => {
                if let Some(end) = chars[i + 1..].iter().position(|&ch| ch == '\u{E001}') {
                    let hex: String = chars[i + 1..i + 1 + end].iter().collect();
                    if let Some(latex) = hex_decode_string(&hex) {
                        units += latex.chars().map(visual_units).sum::<f32>() * 0.9;
                    }
                    i += end + 2;
                    continue;
                }
            }
            ch if !in_tag => units += visual_units(ch),
            _ => {}
        }
        i += 1;
    }

    units
}

fn has_visible_text(s: &str) -> bool {
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag && !ch.is_whitespace() => return true,
            _ => {}
        }
    }
    false
}

fn wrap_code_lines(code: &str, max_units: f32) -> String {
    let mut out = String::new();

    for raw_line in code.lines() {
        let indent: String = raw_line
            .chars()
            .take_while(|ch| ch.is_whitespace())
            .collect();
        let continuation_indent = format!("{indent}  ");
        let mut line = String::new();
        let mut units = 0.0f32;

        for ch in raw_line.chars() {
            let ch_units = visual_units(ch);
            if units + ch_units > max_units && !line.trim().is_empty() {
                out.push_str(line.trim_end());
                out.push('\n');
                line.clear();
                line.push_str(&continuation_indent);
                units = visible_units(&continuation_indent);
            }

            line.push(ch);
            units += ch_units;
        }

        out.push_str(line.trim_end());
        out.push('\n');
    }

    if out.ends_with('\n') {
        out.pop();
    }
    out
}

fn visual_units(ch: char) -> f32 {
    char_visual_width(ch)
}

fn char_visual_width(ch: char) -> f32 {
    if (ch as u32) < 32 {
        0.0
    } else if ch.is_ascii() {
        match ch {
            ' ' => 0.40,
            'A'..='Z' => 0.53,
            'a'..='z' | '0'..='9' => 0.50,
            _ => 0.45,
        }
    } else if is_cjk_punctuation(ch) || is_cjk(ch) {
        1.00
    } else {
        0.9
    }
}

fn is_cjk_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '，' | '。'
            | '！'
            | '？'
            | '：'
            | '；'
            | '（'
            | '）'
            | '【'
            | '】'
            | '“'
            | '”'
            | '、'
            | '《'
            | '》'
            | '「'
            | '」'
            | '『'
            | '』'
            | '—'
            | '…'
    )
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x4E00..=0x9FFF
            | 0x3400..=0x4DBF
            | 0x3000..=0x303F
            | 0xFF00..=0xFFEF
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
