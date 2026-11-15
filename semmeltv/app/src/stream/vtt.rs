use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Sidecar(PathBuf),
    /// A text track inside the video, which the remux writes out as it goes.
    Embedded(usize),
}

/// A subtitle file beside the video as WebVTT.
pub async fn convert(path: &Path) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command.args(["-nostdin", "-hide_banner", "-loglevel", "error"]);
    if std::str::from_utf8(&tokio::fs::read(path).await.map_err(|error| error.to_string())?).is_err() {
        command.args(["-sub_charenc", "CP1252"]);
    }
    let output = command
        .arg("-i")
        .arg(path)
        .args(["-map", "0:s:0", "-c:s", "webvtt", "-f", "webvtt", "-"])
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|error| format!("ffmpeg: {error}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg: {}", error.lines().last().unwrap_or("failed")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_time(text: &str) -> Option<f64> {
    let (clock, millis) = text.trim().split_once('.')?;
    let parts: Vec<f64> = clock.split(':').map(|part| part.parse().ok()).collect::<Option<_>>()?;
    let seconds = match parts.as_slice() {
        [hours, minutes, seconds] => hours * 3600.0 + minutes * 60.0 + seconds,
        [minutes, seconds] => minutes * 60.0 + seconds,
        _ => return None,
    };
    Some(seconds + millis.parse::<f64>().ok()? / 10f64.powi(millis.len() as i32))
}

/// One WebVTT cue, in seconds of file time.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// The cues of a WebVTT file. A file that ffmpeg is still writing may end in
/// a cue it hasn't finished, which is left out until it has.
pub fn cues(vtt: &str) -> Vec<Cue> {
    let vtt = vtt.replace("\r\n", "\n");
    let mut blocks: Vec<&str> = vtt.split("\n\n").collect();
    if !vtt.ends_with("\n\n") {
        blocks.pop();
    }
    let mut found = Vec::new();
    for block in blocks.into_iter().skip(1).map(|block| block.trim_matches('\n')) {
        let mut lines = block.lines();
        let mut timing = None;
        for line in lines.by_ref() {
            if let Some((start, rest)) = line.split_once("-->") {
                let end = rest.split_whitespace().next().unwrap_or("");
                timing = Some((parse_time(start), parse_time(end)));
                break;
            }
        }
        if let Some((Some(start), Some(end))) = timing {
            found.push(Cue {
                start,
                end,
                text: lines.collect::<Vec<_>>().join("\n"),
            });
        }
    }
    found
}

/// Every cue of a WebVTT file that is complete.
pub fn all_cues(vtt: &str) -> Vec<Cue> {
    cues(&format!("{vtt}\n\n"))
}

/// `cues` moved by `delay`, leaving out those that would end before the
/// video starts.
pub fn shifted(cues: Vec<Cue>, delay: f64) -> Vec<Cue> {
    cues.into_iter()
        .map(|cue| Cue {
            start: (cue.start + delay).max(0.0),
            end: cue.end + delay,
            text: cue.text,
        })
        .filter(|cue| cue.end > 0.0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FFMPEG_VTT: &str = "WEBVTT\n\n00:01.000 --> 00:03.000\nHello\n\n00:00:30.000 --> 00:00:33.500 align:start\n<i>World</i>\nagain\n\n";

    #[test]
    fn cues_move_by_the_subtitle_delay_and_cues_before_the_start_go() {
        let rows = [
            (
                FFMPEG_VTT,
                -2.0,
                vec![(0.0, 1.0, "Hello"), (28.0, 31.5, "<i>World</i>\nagain")],
            ),
            (FFMPEG_VTT, -3.5, vec![(26.5, 30.0, "<i>World</i>\nagain")]),
            (
                "WEBVTT\n\n01:02:05.250 --> 01:02:06.000\nLate\n\n",
                1.5,
                vec![(3726.75, 3727.5, "Late")],
            ),
        ];
        for (vtt, delay, expected) in rows {
            let found: Vec<(f64, f64, String)> = shifted(cues(vtt), delay)
                .into_iter()
                .map(|cue| (cue.start, cue.end, cue.text))
                .collect();
            let expected: Vec<(f64, f64, String)> = expected
                .into_iter()
                .map(|(start, end, text)| (start, end, text.to_string()))
                .collect();
            assert_eq!(found, expected, "{delay}");
        }
    }

    #[test]
    fn a_cue_ffmpeg_is_still_writing_waits_for_its_end() {
        let growing = "WEBVTT\n\n00:01.000 --> 00:03.000\nHello\n\n00:00:30.000 --> 00:00:3";
        let found = cues(growing);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].start, found[0].end), (1.0, 3.0));
        assert_eq!(found[0].text, "Hello");
        assert_eq!(cues(FFMPEG_VTT).len(), 2);
        let finished = "WEBVTT\n\n00:01.000 --> 00:03.000\nHello\n\n00:30.000 --> 00:33.000\nWorld\n";
        assert_eq!(cues(finished).len(), 1);
        assert_eq!(all_cues(finished).len(), 2);
        assert_eq!(all_cues(FFMPEG_VTT).len(), 2);
    }
}
