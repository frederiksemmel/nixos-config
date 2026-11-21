use std::{path::Path, sync::LazyLock};

use anyhow::{Context, Result, anyhow};
use typst::{
    Library, LibraryExt, World,
    diag::{FileError, FileResult},
    foundations::{Bytes, Datetime, Dict, Duration, Str, Value},
    syntax::{FileId, Source},
    text::{Font, FontBook},
    utils::LazyHash,
};
use typst_layout::PagedDocument;
use typst_render::RenderOptions;

const FONT_DIR: &str = env!("EPAPER_FONT_DIR");
const CACHE_AGE: usize = 10;

struct Fonts {
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
}

static FONTS: LazyLock<Fonts> = LazyLock::new(|| {
    let mut fonts: Vec<Font> = typst_assets::fonts()
        .flat_map(|data| Font::iter(Bytes::new(data)))
        .collect();
    match load_dir(Path::new(FONT_DIR)) {
        Ok(found) => fonts.extend(found),
        Err(err) => tracing::warn!("fonts in {FONT_DIR}: {err:#}"),
    }
    Fonts {
        book: LazyHash::new(FontBook::from_fonts(&fonts)),
        fonts,
    }
});

fn load_dir(dir: &Path) -> Result<Vec<Font>> {
    let mut fonts = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "ttf" || e == "otf") {
            let data = std::fs::read(&path).with_context(|| path.display().to_string())?;
            fonts.extend(Font::iter(Bytes::new(data)));
        }
    }
    Ok(fonts)
}

/// A rendered page: `width × height` pixels, RGBA, one pixel per point.
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Raster {
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let i = 4 * (y * self.width + x) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2]]
    }
}

struct Job {
    library: LazyHash<Library>,
    main: Source,
}

impl World for Job {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &FONTS.book
    }

    fn main(&self) -> FileId {
        self.main.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            Ok(self.main.clone())
        } else {
            Err(FileError::AccessDenied)
        }
    }

    fn file(&self, _id: FileId) -> FileResult<Bytes> {
        Err(FileError::AccessDenied)
    }

    fn font(&self, index: usize) -> Option<Font> {
        FONTS.fonts.get(index).cloned()
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}

/// Compiles `template` with `inputs` visible as `sys.inputs` and renders its first page.
pub fn typeset(template: &str, inputs: &[(&str, &str)]) -> Result<Raster> {
    let inputs: Dict = inputs
        .iter()
        .map(|(key, value)| (Str::from(*key), Value::Str(Str::from(*value))))
        .collect();
    let job = Job {
        library: LazyHash::new(Library::builder().with_inputs(inputs).build()),
        main: Source::detached(template),
    };
    let compiled = typst::compile::<PagedDocument>(&job).output;
    typst::comemo::evict(CACHE_AGE);
    let document = compiled.map_err(|errors| {
        let messages: Vec<String> = errors.iter().map(|e| e.message.to_string()).collect();
        anyhow!("typst: {}", messages.join("; "))
    })?;
    let page = document.pages().first().context("typst produced no page")?;
    let pixmap = typst_render::render(
        page,
        &RenderOptions {
            pixel_per_pt: 1.0.into(),
            render_bleed: false,
        },
    );
    Ok(Raster {
        width: pixmap.width(),
        height: pixmap.height(),
        rgba: pixmap.data().to_vec(),
    })
}
