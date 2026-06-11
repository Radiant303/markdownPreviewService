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

const BUNDLED_FONTS: &[&[u8]] = &[
    include_bytes!("../sources/fonts/LXGWWenKai-Regular.ttf"),
    include_bytes!("../sources/fonts/LXGWWenKai-Medium.ttf"),
    include_bytes!("../sources/fonts/LXGWWenKaiMono-Medium.ttf"),
];

pub(crate) static FONT_SYSTEM: LazyLock<Mutex<FontSystem>> = LazyLock::new(|| {
    let sources = BUNDLED_FONTS
        .iter()
        .map(|font| fontdb16::Source::Binary(Arc::new(font.to_vec())));
    let mut font_system = FontSystem::new_with_fonts(sources);
    let db = font_system.db_mut();
    db.set_sans_serif_family("LXGW WenKai");
    db.set_serif_family("LXGW WenKai");
    db.set_monospace_family("LXGW WenKai Mono");
    Mutex::new(font_system)
});


/// Build usvg options once with bundled LXGW fonts plus system fallback fonts.
pub(crate) static USVG_OPTS: LazyLock<usvg::Options<'static>> = LazyLock::new(|| {
    let mut db = fontdb::Database::new();
    for font in BUNDLED_FONTS {
        db.load_font_data(font.to_vec());
    }
    db.load_system_fonts();
    let mut opt = usvg::Options::default();
    opt.fontdb = Arc::new(db);
    opt
});
