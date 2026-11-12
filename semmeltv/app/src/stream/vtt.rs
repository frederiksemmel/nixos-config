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

pub fn shift(vtt: &str, delay: f64) -> String {
    let mut out = String::from("WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:0,LOCAL:00:00:00.000\n");
    let mut blocks = vtt
        .split("\n\n")
        .map(|block| block.trim_matches(|c| c == '\n' || c == '\r'));
    blocks.next();
    for block in blocks.filter(|block| !block.is_empty()) {
        let mut lines = block.lines();
        let mut kept = Vec::new();
        let mut timing = None;
        for line in lines.by_ref() {
            if let Some((start, rest)) = line.split_once("-->") {
                let rest = rest.trim_start();
                let (end, settings) = rest.split_once(' ').unwrap_or((rest, ""));
                timing = Some((parse_time(start), parse_time(end), settings.to_string()));
                break;
            }
            kept.push(line);
        }
        let Some((Some(start), Some(end), settings)) = timing else {
            continue;
        };
        let (start, end) = (start + delay, end + delay);
        if end <= 0.0 {
            continue;
        }
        out.push('\n');
        for line in kept {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(&format!("{} --> {}", format_time(start), format_time(end)));
        if !settings.is_empty() {
            out.push(' ');
            out.push_str(&settings);
        }
        out.push('\n');
        for line in lines {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FFMPEG_VTT: &str = "WEBVTT\n\n00:01.000 --> 00:03.000\nHello\n\n00:00:30.000 --> 00:00:33.500 align:start\n<i>World</i>\nagain\n\n";

    #[test]
    fn cues_move_by_the_subtitle_delay_and_cues_before_the_start_go() {
        let shifted = shift(FFMPEG_VTT, -2.0);
        assert!(shifted.starts_with("WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:0,LOCAL:00:00:00.000\n"));
        assert!(shifted.contains("00:00:00.000 --> 00:00:01.000\nHello\n"));
        assert!(shifted.contains("00:00:28.000 --> 00:00:31.500 align:start\n<i>World</i>\nagain\n"));
        assert!(!shift(FFMPEG_VTT, -3.5).contains("Hello"));
    }

    #[test]
    fn hours_are_kept_past_the_first_hour() {
        assert_eq!(format_time(3723.25), "01:02:03.250");
        assert_eq!(parse_time("01:02:03.250"), Some(3723.25));
    }
}
