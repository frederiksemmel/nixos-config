use crate::library::{Library, Movie, Show};
use crate::progress::Progress;
use crate::tmdb::{self, Client, Found, Season};
use image::ImageReader;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufWriter, Cursor};
use std::path::{self, Path, PathBuf};
use std::time::Duration;

pub const CARD_WIDTH: u32 = 640;
pub const CARD_HEIGHT: u32 = 360;
const TMDB_MAX_AGE: Duration = Duration::from_secs(180 * 24 * 60 * 60);
const JPEG_QUALITY: u8 = 85;

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Matches {
    pub titles: BTreeMap<String, Matched>,
    pub episode_names: BTreeMap<String, String>,
    #[serde(default)]
    pub runtimes: BTreeMap<String, u32>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Matched {
    pub id: u64,
    pub name: String,
    pub date: String,
}

pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn art_dir(&self) -> PathBuf {
        self.root.join("art")
    }

    pub fn saved_manifest(&self) -> PathBuf {
        self.root.join("library.last-good.toml")
    }

    fn art(&self, key: &str) -> PathBuf {
        self.art_dir().join(format!("{key}.jpg"))
    }

    pub fn matches_path(&self) -> PathBuf {
        self.root.join("tmdb.json")
    }

    fn progress_path(&self) -> PathBuf {
        self.root.join("progress.json")
    }

    pub fn watch_later_dir(&self, file: &Path) -> PathBuf {
        let parts = file
            .components()
            .filter(|part| matches!(part, path::Component::Normal(_)));
        self.root.join("watch-later").join(parts.collect::<PathBuf>())
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

    pub fn load_progress(&self) -> Progress {
        fs::read(self.progress_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save_progress(&self, progress: &Progress) -> io::Result<()> {
        let path = self.progress_path();
        let temporary = path.with_extension("json.tmp");
        fs::create_dir_all(&self.root)?;
        fs::write(&temporary, serde_json::to_vec_pretty(progress)?)?;
        fs::rename(temporary, path)
    }

    pub fn card_art(&self, poster: Option<&Path>, key: &str) -> Option<PathBuf> {
        poster
            .map(Path::to_path_buf)
            .or_else(|| Some(self.art(key)).filter(|path| path.is_file()))
    }

    pub fn has_tmdb_art(&self) -> bool {
        fs::read_dir(self.art_dir()).is_ok_and(|mut entries| entries.next().is_some())
    }
}

struct Generator<'a> {
    cache: &'a Cache,
    client: Client,
    matches: Matches,
    report: Vec<String>,
}

pub fn generate(library: &Library, library_root: &Path, cache: &Cache, client: Client) -> io::Result<()> {
    if path::absolute(&cache.root)?.starts_with(path::absolute(library_root)?) {
        return Err(io::Error::other("the cache must stay outside the library"));
    }
    fs::create_dir_all(cache.art_dir())?;
    let matches = cache.load_matches();
    let mut generator = Generator {
        cache,
        client,
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
        println!("\nNeeds attention (set tmdb = <id> in library.toml):\n{report}");
    }
    Ok(())
}

impl Generator<'_> {
    fn details(&mut self, kind: tmdb::Kind, id: Option<u64>, label: &str) -> Option<Found> {
        let Some(id) = id else {
            self.report.push(format!("{label}: no TMDB id"));
            return None;
        };
        match self.client.details(kind, id) {
            Ok(found) => Some(found),
            Err(error) => {
                self.report.push(format!("{label}: TMDB {id} failed: {error}"));
                None
            }
        }
    }

    fn fetch(&mut self, key: &str, url_path: Option<&str>, size: &str, label: &str) -> bool {
        let target = self.cache.art(key);
        if is_fresh(&target) {
            return true;
        }
        let downloaded = match url_path {
            Some(url_path) => self
                .client
                .image(size, url_path)
                .map_err(io::Error::other)
                .and_then(|bytes| write_card(&bytes, &target)),
            None => Err(io::Error::other("no image")),
        };
        if let Err(error) = &downloaded {
            let _ = fs::remove_file(&target);
            if url_path.is_some() {
                self.report.push(format!("{label}: image failed: {error}"));
            }
        }
        downloaded.is_ok()
    }

    fn is_current(&self, key: &str) -> bool {
        is_fresh(&self.cache.art(key))
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
        let runtimes_known = show
            .seasons
            .iter()
            .filter(|season| season.number > 0)
            .flat_map(|season| &season.episodes)
            .all(|episode| self.matches.runtimes.contains_key(&show.episode_key(episode)));
        let found = if card_current && episodes_current && runtimes_known {
            None
        } else {
            self.details(tmdb::Kind::Tv, show.tmdb, &label)
        };

        if show.poster.is_none() {
            self.fetch(
                &show.key,
                found.as_ref().and_then(|f| f.backdrop.as_deref()),
                "w780",
                &label,
            );
        }

        for season in &show.seasons {
            let details = match &found {
                Some(found) if season.number > 0 => match self.client.season(found.id, season.number) {
                    Ok(details) => Some(details),
                    Err(error) => {
                        self.report.push(format!("{label} season {}: {error}", season.number));
                        None
                    }
                },
                _ => None,
            };
            for episode in &season.episodes {
                let key = show.episode_key(episode);
                let label = format!("{} {}", show.title, key);
                let still = details.as_ref().and_then(|d: &Season| d.episode(episode.number));
                if let Some(name) = still.and_then(|still| still.name.clone()) {
                    self.matches.episode_names.insert(key.clone(), name);
                }
                if let Some(runtime) = still.and_then(|still| still.runtime).filter(|minutes| *minutes > 0) {
                    self.matches.runtimes.insert(key.clone(), runtime);
                }
                self.fetch(
                    &key,
                    still.and_then(|still| still.still_path.as_deref()),
                    "original",
                    &label,
                );
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
        let art_current = movie.poster.is_some() || self.is_current(&movie.key);
        if art_current && self.matches.runtimes.contains_key(&movie.key) {
            return Ok(());
        }
        let found = self.details(tmdb::Kind::Movie, movie.tmdb, &label);
        if let Some(runtime) = found.as_ref().and_then(|f| f.runtime) {
            self.matches.runtimes.insert(movie.key.clone(), runtime);
        }
        if !art_current {
            self.fetch(
                &movie.key,
                found.as_ref().and_then(|f| f.backdrop.as_deref()),
                "w780",
                &label,
            );
        }
        self.cache.save_matches(&self.matches)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn the_cache_may_not_live_inside_the_library() {
        let root = env::temp_dir().join(format!("semmeltv-test-{}", std::process::id()));
        let cache = Cache::new(root.join("library/cache"));
        let client = Client::new(String::new());
        assert!(generate(&Library::default(), &root.join("library"), &cache, client).is_err());
        assert!(!root.exists());
    }

    #[test]
    fn files_with_the_same_name_keep_their_saved_positions_apart() {
        let cache = Cache::new("/cache".into());
        let one = cache.watch_later_dir(Path::new("/lib/Series/One/S01/E01.mkv"));
        let two = cache.watch_later_dir(Path::new("/lib/Series/Two/S01/E01.mkv"));
        assert_eq!(one, Path::new("/cache/watch-later/lib/Series/One/S01/E01.mkv"));
        assert_ne!(one, two);
    }

    #[test]
    fn a_tmdb_cache_without_runtimes_still_loads() {
        let matches: Matches = serde_json::from_str(r#"{"titles":{},"episode_names":{}}"#).unwrap();
        assert!(matches.runtimes.is_empty());
    }

    #[test]
    fn a_poster_wins_over_tmdb_art_and_no_art_means_none() {
        let root = env::temp_dir().join(format!("semmeltv-art-{}", std::process::id()));
        fs::create_dir_all(root.join("art")).unwrap();
        let cache = Cache::new(root.clone());
        assert_eq!(cache.card_art(None, "dark"), None);
        fs::write(cache.art("dark"), b"").unwrap();
        assert_eq!(cache.card_art(None, "dark"), Some(cache.art("dark")));
        let poster = Path::new("/lib/Series/Dark/poster.jpg");
        assert_eq!(cache.card_art(Some(poster), "dark"), Some(poster.to_path_buf()));
        fs::remove_dir_all(root).unwrap();
    }
}
