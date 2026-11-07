use crate::library::{Library, Movie, Show, normalize};
use crate::tmdb::{self, Client, Found, Lookup, Season};
use image::ImageReader;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufWriter, Cursor};
use std::path::{self, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const CARD_WIDTH: u32 = 640;
pub const CARD_HEIGHT: u32 = 360;
const TMDB_MAX_AGE: Duration = Duration::from_secs(180 * 24 * 60 * 60);
const JPEG_QUALITY: u8 = 85;
const MOVIE_FRAME_AT: f64 = 0.2;
const EPISODE_FRAME_AT: f64 = 0.35;
const FRAME_RETRY_STEP: f64 = 0.13;
const FRAME_ATTEMPTS: usize = 4;
const DARK_FRAME_LUMA: u64 = 32;

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Matches {
    pub titles: BTreeMap<String, Matched>,
    pub episode_names: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Matched {
    pub id: u64,
    pub name: String,
    pub date: String,
    pub checked: u64,
}

pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn art(&self, key: &str, source: &str) -> PathBuf {
        self.root.join("art").join(format!("{key}.{source}.jpg"))
    }

    fn matches_path(&self) -> PathBuf {
        self.root.join("tmdb.json")
    }

    pub fn load_matches(&self) -> Matches {
        fs::read(self.matches_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save_matches(&self, matches: &Matches) -> io::Result<()> {
        let path = self.matches_path();
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(matches)?)?;
        fs::rename(temporary, path)
    }

    pub fn card_art(&self, poster: Option<&Path>, key: &str) -> Option<PathBuf> {
        poster.map(Path::to_path_buf).or_else(|| {
            [self.art(key, "tmdb"), self.art(key, "frame")]
                .into_iter()
                .find(|path| path.is_file())
        })
    }

    pub fn has_tmdb_art(&self) -> bool {
        fs::read_dir(self.root.join("art")).is_ok_and(|mut entries| {
            entries.any(|entry| entry.is_ok_and(|e| e.path().to_string_lossy().ends_with(".tmdb.jpg")))
        })
    }
}

pub struct Options {
    pub frames: bool,
    pub tmdb: Option<Client>,
}

struct Generator<'a> {
    cache: &'a Cache,
    options: Options,
    matches: Matches,
    report: Vec<String>,
}

pub fn generate(library: &Library, library_root: &Path, cache: &Cache, options: Options) -> io::Result<()> {
    if path::absolute(&cache.root)?.starts_with(path::absolute(library_root)?) {
        return Err(io::Error::other("the cache must stay outside the library"));
    }
    fs::create_dir_all(cache.root.join("art"))?;
    let matches = cache.load_matches();
    let mut generator = Generator {
        cache,
        options,
        matches,
        report: Vec::new(),
    };
    for show in &library.shows {
        generator.show(show)?;
    }
    for movie in &library.movies {
        generator.movie(movie)?;
    }
    cache.save_matches(&generator.matches)?;
    let report = generator.report.join("\n");
    fs::write(cache.root.join("tmdb-report.txt"), &report)?;
    if !report.is_empty() {
        println!("\nNeeds attention (pin with tmdb = <id> in meta.toml):\n{report}");
    }
    Ok(())
}

impl Generator<'_> {
    fn matched(
        &mut self,
        key: &str,
        kind: tmdb::Kind,
        pinned: Option<u64>,
        lookup: impl FnOnce(&Client) -> Result<Lookup, ureq::Error>,
        label: &str,
    ) -> Option<Found> {
        let client = self.options.tmdb.as_ref()?;
        let known = self
            .matches
            .titles
            .get(key)
            .filter(|m| age_of(m.checked) < TMDB_MAX_AGE)
            .map(|m| m.id);
        if let Some(id) = pinned.or(known) {
            return match client.details(kind, id) {
                Ok(found) => Some(found),
                Err(error) => {
                    self.report.push(format!("{label}: TMDB {id} failed: {error}"));
                    None
                }
            };
        }
        match lookup(client) {
            Ok(Lookup::Match(found)) => {
                let matched = Matched {
                    id: found.id,
                    name: found.name.clone(),
                    date: found.date.clone(),
                    checked: now(),
                };
                self.matches.titles.insert(key.to_string(), matched);
                println!("{label}: TMDB {} {} ({})", found.id, found.name, found.date);
                Some(found)
            }
            Ok(Lookup::Unsure(candidates)) => {
                let list: Vec<_> = candidates
                    .iter()
                    .map(|c| format!("{} {} ({})", c.id, c.name, c.date))
                    .collect();
                self.report.push(format!("{label}: unsure: {}", list.join("; ")));
                None
            }
            Ok(Lookup::Nothing) => {
                self.report.push(format!("{label}: nothing found"));
                None
            }
            Err(error) => {
                self.report.push(format!("{label}: lookup failed: {error}"));
                None
            }
        }
    }

    fn fetch(&mut self, key: &str, url_path: Option<&str>, size: &str, label: &str) -> bool {
        let target = self.cache.art(key, "tmdb");
        if is_fresh(&target) {
            return true;
        }
        let downloaded = match (self.options.tmdb.as_ref(), url_path) {
            (Some(client), Some(url_path)) => client
                .image(size, url_path)
                .map_err(io::Error::other)
                .and_then(|bytes| write_card(&bytes, &target)),
            _ => Err(io::Error::other("no image")),
        };
        if let Err(error) = &downloaded {
            let _ = fs::remove_file(&target);
            if url_path.is_some() {
                self.report.push(format!("{label}: image failed: {error}"));
            }
        }
        downloaded.is_ok()
    }

    fn frame(&mut self, key: &str, video: &Path, at: f64, label: &str) {
        let target = self.cache.art(key, "frame");
        if !self.options.frames || target.is_file() {
            return;
        }
        if let Err(error) = extract_frame(video, at, &target) {
            self.report.push(format!("{label}: frame failed: {error}"));
        }
    }

    fn is_current(&self, key: &str) -> bool {
        is_fresh(&self.cache.art(key, "tmdb"))
    }

    fn show(&mut self, show: &Show) -> io::Result<()> {
        let label = show.title.clone();
        println!("{label}");
        let episodes_current = show
            .seasons
            .iter()
            .flat_map(|season| &season.episodes)
            .all(|episode| self.is_current(&show.episode_key(episode)));
        let card_current = show.poster.is_some() || self.is_current(&show.key);
        let found = if card_current && episodes_current {
            None
        } else {
            self.matched(
                &show.key,
                tmdb::Kind::Tv,
                show.tmdb,
                |client| client.search_tv(&show.title, show.year),
                &label,
            )
        };

        let has_card = show.poster.is_some()
            || self.fetch(
                &show.key,
                found.as_ref().and_then(|f| f.backdrop.as_deref()),
                "w780",
                &label,
            );
        if !has_card && let Some(first) = show.first_episode() {
            self.frame(&show.key, &first.path, EPISODE_FRAME_AT, &label);
        }

        for season in &show.seasons {
            let details = match (&found, self.options.tmdb.as_ref()) {
                (Some(found), Some(client)) if season.number > 0 => client.season(found.id, season.number).ok(),
                _ => None,
            };
            for episode in &season.episodes {
                let key = show.episode_key(episode);
                let label = format!("{} {}", show.title, key);
                let still = details.as_ref().and_then(|d: &Season| d.episode(episode.number));
                if let Some(name) = still.and_then(|still| still.name.clone()) {
                    self.matches.episode_names.insert(key.clone(), name);
                }
                let path = still.and_then(|still| still.still_path.as_deref());
                if !self.fetch(&key, path, "original", &label) {
                    self.frame(&key, &episode.path, EPISODE_FRAME_AT, &label);
                }
            }
        }
        self.cache.save_matches(&self.matches)
    }

    fn movie(&mut self, movie: &Movie) -> io::Result<()> {
        let label = match movie.year {
            Some(year) => format!("{} ({year})", movie.title),
            None => movie.title.clone(),
        };
        println!("{label}");
        if movie.poster.is_some() || self.is_current(&movie.key) {
            return Ok(());
        }
        let found = self.matched(
            &movie.key,
            tmdb::Kind::Movie,
            movie.tmdb,
            |client| {
                let by_title = client.search_movie(&movie.title, movie.year)?;
                if normalize(&movie.title) == normalize(&movie.file_title) || matches!(by_title, Lookup::Match(_)) {
                    Ok(by_title)
                } else {
                    client.search_movie(&movie.file_title, movie.year)
                }
            },
            &label,
        );
        if !self.fetch(
            &movie.key,
            found.as_ref().and_then(|f| f.backdrop.as_deref()),
            "w780",
            &label,
        ) {
            self.frame(&movie.key, &movie.path, MOVIE_FRAME_AT, &label);
        }
        self.cache.save_matches(&self.matches)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn age_of(checked: u64) -> Duration {
    Duration::from_secs(now().saturating_sub(checked))
}

fn is_fresh(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age < TMDB_MAX_AGE))
}

fn write_card(bytes: &[u8], target: &Path) -> io::Result<()> {
    let image = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .decode()
        .map_err(io::Error::other)?
        .resize_to_fill(CARD_WIDTH, CARD_HEIGHT, FilterType::Triangle)
        .into_rgb8();
    let temporary = target.with_extension("tmp");
    let writer = BufWriter::new(fs::File::create(&temporary)?);
    JpegEncoder::new_with_quality(writer, JPEG_QUALITY)
        .encode_image(&image)
        .map_err(io::Error::other)?;
    fs::rename(temporary, target)
}

fn duration(video: &Path) -> io::Result<f64> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(video)
        .output()?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .map_err(io::Error::other)
}

fn extract_frame(video: &Path, at: f64, target: &Path) -> io::Result<()> {
    let length = duration(video)?;
    let temporary = target.with_extension("tmp.jpg");
    let mut brightest = 0;
    for attempt in 0..FRAME_ATTEMPTS {
        let position = (at + attempt as f64 * FRAME_RETRY_STEP).fract();
        let candidate = temporary.with_extension(format!("{attempt}.jpg"));
        grab_frame(video, length * position, &candidate)?;
        let luma = mean_luma(&candidate)?;
        if luma > brightest {
            brightest = luma;
            fs::rename(&candidate, &temporary)?;
        } else {
            fs::remove_file(&candidate)?;
        }
        if brightest >= DARK_FRAME_LUMA {
            break;
        }
    }
    fs::rename(temporary, target)
}

fn mean_luma(path: &Path) -> io::Result<u64> {
    let image = ImageReader::open(path)?
        .decode()
        .map_err(io::Error::other)?
        .into_luma8();
    let total: u64 = image.pixels().map(|pixel| u64::from(pixel.0[0])).sum();
    Ok(total / u64::from(image.width() * image.height()).max(1))
}

fn grab_frame(video: &Path, seconds: f64, target: &Path) -> io::Result<()> {
    let filter = format!(
        "thumbnail=48,scale={CARD_WIDTH}:{CARD_HEIGHT}:force_original_aspect_ratio=increase,crop={CARD_WIDTH}:{CARD_HEIGHT}"
    );
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y", "-ss", &format!("{seconds:.0}"), "-i"])
        .arg(video)
        .args([
            "-map",
            "0:v:0",
            "-vf",
            &filter,
            "-frames:v",
            "1",
            "-update",
            "1",
            "-q:v",
            "3",
        ])
        .arg(target)
        .stderr(Stdio::inherit())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("ffmpeg exited with {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn the_cache_may_not_live_inside_the_library() {
        let root = env::temp_dir().join(format!("semmeltv-test-{}", std::process::id()));
        let cache = Cache::new(root.join("library/cache"));
        let options = Options {
            frames: false,
            tmdb: None,
        };
        assert!(generate(&Library::default(), &root.join("library"), &cache, options).is_err());
        assert!(!root.exists());
    }

    #[test]
    fn tmdb_art_wins_over_a_frame_and_a_poster_wins_over_both() {
        let root = env::temp_dir().join(format!("semmeltv-art-{}", std::process::id()));
        fs::create_dir_all(root.join("art")).unwrap();
        let cache = Cache::new(root.clone());
        assert_eq!(cache.card_art(None, "dark"), None);
        fs::write(cache.art("dark", "frame"), b"").unwrap();
        assert_eq!(cache.card_art(None, "dark"), Some(cache.art("dark", "frame")));
        fs::write(cache.art("dark", "tmdb"), b"").unwrap();
        assert_eq!(cache.card_art(None, "dark"), Some(cache.art("dark", "tmdb")));
        let poster = Path::new("/lib/Series/Dark/poster.jpg");
        assert_eq!(cache.card_art(Some(poster), "dark"), Some(poster.to_path_buf()));
        fs::remove_dir_all(root).unwrap();
    }
}
