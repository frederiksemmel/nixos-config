use crate::library::{self, Episode, Library, Movie, Show};
use crate::manifest::About;
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
const ABOUT_SIMILARITY: f64 = 0.85;
const NAME_SIMILARITY: f64 = 0.85;
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

pub fn understand_typed_download(text: &str, library: &Library) -> Action {
    match parse_words(text, false) {
        Some(request) => download(&request, &find(library, &request.title)),
        None => Action::Refuse("Say what to download".to_string()),
    }
}

fn parse(transcript: &str) -> Option<Request> {
    parse_words(transcript, true)
}

fn parse_words(transcript: &str, spoken: bool) -> Option<Request> {
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
    let mut verb = if spoken { Verb::Play } else { Verb::Download };
    let title = if spoken {
        strip_edges(&remaining, &mut verb)
    } else {
        &remaining
    };
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub item: library::Item,
    pub because: Option<String>,
}

pub fn search(library: &Library, query: &str) -> Vec<Hit> {
    let query = comparable(query);
    if query.is_empty() {
        return Vec::new();
    }
    let shows = library
        .shows
        .iter()
        .enumerate()
        .map(|(index, show)| (library::Item::Show(index), &show.title, &show.about));
    let movies = library
        .movies
        .iter()
        .enumerate()
        .map(|(index, movie)| (library::Item::Movie(index), &movie.title, &movie.about));
    let mut scored: Vec<_> = shows
        .chain(movies)
        .filter_map(|(item, title, about)| {
            let score = typed_similarity(&query, title);
            let because = (score < ABOUT_SIMILARITY).then(|| about_match(&query, about)).flatten();
            let score = if because.is_some() { ABOUT_SIMILARITY } else { score };
            (score >= MIN_SIMILARITY).then(|| (score, library::sort_title(title), Hit { item, because }))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, hit)| hit).collect()
}

fn about_match(query: &str, about: &About) -> Option<String> {
    if query.len() < MIN_CONTAINED_LENGTH {
        return None;
    }
    let words = |text: &str| format!(" {}", comparable(text)).contains(&format!(" {query}"));
    let person = |name: &str| words(name) || similarity(query, name) >= NAME_SIMILARITY;
    let first = |values: &[String], test: &dyn Fn(&str) -> bool| values.iter().find(|value| test(value)).cloned();
    if let Some(name) = first(&about.directors, &person) {
        return Some(format!("Director: {name}"));
    }
    if let Some(name) = first(&about.creators, &person) {
        return Some(format!("Created by {name}"));
    }
    if let Some(member) = about.cast.iter().find(|member| person(&member.name)) {
        return Some(format!("With {}", member.name));
    }
    if let Some((member, character)) = about.cast.iter().find_map(|member| {
        Some((
            member,
            member.character.as_deref().filter(|character| words(character))?,
        ))
    }) {
        return Some(format!("With {} as {character}", member.name));
    }
    if let Some(original) = about.original_title.as_deref().filter(|original| words(original)) {
        return Some(format!("Original title: {original}"));
    }
    if let Some(network) = first(&about.networks, &words) {
        return Some(format!("On {network}"));
    }
    first(&about.genres, &words).map(|genre| format!("Genre: {genre}"))
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
            None => {
                return Action::Refuse(format!(
                    "\"{}\" is not in the library, so say which season",
                    request.title
                ));
            }
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

    const PATHS: [&str; 12] = [
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

    fn library() -> Library {
        library_from_paths(&PATHS)
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
                .map(|hit| match hit.item {
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
        assert!(search(&library, "outl").iter().all(|hit| hit.because.is_none()));
    }

    #[test]
    fn search_finds_titles_by_their_people_and_says_why() {
        let mut library = library();
        let member = |name: &str, character: Option<&str>| crate::manifest::Member {
            name: name.to_string(),
            character: character.map(str::to_string),
            person: None,
            imdb: None,
        };
        library.movies[0].about = crate::manifest::About {
            directors: vec!["Denis Villeneuve".to_string()],
            cast: vec![
                member("Timothée Chalamet", Some("Paul Atreides")),
                member("Rebecca Ferguson", Some("Lady Jessica")),
            ],
            genres: vec!["Science Fiction".to_string()],
            original_title: Some("Dune: Part One".to_string()),
            ..Default::default()
        };
        let outlander = library.shows.iter().position(|show| show.title == "Outlander").unwrap();
        library.shows[outlander].about = crate::manifest::About {
            creators: vec!["Ronald D. Moore".to_string()],
            cast: vec![
                member("Caitríona Balfe", Some("Claire Fraser")),
                member("Sam Heughan", None),
            ],
            networks: vec!["Starz".to_string()],
            ..Default::default()
        };
        let found = |query: &str| -> Vec<(String, Option<String>)> {
            search(&library, query)
                .into_iter()
                .map(|hit| {
                    let title = match hit.item {
                        library::Item::Show(index) => library.shows[index].title.clone(),
                        library::Item::Movie(index) => library.movies[index].title.clone(),
                    };
                    (title, hit.because)
                })
                .collect()
        };
        let one = |title: &str, because: &str| vec![(title.to_string(), Some(because.to_string()))];
        assert_eq!(found("villeneuve"), one("Dune", "Director: Denis Villeneuve"));
        assert_eq!(found("Denis Villeneuve"), one("Dune", "Director: Denis Villeneuve"));
        assert_eq!(found("chalamet"), one("Dune", "With Timothée Chalamet"));
        assert_eq!(found("timothee"), one("Dune", "With Timothée Chalamet"));
        assert_eq!(found("Rebeca Ferguson"), one("Dune", "With Rebecca Ferguson"));
        assert_eq!(
            found("lady jessica"),
            one("Dune", "With Rebecca Ferguson as Lady Jessica")
        );
        assert_eq!(found("science fic"), one("Dune", "Genre: Science Fiction"));
        assert_eq!(found("part one"), one("Dune", "Original title: Dune: Part One"));
        assert_eq!(found("ronald d moore"), one("Outlander", "Created by Ronald D. Moore"));
        assert_eq!(found("ronald moore"), one("Outlander", "Created by Ronald D. Moore"));
        assert_eq!(found("heughan"), one("Outlander", "With Sam Heughan"));
        assert_eq!(
            found("claire"),
            one("Outlander", "With Caitríona Balfe as Claire Fraser")
        );
        assert_eq!(found("starz"), one("Outlander", "On Starz"));
        assert_eq!(found("Fe"), Vec::new());
        assert_eq!(
            found("dune"),
            vec![("Dune".to_string(), None)],
            "a title match needs no reason"
        );
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
    fn parse_understands_requests_in_every_language() {
        let request = |verb, title: &str, season, episode| {
            Some(Request {
                verb,
                title: title.to_string(),
                season,
                episode,
            })
        };
        let continuing = request(Verb::Play, "outlander", SeasonChoice::Unspecified, None);
        let second_third = request(Verb::Play, "outlander", SeasonChoice::Number(2), Some(3));
        let next_season = request(Verb::Download, "outlander", SeasonChoice::Next, None);
        let rows = [
            ("Continue playing Outlander.", continuing.clone()),
            ("keep watching Outlander please", continuing.clone()),
            ("Can you resume Outlander?", continuing.clone()),
            ("Mach weiter mit Outlander", continuing.clone()),
            ("Outlander weiterschauen", continuing.clone()),
            ("Schau Outlander weiter", continuing.clone()),
            ("Continúa Outlander", continuing.clone()),
            ("Sigue viendo Outlander", continuing.clone()),
            ("Por favor, sigue con Outlander", continuing.clone()),
            ("Ich möchte Outlander weiterschauen", continuing.clone()),
            ("Play Outlander season 2 episode 3", second_third.clone()),
            ("play season two episode three of Outlander", second_third.clone()),
            (
                "Play the second season, third episode of Outlander",
                second_third.clone(),
            ),
            ("Watch Outlander S02E03", second_third.clone()),
            ("Spiele Outlander Staffel 2 Folge 3", second_third.clone()),
            ("Outlander zweite Staffel dritte Folge abspielen", second_third.clone()),
            (
                "Zeig mir die dritte Folge der zweiten Staffel von Outlander",
                second_third.clone(),
            ),
            ("Ich will Staffel 2 Folge 3 von Outlander sehen", second_third.clone()),
            ("Pon Outlander temporada 2 episodio 3 por favor", second_third.clone()),
            (
                "Quiero ver el tercer episodio de la segunda temporada de Outlander",
                second_third.clone(),
            ),
            ("Download the next season of Outlander", next_season.clone()),
            ("Lade die nächste Staffel von Outlander herunter", next_season.clone()),
            ("Die nächste Staffel von Outlander runterladen", next_season.clone()),
            ("Descarga la siguiente temporada de Outlander", next_season.clone()),
            ("Descargar la temporada siguiente de Outlander", next_season.clone()),
            ("Baja la próxima temporada de Outlander", next_season.clone()),
            (
                "Play the 3rd season of Outlander",
                request(Verb::Play, "outlander", SeasonChoice::Number(3), None),
            ),
            (
                "Pon Outlander temporada 1 episodio once",
                request(Verb::Play, "outlander", SeasonChoice::Number(1), Some(11)),
            ),
            (
                "play the man in the high castle season 2",
                request(Verb::Play, "man in the high castle", SeasonChoice::Number(2), None),
            ),
            (
                "Play Season of the Witch",
                request(Verb::Play, "season of the witch", SeasonChoice::Unspecified, None),
            ),
            ("", None),
            ("please", None),
            ("play", None),
            ("weiter", None),
            ("season 2", None),
        ];
        for (text, expected) in rows {
            assert_eq!(parse(text), expected, "{text}");
        }
    }

    #[test]
    fn controls_are_understood_in_every_language() {
        let rows = [
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
            ("Turn on the TV.", Control::TvOn),
            ("Switch the T.V. on", Control::TvOn),
            ("TV on", Control::TvOn),
            ("Schalte den Fernseher ein.", Control::TvOn),
            ("Mach den Fernseher an", Control::TvOn),
            ("Enciende la tele", Control::TvOn),
            ("Turn off the TV", Control::TvOff),
            ("Fernseher aus", Control::TvOff),
            ("Apaga la televisión", Control::TvOff),
            ("Switch to SemmelTV", Control::ShowSemmelTv),
            ("Switch to Semmel TV.", Control::ShowSemmelTv),
            ("SemmelTV input", Control::ShowSemmelTv),
            ("Semmel-TV Eingang", Control::ShowSemmelTv),
            ("Auf SemmelTV umschalten", Control::ShowSemmelTv),
            ("Schalte auf Semmel TV um", Control::ShowSemmelTv),
            ("Wechsel zu SemmelTV", Control::ShowSemmelTv),
            ("Switch the TV to SimilTV", Control::ShowSemmelTv),
            ("simil tv", Control::ShowSemmelTv),
            ("Simmel TV, please.", Control::ShowSemmelTv),
            ("Go to semel tv", Control::ShowSemmelTv),
            ("Sammel T.V.", Control::ShowSemmelTv),
            ("Samuel TV input", Control::ShowSemmelTv),
            ("Simple TV", Control::ShowSemmelTv),
            ("Turn on SemmelTV", Control::ShowSemmelTv),
            ("Stevie the TV, switch to SemmelTV.", Control::ShowSemmelTv),
        ];
        let library = library();
        for (text, expected) in rows {
            let understood = understand(text, &library, &Progress::default());
            assert_eq!(understood, Action::Control(expected), "{text}");
        }
    }

    #[test]
    fn a_control_word_among_other_words_is_not_a_control() {
        let library = library();
        for text in [
            "SemmelTV aus",
            "Switch SemmelTV off",
            "Play Outlander on SemmelTV",
            "Play Outlander",
            "Stop making sense",
            "the tv show Dark",
        ] {
            let understood = understand(text, &library, &Progress::default());
            assert!(!matches!(understood, Action::Control(_)), "{text}");
        }
    }

    #[test]
    fn spoken_titles_resolve_to_files() {
        let library = library_from_paths(
            &[
                &PATHS[..],
                &[
                    "Series/Brooklyn Nine-Nine/Brooklyn.Nine-Nine.S01E01.mkv",
                    "Movies/Paddington 2 (2017)/Paddington.2.2017.1080p.mkv",
                ],
            ]
            .concat(),
        );
        let refuse = |message: &str| Action::Refuse(message.to_string());
        let play = |path: &str| Action::Play(path.into());
        let rows = [
            ("spiele Outlander Staffel 2 Folge 3", episode_path(2, 3)),
            ("play season 2 of Outlander", episode_path(2, 1)),
            ("play Outlander season 9", refuse("Outlander has no season 9")),
            (
                "play Outlander season 1 episode 9",
                refuse("Outlander has no season 1 episode 9"),
            ),
            ("play Out Lander", episode_path(1, 1)),
            ("play outlanders", episode_path(1, 1)),
            ("play Outlender", episode_path(1, 1)),
            ("play Autlander", episode_path(1, 1)),
            (
                "play Andor",
                play("/lib/Series/Star Wars Andor/Star.Wars.Andor.S01E01.mkv"),
            ),
            (
                "play Star Wars",
                refuse("Did you mean Star Wars Ahsoka or Star Wars Andor?"),
            ),
            ("spiele Dune", play("/lib/Movies/Dune (2021)/Dune.2021.1080p.mkv")),
            ("play Severance", refuse("\"severance\" is not in the library")),
            (
                "play brooklyn 9 9",
                play("/lib/Series/Brooklyn Nine-Nine/Brooklyn.Nine-Nine.S01E01.mkv"),
            ),
            (
                "play paddington two",
                play("/lib/Movies/Paddington 2 (2017)/Paddington.2.2017.1080p.mkv"),
            ),
        ];
        for (text, expected) in rows {
            assert_eq!(understand(text, &library, &Progress::default()), expected, "{text}");
        }
    }

    #[test]
    fn continuing_follows_progress() {
        let library = library();
        let rows = [
            ("continue Outlander", Progress::default(), episode_path(1, 1)),
            ("continue Outlander", progress(2, 2, false), episode_path(2, 2)),
            ("weiter mit Outlander", progress(1, 2, true), episode_path(2, 1)),
            (
                "continue Outlander",
                progress(2, 3, true),
                Action::Refuse("You have finished Outlander".to_string()),
            ),
            ("continue Outlander season 2", progress(1, 1, false), episode_path(2, 1)),
            (
                "play the next season of Outlander",
                progress(1, 2, true),
                episode_path(2, 1),
            ),
            (
                "play the next season of Outlander",
                progress(2, 1, false),
                Action::Refuse("Outlander has no season 3".to_string()),
            ),
            ("Outlander", progress(2, 2, false), episode_path(2, 2)),
        ];
        for (text, progress, expected) in rows {
            let position = progress.shows.get("outlander");
            assert_eq!(
                understand(text, &library, &progress),
                expected,
                "{text} at {position:?}"
            );
        }
    }

    #[test]
    fn download_requests_resolve() {
        let library = library();
        let download = |title: &str, season, episode| Action::Download {
            title: title.to_string(),
            season: Some(season),
            episode,
        };
        let rows = [
            ("download the next season of Outlander", download("Outlander", 3, None)),
            ("download Severance season 2", download("severance", 2, None)),
            ("download Outlandish season 1", download("outlandish", 1, None)),
            (
                "download outlander season 8 episode 3",
                download("Outlander", 8, Some(3)),
            ),
        ];
        for (text, expected) in rows {
            assert_eq!(understand(text, &library, &Progress::default()), expected, "{text}");
        }
    }

    #[test]
    fn typed_downloads_keep_every_word_of_the_title() {
        let library = library();
        let download = |title: &str, season, episode| Action::Download {
            title: title.to_string(),
            season,
            episode,
        };
        let rows = [
            ("Would I Lie to You?", download("would i lie to you", None, None)),
            (
                "would i lie to you season 2",
                download("would i lie to you", Some(2), None),
            ),
            ("Get Out", download("get out", None, None)),
            ("The Bear", download("The Bear", None, None)),
            ("Outlander next season", download("Outlander", Some(3), None)),
            ("Severance S02E03", download("severance", Some(2), Some(3))),
        ];
        for (text, expected) in rows {
            assert_eq!(understand_typed_download(text, &library), expected, "{text}");
        }
        assert!(matches!(understand_typed_download("  ", &library), Action::Refuse(_)));
    }
}
