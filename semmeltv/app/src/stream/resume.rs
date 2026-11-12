use std::fs;
use std::io;
use std::path::Path;

const SHIFTS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15,
    21,
];

fn md5(input: &[u8]) -> [u8; 16] {
    let constants: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32)
        .collect();
    let mut message = input.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((input.len() as u64).wrapping_mul(8)).to_le_bytes());
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    for chunk in message.chunks(64) {
        let words: Vec<u32> = chunk
            .chunks(4)
            .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
            .collect();
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let rotated = a
                .wrapping_add(f)
                .wrapping_add(constants[i])
                .wrapping_add(words[g])
                .rotate_left(SHIFTS[i]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(rotated);
        }
        state = [
            state[0].wrapping_add(a),
            state[1].wrapping_add(b),
            state[2].wrapping_add(c),
            state[3].wrapping_add(d),
        ];
    }
    let mut digest = [0u8; 16];
    for (index, word) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    digest
}

pub fn entry_name(file: &Path) -> Option<String> {
    let name = file.file_name()?.to_string_lossy();
    Some(md5(name.as_bytes()).iter().map(|byte| format!("{byte:02X}")).collect())
}

pub fn write(dir: &Path, file: &Path, position: f64) -> io::Result<()> {
    let name = entry_name(file).ok_or_else(|| io::Error::other("no file name"))?;
    fs::create_dir_all(dir)?;
    let path = dir.join(name);
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|line| !line.starts_with("start="))
        .map(str::to_string)
        .collect();
    lines.insert(0, format!("start={position:.6}"));
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, lines.join("\n") + "\n")?;
    fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_is_named_like_mpv_names_it() {
        assert_eq!(
            entry_name(Path::new("/lib/Series/Chernobyl Season 1/Chernobyl S01E01 1 23 45.mkv")).unwrap(),
            "B6AA78104D2DEF066AE6086929D07CA2"
        );
        assert_eq!(
            entry_name(Path::new("/x/")).unwrap(),
            "9DD4E461268C8034F5C8564E155C67A6"
        );
    }

    #[test]
    fn writing_keeps_the_other_settings_and_replaces_the_start() {
        let dir = std::env::temp_dir().join(format!("semmeltv-resume-{}", std::process::id()));
        let file = Path::new("/lib/Movies/Up.mkv");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(entry_name(file).unwrap()), "start=10.000000\naid=2\n").unwrap();
        write(&dir, file, 1234.5).unwrap();
        let written = fs::read_to_string(dir.join(entry_name(file).unwrap())).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(written, "start=1234.500000\naid=2\n");
    }
}
