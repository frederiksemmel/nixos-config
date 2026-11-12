use crate::subtitles::{bibliographic, language_name};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

const SEARCH: &str = "https://rest.opensubtitles.org/search";
const AGENT: &str = "SemmelTV";
const TIME: Duration = Duration::from_secs(30);
const CHUNK: u64 = 64 * 1024;
const MOST_BYTES: u64 = 4 * 1024 * 1024;
const SAME_RELEASE: f64 = 0.6;
const ADVERTS: [&str; 4] = [
    "opensubtitles",
    "osdb.link",
    "advertise your product",
    "subtitles by explosiveskull",
];
const ENCODINGS: [&str; 5] = ["UTF-8", "ASCII", "CP1252", "ISO-8859-1", ""];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Wanted {
    pub video: String,
    pub imdb: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub language: String,
    pub hash: Option<String>,
    pub size: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Match {
    Hash,
    Release,
    Title,
}

#[derive(Clone, Debug, PartialEq)]
struct Candidate {
    id: String,
    link: String,
    release: String,
    hash: bool,
    likeness: f64,
    hearing_impaired: bool,
    trusted: bool,
    downloads: u64,
}

impl Candidate {
    fn matched(&self) -> Match {
        if self.hash {
            Match::Hash
        } else if self.likeness >= SAME_RELEASE {
            Match::Release
        } else {
            Match::Title
        }
    }

    fn score(&self) -> f64 {
        let mut score = self.likeness * 100.0 + (self.downloads as f64 + 1.0).log10() * 3.0;
        if self.hash {
            score += 1000.0;
        }
        if self.trusted {
            score += 5.0;
        }
        if self.hearing_impaired {
            score -= 5.0;
        }
        score
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fetched {
    pub file: String,
    pub language: String,
    pub release: String,
    pub matched: Match,
}

#[derive(Debug, PartialEq)]
pub struct Found {
    pub text: String,
    pub release: String,
    pub matched: Match,
}

pub fn hash(path: &Path) -> io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    if size < CHUNK * 2 {
        return Err(io::Error::other("too small for a movie hash"));
    }
    let mut sum = size;
    let mut chunk = vec![0; CHUNK as usize];
    for start in [0, size - CHUNK] {
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut chunk)?;
        for word in chunk.as_chunks::<8>().0 {
            sum = sum.wrapping_add(u64::from_le_bytes(*word));
        }
    }
    Ok((format!("{sum:016x}"), size))
}

fn words(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn likeness(release: &str, video: &str) -> f64 {
    let ours = words(video);
    let theirs = words(release);
    if ours.is_empty() || theirs.is_empty() {
        return 0.0;
    }
    ours.intersection(&theirs).count() as f64 / ours.union(&theirs).count() as f64
}

fn text(entry: &Value, key: &str) -> String {
    entry[key].as_str().unwrap_or_default().trim().to_string()
}

fn video_stem(video: &str) -> String {
    Path::new(video)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn candidates(results: &[Value], wanted: &Wanted, language: &str) -> Vec<Candidate> {
    let stem = video_stem(&wanted.video);
    let mut seen = HashSet::new();
    let mut found: Vec<Candidate> = results
        .iter()
        .filter(|entry| text(entry, "SubFormat").eq_ignore_ascii_case("srt"))
        .filter(|entry| text(entry, "SubLanguageID") == language)
        .filter(|entry| text(entry, "SubBad") != "1" && text(entry, "SubSumCD") == "1")
        .filter(|entry| text(entry, "SubForeignPartsOnly") != "1" && text(entry, "SubAutoTranslation") != "1")
        .filter(|entry| ENCODINGS.contains(&text(entry, "SubEncoding").to_ascii_uppercase().as_str()))
        .filter(|entry| match (wanted.season, wanted.episode) {
            (Some(season), Some(episode)) => {
                text(entry, "SeriesSeason") == season.to_string() && text(entry, "SeriesEpisode") == episode.to_string()
            }
            _ => true,
        })
        .filter(|entry| seen.insert(text(entry, "IDSubtitleFile")))
        .map(|entry| {
            let named = text(entry, "MovieReleaseName");
            let file = video_stem(&text(entry, "SubFileName"));
            Candidate {
                id: text(entry, "IDSubtitleFile"),
                link: text(entry, "SubDownloadLink"),
                hash: text(entry, "MatchedBy") == "moviehash",
                likeness: likeness(&named, &stem).max(likeness(&file, &stem)),
                release: if named.is_empty() { file } else { named },
                hearing_impaired: text(entry, "SubHearingImpaired") == "1",
                trusted: text(entry, "SubFromTrusted") == "1",
                downloads: text(entry, "SubDownloadsCnt").parse().unwrap_or(0),
            }
        })
        .filter(|candidate| !candidate.link.is_empty())
        .collect();
    found.sort_by(|a, b| b.score().total_cmp(&a.score()));
    found
}

fn searches(wanted: &Wanted, language: &str) -> Vec<String> {
    let mut urls = Vec::new();
    if let (Some(hash), Some(size)) = (&wanted.hash, wanted.size) {
        urls.push(format!(
            "{SEARCH}/moviebytesize-{size}/moviehash-{hash}/sublanguageid-{language}"
        ));
    }
    let digits = wanted.imdb.trim_start_matches("tt");
    if let Ok(number) = digits.parse::<u64>() {
        urls.push(match (wanted.season, wanted.episode) {
            (Some(season), Some(episode)) => {
                format!("{SEARCH}/episode-{episode}/imdbid-{number:07}/season-{season}/sublanguageid-{language}")
            }
            _ => format!("{SEARCH}/imdbid-{number:07}/sublanguageid-{language}"),
        });
    }
    urls
}

fn cp1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘',
        '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    match byte {
        0x80..=0x9f => HIGH[usize::from(byte - 0x80)],
        _ => char::from(byte),
    }
}

fn decode(bytes: Vec<u8>) -> String {
    let text = String::from_utf8(bytes).unwrap_or_else(|error| error.into_bytes().into_iter().map(cp1252).collect());
    text.trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

pub fn clean(text: &str) -> String {
    let mut kept = Vec::new();
    for block in text.split("\n\n").map(str::trim).filter(|block| !block.is_empty()) {
        let mut lines = block.lines();
        let first = lines.next().unwrap_or_default();
        let (timing, rest): (&str, Vec<&str>) = if first.contains("-->") {
            (first, lines.collect())
        } else {
            (lines.next().unwrap_or_default(), lines.collect())
        };
        if !timing.contains("-->") {
            continue;
        }
        let said = rest.join("\n").to_lowercase();
        if ADVERTS.iter().any(|advert| said.contains(advert)) {
            continue;
        }
        kept.push(format!("{}\n{}\n{}", kept.len() + 1, timing.trim(), rest.join("\n")));
    }
    let mut cleaned = kept.join("\n\n");
    cleaned.push('\n');
    cleaned
}

pub struct Client {
    agent: ureq::Agent,
}

impl Default for Client {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIME))
            .user_agent(AGENT)
            .build()
            .new_agent();
        Self { agent }
    }
}

impl Client {
    fn search(&self, url: &str) -> Result<Vec<Value>, String> {
        let mut response = self
            .agent
            .get(url)
            .call()
            .map_err(|error| format!("OpenSubtitles search: {error}"))?;
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|error| format!("OpenSubtitles search: {error}"))?;
        Ok(serde_json::from_str::<Vec<Value>>(&body).unwrap_or_default())
    }

    fn download(&self, link: &str) -> Result<String, String> {
        let mut response = self
            .agent
            .get(link)
            .call()
            .map_err(|error| format!("OpenSubtitles download: {error}"))?;
        let packed = response
            .body_mut()
            .with_config()
            .limit(MOST_BYTES)
            .read_to_vec()
            .map_err(|error| format!("OpenSubtitles download: {error}"))?;
        let mut bytes = Vec::new();
        GzDecoder::new(packed.as_slice())
            .take(MOST_BYTES * 4)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("OpenSubtitles sent a broken file: {error}"))?;
        Ok(clean(&decode(bytes)))
    }

    pub fn fetch(&self, wanted: &Wanted) -> Result<Found, String> {
        let name = language_name(&wanted.language).unwrap_or(&wanted.language);
        let language =
            bibliographic(&wanted.language).ok_or_else(|| format!("{name} is not a language I can look up"))?;
        let mut results = Vec::new();
        for url in searches(wanted, language) {
            results.extend(self.search(&url)?);
        }
        let mut last = None;
        for candidate in candidates(&results, wanted, language).into_iter().take(3) {
            match self.download(&candidate.link) {
                Ok(text) if text.matches("-->").count() >= 10 => {
                    return Ok(Found {
                        text,
                        release: candidate.release.clone(),
                        matched: candidate.matched(),
                    });
                }
                Ok(_) => last = Some(format!("The {name} subtitles from OpenSubtitles were empty")),
                Err(error) => last = Some(error),
            }
        }
        Err(last.unwrap_or_else(|| format!("No {name} subtitles found")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wanted() -> Wanted {
        Wanted {
            video: "Movies/Airplane 1980/Airplane.1980.REMASTERED.1080p.BluRay.x265-RARBG.mp4".to_string(),
            imdb: "tt0080339".to_string(),
            season: None,
            episode: None,
            language: "en".to_string(),
            hash: Some("8e245d9679d31e12".to_string()),
            size: Some(1_467_469_716),
        }
    }

    fn result(id: &str, release: &str, extra: Value) -> Value {
        let mut entry = json!({
            "IDSubtitleFile": id, "SubFormat": "srt", "SubLanguageID": "eng", "SubBad": "0", "SubSumCD": "1",
            "SubEncoding": "UTF-8", "MovieReleaseName": release, "SubFileName": format!("{release}.srt"),
            "MatchedBy": "imdbid", "SubHearingImpaired": "0", "SubDownloadsCnt": "100",
            "SubDownloadLink": format!("https://dl.example/{id}.gz"), "SeriesSeason": "0", "SeriesEpisode": "0",
        });
        if let (Some(entry), Some(extra)) = (entry.as_object_mut(), extra.as_object()) {
            entry.extend(extra.clone());
        }
        entry
    }

    #[test]
    fn searches_go_by_hash_first_then_by_imdb_id_with_padded_digits() {
        let mut episode = wanted();
        assert_eq!(
            searches(&episode, "eng"),
            [
                "https://rest.opensubtitles.org/search/moviebytesize-1467469716/moviehash-8e245d9679d31e12/sublanguageid-eng",
                "https://rest.opensubtitles.org/search/imdbid-0080339/sublanguageid-eng",
            ]
        );
        episode.hash = None;
        episode.imdb = "tt5753856".to_string();
        episode.season = Some(1);
        episode.episode = Some(2);
        assert_eq!(
            searches(&episode, "ger"),
            ["https://rest.opensubtitles.org/search/episode-2/imdbid-5753856/season-1/sublanguageid-ger"]
        );
    }

    #[test]
    fn a_hash_match_wins_then_the_closest_release_and_unusable_files_are_skipped() {
        let results = vec![
            result(
                "1",
                "Airplane.1980.DVDRip.XviD-DEViSE",
                json!({ "SubDownloadsCnt": "90000" }),
            ),
            result("2", "Airplane.1980.REMASTERED.1080p.BluRay.H264.AAC-RARBG", json!({})),
            result(
                "3",
                "Airplane.1980.REMASTERED.1080p.BluRay.x265-RARBG",
                json!({ "SubFormat": "sub" }),
            ),
            result(
                "4",
                "Airplane.1980.REMASTERED.1080p.BluRay.x265-RARBG",
                json!({ "SubEncoding": "CP1251" }),
            ),
            result(
                "5",
                "Airplane.1980.REMASTERED.1080p.BluRay.x265-RARBG",
                json!({ "SubSumCD": "2" }),
            ),
            result(
                "6",
                "Airplane.1980.REMASTERED.1080p.BluRay.x265-RARBG",
                json!({ "SubLanguageID": "ger" }),
            ),
            result("2", "Airplane.1980.REMASTERED.1080p.BluRay.H264.AAC-RARBG", json!({})),
        ];
        let ranked = candidates(&results, &wanted(), "eng");
        let ids: Vec<&str> = ranked.iter().map(|candidate| candidate.id.as_str()).collect();
        assert_eq!(ids, ["2", "1"]);
        assert_eq!(ranked[0].matched(), Match::Release);
        assert_eq!(ranked[1].matched(), Match::Title);
        let mut by_hash = results.clone();
        by_hash.push(result("7", "airplane", json!({ "MatchedBy": "moviehash" })));
        let ranked = candidates(&by_hash, &wanted(), "eng");
        assert_eq!(ranked[0].id, "7");
        assert_eq!(ranked[0].matched(), Match::Hash);
    }

    #[test]
    fn an_episode_takes_only_results_for_its_own_season_and_episode() {
        let mut episode = wanted();
        episode.video = "Series/Dark/Dark S01/DARK.S01E02.WEBRip.x264-ION10.mp4".to_string();
        episode.season = Some(1);
        episode.episode = Some(2);
        let results = vec![
            result(
                "1",
                "Dark.S01E01.WEBRip",
                json!({ "SeriesSeason": "1", "SeriesEpisode": "1" }),
            ),
            result(
                "2",
                "Dark.S01E02.WEBRip",
                json!({ "SeriesSeason": "1", "SeriesEpisode": "2" }),
            ),
        ];
        let ranked = candidates(&results, &episode, "eng");
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].id, "2");
    }

    #[test]
    fn adverts_are_dropped_and_the_cues_renumbered() {
        let text = "1\r\n00:00:06,000 --> 00:00:12,074\r\nWatch Online Movies and Series for FREE\r\nwww.osdb.link/lm\r\n\r\n2\r\n00:01:27,296 --> 00:01:29,287\r\nThe white zone\r\nis for immediate loading\r\n\r\n3\r\n00:01:29,464 --> 00:01:31,955\r\nand unloading of passengers only.\r\n";
        assert_eq!(
            clean(&decode(text.as_bytes().to_vec())),
            "1\n00:01:27,296 --> 00:01:29,287\nThe white zone\nis for immediate loading\n\n2\n00:01:29,464 --> 00:01:31,955\nand unloading of passengers only.\n"
        );
    }

    #[test]
    #[ignore = "asks OpenSubtitles.org over the network"]
    fn airplane_is_found_by_its_movie_hash() {
        let mut airplane = wanted();
        airplane.hash = Some("02df13421c697a86".to_string());
        let found = Client::default().fetch(&airplane).unwrap();
        assert_eq!(found.matched, Match::Hash);
        assert_eq!(found.release, "Airplane.1980.REMASTERED.1080p.BluRay.H264.AAC-RARBG");
        assert!(
            found
                .text
                .starts_with("1\n00:01:27,296 --> 00:01:29,287\nThe white zone")
        );
        let missing = Client::default().fetch(&Wanted {
            language: "ko".to_string(),
            hash: None,
            imdb: "tt0000001".to_string(),
            ..wanted()
        });
        assert_eq!(missing, Err("No Korean subtitles found".to_string()));
    }

    #[test]
    fn windows_1252_text_becomes_utf8() {
        assert_eq!(decode(vec![0xef, 0xbb, 0xbf, b'a']), "a");
        assert_eq!(decode(vec![b'C', b'a', b'f', 0xe9, b' ', 0x93, b'x', 0x94]), "Café “x”");
    }

    #[test]
    fn the_movie_hash_adds_the_size_and_the_first_and_last_64_kib() {
        let path = std::env::temp_dir().join(format!("semmeltv-hash-{}", std::process::id()));
        let mut bytes = vec![0u8; 200_000];
        bytes[0] = 1;
        bytes[199_999] = 2;
        std::fs::write(&path, &bytes).unwrap();
        let (hash, size) = super::hash(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(size, 200_000);
        assert_eq!(hash, format!("{:016x}", 200_000u64 + 1 + (2u64 << 56)));
        assert!(super::hash(Path::new("/nowhere")).is_err());
    }
}
