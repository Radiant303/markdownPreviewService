use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping};

use crate::globals::FONT_SYSTEM;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TextStyle {
    Normal,
    Bold,
    Italic,
    Code,
    Math,
}

#[derive(Clone, Debug)]
pub(crate) struct TextRun {
    pub(crate) text: String,
    pub(crate) style: TextStyle,
    pub(crate) math_scale: Option<f32>,
}

impl TextRun {
    pub(crate) fn new(text: impl Into<String>, style: TextStyle) -> Self {
        Self {
            text: text.into(),
            style,
            math_scale: None,
        }
    }
}

pub(crate) fn push_text_run(runs: &mut Vec<TextRun>, text: impl AsRef<str>, style: TextStyle) {
    let text = text.as_ref();
    if text.is_empty() {
        return;
    }

    if style != TextStyle::Math {
        if let Some(last) = runs.last_mut() {
            if last.style == style {
                last.text.push_str(text);
                return;
            }
        }
    }

    runs.push(TextRun::new(text, style));
}

pub(crate) fn runs_have_visible_text(runs: &[TextRun]) -> bool {
    runs.iter().any(|run| !run.text.trim().is_empty())
}

#[derive(Clone, Debug)]
pub(crate) struct LayoutLine {
    pub(crate) runs: Vec<TextRun>,
    pub(crate) width: f32,
}

pub(crate) fn layout_rich_lines(
    runs: &[TextRun],
    max_width: f32,
    font_size: f32,
    line_height: f32,
) -> Vec<LayoutLine> {
    let mut font_system = FONT_SYSTEM.lock().expect("font system mutex poisoned");
    let logical_lines = split_runs_on_newlines(runs);
    let mut out = Vec::new();

    for logical_runs in logical_lines {
        if logical_runs.is_empty() || !runs_have_visible_text(&logical_runs) {
            out.push(LayoutLine {
                runs: Vec::new(),
                width: 0.0,
            });
            continue;
        }

        let metrics = Metrics::new(font_size, line_height);
        let mut buffer = Buffer::new(&mut font_system, metrics);
        buffer.set_size(&mut font_system, Some(max_width), None);

        let mut full_text = String::new();
        let mut byte_styles = Vec::new();
        let mut spans: Vec<(&str, Attrs)> = Vec::new();

        for run in &logical_runs {
            let start = full_text.len();
            full_text.push_str(&run.text);
            let end = full_text.len();
            byte_styles.push((start, end, run.style));
        }

        for (start, end, style) in &byte_styles {
            spans.push((&full_text[*start..*end], attrs_for_style(*style)));
        }

        buffer.set_rich_text(
            &mut font_system,
            spans,
            attrs_for_style(TextStyle::Normal),
            Shaping::Advanced,
        );

        for layout_run in buffer.layout_runs() {
            let Some(start) = layout_run.glyphs.iter().map(|glyph| glyph.start).min() else {
                out.push(LayoutLine {
                    runs: Vec::new(),
                    width: 0.0,
                });
                continue;
            };
            let Some(end) = layout_run.glyphs.iter().map(|glyph| glyph.end).max() else {
                out.push(LayoutLine {
                    runs: Vec::new(),
                    width: 0.0,
                });
                continue;
            };

            let runs = slice_runs_by_byte_range(
                &full_text,
                &byte_styles,
                start,
                end,
            );
            let width = layout_run.line_w;
            out.push(LayoutLine { runs, width });
        }
    }

    if out.is_empty() {
        out.push(LayoutLine {
            runs: Vec::new(),
            width: 0.0,
        });
    }

    out
}

pub(crate) fn split_runs_on_newlines(runs: &[TextRun]) -> Vec<Vec<TextRun>> {
    let mut lines = vec![Vec::new()];

    for run in runs {
        for segment in run.text.split_inclusive('\n') {
            let text = segment.strip_suffix('\n').unwrap_or(segment);
            if !text.is_empty() {
                push_text_run(lines.last_mut().expect("line exists"), text, run.style);
            }
            if segment.ends_with('\n') {
                lines.push(Vec::new());
            }
        }
    }

    lines
}

pub(crate) fn slice_runs_by_byte_range(
    full_text: &str,
    byte_styles: &[(usize, usize, TextStyle)],
    start: usize,
    end: usize,
) -> Vec<TextRun> {
    let mut runs = Vec::new();

    for (run_start, run_end, style) in byte_styles {
        let s = start.max(*run_start);
        let e = end.min(*run_end);
        if s >= e {
            continue;
        }
        push_text_run(&mut runs, &full_text[s..e], *style);
    }

    runs
}

pub(crate) fn measure_runs_width(runs: &[TextRun], font_size: f32) -> f32 {
    if !runs_have_visible_text(runs) {
        return 0.0;
    }

    let mut font_system = FONT_SYSTEM.lock().expect("font system mutex poisoned");
    let metrics = Metrics::new(font_size, font_size * 1.4);
    let mut buffer = Buffer::new(&mut font_system, metrics);
    buffer.set_size(&mut font_system, None, None);

    let mut full_text = String::new();
    let mut ranges = Vec::new();
    let mut spans: Vec<(&str, Attrs)> = Vec::new();

    for run in runs {
        let start = full_text.len();
        full_text.push_str(&run.text);
        ranges.push((start, full_text.len(), run.style));
    }
    for (start, end, style) in ranges {
        spans.push((&full_text[start..end], attrs_for_style(style)));
    }

    buffer.set_rich_text(
        &mut font_system,
        spans,
        attrs_for_style(TextStyle::Normal),
        Shaping::Advanced,
    );

    buffer
        .layout_runs()
        .map(|run| run.line_w)
        .fold(0.0f32, f32::max)
}

pub(crate) fn attrs_for_style(style: TextStyle) -> Attrs<'static> {
    let mut attrs = Attrs::new().family(Family::Name("LXGW WenKai"));
    match style {
        // The bundled font set does not include all synthetic style faces.
        // Measure with stable base faces; SVG output still applies visual style.
        TextStyle::Bold => {}
        // The bundled font set does not include an italic face. Keep layout on
        // the regular face; SVG rendering still applies font-style="italic".
        TextStyle::Italic => {}
        TextStyle::Code => attrs = attrs.family(Family::Monospace),
        TextStyle::Math => attrs = attrs.family(Family::Serif),
        TextStyle::Normal => {}
    }
    attrs
}

pub(crate) fn svg_tspan_for_run(run: &TextRun) -> (&'static str, String) {
    match run.style {
        TextStyle::Normal => ("", run.text.clone()),
        TextStyle::Bold => ("font-weight=\"700\"", run.text.clone()),
        TextStyle::Italic => ("font-style=\"italic\"", run.text.clone()),
        TextStyle::Code => (
            "fill=\"#ff9500\" font-family=\"LXGW WenKai Mono, monospace\"",
            run.text.clone(),
        ),
        TextStyle::Math => ("fill=\"#0f766e\"", format!("${}$", run.text)),
    }
}
