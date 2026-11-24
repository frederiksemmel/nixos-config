//! A film's or an episode's whole file, saved to a phone or laptop.

use crate::auth::gate::Who;
use crate::core::Core;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use std::path::{Path as FsPath, PathBuf};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

const CHUNK: u64 = 256 * 1024;

pub fn routes<S: Clone + Send + Sync + 'static>(core: Core) -> Router<S> {
    Router::new()
        .route("/api/files/movies/{key}", get(movie))
        .route("/api/files/shows/{key}/{season}/{episode}", get(episode))
        .with_state(core)
}

/// The bytes a request asks for, from its Range header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    Whole,
    /// From the first byte up to and including the last.
    Part(u64, u64),
    Outside,
}

pub fn span(range: Option<&str>, size: u64) -> Span {
    let Some(asked) = range.and_then(|range| range.trim().strip_prefix("bytes=")) else {
        return Span::Whole;
    };
    if asked.contains(',') {
        return Span::Whole;
    }
    let Some((from, to)) = asked.split_once('-') else {
        return Span::Whole;
    };
    let (from, to) = (from.trim(), to.trim());
    let part = match (from.parse::<u64>(), to.parse::<u64>()) {
        (Ok(first), Ok(last)) if first <= last => (first, last.min(size.saturating_sub(1))),
        (Ok(first), Err(_)) if to.is_empty() => (first, size.saturating_sub(1)),
        (Err(_), Ok(tail)) if from.is_empty() && tail > 0 => (size.saturating_sub(tail), size.saturating_sub(1)),
        _ => return Span::Whole,
    };
    if part.0 >= size {
        Span::Outside
    } else {
        Span::Part(part.0, part.1)
    }
}

fn kind(file: &FsPath) -> &'static str {
    match file
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mkv") => "video/x-matroska",
        Some("mp4" | "m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("avi") => "video/x-msvideo",
        Some("ts" | "m2ts") => "video/mp2t",
        _ => "application/octet-stream",
    }
}

/// Saves the file under its own name, with a plain ASCII name for browsers
/// that don't read the encoded one.
pub fn disposition(name: &str) -> String {
    let plain: String = name
        .chars()
        .map(|char| match char {
            ' '..='~' if char != '"' && char != '\\' => char,
            _ => '_',
        })
        .collect();
    let encoded: String = name
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'!'
            | b'#'
            | b'$'
            | b'&'
            | b'+'
            | b'-'
            | b'.'
            | b'^'
            | b'_'
            | b'`'
            | b'|'
            | b'~' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect();
    format!("attachment; filename=\"{plain}\"; filename*=UTF-8''{encoded}")
}

fn missing(what: String) -> Response {
    (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({ "error": what }))).into_response()
}

async fn movie(State(core): State<Core>, Who(profile): Who, Path(key): Path<String>, headers: HeaderMap) -> Response {
    let library = core.library();
    let Some(movie) = library.movies.iter().find(|movie| movie.key == key) else {
        return missing(format!("{key} is not in the library"));
    };
    let file = core.playable(&movie.path);
    send(&file, &headers, &format!("profile {} saves {}", profile.0, movie.title)).await
}

async fn episode(
    State(core): State<Core>,
    Who(profile): Who,
    Path((key, season, number)): Path<(String, u32, u32)>,
    headers: HeaderMap,
) -> Response {
    let library = core.library();
    let Some((show, episode)) = library
        .shows
        .iter()
        .find(|show| show.key == key)
        .and_then(|show| Some((show, show.episode(season, number)?)))
    else {
        return missing(format!("{key} S{season} E{number} is not in the library"));
    };
    let file = core.playable(&episode.path);
    let what = format!("profile {} saves {} {}", profile.0, show.title, episode.label());
    send(&file, &headers, &what).await
}

async fn send(file: &PathBuf, headers: &HeaderMap, what: &str) -> Response {
    let opened = match tokio::fs::File::open(file).await {
        Ok(opened) => opened,
        Err(error) => {
            eprintln!("files: {what}: {}: {error}", file.display());
            return missing(format!("The file can't be read: {error}"));
        }
    };
    let size = match opened.metadata().await {
        Ok(metadata) => metadata.len(),
        Err(error) => return missing(format!("The file can't be read: {error}")),
    };
    let range = headers.get(header::RANGE).and_then(|value| value.to_str().ok());
    let (status, first, last) = match span(range, size) {
        Span::Whole => (StatusCode::OK, 0, size.saturating_sub(1)),
        Span::Part(first, last) => (StatusCode::PARTIAL_CONTENT, first, last),
        Span::Outside => {
            let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            return response;
        }
    };
    let length = if size == 0 { 0 } else { last - first + 1 };
    if first == 0 {
        eprintln!("files: {what} ({} MB)", size / 1_000_000);
    }
    let mut response = Response::new(body(opened, first, length));
    *response.status_mut() = status;
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let set = response.headers_mut();
    set.insert(header::CONTENT_TYPE, HeaderValue::from_static(kind(file)));
    set.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    set.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    set.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    if let Ok(value) = HeaderValue::from_str(&disposition(&name)) {
        set.insert(header::CONTENT_DISPOSITION, value);
    }
    if status == StatusCode::PARTIAL_CONTENT
        && let Ok(value) = HeaderValue::from_str(&format!("bytes {first}-{last}/{size}"))
    {
        set.insert(header::CONTENT_RANGE, value);
    }
    response
}

fn body(file: tokio::fs::File, first: u64, length: u64) -> Body {
    let chunks = futures_util::stream::unfold(
        (file, first, length, false),
        |(mut file, at, left, sought)| async move {
            if left == 0 {
                return None;
            }
            if !sought && let Err(error) = file.seek(std::io::SeekFrom::Start(at)).await {
                return Some((Err(error), (file, at, 0, true)));
            }
            let mut chunk = vec![0; CHUNK.min(left) as usize];
            match file.read(&mut chunk).await {
                Ok(0) => Some((
                    Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "the file ended early",
                    )),
                    (file, at, 0, true),
                )),
                Ok(read) => {
                    chunk.truncate(read);
                    Some((
                        Ok(Bytes::from(chunk)),
                        (file, at + read as u64, left - read as u64, true),
                    ))
                }
                Err(error) => Some((Err(error), (file, at, 0, true))),
            }
        },
    );
    Body::from_stream(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::library_from_paths;

    #[test]
    fn a_range_asks_for_a_part_of_the_file() {
        assert_eq!(span(None, 1000), Span::Whole);
        assert_eq!(span(Some("bytes=0-"), 1000), Span::Part(0, 999));
        assert_eq!(span(Some("bytes=0-1"), 1000), Span::Part(0, 1));
        assert_eq!(span(Some("bytes=500-"), 1000), Span::Part(500, 999));
        assert_eq!(span(Some("bytes=500-5000"), 1000), Span::Part(500, 999));
        assert_eq!(span(Some("bytes=-100"), 1000), Span::Part(900, 999));
        assert_eq!(span(Some("bytes=-5000"), 1000), Span::Part(0, 999));
        assert_eq!(span(Some("bytes=1000-"), 1000), Span::Outside);
        assert_eq!(span(Some("bytes=0-1,5-9"), 1000), Span::Whole);
        assert_eq!(span(Some("bytes=9-5"), 1000), Span::Whole);
        assert_eq!(span(Some("items=0-5"), 1000), Span::Whole);
    }

    #[test]
    fn a_saved_file_keeps_its_name_even_with_umlauts() {
        assert_eq!(
            disposition("Dune.2021.1080p.mkv"),
            "attachment; filename=\"Dune.2021.1080p.mkv\"; filename*=UTF-8''Dune.2021.1080p.mkv"
        );
        assert_eq!(
            disposition("Das Boot \"Ü\".mkv"),
            "attachment; filename=\"Das Boot ___.mkv\"; filename*=UTF-8''Das%20Boot%20%22%C3%9C%22.mkv"
        );
    }

    struct Served {
        base: String,
        _runtime: tokio::runtime::Runtime,
    }

    fn serve(dir: &FsPath) -> Served {
        let mut library = library_from_paths(&[
            "Series/Outlander/Outlander.S01E02.mkv",
            "Movies/Dune (2021)/Dune.2021.1080p.mkv",
        ]);
        let film = dir.join("Dune.2021.1080p.mkv");
        std::fs::write(&film, (0..=255u8).cycle().take(600_000).collect::<Vec<_>>()).unwrap();
        library.movies[0].path = film;
        let episode = dir.join("Outlander.S01E02.mkv");
        std::fs::write(&episode, "the episode").unwrap();
        library.shows[0].seasons[0].episodes[0].path = episode;
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let core = runtime.block_on(Core::detached(library)).0;
        let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        runtime.spawn(async move { axum::serve(listener, routes::<()>(core)).await });
        Served {
            base,
            _runtime: runtime,
        }
    }

    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder().http_status_as_error(false).build().into()
    }

    #[test]
    fn a_film_or_an_episode_is_saved_whole_or_in_parts() {
        let dir = std::env::temp_dir().join(format!("semmeltv-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let served = serve(&dir);
        let film = std::fs::read(dir.join("Dune.2021.1080p.mkv")).unwrap();

        let mut whole = agent()
            .get(format!("{}/api/files/movies/dune-2021", served.base))
            .call()
            .unwrap();
        assert_eq!(whole.status(), 200);
        let header = |response: &ureq::http::Response<ureq::Body>, name: &str| {
            response
                .headers()
                .get(name)
                .map(|value| value.to_str().unwrap().to_string())
        };
        assert_eq!(header(&whole, "content-type").as_deref(), Some("video/x-matroska"));
        assert_eq!(header(&whole, "content-length").as_deref(), Some("600000"));
        assert_eq!(header(&whole, "accept-ranges").as_deref(), Some("bytes"));
        assert!(
            header(&whole, "content-disposition")
                .unwrap()
                .starts_with("attachment; filename=\"Dune.2021.1080p.mkv\"")
        );
        let read = whole.body_mut().with_config().limit(10_000_000).read_to_vec().unwrap();
        assert!(read == film, "the whole film arrives unchanged");

        let mut part = agent()
            .get(format!("{}/api/files/movies/dune-2021", served.base))
            .header("Range", "bytes=300000-300009")
            .call()
            .unwrap();
        assert_eq!(part.status(), 206);
        assert_eq!(
            header(&part, "content-range").as_deref(),
            Some("bytes 300000-300009/600000")
        );
        assert_eq!(part.body_mut().read_to_vec().unwrap(), &film[300_000..300_010]);

        let outside = agent()
            .get(format!("{}/api/files/movies/dune-2021", served.base))
            .header("Range", "bytes=600000-")
            .call()
            .unwrap();
        assert_eq!(outside.status(), 416);

        let mut episode = agent()
            .get(format!("{}/api/files/shows/outlander/1/2", served.base))
            .call()
            .unwrap();
        assert_eq!(episode.status(), 200);
        assert_eq!(episode.body_mut().read_to_string().unwrap(), "the episode");

        let unknown = |path: &str| agent().get(format!("{}{path}", served.base)).call().unwrap().status();
        assert_eq!(unknown("/api/files/movies/nope"), 404);
        assert_eq!(unknown("/api/files/shows/outlander/9/9"), 404);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
