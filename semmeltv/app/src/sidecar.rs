use crate::subtitles::{Offer, language_code, language_name, worth_offering};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use std::{fs, io, thread};

const EXTENSIONS: [&str; 4] = ["srt", "ass", "ssa", "vtt"];
const VIDEO_EXTENSIONS: [&str; 7] = ["mkv", "mp4", "avi", "m4v", "mov", "webm", "ts"];
const FOLDERS: [&str; 3] = ["subs", "subtitles", "sub"];
const FORCED_SHARE: u64 = 5;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidecar {
    pub path: PathBuf,
    pub language: Option<String>,
    pub title: String,
    pub forced: bool,
    pub hearing_impaired: bool,
}

#[derive(Debug)]
struct Entry {
    path: PathBuf,
    name: String,
    dir: bool,
    size: u64,
}

fn listing(dir: &Path) -> Vec<Entry> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<Entry> = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            Some(Entry {
                path: entry.path(),
                name: entry.file_name().to_string_lossy().into_owned(),
                dir: metadata.is_dir(),
                size: metadata.len(),
            })
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

fn extension(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
}

fn is_subtitle(entry: &Entry) -> bool {
    !entry.dir && extension(&entry.name).is_some_and(|extension| EXTENSIONS.contains(&extension.as_str()))
}

fn is_video(entry: &Entry) -> bool {
    !entry.dir && extension(&entry.name).is_some_and(|extension| VIDEO_EXTENSIONS.contains(&extension.as_str()))
}

fn stem(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

struct Described {
    language: Option<String>,
    forced: bool,
    hearing_impaired: bool,
}

fn describe(words: &str) -> Described {
    let mut described = Described {
        language: None,
        forced: false,
        hearing_impaired: false,
    };
    for word in words
        .split(['.', '_', '-', ' ', '(', ')', '[', ']'])
        .filter(|word| !word.is_empty())
    {
        match word.to_ascii_lowercase().as_str() {
            "forced" => described.forced = true,
            "sdh" | "hi" | "cc" => described.hearing_impaired = true,
            _ if word.bytes().all(|byte| byte.is_ascii_digit()) => {}
            _ if described.language.is_none() => described.language = language_code(word).map(str::to_string),
            _ => {}
        }
    }
    described
}

fn candidates(video: &Path) -> Vec<(Entry, String)> {
    let Some(dir) = video.parent() else {
        return Vec::new();
    };
    let video_name = video
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let video_stem = stem(&video_name);
    let lower_stem = video_stem.to_lowercase();
    let here = listing(dir);
    let alone = here.iter().filter(|entry| is_video(entry)).count() == 1;
    let mut found = Vec::new();
    for entry in &here {
        if entry.dir && FOLDERS.contains(&entry.name.to_lowercase().as_str()) {
            for inner in listing(&entry.path) {
                if inner.dir && inner.name.to_lowercase() == lower_stem {
                    for file in listing(&inner.path).into_iter().filter(is_subtitle) {
                        let words = stem(&file.name);
                        found.push((file, words));
                    }
                } else if alone && is_subtitle(&inner) {
                    let words = stem(&inner.name);
                    found.push((inner, words));
                }
            }
        } else if is_subtitle(entry) {
            let own = stem(&entry.name);
            let prefixed = own
                .get(..video_stem.len())
                .is_some_and(|start| start.to_lowercase() == lower_stem);
            let words = match prefixed {
                true => own[video_stem.len()..].to_string(),
                false if alone => own,
                false => continue,
            };
            found.push((
                Entry {
                    path: entry.path.clone(),
                    name: entry.name.clone(),
                    dir: false,
                    size: entry.size,
                },
                words,
            ));
        }
    }
    found
}

pub fn find(video: &Path) -> Vec<Sidecar> {
    let found: Vec<(Entry, Described)> = candidates(video)
        .into_iter()
        .map(|(entry, words)| {
            let described = describe(&words);
            (entry, described)
        })
        .collect();
    let largest = |language: &Option<String>| {
        found
            .iter()
            .filter(|(_, described)| &described.language == language)
            .map(|(entry, _)| entry.size)
            .max()
            .unwrap_or(0)
    };
    let mut sidecars: Vec<(u64, Sidecar)> = found
        .iter()
        .map(|(entry, described)| {
            let forced = described.forced || entry.size * FORCED_SHARE < largest(&described.language);
            let name = described
                .language
                .as_deref()
                .and_then(language_name)
                .unwrap_or("Subtitles");
            let mut title = name.to_string();
            if forced {
                title.push_str(" forced");
            }
            if described.hearing_impaired {
                title.push_str(" SDH");
            }
            (
                entry.size,
                Sidecar {
                    path: entry.path.clone(),
                    language: described.language.clone(),
                    title,
                    forced,
                    hearing_impaired: described.hearing_impaired,
                },
            )
        })
        .collect();
    sidecars.sort_by(|(size_a, a), (size_b, b)| {
        (
            &a.language,
            a.forced,
            a.hearing_impaired,
            std::cmp::Reverse(size_a),
            &a.path,
        )
            .cmp(&(
                &b.language,
                b.forced,
                b.hearing_impaired,
                std::cmp::Reverse(size_b),
                &b.path,
            ))
    });
    sidecars.into_iter().map(|(_, sidecar)| sidecar).collect()
}

/// The sidecars worth loading for someone who reads `languages`.
pub fn wanted(sidecars: Vec<Sidecar>, languages: &[String]) -> Vec<Sidecar> {
    let names: Vec<String> = sidecars
        .iter()
        .map(|sidecar| {
            sidecar
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let offers: Vec<Offer> = sidecars
        .iter()
        .zip(&names)
        .map(|(sidecar, name)| Offer {
            language: sidecar.language.as_deref(),
            title: Some(name),
            forced: sidecar.forced,
            hearing_impaired: sidecar.hearing_impaired,
            usable: true,
        })
        .collect();
    let keep = worth_offering(&offers, languages);
    sidecars
        .into_iter()
        .zip(keep)
        .filter_map(|(sidecar, keep)| keep.then_some(sidecar))
        .collect()
}

/// Reads every sidecar at once, so the pCloud mount holds them all before mpv
/// adds them one after another, which costs about a second each when they
/// are not cached yet. The ones not read `within` stay out of this play and
/// are read to the end behind it, ready for the next.
pub fn warm(sidecars: Vec<Sidecar>, within: Duration) -> Vec<Sidecar> {
    let until = Instant::now() + within;
    let (done, heard) = mpsc::channel();
    for (index, sidecar) in sidecars.iter().enumerate() {
        let (done, path) = (done.clone(), sidecar.path.clone());
        thread::spawn(move || {
            let read = fs::File::open(&path).and_then(|mut file| io::copy(&mut file, &mut io::sink()));
            let _ = done.send((index, read.is_ok()));
        });
    }
    drop(done);
    let mut read = vec![false; sidecars.len()];
    let mut answered = 0;
    while answered < sidecars.len() {
        let Ok((index, ok)) = heard.recv_timeout(until.saturating_duration_since(Instant::now())) else {
            break;
        };
        read[index] = ok;
        answered += 1;
    }
    if answered < sidecars.len() {
        eprintln!(
            "subtitles: {} of {} files were not read within {within:?}, so this play goes without them",
            sidecars.len() - answered,
            sidecars.len()
        );
    }
    sidecars
        .into_iter()
        .zip(read)
        .filter_map(|(sidecar, read)| read.then_some(sidecar))
        .collect()
}

pub fn file_name(video: &Path, language: &str) -> Option<String> {
    let stem = video.file_stem()?.to_string_lossy();
    Some(format!("{stem}.{language}.srt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tree(PathBuf);

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn tree(name: &str, files: &[(&str, usize)]) -> Tree {
        let root = std::env::temp_dir().join(format!("semmeltv-sidecar-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (file, size) in files {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, vec![b'x'; *size]).unwrap();
        }
        Tree(root)
    }

    fn summary(sidecars: &[Sidecar], root: &Path) -> Vec<String> {
        sidecars
            .iter()
            .map(|sidecar| {
                format!(
                    "{} {} {}",
                    sidecar.path.strip_prefix(root).unwrap().display(),
                    sidecar.language.as_deref().unwrap_or("-"),
                    sidecar.title
                )
            })
            .collect()
    }

    #[test]
    fn a_film_takes_the_subs_folder_beside_it() {
        let tree = tree(
            "film",
            &[
                ("Airplane 1980/Airplane.1980.mp4", 10),
                ("Airplane 1980/RARBG.txt", 10),
                ("Airplane 1980/Subs/13_English.srt", 800),
                ("Airplane 1980/Subs/14_English.srt", 100),
                ("Airplane 1980/Subs/2_nor.srt", 700),
                ("Airplane 1980/Subs/3_English-SDH.srt", 900),
                ("Airplane 1980/Subs/4_es-419.srt", 700),
                ("Airplane 1980/Subs/5_Bokmal.srt", 700),
                ("Airplane 1980/Airplane.1980.de.srt", 600),
            ],
        );
        let found = find(&tree.0.join("Airplane 1980/Airplane.1980.mp4"));
        assert_eq!(
            summary(&found, &tree.0),
            [
                "Airplane 1980/Airplane.1980.de.srt de German",
                "Airplane 1980/Subs/13_English.srt en English",
                "Airplane 1980/Subs/3_English-SDH.srt en English SDH",
                "Airplane 1980/Subs/14_English.srt en English forced",
                "Airplane 1980/Subs/4_es-419.srt es Spanish",
                "Airplane 1980/Subs/2_nor.srt no Norwegian",
                "Airplane 1980/Subs/5_Bokmal.srt no Norwegian",
            ]
        );
        assert!(found[3].forced);
        assert!(found[2].hearing_impaired);
        let languages = ["en".to_string(), "de".to_string()];
        assert_eq!(
            summary(&wanted(found, &languages), &tree.0),
            [
                "Airplane 1980/Airplane.1980.de.srt de German",
                "Airplane 1980/Subs/13_English.srt en English",
                "Airplane 1980/Subs/3_English-SDH.srt en English SDH",
                "Airplane 1980/Subs/14_English.srt en English forced",
            ]
        );
    }

    #[test]
    fn of_twenty_seven_languages_an_episode_loads_the_ones_read_here() {
        let tree = tree(
            "thrones",
            &[
                ("GoT S01/GoT.S01E01.mkv", 10),
                ("GoT S01/Subs/GoT.S01E01/1000_en-US.srt", 900),
                ("GoT S01/Subs/GoT.S01E01/1001_en-US.srt", 800),
                ("GoT S01/Subs/GoT.S01E01/1002_bg-BG.srt", 800),
                ("GoT S01/Subs/GoT.S01E01/1005_es-419.srt", 800),
                ("GoT S01/Subs/GoT.S01E01/1025_sr-Latn-RS.srt", 800),
                ("GoT S01/Subs/GoT.S01E01/7_English.srt", 850),
                ("GoT S01/Subs/GoT.S01E01/8_English.srt", 100),
                ("GoT S01/Subs/GoT.S01E01/9_German Commentary.srt", 700),
            ],
        );
        let found = find(&tree.0.join("GoT S01/GoT.S01E01.mkv"));
        assert_eq!(found.len(), 8);
        let languages = ["en".to_string(), "de".to_string()];
        assert_eq!(
            summary(&wanted(found, &languages), &tree.0),
            [
                "GoT S01/Subs/GoT.S01E01/1000_en-US.srt en English",
                "GoT S01/Subs/GoT.S01E01/8_English.srt en English forced",
            ]
        );
    }

    #[test]
    fn an_episode_takes_only_its_own_folder_inside_subs() {
        let tree = tree(
            "episodes",
            &[
                ("Dark S01/DARK.S01E01.WEBRip.mp4", 10),
                ("Dark S01/DARK.S01E02.WEBRip.mp4", 10),
                ("Dark S01/Subs/DARK.S01E01.WEBRip/2_English.srt", 500),
                ("Dark S01/Subs/DARK.S01E01.WEBRip/5_German.srt", 500),
                ("Dark S01/Subs/DARK.S01E02.WEBRip/2_English.srt", 500),
                ("Dark S01/Subs/stray.srt", 500),
                ("Dark S01/DARK.S01E01.WEBRip.en.srt", 500),
                ("Dark S01/DARK.S01E02.WEBRip.srt", 500),
            ],
        );
        let found = find(&tree.0.join("Dark S01/DARK.S01E01.WEBRip.mp4"));
        assert_eq!(
            summary(&found, &tree.0),
            [
                "Dark S01/Subs/DARK.S01E01.WEBRip/5_German.srt de German",
                "Dark S01/DARK.S01E01.WEBRip.en.srt en English",
                "Dark S01/Subs/DARK.S01E01.WEBRip/2_English.srt en English",
            ]
        );
        let second = find(&tree.0.join("Dark S01/DARK.S01E02.WEBRip.mp4"));
        assert_eq!(
            summary(&second, &tree.0),
            [
                "Dark S01/DARK.S01E02.WEBRip.srt - Subtitles",
                "Dark S01/Subs/DARK.S01E02.WEBRip/2_English.srt en English",
            ]
        );
    }

    #[test]
    fn a_missing_folder_has_no_sidecars_and_fetched_files_are_named_after_the_video() {
        assert!(find(Path::new("/nowhere/at/all.mkv")).is_empty());
        assert_eq!(
            file_name(Path::new("/lib/Movies/Up (2009)/Up.2009.1080p.mkv"), "en").as_deref(),
            Some("Up.2009.1080p.en.srt")
        );
    }

    #[test]
    fn subtitles_still_unread_when_the_time_is_up_are_left_out_of_the_play() {
        let tree = tree("warm", &[("Dark S01/Subs/2_English.srt", 500)]);
        let hung = tree.0.join("Dark S01/Subs/5_German.srt");
        let fifo = std::ffi::CString::new(hung.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let sidecar = |name: &str| Sidecar {
            path: tree.0.join("Dark S01/Subs").join(name),
            language: None,
            title: name.to_string(),
            forced: false,
            hearing_impaired: false,
        };
        let sidecars = vec![sidecar("2_English.srt"), sidecar("5_German.srt"), sidecar("gone.srt")];
        let asked = std::time::Instant::now();
        let warmed = warm(sidecars, Duration::from_millis(200));
        assert!(asked.elapsed() < Duration::from_millis(400), "{:?}", asked.elapsed());
        assert_eq!(warmed, [sidecar("2_English.srt")]);
        drop(fs::OpenOptions::new().write(true).open(&hung));
    }

    #[test]
    fn subtitles_are_read_side_by_side_not_one_after_another() {
        let names: Vec<String> = (0..30).map(|n| format!("{n}_English.srt")).collect();
        let tree = tree("side-by-side", &[]);
        fs::create_dir_all(&tree.0).unwrap();
        let mut fifos = Vec::new();
        for name in &names {
            let path = tree.0.join(name);
            let fifo = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
            fifos.push(path);
        }
        let sidecars: Vec<Sidecar> = fifos
            .iter()
            .map(|path| Sidecar {
                path: path.clone(),
                language: None,
                title: String::new(),
                forced: false,
                hearing_impaired: false,
            })
            .collect();
        for path in fifos {
            std::thread::spawn(move || {
                let writer = fs::OpenOptions::new().write(true).open(path);
                std::thread::sleep(Duration::from_millis(100));
                drop(writer);
            });
        }
        let warmed = warm(sidecars.clone(), Duration::from_secs(1));
        assert_eq!(warmed, sidecars);
    }
}
