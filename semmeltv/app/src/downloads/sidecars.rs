use super::deliver::{self, Placed};
use crate::sidecar;
use crate::subfetch::{Client, Fetched, Wanted};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub struct Sidecars {
    pub client: Client,
    pub remote: String,
    pub mount_rc: PathBuf,
    pub work: PathBuf,
}

fn library_folder(video: &str) -> Result<String, String> {
    let path = Path::new(video);
    let inside = path.components().all(|part| matches!(part, Component::Normal(_)));
    let shelved = path.starts_with("Movies") || path.starts_with("Series");
    match path.parent() {
        Some(folder) if inside && shelved && folder.components().count() >= 2 => {
            Ok(folder.to_string_lossy().into_owned())
        }
        _ => Err(format!("{video} is not a video in the library")),
    }
}

impl Sidecars {
    pub fn fetch(&self, wanted: &Wanted) -> Result<Fetched, String> {
        let folder = library_folder(&wanted.video)?;
        let name = sidecar::file_name(Path::new(&wanted.video), &wanted.language)
            .ok_or_else(|| format!("{} has no file name", wanted.video))?;
        let found = self.client.fetch(wanted)?;
        fs::create_dir_all(&self.work).map_err(|error| format!("{}: {error}", self.work.display()))?;
        let local = self.work.join(&name);
        fs::write(&local, &found.text).map_err(|error| format!("{}: {error}", local.display()))?;
        let placed = Placed {
            local: local.clone(),
            destination: format!("{folder}/{name}"),
            size: found.text.len() as u64,
        };
        let uploaded = deliver::upload_to(&self.remote, &placed);
        let _ = fs::remove_file(&local);
        uploaded?;
        if let Err(error) = deliver::refresh(&self.mount_rc, &folder) {
            eprintln!("subtitles: could not refresh {folder} on the mount: {error}");
        }
        eprintln!(
            "subtitles: {} from {} ({:?} match) saved as {}",
            wanted.language, found.release, found.matched, placed.destination
        );
        Ok(Fetched {
            file: placed.destination,
            language: wanted.language.clone(),
            release: found.release,
            matched: found.matched,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitles_only_go_next_to_videos_inside_the_library() {
        assert_eq!(
            library_folder("Movies/Airplane 1980/Airplane.1980.mp4").as_deref(),
            Ok("Movies/Airplane 1980")
        );
        assert_eq!(
            library_folder("Series/Dark/Dark S01/DARK.S01E01.mp4").as_deref(),
            Ok("Series/Dark/Dark S01")
        );
        assert!(library_folder("Movies/../library.toml").is_err());
        assert!(library_folder("/Movies/Up/Up.mkv").is_err());
        assert!(library_folder("Artwork/x/y.mkv").is_err());
        assert!(library_folder("Movies/Up.mkv").is_err());
    }
}
