use crate::subtitles::{language_code, language_name};
use std::fs;
use std::path::{Path, PathBuf};

const EXTENSIONS: [&str; 4] = ["srt", "ass", "ssa", "vtt"];
const VIDEO_EXTENSIONS: [&str; 7] = ["mkv", "mp4", "avi", "m4v", "mov", "webm", "ts"];
const FOLDERS: [&str; 3] = ["subs", "subtitles", "sub"];
const FORCED_SHARE: u64 = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
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
}
