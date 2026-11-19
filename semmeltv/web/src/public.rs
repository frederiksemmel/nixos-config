#[derive(Clone)]
pub struct Public {
    pub api: String,
}

impl Public {
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.api)
    }

    /// The picture at `path`, made `width` px wide by the API.
    pub fn sized(&self, path: &str, width: u32) -> String {
        format!("{}?w={width}", self.url(path))
    }

    /// A `srcset` offering the picture at each of `fit`'s widths.
    pub fn srcset(&self, path: &str, fit: &Fit) -> String {
        fit.widths
            .iter()
            .map(|width| format!("{} {width}w", self.sized(path, *width)))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// How a picture shows on a page: the widths it is offered at, how wide the
/// page draws it (`sizes`), and the stored picture's size, which keeps the
/// layout still before it loads.
pub struct Fit {
    pub widths: &'static [u32],
    pub sizes: &'static str,
    pub width: &'static str,
    pub height: &'static str,
}

impl Fit {
    /// The `src` for browsers that ignore `srcset`.
    pub fn fallback(&self) -> u32 {
        self.widths[self.widths.len() / 2]
    }
}

/// A poster in a row or a grid of tiles.
pub const POSTER_TILE: Fit = Fit {
    widths: &[160, 240, 320],
    sizes: "(min-width: 640px) 200px, 33vw",
    width: "342",
    height: "513",
};

/// A backdrop in a row of wide tiles, at most 300 px wide.
pub const CARD_TILE: Fit = Fit {
    widths: &[160, 320, 480, 640],
    sizes: "(min-width: 417px) 300px, 72vw",
    width: "640",
    height: "360",
};

/// The backdrop across the top of a title's page.
pub const HERO: Fit = Fit {
    widths: &[320, 480, 640],
    sizes: "100vw",
    width: "640",
    height: "360",
};

/// The poster beside a title's name on its page.
pub const DETAIL_POSTER: Fit = Fit {
    widths: &[160, 240, 320],
    sizes: "(min-width: 1000px) 210px, (min-width: 640px) 160px, 104px",
    width: "342",
    height: "513",
};

/// An episode's still in the season list.
pub const STILL: Fit = Fit {
    widths: &[160, 320, 480],
    sizes: "(min-width: 640px) 200px, 136px",
    width: "640",
    height: "360",
};

/// Whether a picture loads at once, and first, or once it nears the screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Priority {
    /// The largest picture on first sight: loads at once, before the others.
    First,
    /// On screen at first sight: loads at once.
    Seen,
    /// Further down or along: loads as it nears the screen.
    Later,
}

impl Priority {
    /// The first `seen` pictures of the page's first row show at once, the
    /// first of them before anything else.
    pub fn in_row(index: usize, leads: bool, seen: usize) -> Self {
        match (leads, index) {
            (true, 0) => Self::First,
            (true, index) if index < seen => Self::Seen,
            _ => Self::Later,
        }
    }

    pub fn loading(self) -> &'static str {
        match self {
            Self::First | Self::Seen => "eager",
            Self::Later => "lazy",
        }
    }

    pub fn fetchpriority(self) -> Option<&'static str> {
        (self == Self::First).then_some("high")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_srcset_names_each_width_the_api_makes() {
        let public = Public {
            api: "https://tv.example".to_string(),
        };
        assert_eq!(
            public.srcset("/api/movies/up-2009/poster", &POSTER_TILE),
            "https://tv.example/api/movies/up-2009/poster?w=160 160w, \
             https://tv.example/api/movies/up-2009/poster?w=240 240w, \
             https://tv.example/api/movies/up-2009/poster?w=320 320w"
        );
        assert_eq!(POSTER_TILE.fallback(), 240);
        assert_eq!(CARD_TILE.fallback(), 480);
    }

    #[test]
    fn only_the_first_row_loads_at_once_and_its_first_picture_first() {
        let first: Vec<_> = (0..4).map(|index| Priority::in_row(index, true, 2)).collect();
        assert_eq!(
            first,
            [Priority::First, Priority::Seen, Priority::Later, Priority::Later]
        );
        assert_eq!(Priority::in_row(0, false, 2), Priority::Later);
        assert_eq!(Priority::First.fetchpriority(), Some("high"));
        assert_eq!(Priority::Seen.fetchpriority(), None);
        assert_eq!(Priority::Seen.loading(), "eager");
        assert_eq!(Priority::Later.loading(), "lazy");
    }
}
