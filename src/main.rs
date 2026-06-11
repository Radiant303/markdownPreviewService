mod ast;
mod code;
mod constants;
mod globals;
mod math;
mod svg_builder;
mod text;
mod util;

use axum::{
    body::Bytes,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use pulldown_cmark::{Options, Parser};

use crate::ast::AstBuilder;
use crate::constants::IMAGE_WIDTH;
use crate::globals::USVG_OPTS;
use crate::svg_builder::{LayoutContext, SvgBuilder};
use resvg::usvg;

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

    let render_result = tokio::task::spawn_blocking(move || {
        let svg = markdown_to_svg(&markdown);
        svg_to_png(&svg)
    })
    .await;

    match render_result {
        Ok(Ok(png)) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "image/png")
            .body(axum::body::Body::from(png))
            .unwrap()
            .into_response(),
        Ok(Err(e)) => {
            eprintln!("[ERROR] render failed: {e}");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(axum::body::Body::from(format!("render error: {e}")))
                .unwrap()
                .into_response()
        }
        Err(e) => {
            eprintln!("[ERROR] render task failed: {e}");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(axum::body::Body::from(format!("render task error: {e}")))
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

    // Phase 1: Build AST from pulldown-cmark events
    let ast = AstBuilder::build(parser.into_iter());

    // Phase 2: Layout + render SVG from AST
    let mut b = SvgBuilder::new(IMAGE_WIDTH, word_count);
    let mut ctx = LayoutContext {
        quote_depth: 0,
        list_depth: 0,
    };
    for node in &ast {
        b.render_node(node, &mut ctx);
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
