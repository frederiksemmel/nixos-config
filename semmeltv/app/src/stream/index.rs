use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

const EBML: u32 = 0x1A45_DFA3;
const SEGMENT: u32 = 0x1853_8067;
const SEEK_HEAD: u32 = 0x114D_9B74;
const SEEK: u32 = 0x4DBB;
const SEEK_ID: u32 = 0x53AB;
const SEEK_POSITION: u32 = 0x53AC;
const INFO: u32 = 0x1549_A966;
const TIMESTAMP_SCALE: u32 = 0x2A_D7B1;
const DURATION: u32 = 0x4489;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_TYPE: u32 = 0x83;
const CUES: u32 = 0x1C53_BB6B;
const CUE_POINT: u32 = 0xBB;
const CUE_TIME: u32 = 0xB3;
const CUE_TRACK_POSITIONS: u32 = 0xB7;
const CUE_TRACK: u32 = 0xF7;
const CLUSTER: u32 = 0x1F43_B675;
const VIDEO: u64 = 1;
const LARGEST_ELEMENT: u64 = 64 << 20;

#[derive(Clone, Debug, PartialEq)]
pub struct Index {
    pub duration: f64,
    pub keyframes: Vec<f64>,
}

pub fn read(path: &Path) -> io::Result<Index> {
    parse(&mut File::open(path)?).map_err(|error| match error.kind() {
        io::ErrorKind::UnexpectedEof => invalid("the file ends early, so it is probably incomplete"),
        _ => error,
    })
}

pub fn parse(file: &mut (impl Read + Seek)) -> io::Result<Index> {
    let mut magic = [0u8; 8];
    file.read_exact(&mut magic)?;
    file.seek(SeekFrom::Start(0))?;
    let index = if magic[..4] == EBML.to_be_bytes() {
        matroska(file)?
    } else if &magic[4..8] == b"ftyp" || &magic[4..8] == b"moov" {
        mp4(file)?
    } else {
        return Err(invalid("neither Matroska nor MP4"));
    };
    if index.keyframes.is_empty() {
        return Err(invalid("the file has no keyframe index"));
    }
    Ok(index)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

fn vint(file: &mut impl Read, keep_marker: bool) -> io::Result<(u64, usize, bool)> {
    let mut first = [0u8; 1];
    file.read_exact(&mut first)?;
    let length = first[0].leading_zeros() as usize + 1;
    if length > 8 {
        return Err(invalid("bad EBML number"));
    }
    let mut value = if keep_marker {
        u64::from(first[0])
    } else {
        u64::from(first[0]) & ((1u64 << (8 - length)) - 1)
    };
    let mut all_ones = value == (1u64 << (8 - length)) - 1;
    let mut rest = [0u8; 8];
    file.read_exact(&mut rest[..length - 1])?;
    for byte in &rest[..length - 1] {
        value = (value << 8) | u64::from(*byte);
        all_ones &= *byte == 0xFF;
    }
    Ok((value, length, all_ones && !keep_marker))
}

fn header(file: &mut impl Read) -> io::Result<(u32, Option<u64>, u64)> {
    let (id, id_length, _) = vint(file, true)?;
    let (size, size_length, unknown) = vint(file, false)?;
    let id = u32::try_from(id).map_err(|_| invalid("bad EBML id"))?;
    Ok((id, (!unknown).then_some(size), (id_length + size_length) as u64))
}

fn children(mut data: &[u8]) -> Vec<(u32, &[u8])> {
    let mut found = Vec::new();
    while !data.is_empty() {
        let mut cursor = io::Cursor::new(data);
        let Ok((id, Some(size), header_length)) = header(&mut cursor) else {
            break;
        };
        let start = header_length as usize;
        let Some(end) = usize::try_from(size).ok().and_then(|size| start.checked_add(size)) else {
            break;
        };
        if end > data.len() {
            break;
        }
        found.push((id, &data[start..end]));
        data = &data[end..];
    }
    found
}

fn unsigned(data: &[u8]) -> u64 {
    data.iter()
        .take(8)
        .fold(0, |value, byte| (value << 8) | u64::from(*byte))
}

fn float(data: &[u8]) -> Option<f64> {
    match data.len() {
        4 => Some(f64::from(f32::from_be_bytes(data.try_into().ok()?))),
        8 => Some(f64::from_be_bytes(data.try_into().ok()?)),
        _ => None,
    }
}

fn element(file: &mut (impl Read + Seek), at: u64) -> io::Result<(u32, Vec<u8>)> {
    file.seek(SeekFrom::Start(at))?;
    let (id, size, _) = header(file)?;
    let size = size.ok_or_else(|| invalid("element of unknown size"))?;
    if size > LARGEST_ELEMENT {
        return Err(invalid("element too large"));
    }
    let mut body = vec![0u8; size as usize];
    file.read_exact(&mut body)?;
    Ok((id, body))
}

#[derive(Default)]
struct Matroska {
    scale: Option<u64>,
    duration: Option<f64>,
    video: Option<u64>,
    cues: Option<Vec<(u64, Vec<u64>)>>,
}

impl Matroska {
    fn take(&mut self, id: u32, body: &[u8]) {
        match id {
            INFO => {
                for (child, data) in children(body) {
                    match child {
                        TIMESTAMP_SCALE => self.scale = Some(unsigned(data)),
                        DURATION => self.duration = float(data),
                        _ => {}
                    }
                }
            }
            TRACKS => {
                for (_, entry) in children(body).into_iter().filter(|(id, _)| *id == TRACK_ENTRY) {
                    let fields = children(entry);
                    let field = |wanted: u32| {
                        fields
                            .iter()
                            .find(|(id, _)| *id == wanted)
                            .map(|(_, data)| unsigned(data))
                    };
                    if field(TRACK_TYPE) == Some(VIDEO) && self.video.is_none() {
                        self.video = field(TRACK_NUMBER);
                    }
                }
            }
            CUES => {
                let points = children(body)
                    .into_iter()
                    .filter(|(id, _)| *id == CUE_POINT)
                    .filter_map(|(_, point)| {
                        let fields = children(point);
                        let time = fields
                            .iter()
                            .find(|(id, _)| *id == CUE_TIME)
                            .map(|(_, data)| unsigned(data))?;
                        let tracks = fields
                            .iter()
                            .filter(|(id, _)| *id == CUE_TRACK_POSITIONS)
                            .flat_map(|(_, positions)| children(positions))
                            .filter(|(id, _)| *id == CUE_TRACK)
                            .map(|(_, data)| unsigned(data))
                            .collect();
                        Some((time, tracks))
                    })
                    .collect();
                self.cues = Some(points);
            }
            _ => {}
        }
    }

    fn index(self) -> io::Result<Index> {
        let scale = self.scale.unwrap_or(1_000_000) as f64 / 1e9;
        let duration = self.duration.ok_or_else(|| invalid("no duration"))? * scale;
        let cues = self.cues.unwrap_or_default();
        let for_video: Vec<u64> = cues
            .iter()
            .filter(|(_, tracks)| self.video.is_some_and(|video| tracks.contains(&video)))
            .map(|(time, _)| *time)
            .collect();
        let times = if for_video.is_empty() {
            cues.iter().map(|(time, _)| *time).collect()
        } else {
            for_video
        };
        Ok(Index {
            duration,
            keyframes: sorted(times.into_iter().map(|time| time as f64 * scale)),
        })
    }
}

fn sorted(times: impl Iterator<Item = f64>) -> Vec<f64> {
    let mut times: Vec<f64> = times.filter(|time| time.is_finite() && *time >= 0.0).collect();
    times.sort_by(f64::total_cmp);
    times.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    times
}

fn matroska(file: &mut (impl Read + Seek)) -> io::Result<Index> {
    let (id, size, length) = header(file)?;
    if id != EBML {
        return Err(invalid("not EBML"));
    }
    let segment_at = length + size.ok_or_else(|| invalid("EBML header of unknown size"))?;
    file.seek(SeekFrom::Start(segment_at))?;
    let (id, segment_size, length) = header(file)?;
    if id != SEGMENT {
        return Err(invalid("no Matroska segment"));
    }
    let start = segment_at + length;
    let end = segment_size.map_or(u64::MAX, |size| start + size);
    let mut found = Matroska::default();
    let mut seen: Vec<u32> = Vec::new();
    let mut positions: HashMap<u32, Vec<u64>> = HashMap::new();
    let mut at = start;
    while at < end {
        file.seek(SeekFrom::Start(at))?;
        let Ok((id, size, length)) = header(file) else {
            break;
        };
        let Some(size) = size else {
            break;
        };
        if id == CLUSTER {
            break;
        }
        if matches!(id, SEEK_HEAD | INFO | TRACKS | CUES) {
            let (_, body) = element(file, at)?;
            if id == SEEK_HEAD {
                seek_head(&body, start, &mut positions);
            } else {
                found.take(id, &body);
                seen.push(id);
            }
        }
        at += length + size;
    }
    let mut followed = Vec::new();
    while let Some(head) = positions.get_mut(&SEEK_HEAD).and_then(Vec::pop) {
        if followed.contains(&head) {
            continue;
        }
        followed.push(head);
        if let Ok((SEEK_HEAD, body)) = element(file, head) {
            seek_head(&body, start, &mut positions);
        }
    }
    for id in [INFO, TRACKS, CUES] {
        if seen.contains(&id) {
            continue;
        }
        if let Some(position) = positions.get(&id).and_then(|all| all.first()) {
            let (got, body) = element(file, *position)?;
            if got == id {
                found.take(id, &body);
            }
        }
    }
    found.index()
}

fn seek_head(body: &[u8], segment_start: u64, positions: &mut HashMap<u32, Vec<u64>>) {
    for (_, seek) in children(body).into_iter().filter(|(id, _)| *id == SEEK) {
        let fields = children(seek);
        let id = fields
            .iter()
            .find(|(id, _)| *id == SEEK_ID)
            .map(|(_, data)| unsigned(data));
        let position = fields
            .iter()
            .find(|(id, _)| *id == SEEK_POSITION)
            .map(|(_, data)| unsigned(data));
        if let (Some(id), Some(position)) = (id, position) {
            let id = u32::try_from(id).unwrap_or(0);
            positions.entry(id).or_default().push(segment_start + position);
        }
    }
}

fn mp4(file: &mut (impl Read + Seek)) -> io::Result<Index> {
    let length = file.seek(SeekFrom::End(0))?;
    let mut at = 0;
    while at + 8 <= length {
        file.seek(SeekFrom::Start(at))?;
        let mut head = [0u8; 16];
        file.read_exact(&mut head[..8])?;
        let mut size = u64::from(u32::from_be_bytes(head[..4].try_into().unwrap()));
        let mut header_length = 8;
        if size == 1 {
            file.read_exact(&mut head[8..16])?;
            size = u64::from_be_bytes(head[8..16].try_into().unwrap());
            header_length = 16;
        } else if size == 0 {
            size = length - at;
        }
        if size < header_length {
            return Err(invalid("bad MP4 box"));
        }
        if &head[4..8] == b"moov" {
            if size > LARGEST_ELEMENT {
                return Err(invalid("moov too large"));
            }
            let mut body = vec![0u8; (size - header_length) as usize];
            file.read_exact(&mut body)?;
            return moov(&body);
        }
        at += size;
    }
    Err(invalid("no moov"))
}

pub fn mp4_boxes(mut data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut found = Vec::new();
    while data.len() >= 8 {
        let mut size = u64::from(u32::from_be_bytes(data[..4].try_into().unwrap()));
        let kind: [u8; 4] = data[4..8].try_into().unwrap();
        let mut header_length = 8;
        if size == 1 {
            if data.len() < 16 {
                break;
            }
            size = u64::from_be_bytes(data[8..16].try_into().unwrap());
            header_length = 16;
        } else if size == 0 {
            size = data.len() as u64;
        }
        let Ok(size) = usize::try_from(size) else {
            break;
        };
        if size < header_length || size > data.len() {
            break;
        }
        found.push((kind, &data[header_length..size]));
        data = &data[size..];
    }
    found
}

pub fn mp4_child<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    mp4_boxes(data)
        .into_iter()
        .find(|(found, _)| found == kind)
        .map(|(_, body)| body)
}

fn be32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn be64(data: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(data.get(at..at + 8)?.try_into().ok()?))
}

fn table<T>(data: &[u8], width: usize, entry: impl Fn(&[u8]) -> Option<T>) -> Option<Vec<T>> {
    let count = be32(data, 4)? as usize;
    (0..count)
        .map(|index| entry(data.get(8 + index * width..8 + (index + 1) * width)?))
        .collect()
}

pub fn edit_shift(trak: &[u8]) -> (i64, u64) {
    let Some(elst) = mp4_child(trak, b"edts").and_then(|edts| mp4_child(edts, b"elst")) else {
        return (0, 0);
    };
    let version = elst.first().copied().unwrap_or(0);
    let width = if version == 1 { 20 } else { 12 };
    let entries = table(elst, width, |entry| {
        if version == 1 {
            Some((be64(entry, 0)?, be64(entry, 8)? as i64))
        } else {
            Some((u64::from(be32(entry, 0)?), i64::from(be32(entry, 4)? as i32)))
        }
    })
    .unwrap_or_default();
    let mut empty = 0;
    for (duration, media_time) in entries {
        if media_time == -1 {
            empty += duration;
        } else {
            return (media_time, empty);
        }
    }
    (0, empty)
}

fn moov(body: &[u8]) -> io::Result<Index> {
    let mvhd = mp4_child(body, b"mvhd").ok_or_else(|| invalid("no mvhd"))?;
    let (movie_scale, movie_duration) = if mvhd.first() == Some(&1) {
        (be32(mvhd, 20), be64(mvhd, 24))
    } else {
        (be32(mvhd, 12), be32(mvhd, 16).map(u64::from))
    };
    let movie_scale = f64::from(
        movie_scale
            .filter(|scale| *scale > 0)
            .ok_or_else(|| invalid("bad mvhd"))?,
    );
    let duration = movie_duration.ok_or_else(|| invalid("bad mvhd"))? as f64 / movie_scale;
    for (_, trak) in mp4_boxes(body).into_iter().filter(|(kind, _)| kind == b"trak") {
        let Some(mdia) = mp4_child(trak, b"mdia") else {
            continue;
        };
        if mp4_child(mdia, b"hdlr").and_then(|hdlr| hdlr.get(8..12)) != Some(b"vide") {
            continue;
        }
        let keyframes = video_keyframes(trak, mdia, movie_scale).ok_or_else(|| invalid("bad video sample table"))?;
        return Ok(Index { duration, keyframes });
    }
    Err(invalid("no video track"))
}

fn video_keyframes(trak: &[u8], mdia: &[u8], movie_scale: f64) -> Option<Vec<f64>> {
    let mdhd = mp4_child(mdia, b"mdhd")?;
    let scale = if mdhd.first() == Some(&1) {
        be32(mdhd, 20)?
    } else {
        be32(mdhd, 12)?
    };
    if scale == 0 {
        return None;
    }
    let stbl = mp4_child(mp4_child(mdia, b"minf")?, b"stbl")?;
    let stts = table(mp4_child(stbl, b"stts")?, 8, |entry| {
        Some((be32(entry, 0)?, be32(entry, 4)?))
    })?;
    let ctts = match mp4_child(stbl, b"ctts") {
        Some(ctts) => {
            let signed = ctts.first() == Some(&1);
            table(ctts, 8, |entry| {
                let offset = be32(entry, 4)?;
                Some((
                    be32(entry, 0)?,
                    if signed {
                        i64::from(offset as i32)
                    } else {
                        i64::from(offset)
                    },
                ))
            })?
        }
        None => Vec::new(),
    };
    let sync: Option<Vec<u32>> = match mp4_child(stbl, b"stss") {
        Some(stss) => Some(table(stss, 4, |entry| be32(entry, 0))?),
        None => None,
    };
    let (media_time, empty) = edit_shift(trak);
    let offset = empty as f64 / movie_scale;
    let mut times = Vec::new();
    let mut wanted = sync.iter().flatten().copied().peekable();
    let mut sample: u32 = 1;
    let mut dts: i64 = 0;
    let mut composition = ctts
        .iter()
        .flat_map(|(count, offset)| std::iter::repeat_n(*offset, *count as usize));
    for (count, delta) in stts {
        for _ in 0..count {
            let cto = composition.next().unwrap_or(0);
            let is_key = match &sync {
                Some(_) => wanted.peek() == Some(&sample),
                None => true,
            };
            if is_key {
                if sync.is_some() {
                    wanted.next();
                }
                times.push((dts + cto - media_time) as f64 / f64::from(scale) + offset);
            }
            sample += 1;
            dts += i64::from(delta);
        }
    }
    Some(sorted(times.into_iter()))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::io::Write;

    fn id_bytes(id: u32) -> Vec<u8> {
        let bytes = id.to_be_bytes();
        let skip = bytes.iter().take_while(|byte| **byte == 0).count();
        bytes[skip..].to_vec()
    }

    pub fn ebml(id: u32, body: &[u8]) -> Vec<u8> {
        let mut out = id_bytes(id);
        let size = body.len() as u64;
        out.push(0x01);
        out.extend_from_slice(&size.to_be_bytes()[1..]);
        out.extend_from_slice(body);
        out
    }

    fn uint(id: u32, value: u64) -> Vec<u8> {
        ebml(id, &value.to_be_bytes())
    }

    fn cue(time: u64, track: u64) -> Vec<u8> {
        let positions = uint(CUE_TRACK, track);
        ebml(
            CUE_POINT,
            &[uint(CUE_TIME, time), ebml(CUE_TRACK_POSITIONS, &positions)].concat(),
        )
    }

    fn temporary(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("semmeltv-index-{}-{name}", std::process::id()));
        File::create(&path).unwrap().write_all(bytes).unwrap();
        path
    }

    fn matroska_file(cues_after_clusters: bool) -> Vec<u8> {
        let info = ebml(
            INFO,
            &[
                uint(TIMESTAMP_SCALE, 1_000_000),
                ebml(DURATION, &90_000.0f64.to_be_bytes()),
            ]
            .concat(),
        );
        let audio = ebml(TRACK_ENTRY, &[uint(TRACK_NUMBER, 1), uint(TRACK_TYPE, 2)].concat());
        let video = ebml(TRACK_ENTRY, &[uint(TRACK_NUMBER, 2), uint(TRACK_TYPE, 1)].concat());
        let tracks = ebml(TRACKS, &[audio, video].concat());
        let cues = ebml(
            CUES,
            &[cue(0, 2), cue(5000, 1), cue(10_010, 2), cue(20_020, 2)].concat(),
        );
        let cluster = ebml(CLUSTER, &[0u8; 64]);
        let head_size = ebml(
            SEEK_HEAD,
            &ebml(SEEK, &[uint(SEEK_ID, u64::from(CUES)), uint(SEEK_POSITION, 0)].concat()),
        )
        .len();
        let mut body_after_head = [info, tracks].concat();
        if cues_after_clusters {
            body_after_head.extend_from_slice(&cluster);
        }
        let cues_position = (head_size + body_after_head.len()) as u64;
        let seek = ebml(
            SEEK,
            &[uint(SEEK_ID, u64::from(CUES)), uint(SEEK_POSITION, cues_position)].concat(),
        );
        let head = ebml(SEEK_HEAD, &seek);
        assert_eq!(head.len(), head_size);
        let segment_body = if cues_after_clusters {
            [head, body_after_head, cues].concat()
        } else {
            [head, body_after_head, cues, cluster].concat()
        };
        [ebml(EBML, &uint(0x4282, 0)), ebml(SEGMENT, &segment_body)].concat()
    }

    #[test]
    fn matroska_keyframes_come_from_the_video_track_cues_even_after_the_clusters() {
        for after in [false, true] {
            let path = temporary(&format!("cues-{after}.mkv"), &matroska_file(after));
            let index = read(&path).unwrap();
            std::fs::remove_file(&path).unwrap();
            assert_eq!(index.duration, 90.0);
            assert_eq!(index.keyframes, vec![0.0, 10.01, 20.02]);
        }
    }

    pub fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        [&((body.len() + 8) as u32).to_be_bytes()[..], kind, body].concat()
    }

    fn full(kind: &[u8; 4], version: u8, body: &[u8]) -> Vec<u8> {
        mp4_box(kind, &[&[version, 0, 0, 0][..], body].concat())
    }

    fn entries(rows: &[&[u32]]) -> Vec<u8> {
        let mut out = (rows.len() as u32).to_be_bytes().to_vec();
        for row in rows {
            for value in *row {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
        out
    }

    #[test]
    fn mp4_keyframes_are_presentation_times_of_sync_samples_after_the_edit_list() {
        let mvhd = full(
            b"mvhd",
            0,
            &[
                &[0u8; 8][..],
                &1000u32.to_be_bytes(),
                &60_000u32.to_be_bytes(),
                &[0u8; 80],
            ]
            .concat(),
        );
        let mdhd = full(
            b"mdhd",
            0,
            &[&[0u8; 8][..], &24_000u32.to_be_bytes(), &[0u8; 8]].concat(),
        );
        let hdlr = full(b"hdlr", 0, &[&[0u8; 4][..], b"vide", &[0u8; 13]].concat());
        let stts = full(b"stts", 0, &entries(&[&[480, 1001]]));
        let ctts = full(b"ctts", 0, &entries(&[&[1, 2002], &[479, 1001]]));
        let stss = full(b"stss", 0, &entries(&[&[1], &[241]]));
        let stbl = mp4_box(b"stbl", &[stts, ctts, stss].concat());
        let minf = mp4_box(b"minf", &stbl);
        let mdia = mp4_box(b"mdia", &[mdhd, hdlr, minf].concat());
        let elst = full(b"elst", 0, &entries(&[&[60_000, 2002, 0x0001_0000]]));
        let trak = mp4_box(b"trak", &[mp4_box(b"edts", &elst), mdia].concat());
        let file = [
            mp4_box(b"ftyp", b"isom"),
            mp4_box(b"mdat", &[0u8; 32]),
            mp4_box(b"moov", &[mvhd, trak].concat()),
        ]
        .concat();
        let path = temporary("sync.mp4", &file);
        let index = read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(index.duration, 60.0);
        assert_eq!(index.keyframes.len(), 2);
        assert!(index.keyframes[0].abs() < 1e-9);
        assert!((index.keyframes[1] - 240.0 * 1001.0 / 24_000.0 + 1001.0 / 24_000.0).abs() < 1e-9);
    }

    struct Ranged {
        agent: ureq::Agent,
        url: String,
        length: u64,
        at: u64,
        buffer: Vec<u8>,
        buffer_at: u64,
        requests: usize,
        fetched: u64,
    }

    impl Ranged {
        fn open(url: String) -> io::Result<Self> {
            let agent: ureq::Agent = ureq::Agent::config_builder().build().into();
            let response = agent.head(&url).call().map_err(io::Error::other)?;
            let length = response
                .headers()
                .get("content-length")
                .and_then(|value| value.to_str().ok()?.parse().ok())
                .ok_or_else(|| io::Error::other("no length"))?;
            Ok(Self {
                agent,
                url,
                length,
                at: 0,
                buffer: Vec::new(),
                buffer_at: 0,
                requests: 0,
                fetched: 0,
            })
        }
    }

    impl Read for Ranged {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.at >= self.length {
                return Ok(0);
            }
            let inside = self.at >= self.buffer_at && self.at < self.buffer_at + self.buffer.len() as u64;
            if !inside {
                let end = (self.at + (256 << 10)).min(self.length) - 1;
                let mut response = self
                    .agent
                    .get(&self.url)
                    .header("range", &format!("bytes={}-{end}", self.at))
                    .call()
                    .map_err(io::Error::other)?;
                self.buffer = response
                    .body_mut()
                    .with_config()
                    .limit(1 << 22)
                    .read_to_vec()
                    .map_err(io::Error::other)?;
                self.buffer_at = self.at;
                self.requests += 1;
                self.fetched += self.buffer.len() as u64;
            }
            let offset = (self.at - self.buffer_at) as usize;
            let count = out.len().min(self.buffer.len() - offset);
            out[..count].copy_from_slice(&self.buffer[offset..offset + count]);
            self.at += count as u64;
            Ok(count)
        }
    }

    impl Seek for Ranged {
        fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
            self.at = match to {
                SeekFrom::Start(at) => at,
                SeekFrom::End(back) => (self.length as i64 + back) as u64,
                SeekFrom::Current(by) => (self.at as i64 + by) as u64,
            };
            Ok(self.at)
        }
    }

    #[test]
    #[ignore = "needs SEMMELTV_INDEX_BASE and SEMMELTV_INDEX_PATHS"]
    fn survey_the_library_keyframe_indexes() {
        let base = std::env::var("SEMMELTV_INDEX_BASE").unwrap();
        let paths = std::fs::read_to_string(std::env::var("SEMMELTV_INDEX_PATHS").unwrap()).unwrap();
        for path in paths.lines().filter(|line| !line.is_empty()) {
            let encoded: String = path
                .split('/')
                .map(|part| {
                    part.bytes()
                        .map(|byte| match byte {
                            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                                (byte as char).to_string()
                            }
                            other => format!("%{other:02X}"),
                        })
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("/");
            let started = std::time::Instant::now();
            let result = Ranged::open(format!("{base}/{encoded}")).and_then(|mut file| {
                let index = parse(&mut file)?;
                Ok((index, file.requests, file.fetched))
            });
            match result {
                Ok((index, requests, fetched)) => {
                    let gaps: Vec<f64> = index.keyframes.windows(2).map(|pair| pair[1] - pair[0]).collect();
                    let longest = gaps.iter().copied().fold(0.0, f64::max);
                    let plan = crate::stream::plan::Plan::new(&index);
                    println!(
                        "OK\t{path}\t{}\t{:.1}\t{:.2}\t{longest:.2}\t{}\t{requests}\t{fetched}\t{:.2}",
                        index.keyframes.len(),
                        index.duration,
                        index.duration / index.keyframes.len() as f64,
                        plan.len(),
                        started.elapsed().as_secs_f64()
                    );
                }
                Err(error) => println!("ERR\t{path}\t{error}"),
            }
        }
    }
}
