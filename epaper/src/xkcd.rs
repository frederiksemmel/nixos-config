//! Comics from xkcd.com, through its JSON API.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use image::{ImageFormat, RgbaImage};
use reqwest::StatusCode;
use serde::Deserialize;

use crate::picture;

const SITE: &str = "https://xkcd.com";
/// The number xkcd left out, as a joke on the HTTP status.
const NOT_FOUND: u32 = 404;
const TIMEOUT: Duration = Duration::from_secs(20);

/// Which comic `/xkcd` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Newest,
    Random,
    Number(u32),
}

impl Pick {
    /// Reads the argument of `/xkcd`: nothing for the newest comic, `random`, or a number such as `927` or `#927`.
    pub fn parse(arg: &str) -> Option<Pick> {
        let arg = arg.trim();
        match arg.to_lowercase().as_str() {
            "" | "newest" | "latest" => Some(Pick::Newest),
            "random" => Some(Pick::Random),
            _ => arg
                .trim_start_matches('#')
                .parse()
                .ok()
                .filter(|&n| n > 0)
                .map(Pick::Number),
        }
    }
}

/// A comic's entry in the JSON API.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Info {
    pub num: u32,
    pub title: String,
    /// The title text, which holds the punchline.
    pub alt: String,
    /// The picture's address; empty or not a picture for interactive comics.
    pub img: String,
}

impl Info {
    pub fn parse(json: &[u8]) -> Result<Info> {
        serde_json::from_slice(json).context("reading an xkcd entry")
    }

    /// The number, the title and the punchline, a line each.
    pub fn caption(&self) -> String {
        format!("xkcd {}: {}\n{}", self.num, self.title, self.alt)
    }

    /// Whether `img` names a PNG or JPEG file.
    fn has_picture(&self) -> bool {
        let file = self.img.rsplit('/').next().unwrap_or_default().to_lowercase();
        [".png", ".jpg", ".jpeg"].iter().any(|ext| file.ends_with(ext))
    }
}

/// What came of fetching a comic, short of the site failing.
#[derive(Debug)]
pub enum Fetched {
    Drawable {
        info: Info,
        picture: Arc<RgbaImage>,
    },
    /// The comic has no PNG or JPEG picture, as some interactive ones.
    Undrawable(Info),
    /// No comic has this number.
    Missing(u32),
}

/// Fetches comics from a site with the xkcd JSON API.
#[derive(Clone)]
pub struct Xkcd {
    client: reqwest::Client,
    site: String,
}

impl Default for Xkcd {
    fn default() -> Self {
        Xkcd::at(SITE)
    }
}

impl Xkcd {
    /// Fetches from `site`, such as `https://xkcd.com`.
    pub fn at(site: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("epaper/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Xkcd {
            client,
            site: site.trim_end_matches('/').to_owned(),
        }
    }

    /// Fetches the comic and its picture; an error means the site could not be reached or broke.
    pub async fn fetch(&self, pick: Pick) -> Result<Fetched> {
        let number = match pick {
            Pick::Newest => None,
            Pick::Number(n) => Some(n),
            Pick::Random => Some(random_number(self.newest().await?.num)),
        };
        let info = match number {
            None => self.newest().await?,
            Some(n) => match self.info(&format!("{}/{n}/info.0.json", self.site)).await? {
                Some(info) => info,
                None => return Ok(Fetched::Missing(n)),
            },
        };
        if !info.has_picture() {
            return Ok(Fetched::Undrawable(info));
        }
        let bytes = self.get(&info.img).await?.context("the picture is gone")?;
        Ok(match decode(&bytes) {
            Some(picture) => Fetched::Drawable {
                info,
                picture: Arc::new(picture),
            },
            None => Fetched::Undrawable(info),
        })
    }

    async fn newest(&self) -> Result<Info> {
        self.info(&format!("{}/info.0.json", self.site))
            .await?
            .context("the site has no newest comic")
    }

    async fn info(&self, url: &str) -> Result<Option<Info>> {
        self.get(url).await?.map(|json| Info::parse(&json)).transpose()
    }

    /// The body at `url`, or `None` when the site says there is nothing there.
    async fn get(&self, url: &str) -> Result<Option<Vec<u8>>> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = response.error_for_status()?;
        Ok(Some(
            response
                .bytes()
                .await
                .with_context(|| format!("reading {url}"))?
                .to_vec(),
        ))
    }
}

/// A comic between 1 and `newest`, never 404.
fn random_number(newest: u32) -> u32 {
    let count = newest - u32::from(newest >= NOT_FOUND);
    nth_comic(rand::random_range(0..count.max(1)), newest)
}

/// The `i`th of the comics 1 to `newest`, counted from 0 and skipping 404.
fn nth_comic(i: u32, newest: u32) -> u32 {
    let n = i + 1;
    if n >= NOT_FOUND && newest >= NOT_FOUND {
        n + 1
    } else {
        n
    }
}

/// The picture in `bytes` if it is a PNG or JPEG that decodes.
fn decode(bytes: &[u8]) -> Option<RgbaImage> {
    let format = image::guess_format(bytes).ok()?;
    if !matches!(format, ImageFormat::Png | ImageFormat::Jpeg) {
        tracing::info!("an xkcd picture is {format:?}, not PNG or JPEG");
        return None;
    }
    picture::decode(bytes)
        .inspect_err(|err| tracing::warn!("an xkcd picture: {err:#}"))
        .ok()
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use image::Rgba;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };

    const STANDARDS: &str = r#"{"month": "7", "num": 927, "link": "", "year": "2011", "news": "", "safe_title": "Standards", "transcript": "", "alt": "Fortunately, the charging one has been solved now that we've all standardized on mini-USB. Or is it micro-USB? Shit.", "img": "https://imgs.xkcd.com/comics/standards.png", "title": "Standards", "day": "20"}"#;

    const HOVERBOARD: &str = r#"{"month": "11", "num": 1608, "link": "", "year": "2015", "news": "", "safe_title": "Hoverboard", "transcript": "", "alt": "I find it hard to believe that, of all the Star Wars actors, Carrie Fisher is the only one who brought up the question of how the hoverboards work.", "img": "https://imgs.xkcd.com/comics/", "title": "Hoverboard", "day": "18"}"#;

    fn info(num: u32, img: &str) -> Info {
        Info {
            num,
            title: format!("Comic {num}"),
            alt: format!("Punchline {num}"),
            img: img.to_owned(),
        }
    }

    /// Serves over HTTP on a free local port and returns the site's address. `routes` gets that address and gives each path its body.
    pub fn serve(routes: impl FnOnce(&str) -> Vec<(String, Vec<u8>)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let site = format!("http://{}", listener.local_addr().unwrap());
        let routes = routes(&site);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).ok();
                let mut header = String::new();
                while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
                    header.clear();
                }
                let path = request.split_whitespace().nth(1).unwrap_or_default();
                let (status, body) = match routes.iter().find(|(p, _)| p == path) {
                    Some((_, body)) => ("200 OK", body.as_slice()),
                    None => ("404 Not Found", &b""[..]),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(head.as_bytes()).ok();
                stream.write_all(body).ok();
            }
        });
        site
    }

    pub fn encoded(picture: &RgbaImage, format: ImageFormat) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(picture.clone())
            .into_rgb8()
            .write_to(&mut bytes, format)
            .unwrap();
        bytes.into_inner()
    }

    pub fn entry(site: &str, num: u32, file: &str) -> Vec<u8> {
        serde_json::json!({
            "num": num,
            "title": format!("Comic {num}"),
            "safe_title": format!("Comic {num}"),
            "alt": format!("Punchline {num}"),
            "img": format!("{site}/comics/{file}"),
            "transcript": "",
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn arguments_pick_the_newest_a_random_or_a_numbered_comic() {
        assert_eq!(Pick::parse(""), Some(Pick::Newest));
        assert_eq!(Pick::parse("  "), Some(Pick::Newest));
        assert_eq!(Pick::parse("Random"), Some(Pick::Random));
        assert_eq!(Pick::parse(" 927 "), Some(Pick::Number(927)));
        assert_eq!(Pick::parse("#927"), Some(Pick::Number(927)));
        assert_eq!(Pick::parse("0"), None);
        assert_eq!(Pick::parse("-3"), None);
        assert_eq!(Pick::parse("standards"), None);
    }

    #[test]
    fn an_entry_reads_from_the_api_json() {
        let info = Info::parse(STANDARDS.as_bytes()).unwrap();
        assert_eq!(info.num, 927);
        assert_eq!(info.title, "Standards");
        assert!(info.alt.ends_with("Or is it micro-USB? Shit."));
        assert!(info.has_picture());
        assert_eq!(
            info.caption(),
            "xkcd 927: Standards\nFortunately, the charging one has been solved now that we've all standardized on mini-USB. Or is it micro-USB? Shit."
        );
        assert!(Info::parse(b"<html>").is_err());
    }

    #[test]
    fn only_png_and_jpeg_files_are_pictures() {
        assert!(!Info::parse(HOVERBOARD.as_bytes()).unwrap().has_picture());
        assert!(!info(961, "https://imgs.xkcd.com/comics/eternal_flame.gif").has_picture());
        assert!(info(1, "https://imgs.xkcd.com/comics/barrel_cropped_(1).JPG").has_picture());
    }

    #[test]
    fn random_comics_skip_404() {
        assert_eq!(nth_comic(0, 3000), 1);
        assert_eq!(nth_comic(402, 3000), 403);
        assert_eq!(nth_comic(403, 3000), 405);
        assert_eq!(nth_comic(2998, 3000), 3000);
        assert_eq!(nth_comic(402, 403), 403);
        for _ in 0..1000 {
            let n = random_number(406);
            assert!((1..=406).contains(&n) && n != 404, "{n}");
        }
        assert_eq!(random_number(1), 1);
    }

    #[test]
    fn only_png_and_jpeg_bytes_decode() {
        let picture = RgbaImage::from_pixel(4, 3, Rgba([0, 0, 0, 255]));
        let png = decode(&encoded(&picture, ImageFormat::Png)).unwrap();
        assert_eq!(png.dimensions(), (4, 3));
        assert!(decode(&encoded(&picture, ImageFormat::Jpeg)).is_some());
        assert!(decode(b"GIF89a\x04\x00\x03\x00").is_none());
        assert!(decode(b"\x89PNG\r\n\x1a\nbroken").is_none());
    }

    #[tokio::test]
    async fn comics_are_fetched_with_their_pictures() {
        let picture = RgbaImage::from_pixel(8, 5, Rgba([0, 0, 0, 255]));
        let png = encoded(&picture, ImageFormat::Png);
        let site = serve(|site| {
            vec![
                ("/info.0.json".to_owned(), entry(site, 3, "three.png")),
                ("/3/info.0.json".to_owned(), entry(site, 3, "three.png")),
                ("/2/info.0.json".to_owned(), entry(site, 2, "")),
                ("/1/info.0.json".to_owned(), entry(site, 1, "one.gif")),
                ("/comics/three.png".to_owned(), png),
            ]
        });
        let xkcd = Xkcd::at(&site);

        let Fetched::Drawable { info, picture } = xkcd.fetch(Pick::Newest).await.unwrap() else {
            panic!("the newest comic has a picture");
        };
        assert_eq!(info.num, 3);
        assert_eq!(picture.dimensions(), (8, 5));
        assert!(matches!(xkcd.fetch(Pick::Number(2)).await.unwrap(), Fetched::Undrawable(i) if i.num == 2));
        assert!(matches!(xkcd.fetch(Pick::Number(1)).await.unwrap(), Fetched::Undrawable(i) if i.num == 1));
        assert!(matches!(
            xkcd.fetch(Pick::Number(9)).await.unwrap(),
            Fetched::Missing(9)
        ));
        assert!(xkcd.fetch(Pick::Random).await.is_ok());
    }

    #[tokio::test]
    async fn an_unreachable_site_is_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let site = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        assert!(Xkcd::at(&site).fetch(Pick::Newest).await.is_err());
    }
}
