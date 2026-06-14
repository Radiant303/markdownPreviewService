use crate::globals::MATH_SVG_CACHE;
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

#[derive(Clone)]
pub(crate) struct InlineMathSvg {
    pub(crate) view_box: String,
    pub(crate) inner: String,
    pub(crate) baseline_offset: f32,
}

#[derive(Clone)]
pub(crate) struct ParsedMathSvg {
    pub(crate) view_box: String,
    pub(crate) inner: String,
    pub(crate) vb_y: f32,
    pub(crate) vb_w: f32,
    pub(crate) vb_h: f32,
}

static PARSED_MATH_SVG_CACHE: LazyLock<Mutex<HashMap<String, ParsedMathSvg>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn math_cache_key(latex: &str, options: &mathjax_svg_rs::Options) -> String {
    let align = match options.horizontal_align {
        mathjax_svg_rs::HorizontalAlign::Left => "left",
        mathjax_svg_rs::HorizontalAlign::Center => "center",
        mathjax_svg_rs::HorizontalAlign::Right => "right",
    };
    format!("{}|{align}|{}", options.font_size, latex.trim())
}

pub(crate) fn render_math_svg_cached(
    latex: &str,
    options: &mathjax_svg_rs::Options,
) -> Result<String, String> {
    let key = math_cache_key(latex, options);

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

pub(crate) fn parsed_math_svg_cached(
    latex: &str,
    options: &mathjax_svg_rs::Options,
) -> Result<ParsedMathSvg, String> {
    let key = math_cache_key(latex, options);

    if let Some(parsed) = PARSED_MATH_SVG_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).cloned())
    {
        return Ok(parsed);
    }

    let math_svg = render_math_svg_cached(latex, options)?;
    let (vb_x, vb_y, vb_w, vb_h) =
        svg_view_box(&math_svg).ok_or_else(|| "missing math svg viewBox".to_string())?;
    let inner = svg_inner_content(&math_svg)
        .ok_or_else(|| "missing math svg content".to_string())?
        .to_string();
    let parsed = ParsedMathSvg {
        view_box: format!("{vb_x} {vb_y} {vb_w} {vb_h}"),
        inner,
        vb_y,
        vb_w,
        vb_h,
    };

    if let Ok(mut cache) = PARSED_MATH_SVG_CACHE.lock() {
        if cache.len() > 512 {
            cache.clear();
        }
        cache.insert(key, parsed.clone());
    }

    Ok(parsed)
}

pub(crate) fn inline_math_svg(latex: &str, font_size: f32) -> Option<(InlineMathSvg, f32, f32)> {
    let options = mathjax_svg_rs::Options {
        font_size: font_size as f64,
        horizontal_align: mathjax_svg_rs::HorizontalAlign::Left,
    };
    let parsed = parsed_math_svg_cached(latex, &options).ok()?;

    // MathJax viewBox units are em-like. Use one consistent scale for all
    // inline formulas so tall constructs are taller, not visually smaller.
    let scale = font_size / 1000.0;
    let target_w = (parsed.vb_w * scale).max(font_size * 0.8);
    let target_h = parsed.vb_h * scale;

    Some((
        InlineMathSvg {
            view_box: parsed.view_box,
            inner: parsed.inner,
            baseline_offset: -parsed.vb_y * scale,
        },
        target_w,
        target_h,
    ))
}

pub(crate) fn svg_view_box(svg: &str) -> Option<(f32, f32, f32, f32)> {
    let marker = "viewBox=\"";
    let start = svg.find(marker)? + marker.len();
    let end = svg[start..].find('"')? + start;
    let mut nums = svg[start..end]
        .split_whitespace()
        .filter_map(|n| n.parse::<f32>().ok());

    let x = nums.next()?;
    let y = nums.next()?;
    let w = nums.next()?;
    let h = nums.next()?;

    if w > 0.0 && h > 0.0 && nums.next().is_none() {
        Some((x, y, w, h))
    } else {
        None
    }
}

pub(crate) fn svg_inner_content(svg: &str) -> Option<&str> {
    let start = svg.find('>')? + 1;
    let end = svg.rfind("</svg>")?;
    if start <= end {
        Some(&svg[start..end])
    } else {
        None
    }
}
