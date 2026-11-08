use crate::library::{self, Episode, Library, Movie, Show};
use crate::progress::Progress;
use deunicode::deunicode;
use regex::Regex;
use std::path::PathBuf;
use std::sync::LazyLock;
use strsim::normalized_damerau_levenshtein;

const SEASON_WORDS: [&str; 4] = ["season", "seasons", "staffel", "temporada"];
const EPISODE_WORDS: [&str; 6] = ["episode", "episodes", "folge", "episodio", "capitulo", "kapitel"];
const NEXT_WORDS: [&str; 6] = ["next", "nachste", "nachsten", "nachster", "siguiente", "proxima"];
const PLAY_PHRASES: [&str; 43] = [
    "continue playing",
    "continue watching",
    "keep playing",
    "keep watching",
    "continue",
    "resume",
    "mach weiter",
    "weiterschauen",
    "weitersehen",
    "weiterspielen",
    "fortsetzen",
    "weiter",
    "continuar viendo",
    "seguir viendo",
    "sigue viendo",
    "continuar",
    "continua",
    "seguir",
    "sigue",
    "reanudar",
    "reanuda",
    "put on",
    "show me",
    "playing",
    "watching",
    "play",
    "watch",
    "start",
    "abspielen",
    "anschauen",
    "ansehen",
    "schauen",
    "spiele",
    "spiel",
    "schau",
    "starte",
    "starten",
    "zeige",
    "zeig",
    "reproducir",
    "reproduce",
    "ponme",
    "pon",
];
const DOWNLOAD_PHRASES: [&str; 15] = [
    "download",
    "get",
    "grab",
    "herunterladen",
    "runterladen",
    "herunter",
    "runter",
    "lade",
    "lad",
    "descargar",
    "descargame",
    "descarga",
    "bajar",
    "bajame",
    "baja",
];
const FILLERS: [&str; 60] = [
    "a", "an", "and", "at", "bitte", "can", "con", "could", "das", "de", "del", "der", "die", "du", "el", "en",
    "favor", "from", "gerne", "hey", "i", "ich", "kannst", "la", "las", "lass", "lets", "los", "mal", "me", "mir",
    "mit", "mochte", "mochten", "movie", "of", "ok", "okay", "pelicula", "please", "por", "puedes", "quiero", "sehe",
    "sehen", "serie", "series", "show", "the", "to", "uns", "us", "ver", "vom", "von", "want", "will", "wir", "with",
    "you",
];
const NUMBERS: [(u32, &[&str], &[&str]); 12] = [
    (
        1,
        &["one", "eins", "ein", "eine", "uno", "una"],
        &["first", "erste", "primer", "primera", "primero"],
    ),
    (
        2,
        &["two", "zwei", "zwo", "dos"],
        &["second", "zweite", "segunda", "segundo"],
    ),
    (
        3,
        &["three", "drei", "tres"],
        &["third", "dritte", "tercera", "tercero", "tercer"],
    ),
    (
        4,
        &["four", "vier", "cuatro"],
        &["fourth", "vierte", "cuarta", "cuarto"],
    ),
    (
        5,
        &["five", "funf", "fuenf", "cinco"],
        &["fifth", "funfte", "fuenfte", "quinta", "quinto"],
    ),
    (6, &["six", "sechs", "seis"], &["sixth", "sechste", "sexta", "sexto"]),
    (
        7,
        &["seven", "sieben", "siete"],
        &["seventh", "siebte", "septima", "septimo"],
    ),
    (8, &["eight", "acht", "ocho"], &["eighth", "achte", "octava", "octavo"]),
    (9, &["nine", "neun", "nueve"], &["ninth", "neunte", "novena", "noveno"]),
    (10, &["ten", "zehn", "diez"], &["tenth", "zehnte", "decima", "decimo"]),
    (11, &["eleven", "elf", "once"], &["eleventh", "elfte"]),
    (12, &["twelve", "zwolf", "doce"], &["twelfth", "zwolfte"]),
];
const MIN_SIMILARITY: f64 = 0.75;
const CONTAINED_SIMILARITY: f64 = 0.9;
const AMBIGUITY_MARGIN: f64 = 0.05;
const MIN_CONTAINED_LENGTH: usize = 3;

static SEASON_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^s(\d{1,2})(?:e(\d{1,3}))?$").unwrap());
static ORDINAL_DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{1,2})(?:st|nd|rd|th)$").unwrap());

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Verb {
    Play,
    Download,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SeasonChoice {
    Unspecified,
    Number(u32),
    Next,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    verb: Verb,
    title: String,
    season: SeasonChoice,
    episode: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Play(PathBuf),
    Download { title: String, season: Option<u32> },
    Refuse(String),
}

pub fn understand(transcript: &str, library: &Library, progress: &Progress) -> Action {
    match parse(transcript) {
        Some(request) => resolve(&request, library, progress),
        None => Action::Refuse(format!("Could not understand \"{}\"", transcript.trim())),
    }
}

#[derive(Clone, Copy)]
struct Number {
    value: u32,
    ordinal: bool,
}

fn number(token: &str) -> Option<Number> {
    if let Ok(value) = token.parse() {
        return Some(Number { value, ordinal: false });
    }
    if let Some(digits) = ORDINAL_DIGITS.captures(token) {
        let value = digits[1].parse().ok()?;
        return Some(Number { value, ordinal: true });
    }
    let stem = token.strip_suffix(['n', 'r', 's']).unwrap_or(token);
    NUMBERS.iter().find_map(|(value, cardinals, ordinals)| {
        if cardinals.contains(&token) {
            Some(Number {
                value: *value,
                ordinal: false,
            })
        } else if ordinals.contains(&token) || ordinals.contains(&stem) {
            Some(Number {
                value: *value,
                ordinal: true,
            })
        } else {
            None
        }
    })
}

fn is_next(token: &str) -> bool {
    NEXT_WORDS.contains(&token)
}

struct Words {
    tokens: Vec<String>,
    used: Vec<bool>,
}

impl Words {
    fn new(tokens: Vec<String>) -> Self {
        let used = vec![false; tokens.len()];
        Self { tokens, used }
    }

    fn free(&self, index: usize) -> Option<&str> {
        match self.used.get(index) {
            Some(false) => Some(&self.tokens[index]),
            _ => None,
        }
    }

    fn free_number(&self, index: Option<usize>) -> Option<Number> {
        index.and_then(|index| self.free(index)).and_then(number)
    }

    fn take_counted(&mut self, words: &[&str]) -> Option<Marked> {
        for at in 0..self.tokens.len() {
            if !self.free(at).is_some_and(|token| words.contains(&token)) {
                continue;
            }
            let before = at.checked_sub(1);
            if let Some(index) = before.filter(|index| self.free(*index).is_some_and(is_next)) {
                self.used[at] = true;
                self.used[index] = true;
                return Some(Marked::Next);
            }
            if self.free(at + 1).is_some_and(is_next) {
                self.used[at] = true;
                self.used[at + 1] = true;
                return Some(Marked::Next);
            }
            let found = match (self.free_number(before), self.free_number(Some(at + 1))) {
                (Some(previous), _) if previous.ordinal => before.map(|index| (index, previous)),
                (_, Some(following)) => Some((at + 1, following)),
                (Some(previous), None) => before.map(|index| (index, previous)),
                (None, None) => None,
            };
            if let Some((index, found)) = found {
                self.used[at] = true;
                self.used[index] = true;
                return Some(Marked::Number(found.value));
            }
        }
        None
    }

    fn take_code(&mut self) -> Option<(u32, Option<u32>)> {
        for at in 0..self.tokens.len() {
            let Some(code) = self.free(at).and_then(|token| SEASON_CODE.captures(token)) else {
                continue;
            };
            let season = code[1].parse().ok()?;
            let episode = code.get(2).and_then(|episode| episode.as_str().parse().ok());
            self.used[at] = true;
            return Some((season, episode));
        }
        None
    }

    fn remaining(&self) -> Vec<&str> {
        (0..self.tokens.len()).filter_map(|index| self.free(index)).collect()
    }
}

enum Marked {
    Number(u32),
    Next,
}

fn starts_with<'a, 'b>(tokens: &'a [&'b str], phrase: &str) -> Option<&'a [&'b str]> {
    let length = phrase.split(' ').count();
    let matches = tokens.len() >= length && tokens[..length].iter().copied().eq(phrase.split(' '));
    matches.then(|| &tokens[length..])
}

fn ends_with<'a, 'b>(tokens: &'a [&'b str], phrase: &str) -> Option<&'a [&'b str]> {
    let length = phrase.split(' ').count();
    let start = tokens.len().checked_sub(length)?;
    let matches = tokens[start..].iter().copied().eq(phrase.split(' '));
    matches.then(|| &tokens[..start])
}

fn phrases() -> impl Iterator<Item = (Verb, &'static str)> {
    let verbs = |verb, list: &'static [&'static str]| list.iter().map(move |phrase| (verb, *phrase));
    verbs(Verb::Play, &PLAY_PHRASES).chain(verbs(Verb::Download, &DOWNLOAD_PHRASES))
}

fn strip_edges<'a, 'b>(mut tokens: &'a [&'b str], verb: &mut Verb) -> &'a [&'b str] {
    loop {
        let start = tokens;
        while let Some((first, rest)) = tokens.split_first()
            && FILLERS.contains(first)
        {
            tokens = rest;
        }
        while let Some((last, rest)) = tokens.split_last()
            && FILLERS.contains(last)
        {
            tokens = rest;
        }
        let spoken = phrases().find_map(|(found, phrase)| {
            starts_with(tokens, phrase)
                .or_else(|| ends_with(tokens, phrase))
                .map(|rest| (found, rest))
        });
        if let Some((found, rest)) = spoken {
            *verb = (*verb).max(found);
            tokens = rest;
        }
        if tokens.len() == start.len() {
            return tokens;
        }
    }
}

fn parse(transcript: &str) -> Option<Request> {
    let folded = library::normalize(&deunicode(transcript));
    let mut words = Words::new(folded.split_whitespace().map(str::to_string).collect());
    let mut season = SeasonChoice::Unspecified;
    let mut episode = None;
    if let Some((code_season, code_episode)) = words.take_code() {
        season = SeasonChoice::Number(code_season);
        episode = code_episode;
    }
    match words.take_counted(&SEASON_WORDS) {
        Some(Marked::Number(number)) => season = SeasonChoice::Number(number),
        Some(Marked::Next) => season = SeasonChoice::Next,
        None => {}
    }
    if let Some(Marked::Number(number)) = words.take_counted(&EPISODE_WORDS) {
        episode = Some(number);
    }
    let remaining = words.remaining();
    let mut verb = Verb::Play;
    let title = strip_edges(&remaining, &mut verb);
    if title.is_empty() {
        return None;
    }
    Some(Request {
        verb,
        title: title.join(" "),
        season,
        episode,
    })
}

#[derive(Clone, Copy)]
enum Titled<'a> {
    Show(&'a Show),
    Movie(&'a Movie),
}

impl Titled<'_> {
    fn title(&self) -> &str {
        match self {
            Self::Show(show) => &show.title,
            Self::Movie(movie) => &movie.title,
        }
    }
}

enum Found<'a> {
    One(Titled<'a>),
    Several(String, String),
    Nothing,
}

fn comparable(title: &str) -> String {
    let normalized = library::normalize(&deunicode(title));
    normalized.strip_prefix("the ").unwrap_or(&normalized).to_string()
}

fn similarity(query: &str, title: &str) -> f64 {
    let (query, title) = (comparable(query), comparable(title));
    let spaced = format!(" {title} ");
    if query.len() >= MIN_CONTAINED_LENGTH && query != title && spaced.contains(&format!(" {query} ")) {
        return CONTAINED_SIMILARITY;
    }
    normalized_damerau_levenshtein(&query.replace(' ', ""), &title.replace(' ', ""))
}

fn find<'a>(library: &'a Library, query: &str) -> Found<'a> {
    let shows = library.shows.iter().map(Titled::Show);
    let movies = library.movies.iter().map(Titled::Movie);
    let mut scored: Vec<_> = shows
        .chain(movies)
        .map(|titled| (similarity(query, titled.title()), titled))
        .filter(|(score, _)| *score >= MIN_SIMILARITY)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    match scored.as_slice() {
        [] => Found::Nothing,
        [(best, first), (second_best, second), ..]
            if best - second_best < AMBIGUITY_MARGIN && first.title() != second.title() =>
        {
            Found::Several(first.title().to_string(), second.title().to_string())
        }
        [(_, first), ..] => Found::One(*first),
    }
}

fn play(episode: &Episode) -> Action {
    Action::Play(episode.path.clone())
}

fn resume(show: &Show, progress: &Progress) -> Action {
    match progress.resume_episode(show) {
        Some(episode) => play(episode),
        None if show.episode_count() > 0 => Action::Refuse(format!("You have finished {}", show.title)),
        None => nothing_in(show),
    }
}

fn nothing_in(show: &Show) -> Action {
    Action::Refuse(format!("{} has no episodes", show.title))
}

fn play_show(show: &Show, season: SeasonChoice, episode: Option<u32>, progress: &Progress) -> Action {
    match (season, episode) {
        (SeasonChoice::Next, _) => {
            let current = progress.shows.get(&show.key).map_or(0, |position| position.season);
            play_show(show, SeasonChoice::Number(current + 1), episode, progress)
        }
        (SeasonChoice::Unspecified, None) => resume(show, progress),
        (SeasonChoice::Unspecified, Some(number)) => show
            .seasons
            .iter()
            .filter(|season| season.number > 0)
            .find_map(|season| show.episode(season.number, number))
            .map_or_else(
                || Action::Refuse(format!("{} has no episode {number}", show.title)),
                play,
            ),
        (SeasonChoice::Number(number), None) => show
            .seasons
            .iter()
            .find(|season| season.number == number)
            .and_then(|season| season.episodes.first())
            .map_or_else(
                || Action::Refuse(format!("{} has no season {number}", show.title)),
                play,
            ),
        (SeasonChoice::Number(number), Some(episode)) => show.episode(number, episode).map_or_else(
            || Action::Refuse(format!("{} has no season {number} episode {episode}", show.title)),
            play,
        ),
    }
}

fn resolve(request: &Request, library: &Library, progress: &Progress) -> Action {
    let found = find(library, &request.title);
    if let Found::Several(first, second) = &found {
        return Action::Refuse(format!("Did you mean {first} or {second}?"));
    }
    if request.verb == Verb::Download {
        return download(request, &found);
    }
    match found {
        Found::One(Titled::Show(show)) => play_show(show, request.season, request.episode, progress),
        Found::One(Titled::Movie(movie)) => Action::Play(movie.path.clone()),
        _ => Action::Refuse(format!("\"{}\" is not in the library", request.title)),
    }
}

fn download(request: &Request, found: &Found) -> Action {
    let show = match found {
        Found::One(Titled::Show(show)) => Some(*show),
        _ => None,
    };
    let title = match found {
        Found::One(titled) => titled.title().to_string(),
        _ => request.title.clone(),
    };
    let season = match request.season {
        SeasonChoice::Number(number) => Some(number),
        SeasonChoice::Next => show.map(|show| show.last_season() + 1),
        SeasonChoice::Unspecified => None,
    };
    Action::Download { title, season }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::library_from_paths;
    use crate::progress::Position;

    fn library() -> Library {
        let paths = [
            "Series/Outlander/Outlander.S01E01.mkv",
            "Series/Outlander/Outlander.S01E02.mkv",
            "Series/Outlander/Outlander.S02E01.mkv",
            "Series/Outlander/Outlander.S02E02.mkv",
            "Series/Outlander/Outlander.S02E03.mkv",
            "Series/The Bear/The.Bear.S01E01.mkv",
            "Series/The Bear/The.Bear.S01E02.mkv",
            "Series/Star Wars Andor/Star.Wars.Andor.S01E01.mkv",
            "Series/Star Wars Andor/Star.Wars.Andor.S01E02.mkv",
            "Series/Star Wars Ahsoka/Star.Wars.Ahsoka.S01E01.mkv",
            "Series/Star Wars Ahsoka/Star.Wars.Ahsoka.S01E02.mkv",
            "Movies/Dune (2021)/Dune.2021.1080p.mkv",
        ];
        library_from_paths(&paths)
    }

    fn progress(season: u32, episode: u32, finished: bool) -> Progress {
        let position = Position {
            season,
            episode,
            finished,
        };
        Progress {
            shows: [("outlander".to_string(), position)].into(),
            ..Progress::default()
        }
    }

    fn request(verb: Verb, season: SeasonChoice, episode: Option<u32>) -> Option<Request> {
        Some(Request {
            verb,
            title: "outlander".to_string(),
            season,
            episode,
        })
    }

    fn episode_path(season: u32, episode: u32) -> Action {
        let path = format!("/lib/Series/Outlander/Outlander.S{season:02}E{episode:02}.mkv");
        Action::Play(path.into())
    }

    #[test]
    fn continuing_a_show_is_understood_in_every_language() {
        for text in [
            "Continue playing Outlander.",
            "keep watching Outlander please",
            "Can you resume Outlander?",
            "Mach weiter mit Outlander",
            "Outlander weiterschauen",
            "Schau Outlander weiter",
            "Continúa Outlander",
            "Sigue viendo Outlander",
            "Por favor, sigue con Outlander",
            "Ich möchte Outlander weiterschauen",
        ] {
            let expected = request(Verb::Play, SeasonChoice::Unspecified, None);
            assert_eq!(parse(text), expected, "{text}");
        }
    }

    #[test]
    fn seasons_and_episodes_are_understood_in_every_language() {
        for text in [
            "Play Outlander season 2 episode 3",
            "play season two episode three of Outlander",
            "Play the second season, third episode of Outlander",
            "Watch Outlander S02E03",
            "Spiele Outlander Staffel 2 Folge 3",
            "Outlander zweite Staffel dritte Folge abspielen",
            "Zeig mir die dritte Folge der zweiten Staffel von Outlander",
            "Ich will Staffel 2 Folge 3 von Outlander sehen",
            "Pon Outlander temporada 2 episodio 3 por favor",
            "Quiero ver el tercer episodio de la segunda temporada de Outlander",
        ] {
            let expected = request(Verb::Play, SeasonChoice::Number(2), Some(3));
            assert_eq!(parse(text), expected, "{text}");
        }
    }

    #[test]
    fn download_requests_are_understood_in_every_language() {
        for text in [
            "Download the next season of Outlander",
            "Lade die nächste Staffel von Outlander herunter",
            "Die nächste Staffel von Outlander runterladen",
            "Descarga la siguiente temporada de Outlander",
            "Descargar la temporada siguiente de Outlander",
            "Baja la próxima temporada de Outlander",
        ] {
            let expected = request(Verb::Download, SeasonChoice::Next, None);
            assert_eq!(parse(text), expected, "{text}");
        }
    }

    #[test]
    fn a_bare_title_means_play() {
        assert_eq!(parse("Outlander"), request(Verb::Play, SeasonChoice::Unspecified, None));
    }

    #[test]
    fn a_sentence_without_a_title_is_not_a_request() {
        for text in ["", "please", "play", "weiter", "season 2"] {
            assert_eq!(parse(text), None, "{text}");
        }
    }

    #[test]
    fn invented_subtitles_from_silence_do_nothing() {
        for text in [
            "Thank you for watching.",
            "Untertitel im Auftrag des ZDF",
            "Gracias por ver el video",
        ] {
            assert!(
                matches!(understand(text, &library(), &Progress::default()), Action::Refuse(_)),
                "{text}"
            );
        }
    }

    #[test]
    fn a_title_keeps_words_that_only_look_like_fillers_inside_it() {
        let parsed = parse("play the man in the high castle season 2").unwrap();
        assert_eq!(parsed.title, "man in the high castle");
    }

    #[test]
    fn season_needs_a_number_to_count_as_a_season() {
        assert_eq!(parse("Play Season of the Witch").unwrap().title, "season of the witch");
    }

    #[test]
    fn numbers_cover_digits_words_and_ordinals() {
        let values = [
            ("7", 7),
            ("3rd", 3),
            ("once", 11),
            ("zweiten", 2),
            ("dritter", 3),
            ("zwolf", 12),
        ];
        for (token, value) in values {
            assert_eq!(number(token).map(|found| found.value), Some(value), "{token}");
        }
        assert!(number("outlander").is_none());
    }

    #[test]
    fn a_specific_episode_plays_that_file() {
        let action = understand("spiele Outlander Staffel 2 Folge 3", &library(), &Progress::default());
        assert_eq!(action, episode_path(2, 3));
    }

    #[test]
    fn a_season_alone_starts_at_its_first_episode() {
        let action = understand("play season 2 of Outlander", &library(), &Progress::default());
        assert_eq!(action, episode_path(2, 1));
    }

    #[test]
    fn missing_seasons_and_episodes_are_refused() {
        let library = library();
        for (text, message) in [
            ("play Outlander season 9", "Outlander has no season 9"),
            (
                "play Outlander season 1 episode 9",
                "Outlander has no season 1 episode 9",
            ),
        ] {
            let expected = Action::Refuse(message.to_string());
            assert_eq!(understand(text, &library, &Progress::default()), expected, "{text}");
        }
    }

    #[test]
    fn continue_starts_at_the_beginning_of_an_unwatched_show() {
        assert_eq!(
            understand("continue Outlander", &library(), &Progress::default()),
            episode_path(1, 1)
        );
    }

    #[test]
    fn continue_returns_to_an_unfinished_episode() {
        let action = understand("continue Outlander", &library(), &progress(2, 2, false));
        assert_eq!(action, episode_path(2, 2));
    }

    #[test]
    fn continue_moves_on_after_a_finished_episode_across_seasons() {
        let action = understand("weiter mit Outlander", &library(), &progress(1, 2, true));
        assert_eq!(action, episode_path(2, 1));
    }

    #[test]
    fn continue_says_so_when_the_show_is_finished() {
        let action = understand("continue Outlander", &library(), &progress(2, 3, true));
        assert_eq!(action, Action::Refuse("You have finished Outlander".to_string()));
    }

    #[test]
    fn continuing_with_a_season_goes_to_that_season() {
        let action = understand("continue Outlander season 2", &library(), &progress(1, 1, false));
        assert_eq!(action, episode_path(2, 1));
    }

    #[test]
    fn the_next_season_counts_from_what_you_are_watching() {
        let library = library();
        let action = understand("play the next season of Outlander", &library, &progress(1, 2, true));
        assert_eq!(action, episode_path(2, 1));
        let action = understand("play the next season of Outlander", &library, &progress(2, 1, false));
        assert_eq!(action, Action::Refuse("Outlander has no season 3".to_string()));
    }

    #[test]
    fn a_bare_title_continues_where_you_left_off() {
        assert_eq!(
            understand("Outlander", &library(), &progress(2, 2, false)),
            episode_path(2, 2)
        );
    }

    #[test]
    fn transcription_slips_still_find_the_title() {
        for text in ["play Out Lander", "play outlanders", "play Outlender", "play Autlander"] {
            assert_eq!(
                understand(text, &library(), &Progress::default()),
                episode_path(1, 1),
                "{text}"
            );
        }
    }

    #[test]
    fn part_of_a_title_finds_it() {
        let action = understand("play Andor", &library(), &Progress::default());
        let path = "/lib/Series/Star Wars Andor/Star.Wars.Andor.S01E01.mkv";
        assert_eq!(action, Action::Play(path.into()));
    }

    #[test]
    fn an_ambiguous_title_asks_instead_of_guessing() {
        let Action::Refuse(message) = understand("play Star Wars", &library(), &Progress::default()) else {
            panic!("expected a question");
        };
        assert!(message.starts_with("Did you mean"), "{message}");
        assert!(message.contains("Andor") && message.contains("Ahsoka"), "{message}");
    }

    #[test]
    fn movies_play_directly() {
        let action = understand("spiele Dune", &library(), &Progress::default());
        assert_eq!(
            action,
            Action::Play("/lib/Movies/Dune (2021)/Dune.2021.1080p.mkv".into())
        );
    }

    #[test]
    fn an_unknown_title_is_refused_for_playing() {
        let action = understand("play Severance", &library(), &Progress::default());
        assert_eq!(
            action,
            Action::Refuse("\"severance\" is not in the library".to_string())
        );
    }

    #[test]
    fn downloading_the_next_season_counts_from_the_library() {
        let action = understand(
            "download the next season of Outlander",
            &library(),
            &Progress::default(),
        );
        let expected = Action::Download {
            title: "Outlander".to_string(),
            season: Some(3),
        };
        assert_eq!(action, expected);
    }

    #[test]
    fn a_show_outside_the_library_can_still_be_requested() {
        let action = understand("download Severance season 2", &library(), &Progress::default());
        let expected = Action::Download {
            title: "severance".to_string(),
            season: Some(2),
        };
        assert_eq!(action, expected);
    }
}
