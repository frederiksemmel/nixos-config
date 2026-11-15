use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Sidecar(PathBuf),
    Embedded(PathBuf, usize),
}

pub async fn convert(source: &Source, delay: f64) -> Result<String, String> {
    let mut command = Command::new("ffmpeg");
    command.args(["-nostdin", "-hide_banner", "-loglevel", "error"]);
    match source {
        Source::Sidecar(path) => {
            if std::str::from_utf8(&tokio::fs::read(path).await.map_err(|error| error.to_string())?).is_err() {
                command.args(["-sub_charenc", "CP1252"]);
            }
            command.arg("-i").arg(path).args(["-map", "0:s:0"]);
        }
        Source::Embedded(video, index) => {
            command.arg("-i").arg(video).args(["-map", &format!("0:s:{index}")]);
        }
    }
    let output = command
        .args(["-c:s", "webvtt", "-f", "webvtt", "-"])
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|error| format!("ffmpeg: {error}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg: {}", error.lines().last().unwrap_or("failed")));
    }
    Ok(shift(&String::from_utf8_lossy(&output.stdout), delay))
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

fn format_time(seconds: f64) -> String {
    let millis = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        millis / 3_600_000,
        millis / 60_000 % 60,
        millis / 1000 % 60,
        millis % 1000
    )
}

/// One WebVTT cue: its identifier lines, timing and settings, and text.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub settings: String,
    pub before: Vec<String>,
    pub text: Vec<String>,
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
        let mut before = Vec::new();
        let mut timing = None;
        for line in lines.by_ref() {
            if let Some((start, rest)) = line.split_once("-->") {
                let rest = rest.trim_start();
                let (end, settings) = rest.split_once(' ').unwrap_or((rest, ""));
                timing = Some((parse_time(start), parse_time(end), settings.to_string()));
                break;
            }
            before.push(line.to_string());
        }
        if let Some((Some(start), Some(end), settings)) = timing {
            found.push(Cue {
                start,
                end,
                settings,
                before,
                text: lines.map(str::to_string).collect(),
            });
        }
    }
    found
}

/// A WebVTT file of `cues`, each moved by `delay`, leaving out those that would
/// end before the video starts.
pub fn render(cues: &[Cue], delay: f64) -> String {
    let mut out = String::from("WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:0,LOCAL:00:00:00.000\n");
    for cue in cues {
        let (start, end) = (cue.start + delay, cue.end + delay);
        if end <= 0.0 {
            continue;
        }
        out.push('\n');
        for line in &cue.before {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(&format!("{} --> {}", format_time(start), format_time(end)));
        if !cue.settings.is_empty() {
            out.push(' ');
            out.push_str(&cue.settings);
        }
        out.push('\n');
        for line in &cue.text {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

pub fn shift(vtt: &str, delay: f64) -> String {
    let mut whole = vtt.to_string();
    if !whole.ends_with("\n\n") {
        whole.push_str("\n\n");
    }
    render(&cues(&whole), delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FFMPEG_VTT: &str = "WEBVTT\n\n00:01.000 --> 00:03.000\nHello\n\n00:00:30.000 --> 00:00:33.500 align:start\n<i>World</i>\nagain\n\n";

    #[test]
    fn vtt_cues_shift_by_the_delay() {
        let late = "WEBVTT\n\n01:02:05.250 --> 01:02:06.000\nLate\n\n";
        let rows = [
            (
                FFMPEG_VTT,
                -2.0,
                vec![
                    "00:00:00.000 --> 00:00:01.000\nHello\n",
                    "00:00:28.000 --> 00:00:31.500 align:start\n<i>World</i>\nagain\n",
                ],
                vec![],
            ),
            (FFMPEG_VTT, -3.5, vec!["World"], vec!["Hello"]),
            (late, -2.0, vec!["01:02:03.250 --> 01:02:04.000\nLate\n"], vec![]),
        ];
        for (vtt, delay, kept, gone) in rows {
            let shifted = shift(vtt, delay);
            assert!(
                shifted.starts_with("WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:0,LOCAL:00:00:00.000\n"),
                "{delay}: {shifted}"
            );
            for cue in kept {
                assert!(shifted.contains(cue), "{delay}: {cue:?} in {shifted}");
            }
            for cue in gone {
                assert!(!shifted.contains(cue), "{delay}: {cue:?} in {shifted}");
            }
        }
    }

    #[test]
    fn a_cue_ffmpeg_is_still_writing_waits_for_its_end() {
        let growing = "WEBVTT\n\n00:01.000 --> 00:03.000\nHello\n\n00:00:30.000 --> 00:00:3";
        let found = cues(growing);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].start, found[0].end), (1.0, 3.0));
        assert_eq!(found[0].text, vec!["Hello".to_string()]);
        assert_eq!(cues(FFMPEG_VTT).len(), 2);
    }

}
