use crate::subtitles::{language_code, language_name};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use tokio::process::Command;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Media {
    pub video: Option<Video>,
    pub audio: Vec<Audio>,
    pub subtitles: Vec<Embedded>,
    pub bitrate: Option<u64>,
    pub credits: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Video {
    pub codec: String,
    pub profile: Option<String>,
    pub ten_bit: bool,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Audio {
    pub index: usize,
    pub codec: String,
    pub channels: u32,
    pub language: Option<String>,
    pub name: String,
    pub default: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Embedded {
    pub index: usize,
    pub codec: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub forced: bool,
    pub hearing_impaired: bool,
}

impl Embedded {
    pub fn is_text(&self) -> bool {
        matches!(
            self.codec.as_str(),
            "subrip" | "ass" | "ssa" | "webvtt" | "mov_text" | "text"
        )
    }
}

pub async fn probe(path: &Path) -> Result<Media, String> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-probesize",
            "2M",
            "-analyzeduration",
            "0",
            "-show_streams",
            "-show_chapters",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|error| format!("ffprobe: {error}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffprobe: {}", error.lines().last().unwrap_or("failed")));
    }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|error| format!("ffprobe: {error}"))?;
    Ok(parse(&value))
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).filter(|text| !text.is_empty())
}

fn tag<'a>(stream: &'a Value, key: &str) -> Option<&'a str> {
    stream.get("tags").and_then(|tags| text(tags, key))
}

fn flag(stream: &Value, key: &str) -> bool {
    stream
        .get("disposition")
        .and_then(|disposition| disposition.get(key))
        .and_then(Value::as_i64)
        == Some(1)
}

fn language(stream: &Value) -> Option<String> {
    let tagged = tag(stream, "language").filter(|code| *code != "und")?;
    Some(language_code(tagged).map_or_else(|| tagged.to_lowercase(), str::to_string))
}

pub fn parse(value: &Value) -> Media {
    let streams = value
        .get("streams")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut media = Media::default();
    let mut audio_index = 0;
    let mut subtitle_index = 0;
    for stream in &streams {
        let codec = text(stream, "codec_name").unwrap_or("").to_string();
        match text(stream, "codec_type") {
            Some("video") if media.video.is_none() && !flag(stream, "attached_pic") => {
                let pixels = text(stream, "pix_fmt").unwrap_or("");
                let profile = text(stream, "profile").map(str::to_string);
                media.video = Some(Video {
                    ten_bit: pixels.contains("10") || profile.as_deref().is_some_and(|profile| profile.contains("10")),
                    profile,
                    width: stream.get("width").and_then(Value::as_u64).unwrap_or(0) as u32,
                    height: stream.get("height").and_then(Value::as_u64).unwrap_or(0) as u32,
                    codec,
                });
            }
            Some("audio") => {
                let channels = stream.get("channels").and_then(Value::as_u64).unwrap_or(2) as u32;
                let language = language(stream);
                let named = language.as_deref().and_then(language_name).map(str::to_string);
                let layout = match channels {
                    1 => "Mono".to_string(),
                    2 => "Stereo".to_string(),
                    6 => "5.1".to_string(),
                    8 => "7.1".to_string(),
                    other => format!("{other} channels"),
                };
                let name = match (tag(stream, "title"), named) {
                    (Some(title), _) => title.to_string(),
                    (None, Some(named)) => format!("{named} {layout}"),
                    (None, None) => layout,
                };
                media.audio.push(Audio {
                    index: audio_index,
                    codec,
                    channels,
                    language,
                    name,
                    default: flag(stream, "default"),
                });
                audio_index += 1;
            }
            Some("subtitle") => {
                let title = tag(stream, "title").map(str::to_string);
                let lowered = title.as_deref().unwrap_or("").to_lowercase();
                media.subtitles.push(Embedded {
                    index: subtitle_index,
                    codec,
                    language: language(stream),
                    forced: flag(stream, "forced") || lowered.contains("forced"),
                    hearing_impaired: flag(stream, "hearing_impaired") || lowered.contains("sdh"),
                    title,
                });
                subtitle_index += 1;
            }
            _ => {}
        }
    }
    media.bitrate = value
        .get("format")
        .and_then(|format| text(format, "bit_rate"))
        .and_then(|rate| rate.parse().ok());
    media.credits = value
        .get("chapters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|chapter| tag(chapter, "title").is_some_and(|title| title.to_lowercase().contains("credits")))
        .and_then(|chapter| text(chapter, "start_time"))
        .and_then(|start| start.parse().ok());
    media
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Deserialize)]
pub struct Client {
    #[serde(default)]
    pub hevc: bool,
    #[serde(default)]
    pub eac3: bool,
    #[serde(default)]
    pub ac3: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Plays { hevc: bool, copy_audio: bool },
    Refused(String),
}

pub fn verdict(media: &Media, audio: Option<&Audio>, client: Client) -> Verdict {
    let Some(video) = &media.video else {
        return Verdict::Refused("This file has no video".to_string());
    };
    let hevc = match video.codec.as_str() {
        "hevc" if client.hevc => true,
        "hevc" => {
            return Verdict::Refused(
                "This browser can't play HEVC video. Safari on an iPhone, iPad or Mac can.".to_string(),
            );
        }
        "h264" if video.ten_bit => return Verdict::Refused("10-bit H.264 video doesn't play in browsers".to_string()),
        "h264" => false,
        "av1" => {
            return Verdict::Refused(
                "AV1 video doesn't play on this phone, and the TV box can't convert it".to_string(),
            );
        }
        other => return Verdict::Refused(format!("{} video only plays on the TV", other.to_uppercase())),
    };
    let copy_audio = audio.is_some_and(|audio| match audio.codec.as_str() {
        "aac" => true,
        "eac3" => client.eac3,
        "ac3" => client.ac3,
        _ => false,
    });
    Verdict::Plays { hevc, copy_audio }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn probed() -> Value {
        json!({
            "streams": [
                {"codec_type": "video", "codec_name": "hevc", "profile": "Main 10", "pix_fmt": "yuv420p10le", "width": 1920, "height": 1080},
                {"codec_type": "audio", "codec_name": "eac3", "channels": 6, "tags": {"language": "eng"}, "disposition": {"default": 1}},
                {"codec_type": "audio", "codec_name": "aac", "channels": 2, "tags": {"language": "ger", "title": "Kommentar"}},
                {"codec_type": "subtitle", "codec_name": "subrip", "tags": {"language": "eng", "title": "English (SDH)"}},
                {"codec_type": "subtitle", "codec_name": "hdmv_pgs_subtitle", "tags": {"language": "ger"}, "disposition": {"forced": 1}}
            ],
            "chapters": [
                {"start_time": "0.000000", "tags": {"title": "Opening"}},
                {"start_time": "5321.400000", "tags": {"title": "End Credits"}}
            ],
            "format": {"bit_rate": "4769403"}
        })
    }

    #[test]
    fn tracks_are_numbered_per_kind_and_named_for_people() {
        let media = parse(&probed());
        let video = media.video.as_ref().unwrap();
        assert!(video.ten_bit);
        assert_eq!(media.audio[0].name, "English 5.1");
        assert_eq!(media.audio[0].language.as_deref(), Some("en"));
        assert!(media.audio[0].default);
        assert_eq!(media.audio[1].name, "Kommentar");
        assert_eq!(media.audio[1].index, 1);
        assert!(media.subtitles[0].hearing_impaired && media.subtitles[0].is_text());
        assert!(media.subtitles[1].forced && !media.subtitles[1].is_text());
        assert_eq!(media.credits, Some(5321.4));
        assert_eq!(media.bitrate, Some(4_769_403));
    }

    #[test]
    fn safari_gets_the_files_audio_and_other_browsers_get_stereo_aac() {
        let media = parse(&probed());
        let iphone = Client {
            hevc: true,
            eac3: true,
            ac3: true,
        };
        assert_eq!(
            verdict(&media, media.audio.first(), iphone),
            Verdict::Plays {
                hevc: true,
                copy_audio: true
            }
        );
        let chrome = Client {
            hevc: true,
            ..Client::default()
        };
        assert_eq!(
            verdict(&media, media.audio.first(), chrome),
            Verdict::Plays {
                hevc: true,
                copy_audio: false
            }
        );
        assert!(matches!(verdict(&media, None, Client::default()), Verdict::Refused(_)));
    }
}
