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
const PAUSE_PHRASES: [&str; 8] = [
    "pause",
    "pausa",
    "pausieren",
    "pausiere",
    "anhalten",
    "halt an",
    "pon pausa",
    "hold on",
];
const STOP_PHRASES: [&str; 9] = [
    "stop", "stopp", "quit", "beenden", "beende", "aufhoren", "para", "parar", "detener",
];
const MUTE_PHRASES: [&str; 10] = [
    "mute",
    "sound off",
    "stumm",
    "stummschalten",
    "schalte stumm",
    "ton aus",
    "silencio",
    "silenciar",
    "silencia",
    "quita el sonido",
];
const UNMUTE_PHRASES: [&str; 9] = [
    "unmute",
    "sound on",
    "ton an",
    "ton ein",
    "stumm aus",
    "laut",
    "quita el silencio",
    "activa el sonido",
    "pon el sonido",
];
const WAKE_WORDS: [&str; 3] = ["stevie", "the", "tv"];
const TV_WORDS: [&str; 7] = ["tv", "telly", "television", "fernseher", "glotze", "tele", "televisor"];
const ON_WORDS: [&str; 9] = [
    "on",
    "an",
    "ein",
    "einschalten",
    "anschalten",
    "anmachen",
    "enciende",
    "encender",
    "prende",
];
const OFF_WORDS: [&str; 7] = [
    "off",
    "aus",
    "ausschalten",
    "ausmachen",
    "abschalten",
    "apaga",
    "apagar",
];
const SWITCH_WORDS: [&str; 8] = ["turn", "switch", "schalte", "schalt", "mach", "mache", "den", "put"];
const SEMMELTV: &str = "semmeltv";
const INPUT_WORDS: [&str; 22] = [
    "change",
    "go",
    "input",
    "source",
    "hdmi",
    "back",
    "select",
    "open",
    "auf",
    "um",
    "umschalten",
    "zu",
    "zum",
    "zuruck",
    "wechsel",
    "wechsle",
    "wechseln",
    "eingang",
    "quelle",
    "zeig",
    "zeige",
    "cambia",
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
const PREFIX_SIMILARITY: f64 = 0.95;
const AMBIGUITY_MARGIN: f64 = 0.05;
const MIN_CONTAINED_LENGTH: usize = 3;

static SEASON_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^s(\d{1,2})(?:e(\d{1,3}))?$").unwrap());
static ORDINAL_DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{1,2})(?:st|nd|rd|th)$").unwrap());
static SEMMELTV_SPOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:s[aeiouy]{1,2}mm?[aeiouy]{1,2}ll?|simple) ?tv\b").unwrap());

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
    Download {
        title: String,
        season: Option<u32>,
        episode: Option<u32>,
    },
    Control(Control),
    Refuse(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Pause,
    Resume,
    Stop,
    Mute,
    Unmute,
    TvOn,
    TvOff,
    ShowSemmelTv,
}

impl Control {
    pub fn is_tv(self) -> bool {
        matches!(self, Self::TvOn | Self::TvOff | Self::ShowSemmelTv)
    }
}

pub fn understand(transcript: &str, library: &Library, progress: &Progress) -> Action {
    if let Some(command) = after_wake_word(transcript) {
        if command.is_empty() {
            return Action::Refuse(format!("Heard only the wake word in \"{}\"", transcript.trim()));
        }
        let action = interpret(&command, library, progress);
        if !matches!(action, Action::Refuse(_)) {
            return action;
        }
    }
    interpret(transcript, library, progress)
}

fn after_wake_word(transcript: &str) -> Option<String> {
    let words: Vec<&str> = transcript.split_whitespace().collect();
    let key = |word: &str| library::normalize(&deunicode(word)).replace(' ', "");
    let leftover = (0..WAKE_WORDS.len())
        .map(|start| &WAKE_WORDS[start..])
        .chain([&["v"][..]])
        .find(|tail| {
            words.len() >= tail.len() && words.iter().zip(tail.iter()).all(|(word, wake)| key(word) == *wake)
        })?;
    Some(words[leftover.len()..].join(" "))
}

const YES_WORDS: [&str; 12] = [
    "yes", "yeah", "yep", "yup", "ok", "okay", "sure", "ja", "si", "go", "do", "download",
];
const NO_WORDS: [&str; 6] = ["no", "nope", "cancel", "nein", "dont", "stop"];
const REPLY_FILLERS: [&str; 10] = [
    "stevie", "tv", "it", "please", "that", "the", "thanks", "thank", "you", "so",
];

pub fn reply(transcript: &str) -> Option<bool> {
    let command = after_wake_word(transcript).unwrap_or_else(|| transcript.to_string());
    let folded = library::normalize(&deunicode(&command));
    let words: Vec<&str> = folded
        .split_whitespace()
        .filter(|word| !REPLY_FILLERS.contains(word))
        .collect();
    match words.as_slice() {
        [word] if NO_WORDS.contains(word) => Some(false),
        [word] if YES_WORDS.contains(word) => Some(true),
        [first, second] if YES_WORDS.contains(first) && YES_WORDS.contains(second) => Some(true),
        _ => None,
    }
}

fn interpret(transcript: &str, library: &Library, progress: &Progress) -> Action {
    if let Some(control) = control(transcript) {
        return Action::Control(control);
    }
    match parse(transcript) {
        Some(request) => resolve(&request, library, progress),
        None => Action::Refuse(format!("Could not understand \"{}\"", transcript.trim())),
    }
}

fn control(transcript: &str) -> Option<Control> {
    let folded = library::normalize(&deunicode(transcript)).replace("t v", "tv");
    let folded = SEMMELTV_SPOKEN.replace_all(&folded, SEMMELTV);
    let tokens: Vec<&str> = folded
        .split_whitespace()
        .filter(|token| !FILLERS.contains(token))
        .collect();
    if tokens.contains(&SEMMELTV) {
        return show_semmeltv(&tokens);
    }
    if tokens.iter().any(|token| TV_WORDS.contains(token)) {
        return tv_power(&tokens);
    }
    let is = |phrases: &[&str]| {
        phrases.iter().any(|phrase| {
            phrase
                .split(' ')
                .filter(|word| !FILLERS.contains(word))
                .eq(tokens.iter().copied())
        })
    };
    if is(&PAUSE_PHRASES) {
        Some(Control::Pause)
    } else if is(&STOP_PHRASES) {
        Some(Control::Stop)
    } else if is(&MUTE_PHRASES) {
        Some(Control::Mute)
    } else if is(&UNMUTE_PHRASES) {
        Some(Control::Unmute)
    } else if is(&PLAY_PHRASES) {
        Some(Control::Resume)
    } else {
        None
    }
}

fn tv_power(tokens: &[&str]) -> Option<Control> {
    let mut off = false;
    for token in tokens {
        if OFF_WORDS.contains(token) {
            off = true;
        } else if !TV_WORDS.contains(token) && !ON_WORDS.contains(token) && !SWITCH_WORDS.contains(token) {
            return None;
        }
    }
    Some(if off { Control::TvOff } else { Control::TvOn })
}

fn show_semmeltv(tokens: &[&str]) -> Option<Control> {
    tokens
        .iter()
        .all(|token| {
            *token == SEMMELTV
                || INPUT_WORDS.contains(token)
                || SWITCH_WORDS.contains(token)
                || ON_WORDS.contains(token)
                || TV_WORDS.contains(token)
        })
        .then_some(Control::ShowSemmelTv)
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
    let words: Vec<String> = normalized
        .split(' ')
        .map(|word| cardinal(word).map_or_else(|| word.to_string(), |value| value.to_string()))
        .collect();
    let joined = words.join(" ");
    joined.strip_prefix("the ").unwrap_or(&joined).to_string()
}

fn cardinal(word: &str) -> Option<u32> {
    NUMBERS
        .iter()
        .find(|(_, cardinals, _)| cardinals.contains(&word))
        .map(|(value, _, _)| *value)
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
    if request.verb == Verb::Download {
        return download(request, &found);
    }
    if let Found::Several(first, second) = &found {
        return Action::Refuse(format!("Did you mean {first} or {second}?"));
    }
    match found {
        Found::One(Titled::Show(show)) => play_show(show, request.season, request.episode, progress),
        Found::One(Titled::Movie(movie)) => Action::Play(movie.path.clone()),
        _ => Action::Refuse(format!("\"{}\" is not in the library", request.title)),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    Movie(String),
    Show {
        key: String,
        season: Option<u32>,
        episode: Option<u32>,
    },
}

pub fn choose(pick: &Pick, library: &Library, progress: &Progress) -> Action {
    match pick {
        Pick::Movie(key) => library
            .movies
            .iter()
            .find(|movie| &movie.key == key)
            .map_or_else(|| missing(key), |movie| Action::Play(movie.path.clone())),
        Pick::Show { key, season, episode } => {
            let Some(show) = library.shows.iter().find(|show| &show.key == key) else {
                return missing(key);
            };
            let season = season.map_or(SeasonChoice::Unspecified, SeasonChoice::Number);
            play_show(show, season, *episode, progress)
        }
    }
}

fn missing(key: &str) -> Action {
    Action::Refuse(format!("\"{key}\" is not in the library"))
}

pub fn search(library: &Library, query: &str) -> Vec<library::Item> {
    let query = comparable(query);
    if query.is_empty() {
        return Vec::new();
    }
    let shows = library
        .shows
        .iter()
        .enumerate()
        .map(|(index, show)| (library::Item::Show(index), &show.title));
    let movies = library
        .movies
        .iter()
        .enumerate()
        .map(|(index, movie)| (library::Item::Movie(index), &movie.title));
    let mut scored: Vec<_> = shows
        .chain(movies)
        .filter_map(|(item, title)| {
            let score = typed_similarity(&query, title);
            (score >= MIN_SIMILARITY).then(|| (score, library::sort_title(title), item))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, item)| item).collect()
}

fn typed_similarity(query: &str, title: &str) -> f64 {
    let comparable_title = comparable(title);
    if comparable_title == query {
        return 1.0;
    }
    if comparable_title.starts_with(query) {
        return PREFIX_SIMILARITY;
    }
    if format!(" {comparable_title}").contains(&format!(" {query}")) {
        return CONTAINED_SIMILARITY;
    }
    similarity(query, title)
}

fn download(request: &Request, found: &Found) -> Action {
    let known = match found {
        Found::One(titled) if comparable(titled.title()) == comparable(&request.title) => Some(*titled),
        _ => None,
    };
    let show = match known {
        Some(Titled::Show(show)) => Some(show),
        _ => None,
    };
    let title = known.map_or_else(|| request.title.clone(), |titled| titled.title().to_string());
    let season = match request.season {
        SeasonChoice::Number(number) => Some(number),
        SeasonChoice::Next => match show {
            Some(show) => Some(show.last_season() + 1),
            None => return Action::Refuse(format!("\"{}\" is not in the library, so say which season", request.title)),
        },
        SeasonChoice::Unspecified => None,
    };
    Action::Download {
        title,
        season,
        episode: request.episode,
    }
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
    fn a_picked_title_plays_like_the_same_spoken_request() {
        let library = library();
        let progress = progress(1, 2, true);
        let show = |season, episode| Pick::Show {
            key: "outlander".to_string(),
            season,
            episode,
        };
        assert_eq!(choose(&show(None, None), &library, &progress), episode_path(2, 1));
        assert_eq!(
            choose(&show(None, None), &library, &progress),
            understand("continue Outlander", &library, &progress)
        );
        assert_eq!(choose(&show(Some(2), Some(3)), &library, &progress), episode_path(2, 3));
        assert_eq!(choose(&show(Some(2), None), &library, &progress), episode_path(2, 1));
        assert_eq!(
            choose(&show(Some(9), None), &library, &progress),
            Action::Refuse("Outlander has no season 9".to_string())
        );
        assert_eq!(
            choose(&Pick::Movie("dune-2021".to_string()), &library, &progress),
            Action::Play("/lib/Movies/Dune (2021)/Dune.2021.1080p.mkv".into())
        );
        assert!(matches!(
            choose(&Pick::Movie("jaws".to_string()), &library, &progress),
            Action::Refuse(_)
        ));
    }

    #[test]
    fn search_ranks_whole_titles_then_prefixes_then_words_then_near_misses() {
        let library = library();
        let titles = |query: &str| -> Vec<String> {
            search(&library, query)
                .into_iter()
                .map(|item| match item {
                    library::Item::Show(index) => library.shows[index].title.clone(),
                    library::Item::Movie(index) => library.movies[index].title.clone(),
                })
                .collect()
        };
        assert_eq!(titles("star wars"), ["Star Wars Ahsoka", "Star Wars Andor"]);
        assert_eq!(titles("andor"), ["Star Wars Andor"]);
        assert_eq!(titles("The B"), ["The Bear"]);
        assert_eq!(titles("dune"), ["Dune"]);
        assert_eq!(titles("outlnder"), ["Outlander"]);
        assert!(titles("").is_empty());
        assert!(titles("x").is_empty());
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
    fn playback_controls_are_understood_in_every_language() {
        let cases = [
            ("Pause.", Control::Pause),
            ("Pause, please.", Control::Pause),
            ("Pausa", Control::Pause),
            ("Halt an", Control::Pause),
            ("Play.", Control::Resume),
            ("Continue", Control::Resume),
            ("Keep watching", Control::Resume),
            ("Weiter", Control::Resume),
            ("Sigue", Control::Resume),
            ("Stop", Control::Stop),
            ("Mute.", Control::Mute),
            ("Ton aus", Control::Mute),
            ("Silencio", Control::Mute),
            ("Unmute", Control::Unmute),
            ("Ton an", Control::Unmute),
        ];
        for (text, expected) in cases {
            let understood = understand(text, &library(), &Progress::default());
            assert_eq!(understood, Action::Control(expected), "{text}");
        }
    }

    #[test]
    fn the_tv_is_switched_in_every_language() {
        let cases = [
            ("Turn on the TV.", Control::TvOn),
            ("Switch the T.V. on", Control::TvOn),
            ("TV on", Control::TvOn),
            ("Schalte den Fernseher ein.", Control::TvOn),
            ("Mach den Fernseher an", Control::TvOn),
            ("Enciende la tele", Control::TvOn),
            ("Turn off the TV", Control::TvOff),
            ("Fernseher aus", Control::TvOff),
            ("Apaga la televisión", Control::TvOff),
        ];
        for (text, expected) in cases {
            let understood = understand(text, &library(), &Progress::default());
            assert_eq!(understood, Action::Control(expected), "{text}");
        }
    }

    #[test]
    fn switching_to_semmeltv_is_understood_however_it_is_transcribed() {
        for text in [
            "Switch to SemmelTV",
            "Switch to Semmel TV.",
            "SemmelTV input",
            "Semmel-TV Eingang",
            "Auf SemmelTV umschalten",
            "Schalte auf Semmel TV um",
            "Wechsel zu SemmelTV",
            "Switch the TV to SimilTV",
            "simil tv",
            "Simmel TV, please.",
            "Go to semel tv",
            "Sammel T.V.",
            "Samuel TV input",
            "Simple TV",
            "Turn on SemmelTV",
            "Stevie the TV, switch to SemmelTV.",
        ] {
            let understood = understand(text, &library(), &Progress::default());
            assert_eq!(understood, Action::Control(Control::ShowSemmelTv), "{text}");
        }
    }

    #[test]
    fn semmeltv_next_to_other_words_is_not_an_input_switch() {
        for text in ["SemmelTV aus", "Switch SemmelTV off", "Play Outlander on SemmelTV"] {
            let understood = understand(text, &library(), &Progress::default());
            assert_ne!(understood, Action::Control(Control::ShowSemmelTv), "{text}");
        }
    }

    #[test]
    fn a_control_word_inside_a_longer_sentence_is_not_a_control() {
        for text in [
            "Play Outlander",
            "Stop making sense",
            "Thank you for watching.",
            "the tv show Dark",
        ] {
            let understood = understand(text, &library(), &Progress::default());
            assert!(!matches!(understood, Action::Control(_)), "{text}");
        }
    }

    #[test]
    fn the_end_of_the_wake_word_before_a_command_is_ignored() {
        let library = library();
        let progress = Progress::default();
        for (heard, meant) in [
            ("TV, play Outlander.", "play Outlander"),
            ("V. Play Outlander", "play Outlander"),
            ("the TV pause", "pause"),
            ("Stevie the TV, stop.", "stop"),
        ] {
            assert_eq!(
                understand(heard, &library, &progress),
                understand(meant, &library, &progress),
                "{heard}"
            );
        }
        assert_eq!(
            understand("TV off", &library, &progress),
            Action::Control(Control::TvOff)
        );
        assert_eq!(
            understand("TV, TV off", &library, &progress),
            Action::Control(Control::TvOff)
        );
    }

    #[test]
    fn numbers_in_titles_match_as_digits_or_words() {
        assert_eq!(similarity("brooklyn 9 9", "Brooklyn Nine-Nine"), 1.0);
        assert_eq!(similarity("paddington two", "Paddington 2"), 1.0);
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
    fn a_short_yes_or_no_answers_an_offer() {
        assert_eq!(reply("yes"), Some(true));
        assert_eq!(reply("Yes, please."), Some(true));
        assert_eq!(reply("Stevie, do it"), Some(true));
        assert_eq!(reply("download it"), Some(true));
        assert_eq!(reply("ok go"), Some(true));
        assert_eq!(reply("no"), Some(false));
        assert_eq!(reply("Nein."), Some(false));
        assert_eq!(reply("play outlander"), None);
        assert_eq!(reply("download outlander season 8"), None);
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
            episode: None,
        };
        assert_eq!(action, expected);
    }

    #[test]
    fn a_show_outside_the_library_can_still_be_requested() {
        let action = understand("download Severance season 2", &library(), &Progress::default());
        let expected = Action::Download {
            title: "severance".to_string(),
            season: Some(2),
            episode: None,
        };
        assert_eq!(action, expected);
    }

    #[test]
    fn a_download_close_to_a_library_title_keeps_its_own_title() {
        let action = understand("download Outlandish season 1", &library(), &Progress::default());
        let expected = Action::Download {
            title: "outlandish".to_string(),
            season: Some(1),
            episode: None,
        };
        assert_eq!(action, expected);
        let action = understand("download outlander season 8 episode 3", &library(), &Progress::default());
        let expected = Action::Download {
            title: "Outlander".to_string(),
            season: Some(8),
            episode: Some(3),
        };
        assert_eq!(action, expected);
    }
}
