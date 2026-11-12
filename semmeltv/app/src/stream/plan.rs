use super::index::Index;
use std::fmt::Write;

const TARGET: f64 = 6.0;
const SHORTEST_LAST: f64 = 2.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub starts: Vec<f64>,
    pub duration: f64,
    keyframes: Vec<f64>,
}

impl Plan {
    pub fn new(index: &Index) -> Self {
        let mut starts = vec![0.0];
        for keyframe in &index.keyframes {
            let last = *starts.last().unwrap();
            if *keyframe - last >= TARGET && index.duration - *keyframe >= SHORTEST_LAST {
                starts.push(*keyframe);
            }
        }
        Self {
            starts,
            duration: index.duration,
            keyframes: index.keyframes.clone(),
        }
    }

    pub fn len(&self) -> usize {
        self.starts.len()
    }

    pub fn start(&self, segment: usize) -> f64 {
        self.starts[segment]
    }

    pub fn end(&self, segment: usize) -> f64 {
        self.starts.get(segment + 1).copied().unwrap_or(self.duration)
    }

    pub fn segment_at(&self, time: f64) -> usize {
        self.starts
            .partition_point(|start| *start <= time + 0.001)
            .saturating_sub(1)
    }

    pub fn seek_for(&self, segment: usize) -> Option<f64> {
        if segment == 0 {
            return None;
        }
        let start = self.start(segment);
        let before = self
            .keyframes
            .iter()
            .rev()
            .find(|keyframe| **keyframe < start - 0.001)
            .copied()
            .unwrap_or(0.0);
        Some(before + (start - before) / 4.0)
    }

    pub fn media_playlist(&self, start: f64) -> String {
        let longest = (0..self.len())
            .map(|segment| self.end(segment) - self.start(segment))
            .fold(0.0, f64::max);
        let mut text = String::new();
        let _ = write!(
            text,
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-MAP:URI=\"init.mp4\"\n",
            longest.round().max(1.0) as u64
        );
        if start > 0.0 {
            let _ = writeln!(text, "#EXT-X-START:TIME-OFFSET={start:.3},PRECISE=YES");
        }
        for segment in 0..self.len() {
            let _ = write!(
                text,
                "#EXTINF:{:.6},\nsegment/{segment}.m4s\n",
                self.end(segment) - self.start(segment)
            );
        }
        text.push_str("#EXT-X-ENDLIST\n");
        text
    }
}

pub fn subtitle_playlist(duration: f64, uri: &str) -> String {
    format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:{duration:.6},\n{uri}\n#EXT-X-ENDLIST\n",
        duration.ceil().max(1.0) as u64
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(keyframes: &[f64], duration: f64) -> Plan {
        Plan::new(&Index {
            duration,
            keyframes: keyframes.to_vec(),
        })
    }

    #[test]
    fn segments_gather_keyframes_until_six_seconds_have_passed() {
        let made = plan(&[0.0, 2.0, 4.0, 6.0, 8.0, 10.4, 12.5, 19.0, 21.0], 22.0);
        assert_eq!(made.starts, vec![0.0, 6.0, 12.5, 19.0]);
        assert_eq!(made.end(3), 22.0);
        assert_eq!(made.segment_at(0.0), 0);
        assert_eq!(made.segment_at(6.0), 1);
        assert_eq!(made.segment_at(11.0), 1);
        assert_eq!(made.segment_at(500.0), 3);
        assert_eq!(plan(&[0.0, 6.0, 21.0], 22.0).starts, vec![0.0, 6.0]);
    }

    #[test]
    fn a_run_for_a_segment_starts_inside_the_keyframe_interval_before_it() {
        let plan = plan(&[0.0, 2.0, 4.0, 6.0, 8.0, 10.4, 12.5, 19.0, 21.0], 22.0);
        assert_eq!(plan.seek_for(0), None);
        assert_eq!(plan.seek_for(1), Some(4.5));
        assert_eq!(plan.seek_for(2), Some(10.4 + (12.5 - 10.4) / 4.0));
    }

    #[test]
    fn the_media_playlist_lists_every_segment_with_its_exact_length() {
        let text = plan(&[0.0, 6.0, 12.0], 15.0).media_playlist(0.0);
        assert!(!text.contains("EXT-X-START"));
        assert!(
            plan(&[0.0, 6.0, 12.0], 15.0)
                .media_playlist(7.25)
                .contains("#EXT-X-START:TIME-OFFSET=7.250,PRECISE=YES\n")
        );
        assert!(text.contains("#EXT-X-TARGETDURATION:6\n"));
        assert!(text.contains("#EXTINF:6.000000,\nsegment/0.m4s\n"));
        assert!(text.contains("#EXTINF:3.000000,\nsegment/2.m4s\n#EXT-X-ENDLIST\n"));
    }
}
