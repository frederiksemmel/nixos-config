use regex::Regex;
use std::iter;
use std::path::Path;
use std::sync::LazyLock;

static VIDEO: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
    words(&[
        (r"\b(?:[xh][ .]?265|hevc)\b", "HEVC"),
        (r"\b(?:[xh][ .]?264|avc)\b", "H.264"),
        (r"\bav1\b", "AV1"),
        (r"\bvp9\b", "VP9"),
        (r"\bvc[ .-]?1\b", "VC-1"),
        (r"\bmpeg[ .-]?2\b", "MPEG-2"),
        (r"\bxvid\b", "XviD"),
        (r"\bdivx\b", "DivX"),
    ])
});
static AUDIO_CODECS: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
    words(&[
        (r"\b(?:ddp|e-?ac-?3)\b", "DD+"),
        (r"\bac-?3\b", "AC3"),
        (r"\baac\b", "AAC"),
        (r"\bdts\b", "DTS"),
        (r"\btruehd\b", "TrueHD"),
        (r"\bflac\b", "FLAC"),
        (r"\bopus\b", "Opus"),
        (r"\bmp3\b", "MP3"),
    ])
});
static TEN_BIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b10[ .-]?bits?\b").unwrap());
static RESOLUTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(?:2160|1440|1080|720|576|480)[pi]\b").unwrap());
static ULTRA_HD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(?:4k|uhd)\b").unwrap());
static CHANNELS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)\b(?:
            ddp?\+? | e?-?ac-?3 | aac | truehd | flac | opus | l?pcm | mp3
            | dts(?:[\s.-]?(?:hd|x|es))?(?:[\s.-]?ma)?
        )[\s.+-]*(?:atmos[\s.+-]*)?([1-9])[\s.]([01])\b",
    )
    .unwrap()
});
static CHANNEL_COUNT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b([268])ch\b").unwrap());
static ATMOS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\batmos\b").unwrap());

fn words(patterns: &[(&str, &'static str)]) -> Vec<(Regex, &'static str)> {
    patterns
        .iter()
        .map(|(pattern, name)| (Regex::new(pattern).unwrap(), *name))
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tags {
    pub video: Option<String>,
    pub resolution: Option<String>,
    pub audio: Option<String>,
}

impl Tags {
    fn or(self, other: Self) -> Self {
        Self {
            video: self.video.or(other.video),
            resolution: self.resolution.or(other.resolution),
            audio: self.audio.or(other.audio),
        }
    }
}

pub fn from_path(path: &str) -> Tags {
    let path = Path::new(path);
    let stem = path.file_stem().into_iter();
    let folders = path.ancestors().skip(1).filter_map(Path::file_name);
    stem.chain(folders)
        .map(|name| from_name(&name.to_string_lossy()))
        .fold(Tags::default(), Tags::or)
}

pub fn from_name(name: &str) -> Tags {
    let name = name.to_lowercase().replace('_', " ");
    Tags {
        video: video(&name),
        resolution: resolution(&name),
        audio: audio(&name, release_info(&name)),
    }
}

fn release_info(name: &str) -> &str {
    let video = VIDEO.iter().filter_map(|(pattern, _)| pattern.find(name));
    let start = iter::once(RESOLUTION.find(name))
        .flatten()
        .chain(video)
        .map(|found| found.start())
        .min();
    start.map_or("", |start| &name[start..])
}

fn video(name: &str) -> Option<String> {
    let codec = VIDEO.iter().find(|(pattern, _)| pattern.is_match(name));
    match (codec, TEN_BIT.is_match(name)) {
        (Some((_, codec)), true) => Some(format!("{codec} 10-bit")),
        (Some((_, codec)), false) => Some((*codec).to_string()),
        (None, true) => Some("10-bit".to_string()),
        (None, false) => None,
    }
}

fn resolution(name: &str) -> Option<String> {
    let found = RESOLUTION.find(name).map(|found| found.as_str().to_string());
    found.or_else(|| ULTRA_HD.is_match(name).then(|| "2160p".to_string()))
}

fn channels(name: &str) -> Option<String> {
    let named = CHANNELS
        .captures(name)
        .map(|found| format!("{}.{}", &found[1], &found[2]));
    named.or_else(|| {
        let count = CHANNEL_COUNT.captures(name)?;
        let channels = match &count[1] {
            "8" => "7.1",
            "6" => "5.1",
            _ => "2.0",
        };
        Some(channels.to_string())
    })
}

fn audio(name: &str, release_info: &str) -> Option<String> {
    let codec = || AUDIO_CODECS.iter().find(|(pattern, _)| pattern.is_match(release_info));
    match (channels(name), ATMOS.is_match(release_info)) {
        (Some(channels), true) => Some(format!("{channels} Atmos")),
        (Some(channels), false) => Some(channels),
        (None, true) => Some("Atmos".to_string()),
        (None, false) => codec().map(|(_, codec)| (*codec).to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(path: &str) -> (Option<String>, Option<String>, Option<String>) {
        let tags = from_path(path);
        (tags.video, tags.resolution, tags.audio)
    }

    fn expect(video: &str, resolution: &str, audio: &str) -> (Option<String>, Option<String>, Option<String>) {
        let known = |text: &str| Some(text.to_string()).filter(|text| !text.is_empty());
        (known(video), known(resolution), known(audio))
    }

    #[test]
    fn real_release_names_give_their_tags() {
        for (name, video, resolution, audio) in [
            (
                "Anyone.But.You.2023.REPACK.1080p.WEB-DL.DD5.1.H.264-EniaHD.mkv",
                "H.264",
                "1080p",
                "5.1",
            ),
            (
                "Barbie 2023 MULTiSUBS 1080p WEB-DL DD5.1 HEVC x265-RMTeam .mkv",
                "HEVC",
                "1080p",
                "5.1",
            ),
            (
                "House.of.the.Dragon.S02E08.1080p.WEBRip.10bit.DDP5.1.x265-HODL.mkv",
                "HEVC 10-bit",
                "1080p",
                "5.1",
            ),
            (
                "House.of.the.Dragon.S02E06.1080p.10bit.WEBRip.6CH.x265.HEVC-PSA.mkv",
                "HEVC 10-bit",
                "1080p",
                "5.1",
            ),
            (
                "Silicon.Valley.S04E05.The.Blood.Boy.1080p.AMZN.WEB-DL.DD.5.1.H.265-SiGMA.mkv",
                "HEVC",
                "1080p",
                "5.1",
            ),
            (
                "The.White.Lotus.S01E06.Departures.1080p.AMZN.WEBRip.DDP.5.1.x265-edge2020.mkv",
                "HEVC",
                "1080p",
                "5.1",
            ),
            (
                "The.Last.of.Us.S01E03.Long.Long.Time.1080p.HMAX.WEB-DL.DDP5.1.Atmos.H.264-SMURF.mkv",
                "H.264",
                "1080p",
                "5.1 Atmos",
            ),
            (
                "Wonka 2023 1080p PROPER WEBRip AV1 Opus 5.1 [981].mkv",
                "AV1",
                "1080p",
                "5.1",
            ),
            (
                "Jojo.Rabbit.2019.1080p.BluRay.H264.AAC-RARBG.mp4",
                "H.264",
                "1080p",
                "AAC",
            ),
            (
                "Greyhound.2020.1080p.ATVP.WEBRip.1400MB.DD5.1.x264-GalaxyRG.mkv",
                "H.264",
                "1080p",
                "5.1",
            ),
            (
                "Foundation (2021) - S02E10 - Creation Myths (1080p ATVP WEB-DL x265 Silence).mkv",
                "HEVC",
                "1080p",
                "",
            ),
            (
                "Oppenheimer.2023.2160p.UHD.BluRay.REMUX.HEVC.TrueHD.Atmos.7.1-FGT.mkv",
                "HEVC",
                "2160p",
                "7.1 Atmos",
            ),
            (
                "Heat.1995.1080p.BluRay.AVC.DTS-HD.MA.5.1-SiCaRiO.mkv",
                "H.264",
                "1080p",
                "5.1",
            ),
            ("Up.2009.720p.BluRay.x264.AAC2.0-YTS.mp4", "H.264", "720p", "2.0"),
            ("Dune.2021.4K.WEB-DL.DDP5.1.AV1.mkv", "AV1", "2160p", "5.1"),
            ("DARK.S01E03.WEBRip.x264-ION10.mp4", "H.264", "", ""),
        ] {
            assert_eq!(tags(name), expect(video, resolution, audio), "{name}");
        }
    }

    #[test]
    fn a_name_without_tags_gives_nothing() {
        for name in [
            "Chernobyl S01E01 1 23 45.mkv",
            "Marvels Daredevil S02E06.mp4",
            "Ip Man 4.mp4",
            "Mr Hollands Opus 1995.mkv",
            "Atmos.mkv",
            "Fantastic Beasts The Crimes of Grindelwald.mp4",
        ] {
            assert_eq!(tags(name), expect("", "", ""), "{name}");
        }
    }

    #[test]
    fn folders_fill_what_the_file_name_leaves_out_and_never_override_it() {
        assert_eq!(
            tags("Movies/Shutter Island (2010) (1080p BluRay x265 HEVC 10bit AAC 5.1 Tigole)/Shutter.Island.mkv"),
            expect("HEVC 10-bit", "1080p", "5.1")
        );
        assert_eq!(tags("Movies/Brave (2012) [1080p]/Brave.mp4"), expect("", "1080p", ""));
        assert_eq!(
            tags("Series/Dark 1080p x265/Dark.S01E01.720p.mkv"),
            expect("HEVC", "720p", "")
        );
    }
}
