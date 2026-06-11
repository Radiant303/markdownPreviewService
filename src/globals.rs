use cosmic_text::FontSystem;
use resvg::usvg;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

pub(crate) static SS: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
pub(crate) static TS: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);
pub(crate) static MATH_SVG_CACHE: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
pub(crate) static FONT_SYSTEM: LazyLock<Mutex<FontSystem>> =
    LazyLock::new(|| Mutex::new(FontSystem::new()));

/// Build usvg options once (loads system fonts for CJK support).
pub(crate) static USVG_OPTS: LazyLock<usvg::Options<'static>> = LazyLock::new(|| {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut opt = usvg::Options::default();
    opt.fontdb = Arc::new(db);
    opt
});
