use crate::ast::{Inline, Node, TableAlignment};
use crate::code::{highlight_code, wrap_highlighted_code_lines};
use crate::constants::*;
use crate::math::{inline_math_svg, render_math_svg_cached, svg_inner_content, svg_view_box};
use crate::text::{
    layout_rich_lines as layout_text_lines, measure_runs_width, push_text_run,
    runs_have_visible_text, split_runs_on_newlines, svg_tspan_for_run, LayoutLine, TextRun,
    TextStyle,
};
use crate::util::esc;
use std::collections::HashMap;

// ═══════════════════════════════════════════════════════════════════════════════
//  Inline → TextRun conversion
// ═══════════════════════════════════════════════════════════════════════════════

pub(crate) fn inlines_to_runs(inlines: &[Inline]) -> Vec<TextRun> {
    let mut runs = Vec::new();
    for inline in inlines {
        match inline {
            Inline::Text(s) => push_text_run(&mut runs, s, TextStyle::Normal),
            Inline::Bold(s) => push_text_run(&mut runs, s, TextStyle::Bold),
            Inline::Italic(s) => push_text_run(&mut runs, s, TextStyle::Italic),
            Inline::Code(s) => push_text_run(&mut runs, format!(" {s} "), TextStyle::Code),
            Inline::Math(s) => runs.push(TextRun::new(s, TextStyle::Math)),
        }
    }
    runs
}

fn layout_rich_lines(
    runs: &[TextRun],
    max_width: f32,
    font_size: f32,
    line_height: f32,
) -> Vec<LayoutLine> {
    if !runs.iter().any(|run| run.style == TextStyle::Math) {
        return layout_text_lines(runs, max_width, font_size, line_height);
    }

    layout_rich_lines_with_atomic_math(runs, max_width, font_size)
}

fn layout_rich_lines_with_atomic_math(
    runs: &[TextRun],
    max_width: f32,
    font_size: f32,
) -> Vec<LayoutLine> {
    let mut out = Vec::new();
    let mut char_widths = HashMap::new();

    for logical_runs in split_runs_on_newlines(runs) {
        if logical_runs.is_empty() || !runs_have_visible_text(&logical_runs) {
            out.push(LayoutLine {
                runs: Vec::new(),
                width: 0.0,
            });
            continue;
        }

        let mut line_runs = Vec::new();
        let mut line_width = 0.0f32;

        for run in logical_runs {
            if run.style == TextStyle::Math {
                for (math_run, run_width) in math_layout_runs(&run, font_size, max_width) {
                    if line_width + run_width > max_width && runs_have_visible_text(&line_runs) {
                        push_layout_line(
                            &mut out,
                            std::mem::take(&mut line_runs),
                            font_size,
                        );
                        line_width = 0.0;
                    }

                    line_runs.push(math_run);
                    line_width += run_width;
                }
                continue;
            }

            for ch in run.text.chars() {
                let ch_width = text_char_width(ch, run.style, font_size, &mut char_widths);
                if line_width + ch_width > max_width && runs_have_visible_text(&line_runs) {
                    push_layout_line(
                        &mut out,
                        std::mem::take(&mut line_runs),
                        font_size,
                    );
                    line_width = 0.0;
                }

                let mut buf = [0; 4];
                push_text_run(&mut line_runs, ch.encode_utf8(&mut buf), run.style);
                line_width += ch_width;
            }
        }

        push_layout_line(&mut out, line_runs, font_size);
    }

    if out.is_empty() {
        out.push(LayoutLine {
            runs: Vec::new(),
            width: 0.0,
        });
    }

    out
}

fn push_layout_line(
    out: &mut Vec<LayoutLine>,
    runs: Vec<TextRun>,
    font_size: f32,
) {
    let width = measure_line_width_with_atomic_math(&runs, font_size);
    out.push(LayoutLine { runs, width });
}

fn measure_line_width_with_atomic_math(runs: &[TextRun], font_size: f32) -> f32 {
    let mut width = 0.0f32;
    let mut text_group = Vec::new();

    for run in runs {
        if run.style == TextStyle::Math {
            if !text_group.is_empty() {
                width += measure_runs_width(&text_group, font_size);
                text_group.clear();
            }
            width += math_run_width(run, font_size);
        } else {
            push_text_run(&mut text_group, &run.text, run.style);
        }
    }

    if !text_group.is_empty() {
        width += measure_runs_width(&text_group, font_size);
    }

    width
}

fn math_run_width(run: &TextRun, font_size: f32) -> f32 {
    inline_math_render(&run.text, font_size, run.math_scale)
        .map(|(_, width, _)| width + font_size * 0.18)
        .unwrap_or_else(|| {
            measure_runs_width(
                &[TextRun::new(format!("${}$", run.text), TextStyle::Math)],
                font_size,
            )
        })
}

fn math_layout_runs(run: &TextRun, font_size: f32, max_width: f32) -> Vec<(TextRun, f32)> {
    let full_width = math_run_width(run, font_size);
    if full_width <= max_width {
        return vec![(run.clone(), full_width)];
    }

    let Some(scale) = inline_math_scale(&run.text, font_size) else {
        return vec![(run.clone(), full_width)];
    };
    let pieces = split_latex_for_wrap(&run.text);
    if pieces.len() <= 1 {
        return vec![(run.clone(), full_width)];
    }

    pieces
        .into_iter()
        .map(|piece| {
            let mut run = TextRun::new(piece, TextStyle::Math);
            run.math_scale = Some(scale);
            let width = math_run_width(&run, font_size);
            (run, width)
        })
        .collect()
}

fn inline_math_scale(latex: &str, font_size: f32) -> Option<f32> {
    let options = mathjax_svg_rs::Options {
        font_size: font_size as f64,
        horizontal_align: mathjax_svg_rs::HorizontalAlign::Left,
    };
    let math_svg = render_math_svg_cached(latex.trim(), &options).ok()?;
    let (_, _, _, vb_h) = svg_view_box(&math_svg)?;
    Some(font_size * 1.15 / vb_h)
}

fn inline_math_render(
    latex: &str,
    font_size: f32,
    scale: Option<f32>,
) -> Option<(crate::math::InlineMathSvg, f32, f32)> {
    let Some(scale) = scale else {
        return inline_math_svg(latex, font_size);
    };

    let options = mathjax_svg_rs::Options {
        font_size: font_size as f64,
        horizontal_align: mathjax_svg_rs::HorizontalAlign::Left,
    };
    let math_svg = render_math_svg_cached(latex.trim(), &options).ok()?;
    let (vb_x, vb_y, vb_w, vb_h) = svg_view_box(&math_svg)?;
    let inner = svg_inner_content(&math_svg)?.to_string();

    Some((
        crate::math::InlineMathSvg {
            view_box: format!("{vb_x} {vb_y} {vb_w} {vb_h}"),
            inner,
        },
        vb_w * scale,
        vb_h * scale,
    ))
}

fn split_latex_for_wrap(latex: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut escaped = false;

    for (idx, ch) in latex.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '{' => depth += 1,
            '}' => depth = (depth - 1).max(0),
            '=' | '+' if depth == 0 => {
                let end = idx + ch.len_utf8();
                push_latex_piece(&mut pieces, &latex[start..end]);
                start = end;
            }
            '-' if depth == 0 && idx > start => {
                let end = idx + ch.len_utf8();
                push_latex_piece(&mut pieces, &latex[start..end]);
                start = end;
            }
            _ => {}
        }
    }

    push_latex_piece(&mut pieces, &latex[start..]);
    pieces
}

fn push_latex_piece(pieces: &mut Vec<String>, piece: &str) {
    let piece = piece.trim();
    if !piece.is_empty() {
        pieces.push(piece.to_string());
    }
}

fn text_char_width(
    ch: char,
    style: TextStyle,
    font_size: f32,
    cache: &mut HashMap<(TextStyle, char), f32>,
) -> f32 {
    *cache.entry((style, ch)).or_insert_with(|| {
        let mut buf = [0; 4];
        measure_runs_width(&[TextRun::new(ch.encode_utf8(&mut buf), style)], font_size)
    })
}

fn nodes_have_visible_content(nodes: &[Node]) -> bool {
    nodes.iter().any(node_has_visible_content)
}

fn node_has_visible_content(node: &Node) -> bool {
    match node {
        Node::Heading { content, .. } | Node::Paragraph(content) => {
            inlines_have_visible_content(content)
        }
        Node::Quote { children } | Node::ListItem { children } => {
            nodes_have_visible_content(children)
        }
        Node::List { items, .. } => nodes_have_visible_content(items),
        Node::CodeBlock { content, .. } => !content.trim().is_empty(),
        Node::MathBlock { latex } => !latex.trim().is_empty(),
        Node::Table { header, rows, .. } => header
            .iter()
            .chain(rows.iter().flat_map(|row| row.iter()))
            .any(|cell| inlines_have_visible_content(cell)),
        Node::Rule => true,
    }
}

struct TableCellLayout {
    lines: Vec<LayoutLine>,
    line_widths: Vec<f32>,
    align: TableAlignment,
}

struct TableLayout {
    col_widths: Vec<f32>,
    row_heights: Vec<f32>,
    rows: Vec<Vec<TableCellLayout>>,
    header_rows: usize,
    width: f32,
}

const TABLE_FONT_SIZE: f32 = 28.0;
const TABLE_LINE_HEIGHT: f32 = 44.0;
const TABLE_CELL_PAD_X: f32 = 18.0;
const TABLE_CELL_PAD_Y: f32 = 16.0;
const MATH_BLOCK_TOP_GAP: f32 = 10.0;
const MATH_BLOCK_BOTTOM_GAP: f32 = 20.0;
const MATH_BLOCK_MIN_HEIGHT: f32 = BODY_FONT_SIZE * 1.1;

fn layout_table(
    available_w: f32,
    alignments: &[TableAlignment],
    header: &[Vec<Inline>],
    rows: &[Vec<Vec<Inline>>],
) -> Option<TableLayout> {
    struct SourceCell {
        runs: Vec<TextRun>,
        align: TableAlignment,
    }

    let col_count = alignments
        .len()
        .max(header.len())
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));

    if col_count == 0 || available_w <= 0.0 {
        return None;
    }

    let align_for_col = |idx: usize| alignments.get(idx).copied().unwrap_or(TableAlignment::None);

    let mut source_rows: Vec<Vec<SourceCell>> = Vec::new();
    if !header.is_empty() {
        source_rows.push(
            (0..col_count)
                .map(|idx| SourceCell {
                    runs: header.get(idx).map_or_else(Vec::new, |cell| {
                        let mut runs = inlines_to_runs(cell);
                        for run in &mut runs {
                            if run.style == TextStyle::Normal {
                                run.style = TextStyle::Bold;
                            }
                        }
                        runs
                    }),
                    align: align_for_col(idx),
                })
                .collect(),
        );
    }

    for row in rows {
        source_rows.push(
            (0..col_count)
                .map(|idx| SourceCell {
                    runs: row
                        .get(idx)
                        .map_or_else(Vec::new, |cell| inlines_to_runs(cell)),
                    align: align_for_col(idx),
                })
                .collect(),
        );
    }

    if source_rows.is_empty() {
        return None;
    }

    let mut ideal_widths = vec![0.0f32; col_count];
    for row in &source_rows {
        for (idx, cell) in row.iter().enumerate() {
            let text_w = if runs_have_visible_text(&cell.runs) {
                measure_runs_width(&cell.runs, TABLE_FONT_SIZE)
            } else {
                TABLE_FONT_SIZE
            };
            ideal_widths[idx] = ideal_widths[idx].max(text_w + TABLE_CELL_PAD_X * 2.0);
        }
    }

    let width_limit = available_w.max(col_count as f32 * 44.0);
    let min_col_w = (width_limit / col_count as f32).min(110.0).max(44.0);
    let mut col_widths: Vec<f32> = ideal_widths
        .into_iter()
        .map(|width| width.max(min_col_w))
        .collect();
    let ideal_total: f32 = col_widths.iter().sum();

    if ideal_total > width_limit {
        let min_total = min_col_w * col_count as f32;
        if min_total >= width_limit {
            col_widths.fill(width_limit / col_count as f32);
        } else {
            let scale = (width_limit - min_total) / (ideal_total - min_total);
            for width in &mut col_widths {
                *width = min_col_w + (*width - min_col_w) * scale;
            }
        }
    }

    let mut laid_out_rows = Vec::with_capacity(source_rows.len());
    let mut row_heights = Vec::with_capacity(source_rows.len());

    for row in source_rows {
        let mut row_layout = Vec::with_capacity(col_count);
        let mut row_h = 0.0f32;

        for (idx, cell) in row.into_iter().enumerate() {
            let content_w = (col_widths[idx] - TABLE_CELL_PAD_X * 2.0).max(12.0);
            let lines: Vec<LayoutLine> = if runs_have_visible_text(&cell.runs) {
                layout_rich_lines(&cell.runs, content_w, TABLE_FONT_SIZE, TABLE_LINE_HEIGHT)
                    .into_iter()
                    .filter(|line| runs_have_visible_text(&line.runs))
                    .collect()
            } else {
                Vec::new()
            };
            let line_widths = lines.iter().map(|line| line.width).collect::<Vec<_>>();
            row_h =
                row_h.max(lines.len().max(1) as f32 * TABLE_LINE_HEIGHT + TABLE_CELL_PAD_Y * 2.0);
            row_layout.push(TableCellLayout {
                lines,
                line_widths,
                align: cell.align,
            });
        }

        row_heights.push(row_h.max(68.0));
        laid_out_rows.push(row_layout);
    }

    let width = col_widths.iter().sum();
    Some(TableLayout {
        col_widths,
        row_heights,
        rows: laid_out_rows,
        header_rows: usize::from(!header.is_empty()),
        width,
    })
}

fn aligned_table_text_x(x: f32, available_w: f32, line_w: f32, align: TableAlignment) -> f32 {
    match align {
        TableAlignment::Center => x + (available_w - line_w).max(0.0) / 2.0,
        TableAlignment::Right => x + (available_w - line_w).max(0.0),
        TableAlignment::None | TableAlignment::Left => x,
    }
}

fn inlines_have_visible_content(inlines: &[Inline]) -> bool {
    inlines.iter().any(|inline| match inline {
        Inline::Text(s)
        | Inline::Bold(s)
        | Inline::Italic(s)
        | Inline::Code(s)
        | Inline::Math(s) => !s.trim().is_empty(),
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Layout context for recursive rendering
// ═══════════════════════════════════════════════════════════════════════════════

pub(crate) struct LayoutContext {
    pub(crate) quote_depth: usize,
    pub(crate) list_depth: usize,
}

// ═══════════════════════════════════════════════════════════════════════════════
//  SVG Builder
// ═══════════════════════════════════════════════════════════════════════════════

pub(crate) struct SvgBuilder {
    elems: Vec<String>,
    y: f32,
    w: u32,
    word_count: usize,
    has_rendered_block: bool,
    pending_block_gap: f32,
}

impl SvgBuilder {
    pub(crate) fn new(width: u32, word_count: usize) -> Self {
        Self {
            elems: Vec::new(),
            y: OUTER_PADDING + HEADER_TOP + HEADER_BOTTOM + 88.0,
            w: width,
            word_count,
            has_rendered_block: false,
            pending_block_gap: 0.0,
        }
    }

    fn text_area_width(&self) -> f32 {
        self.w as f32 - 2.0 * PADDING
    }

    fn apply_block_top_gap(&mut self, top_gap: f32) {
        let gap = if self.has_rendered_block {
            self.pending_block_gap.max(top_gap)
        } else {
            top_gap
        };
        self.y += gap;
        self.pending_block_gap = 0.0;
        self.has_rendered_block = true;
    }

    fn set_block_bottom_gap(&mut self, bottom_gap: f32) {
        self.pending_block_gap = self.pending_block_gap.max(bottom_gap);
    }

    fn flush_pending_block_gap(&mut self) {
        self.y += self.pending_block_gap;
        self.pending_block_gap = 0.0;
    }

    // ── Heading ───────────────────────────────────────────────────────
    fn add_heading(&mut self, runs: &[TextRun], font_size: f32, line_height: f32, level: u8) {
        let fill = match level {
            1 => "#000000",
            2 => "#111111",
            _ => "#333333",
        };
        let available_w = self.text_area_width();
        let lines = layout_rich_lines(runs, available_w, font_size, line_height);

        let top_gap = if self.has_rendered_block {
            if level == 1 {
                18.0
            } else {
                42.0
            }
        } else {
            0.0
        };
        self.apply_block_top_gap(top_gap);

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
            self.render_rich_line(
                PADDING,
                available_w,
                self.y,
                font_size,
                fill,
                "font-weight=\"700\" letter-spacing=\"0.02em\"",
                &line.runs,
            );
            self.y += line_height;
        }
        self.set_block_bottom_gap(12.0);
    }

    // ── Paragraph ─────────────────────────────────────────────────────
    fn add_paragraph(&mut self, runs: &[TextRun]) {
        if !runs_have_visible_text(runs) {
            return;
        }

        let available_w = self.text_area_width();
        let lines = layout_rich_lines(runs, available_w, BODY_FONT_SIZE, LINE_HEIGHT);

        self.apply_block_top_gap(6.0);
        for line in &lines {
            self.render_rich_line(
                PADDING,
                available_w,
                self.y,
                BODY_FONT_SIZE,
                COLOR_TEXT,
                "letter-spacing=\"0.7\"",
                &line.runs,
            );
            self.y += LINE_HEIGHT;
        }
        self.set_block_bottom_gap(6.0);
    }

    // ── Inline rich text line ──────────────────────────────────────────
    fn render_rich_line(
        &mut self,
        x: f32,
        _max_width: f32,
        baseline_y: f32,
        font_size: f32,
        fill: &str,
        attrs: &str,
        runs: &[TextRun],
    ) {
        if !runs.iter().any(|run| run.style == TextStyle::Math) {
            self.push_text_group(x, baseline_y, font_size, fill, attrs, runs);
            return;
        }

        let mut current_x = x;
        let mut text_group: Vec<TextRun> = Vec::new();

        for run in runs {
            if run.text.is_empty() {
                continue;
            }

            if run.style == TextStyle::Math {
                current_x += self.render_text_group(
                    current_x,
                    baseline_y,
                    font_size,
                    fill,
                    attrs,
                    &text_group,
                );
                text_group.clear();

                if let Some((svg, w, h)) = inline_math_render(&run.text, font_size, run.math_scale)
                {
                    let y = baseline_y - h * 0.78;
                    self.elems.push(format!(
                        "<svg x=\"{current_x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" viewBox=\"{}\" color=\"#0f766e\">{}</svg>",
                        svg.view_box, svg.inner
                    ));
                    current_x += w + font_size * 0.18;
                } else {
                    let fallback = TextRun::new(format!("${}$", run.text), TextStyle::Math);
                    current_x += self.render_text_group(
                        current_x,
                        baseline_y,
                        font_size,
                        fill,
                        attrs,
                        &[fallback],
                    );
                }
            } else {
                text_group.push(run.clone());
            }
        }

        self.render_text_group(current_x, baseline_y, font_size, fill, attrs, &text_group);
    }

    fn push_text_group(
        &mut self,
        x: f32,
        baseline_y: f32,
        font_size: f32,
        fill: &str,
        attrs: &str,
        runs: &[TextRun],
    ) {
        if !runs_have_visible_text(runs) {
            return;
        }

        let tspans: String = runs
            .iter()
            .filter(|run| !run.text.is_empty())
            .map(|run| {
                let (tspan_attrs, text) = svg_tspan_for_run(run);
                format!("<tspan {tspan_attrs}>{}</tspan>", esc(&text))
            })
            .collect();

        self.elems.push(format!(
            "<text x=\"{x}\" y=\"{baseline_y}\" font-size=\"{font_size}\" fill=\"{fill}\" {attrs}>{tspans}</text>"
        ));
    }

    fn render_text_group(
        &mut self,
        x: f32,
        baseline_y: f32,
        font_size: f32,
        fill: &str,
        attrs: &str,
        runs: &[TextRun],
    ) -> f32 {
        if !runs_have_visible_text(runs) {
            return 0.0;
        }

        self.push_text_group(x, baseline_y, font_size, fill, attrs, runs);
        measure_runs_width(runs, font_size)
    }

    // ── Math block (MathJax-rendered SVG) ──────────────────────────────
    fn add_math_block(&mut self, latex: &str) {
        if latex.trim().is_empty() {
            return;
        }

        let area_w = self.text_area_width();
        let max_w = area_w - 48.0;
        let fallback_runs = [TextRun::new(
            format!("$$ {} $$", latex.trim()),
            TextStyle::Math,
        )];

        let mj_font_size = BODY_FONT_SIZE as f64;
        let options = mathjax_svg_rs::Options {
            font_size: mj_font_size,
            horizontal_align: mathjax_svg_rs::HorizontalAlign::Center,
        };
        let Ok(math_svg) = render_math_svg_cached(latex.trim(), &options) else {
            self.add_paragraph(&fallback_runs);
            return;
        };

        let Some((vb_x, vb_y, vb_w, vb_h)) = svg_view_box(&math_svg) else {
            self.add_paragraph(&fallback_runs);
            return;
        };

        self.apply_block_top_gap(MATH_BLOCK_TOP_GAP);

        let scale_factor = BODY_FONT_SIZE / 1000.0;
        let natural_h = vb_h * scale_factor;
        let natural_w = vb_w * scale_factor;

        let (render_w, render_h) = if natural_w > max_w {
            let shrink = max_w / natural_w;
            (max_w, natural_h * shrink)
        } else {
            (natural_w, natural_h)
        };

        let (svg_y, block_h) = if render_h < MATH_BLOCK_MIN_HEIGHT {
            let offset = (MATH_BLOCK_MIN_HEIGHT - render_h) / 2.0;
            (self.y + offset, MATH_BLOCK_MIN_HEIGHT)
        } else {
            (self.y, render_h)
        };

        let x = PADDING + (area_w - render_w) / 2.0;
        let inner = svg_inner_content(&math_svg).unwrap_or(math_svg.as_str());

        self.elems.push(format!(
            "<svg x=\"{x}\" y=\"{svg_y}\" width=\"{render_w}\" height=\"{render_h}\" viewBox=\"{vb_x} {vb_y} {vb_w} {vb_h}\" color=\"#0f766e\">{inner}</svg>"
        ));

        self.y += block_h;
        self.set_block_bottom_gap(MATH_BLOCK_BOTTOM_GAP);
    }

    // ── Code block ────────────────────────────────────────────────────
    fn add_code_block(&mut self, code: &str, language: &str) {
        self.apply_block_top_gap(16.0);

        let pad_x = 30.0;
        let chrome_h = 50.0;
        let pad_bottom = 20.0;
        let content_y_offset = 6.0;
        let block_w = self.text_area_width();
        let code_area_w = block_w - pad_x * 2.0;
        let highlighted = wrap_highlighted_code_lines(
            highlight_code(code, language),
            code_area_w,
            CODE_FONT_SIZE,
        );
        let block_h = highlighted.len() as f32 * CODE_LH + chrome_h + pad_bottom + content_y_offset;

        self.elems.push(format!(
            "<rect x=\"{PADDING}\" y=\"{}\" width=\"{block_w}\" height=\"{block_h}\" rx=\"12\" fill=\"{COLOR_CODE_BG}\" stroke=\"{COLOR_CODE_BORDER}\" stroke-width=\"1\"/>",
            self.y,
        ));

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

        let mut current_code_y = self.y + chrome_h + CODE_FONT_SIZE * 0.72 + content_y_offset;

        for tokens in &highlighted {
            let tspans: String = tokens
                .iter()
                .map(|(c, t)| format!("<tspan fill=\"{c}\">{}</tspan>", esc(t)))
                .collect();

            self.elems.push(format!(
                "<text x=\"{}\" y=\"{current_code_y}\" font-size=\"{CODE_FONT_SIZE}\" style=\"font-family:'LXGW WenKai Mono','LXGWWenKaiMono','Microsoft YaHei','SimHei','Noto Sans CJK SC',sans-serif\" xml:space=\"preserve\">{tspans}</text>",
                PADDING + pad_x,
            ));

            current_code_y += CODE_LH;
        }

        self.y += block_h;
        self.set_block_bottom_gap(72.0);
    }

    // ── Horizontal rule ───────────────────────────────────────────────
    fn add_rule(&mut self) {
        self.apply_block_top_gap(0.0);
        self.elems.push(format!(
            "<line x1=\"{PADDING}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{COLOR_BORDER}\" stroke-width=\"2\" stroke-dasharray=\"8 8\"/>",
            self.y,
            self.w as f32 - PADDING,
            self.y,
        ));
        self.set_block_bottom_gap(64.0);
    }

    // ── Table ────────────────────────────────────────────────────────
    fn add_table(
        &mut self,
        x: f32,
        available_w: f32,
        alignments: &[TableAlignment],
        header: &[Vec<Inline>],
        rows: &[Vec<Vec<Inline>>],
    ) {
        let Some(layout) = layout_table(available_w, alignments, header, rows) else {
            return;
        };

        self.apply_block_top_gap(22.0);
        let table_x = x;
        let table_y = self.y;
        let table_h: f32 = layout.row_heights.iter().sum();

        self.elems.push(format!(
            "<rect x=\"{table_x}\" y=\"{table_y}\" width=\"{}\" height=\"{table_h}\" rx=\"18\" fill=\"#ffffff\" stroke=\"#e5e7eb\" stroke-width=\"1.5\"/>",
            layout.width,
        ));

        if layout.header_rows > 0 {
            let header_h: f32 = layout.row_heights.iter().take(layout.header_rows).sum();
            self.elems.push(format!(
                "<path d=\"M {} {table_y} H {} Q {} {table_y} {} {} V {} H {table_x} V {} Q {table_x} {table_y} {} {table_y} Z\" fill=\"#fff7ed\"/>",
                table_x + 18.0,
                table_x + layout.width - 18.0,
                table_x + layout.width,
                table_x + layout.width,
                table_y + 18.0,
                table_y + header_h,
                table_y + 18.0,
                table_x + 18.0,
            ));
        }

        let mut row_y = table_y;
        for (row_idx, row_h) in layout.row_heights.iter().enumerate() {
            if row_idx >= layout.header_rows && (row_idx - layout.header_rows) % 2 == 1 {
                self.elems.push(format!(
                    "<rect x=\"{table_x}\" y=\"{row_y}\" width=\"{}\" height=\"{row_h}\" fill=\"#f8fafc\" opacity=\"0.72\"/>",
                    layout.width,
                ));
            }

            row_y += row_h;
            if row_idx + 1 < layout.row_heights.len() {
                self.elems.push(format!(
                    "<line x1=\"{table_x}\" y1=\"{row_y}\" x2=\"{}\" y2=\"{row_y}\" stroke=\"#e5e7eb\" stroke-width=\"1\"/>",
                    table_x + layout.width,
                ));
            }
        }

        let mut col_x = table_x;
        for width in layout
            .col_widths
            .iter()
            .take(layout.col_widths.len().saturating_sub(1))
        {
            col_x += width;
            self.elems.push(format!(
                "<line x1=\"{col_x}\" y1=\"{table_y}\" x2=\"{col_x}\" y2=\"{}\" stroke=\"#edf2f7\" stroke-width=\"1\"/>",
                table_y + table_h,
            ));
        }

        let mut cell_y = table_y;
        for (row_idx, row) in layout.rows.iter().enumerate() {
            let mut cell_x = table_x;
            let row_h = layout.row_heights[row_idx];
            let is_header = row_idx < layout.header_rows;

            for (col_idx, cell) in row.iter().enumerate() {
                let col_w = layout.col_widths[col_idx];
                let text_w = col_w - TABLE_CELL_PAD_X * 2.0;
                let text_h = (cell.lines.len().max(1) as f32) * TABLE_LINE_HEIGHT;
                let mut baseline_y = cell_y
                    + TABLE_CELL_PAD_Y
                    + (row_h - TABLE_CELL_PAD_Y * 2.0 - text_h) / 2.0
                    + TABLE_FONT_SIZE;

                for (line, line_w) in cell.lines.iter().zip(&cell.line_widths) {
                    let text_x = aligned_table_text_x(
                        cell_x + TABLE_CELL_PAD_X,
                        text_w,
                        *line_w,
                        cell.align,
                    );

                    self.render_rich_line(
                        text_x,
                        text_w,
                        baseline_y,
                        TABLE_FONT_SIZE,
                        if is_header { "#1f2937" } else { COLOR_TEXT },
                        if is_header {
                            "font-weight=\"700\" letter-spacing=\"0.35\""
                        } else {
                            "letter-spacing=\"0.35\""
                        },
                        &line.runs,
                    );
                    baseline_y += TABLE_LINE_HEIGHT;
                }

                cell_x += col_w;
            }

            cell_y += row_h;
        }

        self.y = table_y + table_h;
        self.set_block_bottom_gap(42.0);
    }

    // ── Finalise SVG document ─────────────────────────────────────────
    pub(crate) fn build(&self) -> String {
        let footer_top = self.y + self.pending_block_gap.max(40.0);
        let h = (footer_top + 100.0 + OUTER_PADDING).max(220.0) as u32;
        let card_w = self.w as f32 - OUTER_PADDING * 2.0;
        let card_h = h as f32 - OUTER_PADDING * 2.0;

        let mut svg =
            String::with_capacity(self.elems.iter().map(String::len).sum::<usize>() + 4096);
        svg.push_str(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{}\" height=\"{h}\" viewBox=\"0 0 {} {h}\">",
            self.w, self.w,
        ));

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
        let dot_radius = 4.0;
        let dot_cx = PADDING + dot_radius;
        let date_x = dot_cx + dot_radius + 8.0;
        let words_x = self.w as f32 - PADDING + 3.5;

        format!(
            "<circle cx=\"{dot_cx}\" cy=\"{}\" r=\"{dot_radius}\" fill=\"{COLOR_SEED}\"/><text x=\"{date_x}\" y=\"{header_y}\" font-size=\"25\" font-weight=\"700\" fill=\"{COLOR_TEXT_MUTED}\" stroke=\"{COLOR_TEXT_MUTED}\" stroke-width=\"0.8\" style=\"{font_stack}\" letter-spacing=\"1.2\">{}</text><text x=\"{words_x}\" y=\"{header_y}\" font-size=\"25\" font-weight=\"700\" fill=\"{COLOR_TEXT_MUTED}\" stroke=\"{COLOR_TEXT_MUTED}\" stroke-width=\"0.8\" style=\"{font_stack}\" letter-spacing=\"1.2\" text-anchor=\"end\">{}</text>",
            header_y - 7.0,
            date_str,
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

    // ── AST-driven rendering ───────────────────────────────────────────

    pub(crate) fn render_node(&mut self, node: &Node, ctx: &mut LayoutContext) {
        match node {
            Node::Heading { level, content } => {
                let (font_size, line_height) = match *level {
                    1 => (H1_SIZE, H1_LH),
                    2 => (H2_SIZE, H2_LH),
                    _ => (H3_SIZE, H3_LH),
                };
                let runs = inlines_to_runs(content);
                self.add_heading(&runs, font_size, line_height, *level);
            }
            Node::Paragraph(inlines) => {
                let runs = inlines_to_runs(inlines);
                self.add_paragraph(&runs);
            }
            Node::Quote { children } => {
                self.render_quote(children, ctx);
            }
            Node::List {
                ordered,
                start,
                items,
            } => {
                self.render_list(*ordered, *start, items, ctx);
            }
            Node::ListItem { children } => {
                for child in children {
                    self.render_node(child, ctx);
                }
            }
            Node::CodeBlock { language, content } => {
                self.add_code_block(content, language);
            }
            Node::MathBlock { latex } => {
                self.add_math_block(latex);
            }
            Node::Table {
                alignments,
                header,
                rows,
            } => {
                self.add_table(PADDING, self.text_area_width(), alignments, header, rows);
            }
            Node::Rule => {
                self.add_rule();
            }
        }
    }

    fn render_quote(&mut self, children: &[Node], ctx: &mut LayoutContext) {
        if !nodes_have_visible_content(children) {
            return;
        }

        ctx.quote_depth += 1;

        self.apply_block_top_gap(0.0);
        let quote_w = self.text_area_width();
        let quote_pad_y = 36.0;
        let block_y = self.y;
        let inner_x = PADDING + 44.0;
        let inner_w = quote_w - 88.0;

        let rect_idx = self.elems.len();
        self.elems.push(String::new());
        self.elems.push(format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"96\" font-weight=\"700\" fill=\"{COLOR_SEED}\" opacity=\"0.35\">&quot;</text>",
            PADDING + 22.0,
            block_y + 70.0,
        ));

        self.y += quote_pad_y + BODY_FONT_SIZE;
        for child in children {
            self.render_quote_node_at(inner_x, inner_w, child, ctx);
        }
        self.flush_pending_block_gap();

        let block_h = (self.y - block_y + quote_pad_y).max(LINE_HEIGHT + quote_pad_y * 2.0);
        self.elems[rect_idx] = format!(
            "<rect x=\"{PADDING}\" y=\"{block_y}\" width=\"{quote_w}\" height=\"{block_h}\" rx=\"24\" fill=\"#f8fafc\" stroke=\"#ffffff\" stroke-width=\"1\"/>"
        );

        self.y = block_y + block_h;
        self.set_block_bottom_gap(LINE_HEIGHT);
        ctx.quote_depth -= 1;
    }

    fn render_quote_node_at(
        &mut self,
        x: f32,
        available_w: f32,
        node: &Node,
        ctx: &mut LayoutContext,
    ) {
        match node {
            Node::Paragraph(inlines) => {
                let runs = inlines_to_runs(inlines);
                if !runs_have_visible_text(&runs) {
                    return;
                }

                let lines: Vec<LayoutLine> =
                    layout_rich_lines(&runs, available_w, BODY_FONT_SIZE, LINE_HEIGHT)
                        .into_iter()
                        .filter(|line| runs_have_visible_text(&line.runs))
                        .collect();

                for line in &lines {
                    self.render_rich_line(
                        x,
                        available_w,
                        self.y,
                        BODY_FONT_SIZE,
                        "#374151",
                        "font-style=\"italic\" letter-spacing=\"0.7\"",
                        &line.runs,
                    );
                    self.y += LINE_HEIGHT;
                }
            }
            Node::Quote { children } => {
                self.render_quote(children, ctx);
            }
            _ => {
                self.render_node_at(x, available_w, node, ctx);
            }
        }
    }

    fn render_list(
        &mut self,
        ordered: bool,
        start: Option<u64>,
        items: &[Node],
        ctx: &mut LayoutContext,
    ) {
        ctx.list_depth += 1;
        let mut idx = start.unwrap_or(1);

        for item in items {
            if let Node::ListItem { children } = item {
                self.render_list_item(ordered, &mut idx, children, ctx);
            } else {
                self.render_node(item, ctx);
            }
        }

        ctx.list_depth -= 1;
    }

    fn render_list_item(
        &mut self,
        ordered: bool,
        idx: &mut u64,
        children: &[Node],
        ctx: &mut LayoutContext,
    ) {
        self.apply_block_top_gap(8.0);
        let quote_indent = ctx.quote_depth as f32 * 40.0;
        let list_indent = ctx.list_depth as f32 * 30.0;
        let content_x = PADDING + quote_indent + list_indent;

        let mut first_para = true;
        for child in children {
            match child {
                Node::Paragraph(inlines) if first_para => {
                    first_para = false;
                    let runs = inlines_to_runs(inlines);
                    if !runs_have_visible_text(&runs) {
                        continue;
                    }

                    let area_w = self.text_area_width();
                    let available_w = area_w - quote_indent - list_indent - 44.0;
                    let lines = layout_rich_lines(&runs, available_w, BODY_FONT_SIZE, LINE_HEIGHT);

                    let marker_x = content_x - 36.0;
                    if ordered {
                        let prefix = format!("{idx}. ");
                        *idx += 1;
                        let escaped = esc(&prefix);
                        self.elems.push(format!(
                            "<text x=\"{marker_x}\" y=\"{}\" font-size=\"{BODY_FONT_SIZE}\" fill=\"{COLOR_SEED}\" font-weight=\"700\">{escaped}</text>",
                            self.y,
                        ));
                    } else {
                        let dot_cy = self.y - BODY_FONT_SIZE * 0.35;
                        self.elems.push(format!(
                            "<ellipse cx=\"{}\" cy=\"{dot_cy}\" rx=\"6\" ry=\"5\" fill=\"{COLOR_SEED}\" opacity=\"0.8\" transform=\"rotate(-15, {}, {dot_cy})\"/>",
                            marker_x + 16.0,
                            marker_x + 16.0,
                        ));
                    }

                    for line in &lines {
                        self.render_rich_line(
                            content_x,
                            available_w,
                            self.y,
                            BODY_FONT_SIZE,
                            COLOR_TEXT,
                            "letter-spacing=\"0.7\"",
                            &line.runs,
                        );
                        self.y += LINE_HEIGHT;
                    }
                }
                Node::ListItem {
                    children: sub_children,
                } => {
                    self.render_list_item(ordered, idx, sub_children, ctx);
                }
                _ => {
                    self.render_node_at(
                        content_x,
                        self.text_area_width() - quote_indent - list_indent,
                        child,
                        ctx,
                    );
                }
            }
        }
        self.flush_pending_block_gap();
        self.set_block_bottom_gap(8.0);
    }

    fn render_node_at(&mut self, x: f32, available_w: f32, node: &Node, ctx: &mut LayoutContext) {
        match node {
            Node::Heading { level, content } => {
                let (font_size, line_height) = match *level {
                    1 => (H1_SIZE, H1_LH),
                    2 => (H2_SIZE, H2_LH),
                    _ => (H3_SIZE, H3_LH),
                };
                let runs = inlines_to_runs(content);
                let lines = layout_rich_lines(&runs, available_w, font_size, line_height);

                let top_gap = if self.has_rendered_block {
                    if *level == 1 {
                        18.0
                    } else {
                        42.0
                    }
                } else {
                    0.0
                };
                self.apply_block_top_gap(top_gap);

                if *level <= 2 {
                    self.elems.push(format!(
                        "<rect x=\"{}\" y=\"{}\" width=\"6\" height=\"{}\" rx=\"3\" fill=\"{COLOR_SEED}\"/>",
                        x - 24.0,
                        self.y - font_size * 0.85,
                        font_size * 1.1,
                    ));
                }

                let fill = match *level {
                    1 => "#000000",
                    2 => "#111111",
                    _ => "#333333",
                };
                for line in &lines {
                    self.render_rich_line(
                        x,
                        available_w,
                        self.y,
                        font_size,
                        fill,
                        "font-weight=\"700\" letter-spacing=\"0.02em\"",
                        &line.runs,
                    );
                    self.y += line_height;
                }
                self.set_block_bottom_gap(12.0);
            }
            Node::Paragraph(inlines) => {
                let runs = inlines_to_runs(inlines);
                if !runs_have_visible_text(&runs) {
                    return;
                }
                let lines = layout_rich_lines(&runs, available_w, BODY_FONT_SIZE, LINE_HEIGHT);
                self.apply_block_top_gap(6.0);
                for line in &lines {
                    self.render_rich_line(
                        x,
                        available_w,
                        self.y,
                        BODY_FONT_SIZE,
                        COLOR_TEXT,
                        "letter-spacing=\"0.7\"",
                        &line.runs,
                    );
                    self.y += LINE_HEIGHT;
                }
                self.set_block_bottom_gap(6.0);
            }
            Node::Quote { children } => {
                self.render_quote(children, ctx);
            }
            Node::List {
                ordered,
                start,
                items,
            } => {
                self.render_list(*ordered, *start, items, ctx);
            }
            Node::ListItem { children } => {
                for child in children {
                    self.render_node_at(x, available_w, child, ctx);
                }
            }
            Node::CodeBlock { language, content } => {
                self.add_code_block(content, language);
            }
            Node::MathBlock { latex } => {
                self.add_math_block(latex);
            }
            Node::Table {
                alignments,
                header,
                rows,
            } => {
                self.add_table(x, available_w, alignments, header, rows);
            }
            Node::Rule => {
                self.add_rule();
            }
        }
    }
}
