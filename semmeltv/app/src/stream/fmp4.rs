use super::index::{edit_shift, mp4_boxes, mp4_child};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Init {
    pub video: u32,
    pub timescale: u32,
    pub shift: i64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub start: f64,
    pub dts: i64,
    pub dts_end: i64,
}

fn be32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

pub fn init(data: &[u8]) -> Option<Init> {
    let moov = mp4_child(data, b"moov")?;
    for (_, trak) in mp4_boxes(moov).into_iter().filter(|(kind, _)| kind == b"trak") {
        let mdia = mp4_child(trak, b"mdia")?;
        if mp4_child(mdia, b"hdlr").and_then(|hdlr| hdlr.get(8..12)) != Some(b"vide") {
            continue;
        }
        let tkhd = mp4_child(trak, b"tkhd")?;
        let video = if tkhd.first() == Some(&1) {
            be32(tkhd, 20)?
        } else {
            be32(tkhd, 12)?
        };
        let mdhd = mp4_child(mdia, b"mdhd")?;
        let timescale = if mdhd.first() == Some(&1) {
            be32(mdhd, 20)?
        } else {
            be32(mdhd, 12)?
        };
        let (shift, _) = edit_shift(trak);
        return (timescale > 0).then_some(Init {
            video,
            timescale,
            shift,
        });
    }
    None
}

struct Run {
    tfdt: i64,
    first_offset: i64,
    total: i64,
}

fn traf(traf: &[u8]) -> Option<(u32, Run)> {
    let tfhd = mp4_child(traf, b"tfhd")?;
    let flags = be32(tfhd, 0)? & 0x00FF_FFFF;
    let track = be32(tfhd, 4)?;
    let mut at = 8;
    if flags & 0x1 != 0 {
        at += 8;
    }
    if flags & 0x2 != 0 {
        at += 4;
    }
    let default_duration = if flags & 0x8 != 0 { Some(be32(tfhd, at)?) } else { None };
    let tfdt = mp4_child(traf, b"tfdt")?;
    let decode = if tfdt.first() == Some(&1) {
        i64::from_be_bytes(tfdt.get(4..12)?.try_into().ok()?)
    } else {
        i64::from(be32(tfdt, 4)?)
    };
    let mut total: i64 = 0;
    let mut first_offset = None;
    for (_, trun) in mp4_boxes(traf).into_iter().filter(|(kind, _)| kind == b"trun") {
        let version = trun.first().copied()?;
        let flags = be32(trun, 0)? & 0x00FF_FFFF;
        let count = be32(trun, 4)?;
        let mut at = 8;
        if flags & 0x1 != 0 {
            at += 4;
        }
        if flags & 0x4 != 0 {
            at += 4;
        }
        for _ in 0..count {
            let duration = if flags & 0x100 != 0 {
                let duration = be32(trun, at)?;
                at += 4;
                duration
            } else {
                default_duration?
            };
            if flags & 0x200 != 0 {
                at += 4;
            }
            if flags & 0x400 != 0 {
                at += 4;
            }
            let offset = if flags & 0x800 != 0 {
                let raw = be32(trun, at)?;
                at += 4;
                if version == 1 {
                    i64::from(raw as i32)
                } else {
                    i64::from(raw)
                }
            } else {
                0
            };
            first_offset.get_or_insert(offset);
            total += i64::from(duration);
        }
    }
    Some((
        track,
        Run {
            tfdt: decode,
            first_offset: first_offset.unwrap_or(0),
            total,
        },
    ))
}

pub fn timing(data: &[u8], init: &Init) -> Option<Timing> {
    let mut found: Option<Timing> = None;
    for (_, moof) in mp4_boxes(data).into_iter().filter(|(kind, _)| kind == b"moof") {
        for (_, body) in mp4_boxes(moof).into_iter().filter(|(kind, _)| kind == b"traf") {
            let Some((track, run)) = traf(body) else {
                continue;
            };
            if track != init.video {
                continue;
            }
            let end = run.tfdt + run.total;
            found = Some(match found {
                Some(timing) => Timing { dts_end: end, ..timing },
                None => Timing {
                    start: (run.tfdt + run.first_offset - init.shift) as f64 / f64::from(init.timescale),
                    dts: run.tfdt,
                    dts_end: end,
                },
            });
        }
    }
    found
}

struct Patch {
    video: u32,
    offset: i64,
    track: u32,
}

fn walk(data: &mut [u8], base: usize, end: usize, sequence: &mut u32, patch: &mut Patch) {
    let mut at = base;
    while at + 8 <= end {
        let size = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        if size < 8 || at + size > end {
            return;
        }
        let kind: [u8; 4] = data[at + 4..at + 8].try_into().unwrap();
        match &kind {
            b"moof" | b"traf" => walk(data, at + 8, at + size, sequence, patch),
            b"tfhd" if size >= 16 => {
                patch.track = u32::from_be_bytes(data[at + 12..at + 16].try_into().unwrap());
            }
            b"mfhd" if size >= 16 => {
                data[at + 12..at + 16].copy_from_slice(&sequence.to_be_bytes());
                *sequence += 1;
            }
            b"tfdt" if size >= 20 && data[at + 8] == 1 => {
                let mut value = i64::from_be_bytes(data[at + 12..at + 20].try_into().unwrap());
                if patch.track == patch.video {
                    value -= patch.offset;
                }
                data[at + 12..at + 20].copy_from_slice(&value.max(0).to_be_bytes());
            }
            b"tfdt" if size >= 16 && patch.track == patch.video && patch.offset != 0 => {
                let value = i64::from(u32::from_be_bytes(data[at + 12..at + 16].try_into().unwrap())) - patch.offset;
                data[at + 12..at + 16].copy_from_slice(&(value.max(0) as u32).to_be_bytes());
            }
            _ => {}
        }
        at += size;
    }
}

pub fn media(data: &[u8], sequence: &mut u32, video: u32, offset: i64) -> Vec<u8> {
    let mut at = 0;
    let mut start = None;
    while at + 8 <= data.len() {
        let size = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        if size < 8 || at + size > data.len() {
            break;
        }
        if &data[at + 4..at + 8] == b"moof" {
            start = Some(at);
            break;
        }
        at += size;
    }
    let Some(start) = start else {
        return Vec::new();
    };
    let mut out = data[start..].to_vec();
    let end = out.len();
    let mut patch = Patch {
        video,
        offset,
        track: 0,
    };
    walk(&mut out, 0, end, sequence, &mut patch);
    out
}

fn sample_entry<'a>(init: &'a [u8], handler: &[u8; 4]) -> Option<([u8; 4], &'a [u8])> {
    let moov = mp4_child(init, b"moov")?;
    for (_, trak) in mp4_boxes(moov).into_iter().filter(|(kind, _)| kind == b"trak") {
        let mdia = mp4_child(trak, b"mdia")?;
        if mp4_child(mdia, b"hdlr").and_then(|hdlr| hdlr.get(8..12)) != Some(handler) {
            continue;
        }
        let stsd = mp4_child(mp4_child(mp4_child(mdia, b"minf")?, b"stbl")?, b"stsd")?;
        let (kind, body) = mp4_boxes(stsd.get(8..)?).into_iter().next()?;
        return Some((kind, body));
    }
    None
}

fn hevc_codec(kind: &[u8; 4], config: &[u8]) -> Option<String> {
    let profile_byte = *config.get(1)?;
    let space = ["", "A", "B", "C"][usize::from(profile_byte >> 6)];
    let tier = if profile_byte & 0x20 != 0 { "H" } else { "L" };
    let profile = profile_byte & 0x1F;
    let compatibility = be32(config, 2)?.reverse_bits();
    let constraints = config.get(6..12)?;
    let level = *config.get(12)?;
    let mut text = format!(
        "{}.{space}{profile}.{compatibility:X}.{tier}{level}",
        String::from_utf8_lossy(kind)
    );
    let used = constraints
        .iter()
        .rposition(|byte| *byte != 0)
        .map_or(0, |last| last + 1);
    for byte in &constraints[..used] {
        text.push_str(&format!(".{byte:X}"));
    }
    Some(text)
}

pub fn codecs(init: &[u8]) -> Option<String> {
    let (kind, entry) = sample_entry(init, b"vide")?;
    let config_boxes = mp4_boxes(entry.get(78..)?);
    let video = match &kind {
        b"hvc1" | b"hev1" => {
            let (_, config) = config_boxes.iter().find(|(kind, _)| kind == b"hvcC")?;
            hevc_codec(&kind, config)?
        }
        b"avc1" | b"avc3" => {
            let (_, config) = config_boxes.iter().find(|(kind, _)| kind == b"avcC")?;
            format!(
                "{}.{:02X}{:02X}{:02X}",
                String::from_utf8_lossy(&kind),
                config.get(1)?,
                config.get(2)?,
                config.get(3)?
            )
        }
        _ => return None,
    };
    let audio = match sample_entry(init, b"soun") {
        Some((kind, _)) => match &kind {
            b"mp4a" => Some("mp4a.40.2"),
            b"ec-3" => Some("ec-3"),
            b"ac-3" => Some("ac-3"),
            _ => return None,
        },
        None => None,
    };
    Some(match audio {
        Some(audio) => format!("{video},{audio}"),
        None => video,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::index::tests::mp4_box;

    fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
        let head = (u32::from(version) << 24) | flags;
        mp4_box(kind, &[&head.to_be_bytes()[..], body].concat())
    }

    fn traf(track: u32, tfdt: i64, samples: &[(u32, i32)]) -> Vec<u8> {
        let tfhd = full(b"tfhd", 0, 0x020000, &track.to_be_bytes());
        let tfdt = full(b"tfdt", 1, 0, &tfdt.to_be_bytes());
        let mut body = (samples.len() as u32).to_be_bytes().to_vec();
        body.extend_from_slice(&0u32.to_be_bytes());
        for (duration, offset) in samples {
            body.extend_from_slice(&duration.to_be_bytes());
            body.extend_from_slice(&offset.to_be_bytes());
        }
        let trun = full(b"trun", 1, 0x1 | 0x100 | 0x800, &body);
        mp4_box(b"traf", &[tfhd, tfdt, trun].concat())
    }

    fn fragment(sequence: u32, video_tfdt: i64, audio_tfdt: i64) -> Vec<u8> {
        let mfhd = full(b"mfhd", 0, 0, &sequence.to_be_bytes());
        let moof = mp4_box(
            b"moof",
            &[
                mfhd,
                traf(1, video_tfdt, &[(667, 1344), (667, 0)]),
                traf(2, audio_tfdt, &[(1024, 0)]),
            ]
            .concat(),
        );
        [
            mp4_box(b"styp", b"msdh"),
            mp4_box(b"sidx", &[0u8; 24]),
            moof,
            mp4_box(b"mdat", &[7u8; 16]),
        ]
        .concat()
    }

    const INIT: Init = Init {
        video: 1,
        timescale: 16_000,
        shift: 1344,
    };

    #[test]
    fn a_fragment_starts_at_its_first_presentation_time() {
        let timing = timing(&fragment(9, 319_632, 955_152), &INIT).unwrap();
        assert!((timing.start - 319_632.0 / 16_000.0).abs() < 1e-9);
        assert_eq!(timing.dts, 319_632);
        assert_eq!(timing.dts_end, 319_632 + 1334);
    }

    #[test]
    fn served_fragments_lose_their_headers_count_up_and_never_start_before_zero() {
        let mut sequence = 40;
        let out = media(&fragment(1, 672, -240), &mut sequence, 1, 672);
        assert_eq!(&out[4..8], b"moof");
        assert_eq!(sequence, 41);
        let boxes = mp4_boxes(&out);
        let moof = boxes[0].1;
        assert_eq!(be32(mp4_child(moof, b"mfhd").unwrap(), 4), Some(40));
        let trafs: Vec<&[u8]> = mp4_boxes(moof)
            .into_iter()
            .filter(|(kind, _)| kind == b"traf")
            .map(|(_, body)| body)
            .collect();
        let tfdt = |traf: &[u8]| i64::from_be_bytes(mp4_child(traf, b"tfdt").unwrap()[4..12].try_into().unwrap());
        assert_eq!(tfdt(trafs[0]), 0);
        assert_eq!(tfdt(trafs[1]), 0);
        assert_eq!(boxes[1].0, *b"mdat");
    }

    #[test]
    fn hevc_main10_has_the_codec_string_safari_expects() {
        let mut config = vec![1u8, 0x02, 0x20, 0, 0, 0, 0x90, 0, 0, 0, 0, 0, 120];
        config.extend_from_slice(&[0u8; 10]);
        assert_eq!(hevc_codec(b"hvc1", &config).unwrap(), "hvc1.2.4.L120.90");
    }
}
