use crate::globals::MATH_SVG_CACHE;

pub(crate) struct InlineMathSvg {
    pub(crate) view_box: String,
    pub(crate) inner: String,
}

pub(crate) fn render_math_svg_cached(
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

pub(crate) fn inline_math_svg(latex: &str, font_size: f32) -> Option<(InlineMathSvg, f32, f32)> {
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

pub(crate) fn svg_view_box(svg: &str) -> Option<(f32, f32, f32, f32)> {
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

pub(crate) fn svg_inner_content(svg: &str) -> Option<&str> {
    let start = svg.find('>')? + 1;
    let end = svg.rfind("</svg>")?;
    if start <= end {
        Some(&svg[start..end])
    } else {
        None
    }
}
