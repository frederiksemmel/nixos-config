//! The CODECS attribute of the master playlist, read from ffmpeg's init segment.

fn be32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn boxes(mut data: &[u8]) -> Vec<([u8; 4], &[u8])> {
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

fn child<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(data)
        .into_iter()
        .find(|(found, _)| found == kind)
        .map(|(_, body)| body)
}

fn sample_entry<'a>(init: &'a [u8], handler: &[u8; 4]) -> Option<([u8; 4], &'a [u8])> {
    let moov = child(init, b"moov")?;
    for (_, trak) in boxes(moov).into_iter().filter(|(kind, _)| kind == b"trak") {
        let mdia = child(trak, b"mdia")?;
        if child(mdia, b"hdlr").and_then(|hdlr| hdlr.get(8..12)) != Some(handler) {
            continue;
        }
        let stsd = child(child(child(mdia, b"minf")?, b"stbl")?, b"stsd")?;
        let (kind, body) = boxes(stsd.get(8..)?).into_iter().next()?;
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

/// The RFC 6381 codec string of the video and audio in `init`, such as
/// `hvc1.2.4.L120.90,ec-3`.
pub fn codecs(init: &[u8]) -> Option<String> {
    let (kind, entry) = sample_entry(init, b"vide")?;
    let config_boxes = boxes(entry.get(78..)?);
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

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        [&((body.len() + 8) as u32).to_be_bytes()[..], kind, body].concat()
    }

    fn trak(handler: &[u8; 4], entry: Vec<u8>) -> Vec<u8> {
        let hdlr = mp4_box(b"hdlr", &[&[0u8; 8][..], handler, &[0u8; 12]].concat());
        let stsd = mp4_box(b"stsd", &[&[0u8; 8][..], &entry].concat());
        let stbl = mp4_box(b"stbl", &stsd);
        let minf = mp4_box(b"minf", &stbl);
        mp4_box(b"trak", &mp4_box(b"mdia", &[hdlr, minf].concat()))
    }

    fn hevc_main10() -> Vec<u8> {
        let mut config = vec![1u8, 0x02, 0x20, 0, 0, 0, 0x90, 0, 0, 0, 0, 0, 120];
        config.extend_from_slice(&[0u8; 10]);
        config
    }

    #[test]
    fn hevc_main10_has_the_codec_string_safari_expects() {
        assert_eq!(hevc_codec(b"hvc1", &hevc_main10()).unwrap(), "hvc1.2.4.L120.90");
    }

    #[test]
    fn an_init_segment_names_its_video_and_audio() {
        let video = mp4_box(b"hvc1", &[&[0u8; 78][..], &mp4_box(b"hvcC", &hevc_main10())].concat());
        let audio = mp4_box(b"ec-3", &[0u8; 28]);
        let init = mp4_box(b"moov", &[trak(b"vide", video.clone()), trak(b"soun", audio)].concat());
        assert_eq!(codecs(&init).as_deref(), Some("hvc1.2.4.L120.90,ec-3"));
        let silent = mp4_box(b"moov", &trak(b"vide", video));
        assert_eq!(codecs(&silent).as_deref(), Some("hvc1.2.4.L120.90"));
        let h264 = mp4_box(
            b"avc1",
            &[&[0u8; 78][..], &mp4_box(b"avcC", &[1, 0x64, 0x00, 0x28, 0xff])].concat(),
        );
        let aac = mp4_box(
            b"moov",
            &[trak(b"vide", h264.clone()), trak(b"soun", mp4_box(b"mp4a", &[0u8; 28]))].concat(),
        );
        assert_eq!(codecs(&aac).as_deref(), Some("avc1.640028,mp4a.40.2"));
        let opus = mp4_box(
            b"moov",
            &[trak(b"vide", h264), trak(b"soun", mp4_box(b"Opus", &[0u8; 28]))].concat(),
        );
        assert_eq!(codecs(&opus), None);
    }
}
