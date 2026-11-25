//! The tv's side of subtitles: the tracks mpv reports, what the subtitle key
//! and the phones change on them, and what the core should remember about
//! the file that plays.

use super::{Change, Offer, Playing, Remembered, Settings, Track, language_name, same_language, worth_offering};
use crate::link::messages::PlaySubtitles;
use crate::progress::Tracks;
use crate::sidecar::Sidecar;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const OBSERVED: [&str; 5] = ["track-list", "sid", "sub-visibility", "aid", "sub-delay"];
const NO_SUBTITLES: &str = "No subtitles in this file";
const OFF: &str = "Subtitles off";
const MOST_DELAY: f64 = 60.0;

impl Track {
    fn parse(track: &Value, kind: &str) -> Option<Self> {
        if track["type"] != kind {
            return None;
        }
        let id = track["id"].as_i64()?;
        let text = |key: &str| {
            track[key]
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        };
        let language = text("lang");
        let title = text("title");
        let flag = |key: &str| track[key].as_bool().unwrap_or(false);
        let titled = |word: &str| {
            title
                .as_deref()
                .is_some_and(|title| title.split_whitespace().any(|part| part.eq_ignore_ascii_case(word)))
        };
        let name = language.as_deref().and_then(language_name).map(str::to_string);
        let label = name
            .or_else(|| title.clone())
            .or_else(|| language.clone())
            .unwrap_or_else(|| format!("Track {id}"));
        let forced = flag("forced") || titled("forced");
        let hearing_impaired = flag("hearing-impaired") || titled("SDH");
        Some(Self {
            id,
            language,
            title,
            label,
            forced,
            hearing_impaired,
            external: flag("external"),
        })
    }
}

impl Track {
    fn offer(&self) -> Offer<'_> {
        Offer {
            language: self.language.as_deref(),
            title: self.title.as_deref(),
            forced: self.forced,
            hearing_impaired: self.hearing_impaired,
            usable: true,
        }
    }
}

impl Playing {
    /// Offers the tracks worth it to someone who reads `languages`, and the
    /// one that shows, whatever it is.
    fn offer(&mut self, languages: &[String]) {
        let offers: Vec<Offer> = self.every.iter().map(Track::offer).collect();
        let keep = worth_offering(&offers, languages);
        self.tracks = self
            .every
            .iter()
            .zip(keep)
            .filter(|(track, keep)| *keep || self.track == Some(track.id))
            .map(|(track, _)| track.clone())
            .collect();
    }

    fn label(&self, id: i64) -> String {
        self.tracks
            .iter()
            .find(|track| track.id == id)
            .map_or_else(|| format!("track {id}"), |track| track.label.clone())
    }

    fn preferred(&self, languages: &[String]) -> Option<i64> {
        let full = || self.tracks.iter().filter(|track| !track.forced);
        languages
            .iter()
            .find_map(|wanted| {
                let speaking = || {
                    full().filter(|track| {
                        track
                            .language
                            .as_deref()
                            .is_some_and(|code| same_language(code, wanted))
                    })
                };
                speaking()
                    .find(|track| !track.hearing_impaired)
                    .or_else(|| speaking().next())
            })
            .or_else(|| full().next())
            .or_else(|| self.tracks.first())
            .map(|track| track.id)
    }

    fn show(&self, id: i64) -> (Vec<Value>, String) {
        let mut commands = Vec::new();
        if self.track != Some(id) {
            commands.push(json!(["set_property", "sid", id]));
        }
        commands.push(json!(["set_property", "sub-visibility", true]));
        (commands, format!("Subtitles: {}", self.label(id)))
    }

    fn hide() -> (Vec<Value>, String) {
        (vec![json!(["set_property", "sub-visibility", false])], OFF.to_string())
    }

    fn toggle(&self, languages: &[String]) -> Answer {
        match (self.visible, self.track) {
            _ if self.tracks.is_empty() => Answer::refused(NO_SUBTITLES.to_string()),
            (true, Some(_)) => Answer::done(Self::hide()),
            (_, Some(id)) => Answer::done(self.show(id)),
            (_, None) => Answer::done(self.preferred(languages).map_or_else(Self::hide, |id| self.show(id))),
        }
    }

    fn choose_audio(&self, id: i64) -> Result<(Vec<Value>, String), String> {
        let track = self
            .audio_tracks
            .iter()
            .find(|track| track.id == id)
            .ok_or_else(|| format!("There is no audio track {id}"))?;
        Ok((
            vec![json!(["set_property", "aid", id])],
            format!("Audio: {}", track.label),
        ))
    }

    fn choose(&self, track: Option<i64>) -> Result<(Vec<Value>, String), String> {
        match track {
            None => Ok(Self::hide()),
            Some(id) if self.tracks.iter().any(|track| track.id == id) => Ok(self.show(id)),
            Some(id) => Err(format!("There is no subtitle track {id}")),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Observed {
    Tracks { subtitles: Vec<Track>, audio: Vec<Track> },
    Track(Option<i64>),
    Audio(Option<i64>),
    Visible(bool),
    Delay(f64),
}

impl Observed {
    pub fn parse(message: &Value) -> Option<Self> {
        let data = &message["data"];
        match message["name"].as_str()? {
            "track-list" => {
                let list = data.as_array()?;
                let of = |kind: &str| list.iter().filter_map(|track| Track::parse(track, kind)).collect();
                Some(Self::Tracks {
                    subtitles: of("sub"),
                    audio: of("audio"),
                })
            }
            "sid" => Some(Self::Track(data.as_i64())),
            "aid" => Some(Self::Audio(data.as_i64())),
            "sub-visibility" => data.as_bool().map(Self::Visible),
            "sub-delay" => data.as_f64().map(Self::Delay),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Answer {
    pub commands: Vec<Value>,
    pub message: String,
    pub ok: bool,
}

impl Answer {
    fn done((commands, message): (Vec<Value>, String)) -> Self {
        Self {
            commands,
            message,
            ok: true,
        }
    }

    pub fn refused(message: String) -> Self {
        Self {
            commands: Vec::new(),
            message,
            ok: false,
        }
    }
}

fn delay_label(delay: f64) -> String {
    if delay == 0.0 {
        "Subtitle delay off".to_string()
    } else {
        format!("Subtitle delay {delay:+.1} s")
    }
}

/// The subtitle and audio tracks of the file the tv plays. It starts from
/// what the core sent with `Play` and keeps what mpv reports.
#[derive(Debug, Default)]
pub struct SubtitleTracks {
    settings: Settings,
    file: Option<PathBuf>,
    remembered: Remembered,
    sidecars: Vec<Sidecar>,
    awaited: usize,
    playing: Option<Playing>,
    unsaid: Option<Remembered>,
}

impl SubtitleTracks {
    pub fn new(file: &Path, subtitles: PlaySubtitles) -> Self {
        let PlaySubtitles {
            settings,
            remembered,
            sidecars,
        } = subtitles;
        Self {
            settings,
            file: Some(file.to_path_buf()),
            remembered,
            awaited: sidecars.len(),
            sidecars,
            playing: Some(Playing::default()),
            unsaid: None,
        }
    }

    pub fn mpv_args(&self) -> Vec<String> {
        let mut args = self.settings.mpv_args();
        args.push(format!("--sub-delay={}", self.remembered.delay));
        args
    }

    pub fn playing(&self) -> Option<Playing> {
        self.playing.clone()
    }

    pub fn chosen(&self) -> (Option<i64>, Option<i64>) {
        self.playing.as_ref().map_or((None, None), |playing| {
            (playing.track.filter(|_| playing.visible), playing.audio)
        })
    }

    pub fn audio_to_restore(&self) -> Option<i64> {
        self.playing.as_ref().and_then(|playing| {
            playing
                .last_audio
                .or_else(|| playing.audio_tracks.first().map(|track| track.id))
        })
    }

    pub fn delay(&self) -> f64 {
        self.playing.as_ref().map_or(0.0, |playing| playing.delay)
    }

    pub fn tracks(&self) -> Option<Tracks> {
        self.playing.as_ref().map(|playing| Tracks {
            audio: playing.audio,
            subtitle: playing.track,
            subtitles_shown: Some(playing.visible),
        })
    }

    pub fn stopped(&mut self) {
        self.playing = None;
        self.file = None;
        self.sidecars.clear();
        self.awaited = 0;
    }

    /// What changed in the file's memory since it was last asked, for the
    /// core to keep.
    pub fn take_remembered(&mut self) -> Option<(PathBuf, Remembered)> {
        let remembered = self.unsaid.take()?;
        Some((self.file.clone()?, remembered))
    }

    fn remember(&mut self, change: impl FnOnce(&mut Remembered)) {
        if self.file.is_none() {
            return;
        }
        let before = self.remembered;
        change(&mut self.remembered);
        if self.remembered != before {
            self.unsaid = Some(self.remembered);
        }
    }

    pub fn loaded(&self) -> Vec<Value> {
        self.sidecars
            .iter()
            .map(|sidecar| {
                json!([
                    "sub-add",
                    sidecar.path,
                    "auto",
                    sidecar.title,
                    sidecar.language.as_deref().unwrap_or_default()
                ])
            })
            .collect()
    }

    pub fn observe(&mut self, observed: Observed) -> Vec<Value> {
        let mut commands = Vec::new();
        let mut embedded = None;
        let playing = self.playing.get_or_insert_with(Playing::default);
        match observed {
            Observed::Tracks { subtitles, audio } => {
                let external = subtitles.iter().filter(|track| track.external).count();
                embedded = Some(subtitles.len() - external);
                playing.every = subtitles;
                playing.audio_tracks = audio;
                playing.offer(&self.settings.languages);
                let settled = self.awaited > 0 && external >= self.awaited;
                if settled
                    && self.settings.shown
                    && let Some(id) = playing.preferred(&self.settings.languages)
                {
                    commands = playing.show(id).0;
                }
                if settled {
                    self.awaited = 0;
                }
            }
            Observed::Track(track) => {
                playing.track = track;
                playing.offer(&self.settings.languages);
            }
            Observed::Audio(track) => {
                playing.audio = track;
                playing.last_audio = track.or(playing.last_audio);
            }
            Observed::Visible(visible) => playing.visible = visible,
            Observed::Delay(delay) => playing.delay = delay,
        }
        if let Some(count) = embedded {
            self.remember(|remembered| remembered.embedded = Some(count));
        }
        commands
    }

    pub fn change(&mut self, change: Change) -> Answer {
        let playing = self.playing.clone().unwrap_or_default();
        match change {
            Change::Restyle(settings) => {
                let commands = settings
                    .style()
                    .into_iter()
                    .map(|(name, value)| json!(["set_property", name, value]))
                    .collect();
                if let Some(playing) = &mut self.playing {
                    playing.offer(&settings.languages);
                }
                self.settings = settings;
                Answer::done((commands, "Subtitle style changed".to_string()))
            }
            Change::Toggle => playing.toggle(&self.settings.languages),
            Change::Track(track) => playing.choose(track).map_or_else(Answer::refused, Answer::done),
            Change::Audio(track) => playing.choose_audio(track).map_or_else(Answer::refused, Answer::done),
            Change::Delay(delay) => {
                if self.file.is_none() {
                    return Answer::refused("Nothing is playing".to_string());
                }
                if !delay.is_finite() {
                    return Answer::refused("The delay must be a number of seconds".to_string());
                }
                let delay = ((delay * 10.0).round() / 10.0).clamp(-MOST_DELAY, MOST_DELAY);
                self.remember(|remembered| remembered.delay = delay);
                Answer::done((vec![json!(["set_property", "sub-delay", delay])], delay_label(delay)))
            }
            Change::Add { video, sidecar } => {
                if self.file.as_deref() != Some(video.as_path()) {
                    return Answer::refused("That is not what plays now".to_string());
                }
                self.sidecars.push(sidecar.clone());
                let commands = vec![
                    json!([
                        "sub-add",
                        sidecar.path,
                        "select",
                        sidecar.title,
                        sidecar.language.as_deref().unwrap_or_default()
                    ]),
                    json!(["set_property", "sub-visibility", true]),
                ];
                Answer::done((commands, format!("Subtitles: {}", sidecar.title)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(id: i64, lang: &str, title: &str) -> Value {
        json!({ "id": id, "type": "sub", "lang": lang, "title": title, "selected": false })
    }

    fn endless_summer() -> Value {
        json!([
            { "id": 1, "type": "video", "title": "The.Endless.Summer" },
            { "id": 1, "type": "audio", "lang": "en" },
            sub(1, "en", "English SRT"),
            { "id": 2, "type": "sub", "lang": "en", "title": "English SRT-SDH", "hearing-impaired": true },
            sub(3, "fr", "French SRT"),
        ])
    }

    fn track_list(data: Value) -> Observed {
        Observed::parse(&json!({ "name": "track-list", "data": data })).unwrap()
    }

    fn playing(tracks: Value, track: Option<i64>, visible: bool) -> Playing {
        let Observed::Tracks { subtitles, audio } = track_list(tracks) else {
            panic!("a track list");
        };
        Playing {
            visible,
            track,
            tracks: subtitles,
            audio_tracks: audio,
            ..Playing::default()
        }
    }

    fn up() -> SubtitleTracks {
        SubtitleTracks::new(Path::new("/lib/Movies/Up/Up.mkv"), PlaySubtitles::default())
    }

    fn english(path: &str) -> Sidecar {
        Sidecar {
            path: PathBuf::from(path),
            language: Some("en".to_string()),
            title: "English".to_string(),
            forced: false,
            hearing_impaired: false,
        }
    }

    #[test]
    fn only_subtitle_tracks_are_listed_and_named_after_their_language() {
        let playing = playing(endless_summer(), Some(1), true);
        let labels: Vec<_> = playing.tracks.iter().map(|track| track.label.as_str()).collect();
        assert_eq!(labels, ["English", "English", "French"]);
        assert!(playing.tracks[1].hearing_impaired);
        assert_eq!(playing.tracks[2].title.as_deref(), Some("French SRT"));
        let unnamed = Track::parse(&json!({ "id": 4, "type": "sub" }), "sub").unwrap();
        assert_eq!(unnamed.label, "Track 4");
        let titled = Track::parse(
            &json!({ "id": 5, "type": "sub", "lang": "xx", "title": "Commentary" }),
            "sub",
        )
        .unwrap();
        assert_eq!(titled.label, "Commentary");
        assert_eq!(
            Track::parse(&json!({ "id": 6, "type": "sub", "lang": "ger" }), "sub")
                .unwrap()
                .label,
            "German"
        );
    }

    #[test]
    fn region_tags_name_and_match_their_language() {
        let maul = json!([
            { "id": 1, "type": "sub", "lang": "en-US" },
            { "id": 2, "type": "sub", "lang": "en-US", "title": "SDH", "hearing-impaired": true },
            { "id": 3, "type": "sub", "lang": "zh-Hant-HK", "title": "Hong Kong" },
            { "id": 7, "type": "sub", "lang": "de-DE" },
            { "id": 10, "type": "sub", "lang": "es-419", "title": "Latin America" },
            { "id": 13, "type": "sub", "lang": "hu-HU" },
        ]);
        let playing = playing(maul, None, false);
        let labels: Vec<_> = playing.tracks.iter().map(|track| track.label.as_str()).collect();
        assert_eq!(
            labels,
            ["English", "English", "Chinese", "German", "Spanish", "Hungarian"]
        );
        assert_eq!(playing.preferred(&["de".to_string(), "en".to_string()]), Some(7));
        assert_eq!(playing.preferred(&["spa".to_string()]), Some(10));
        assert_eq!(playing.preferred(&["en".to_string()]), Some(1));
    }

    #[test]
    fn mpv_reports_the_track_and_whether_it_shows() {
        let sid = json!({ "event": "property-change", "name": "sid", "data": 2 });
        let off = json!({ "event": "property-change", "name": "sid", "data": false });
        let visible = json!({ "event": "property-change", "name": "sub-visibility", "data": false });
        assert_eq!(Observed::parse(&sid), Some(Observed::Track(Some(2))));
        assert_eq!(Observed::parse(&off), Some(Observed::Track(None)));
        assert_eq!(Observed::parse(&visible), Some(Observed::Visible(false)));
        assert_eq!(Observed::parse(&json!({ "name": "pause", "data": true })), None);
    }

    #[test]
    fn the_subtitle_key_hides_or_shows_the_selected_track() {
        let no_subtitles = json!([{ "id": 1, "type": "video" }, { "id": 1, "type": "audio" }]);
        let cases = [
            (
                "shown",
                playing(endless_summer(), Some(1), true),
                vec![],
                vec![json!(["set_property", "sub-visibility", false])],
                "Subtitles off",
                true,
            ),
            (
                "hidden",
                playing(endless_summer(), Some(3), false),
                vec!["en".to_string()],
                vec![json!(["set_property", "sub-visibility", true])],
                "Subtitles: French",
                true,
            ),
            (
                "no subtitles",
                playing(no_subtitles, None, true),
                vec!["en".to_string()],
                vec![],
                "No subtitles in this file",
                false,
            ),
        ];
        for (name, playing, languages, commands, message, ok) in cases {
            let answer = playing.toggle(&languages);
            assert_eq!(answer.commands, commands, "{name}");
            assert_eq!(answer.message, message, "{name}");
            assert_eq!(answer.ok, ok, "{name}");
        }
    }

    #[test]
    fn without_a_selected_track_the_key_picks_the_first_preferred_language() {
        let languages = ["de".to_string(), "fr".to_string(), "en".to_string()];
        let Answer { commands, message, .. } = playing(endless_summer(), None, false).toggle(&languages);
        assert_eq!(
            commands,
            [
                json!(["set_property", "sid", 3]),
                json!(["set_property", "sub-visibility", true])
            ]
        );
        assert_eq!(message, "Subtitles: French");
        let english = playing(endless_summer(), None, true).preferred(&["eng".to_string()]);
        assert_eq!(english, Some(1));
        let forced = playing(
            json!([{ "id": 1, "type": "sub", "forced": true }, sub(2, "it", "")]),
            None,
            false,
        );
        assert_eq!(forced.preferred(&["en".to_string()]), Some(2));
    }

    #[test]
    fn a_chosen_track_must_exist() {
        let playing = playing(endless_summer(), Some(1), false);
        assert_eq!(playing.choose(Some(2)).unwrap().1, "Subtitles: English");
        assert_eq!(playing.choose(None).unwrap().1, "Subtitles off");
        assert!(playing.choose(Some(9)).is_err());
    }

    #[test]
    fn audio_tracks_are_named_after_their_language_and_chosen_by_id() {
        let tracks = json!([
            { "id": 1, "type": "audio", "lang": "eng", "title": "Surround 5.1" },
            { "id": 2, "type": "audio", "lang": "ger" },
            sub(1, "en", "English SRT"),
        ]);
        let mut playing = playing(tracks, None, false);
        let labels: Vec<_> = playing.audio_tracks.iter().map(|track| track.label.as_str()).collect();
        assert_eq!(labels, ["English", "German"]);
        assert_eq!(playing.tracks.len(), 1);
        let aid = json!({ "event": "property-change", "name": "aid", "data": 2 });
        assert_eq!(Observed::parse(&aid), Some(Observed::Audio(Some(2))));
        playing.audio = Some(1);
        let (commands, message) = playing.choose_audio(2).unwrap();
        assert_eq!(commands, [json!(["set_property", "aid", 2])]);
        assert_eq!(message, "Audio: German");
        assert_eq!(playing.choose_audio(7), Err("There is no audio track 7".to_string()));
    }

    #[test]
    fn a_restyle_sets_every_style_property_and_keeps_the_new_settings() {
        let mut tracks = up();
        let bigger = Settings {
            size: 1.5,
            languages: vec!["fr".to_string()],
            ..Settings::default()
        };
        let commands = tracks.change(Change::Restyle(bigger.clone())).commands;
        assert_eq!(commands.len(), bigger.style().len());
        assert!(commands.contains(&json!(["set_property", "sub-scale", 1.5])));
        tracks.observe(track_list(endless_summer()));
        assert_eq!(tracks.change(Change::Toggle).message, "Subtitles: French");
    }

    #[test]
    fn only_tracks_worth_it_are_offered_but_every_one_is_counted() {
        let mut tracks = up();
        tracks.observe(track_list(json!([
            sub(1, "eng", ""),
            sub(2, "ara", ""),
            sub(3, "eng", "Commentary"),
            sub(4, "ger", ""),
            sub(5, "ger", ""),
            sub(6, "hun", ""),
        ])));
        let offered = |tracks: &SubtitleTracks| -> Vec<i64> {
            tracks.playing().unwrap().tracks.iter().map(|track| track.id).collect()
        };
        assert_eq!(offered(&tracks), [1, 4]);
        assert_eq!(tracks.take_remembered().unwrap().1.embedded, Some(6));
        tracks.observe(Observed::Track(Some(6)));
        assert_eq!(offered(&tracks), [1, 4, 6]);
        tracks.observe(Observed::Track(Some(1)));
        assert_eq!(offered(&tracks), [1, 4]);
        let hungarian = Settings {
            languages: vec!["hu".to_string()],
            ..Settings::default()
        };
        tracks.change(Change::Restyle(hungarian));
        assert_eq!(offered(&tracks), [1, 6]);
        assert_eq!(
            tracks.change(Change::Track(Some(2))).message,
            "There is no subtitle track 2"
        );
    }

    #[test]
    fn the_state_follows_what_mpv_reports_until_playback_stops() {
        let mut tracks = up();
        tracks.observe(track_list(endless_summer()));
        tracks.observe(Observed::Track(Some(1)));
        tracks.observe(Observed::Visible(true));
        assert_eq!(tracks.change(Change::Toggle).message, "Subtitles off");
        assert_eq!(tracks.chosen(), (Some(1), None));
        tracks.observe(Observed::Visible(false));
        tracks.observe(Observed::Audio(Some(1)));
        assert_eq!(tracks.chosen(), (None, Some(1)));
        tracks.stopped();
        assert_eq!(tracks.playing(), None);
    }

    #[test]
    fn library_subtitles_are_added_once_the_file_loads_and_shown_when_wanted() {
        let film = Path::new("/lib/Movies/Airplane 1980/Airplane.mp4");
        let mut tracks = SubtitleTracks::new(
            film,
            PlaySubtitles {
                settings: Settings {
                    shown: true,
                    ..Settings::default()
                },
                remembered: Remembered::default(),
                sidecars: vec![english("/lib/Movies/Airplane 1980/Subs/13_English.srt")],
            },
        );
        assert!(tracks.mpv_args().contains(&"--sub-auto=no".to_string()));
        assert_eq!(
            tracks.loaded(),
            [json!([
                "sub-add",
                "/lib/Movies/Airplane 1980/Subs/13_English.srt",
                "auto",
                "English",
                "en"
            ])]
        );
        let none = json!([{ "id": 1, "type": "audio", "lang": "en" }]);
        assert!(tracks.observe(track_list(none)).is_empty());
        let added = json!([
            { "id": 1, "type": "audio", "lang": "en" },
            { "id": 1, "type": "sub", "lang": "en", "title": "English", "external": true },
        ]);
        assert_eq!(
            tracks.observe(track_list(added.clone())),
            [
                json!(["set_property", "sid", 1]),
                json!(["set_property", "sub-visibility", true])
            ]
        );
        assert!(tracks.observe(track_list(added)).is_empty());
        assert_eq!(
            tracks.take_remembered(),
            Some((
                film.to_path_buf(),
                Remembered {
                    delay: 0.0,
                    embedded: Some(0)
                }
            ))
        );
    }

    #[test]
    fn the_delay_applies_live_and_is_handed_to_the_core_take_remembered() {
        let film = Path::new("/lib/Movies/Up/Up.mkv");
        let mut tracks = SubtitleTracks::new(
            film,
            PlaySubtitles {
                remembered: Remembered {
                    delay: 2.0,
                    embedded: None,
                },
                ..PlaySubtitles::default()
            },
        );
        assert!(tracks.mpv_args().contains(&"--sub-delay=2".to_string()));
        let answer = tracks.change(Change::Delay(-1.04));
        assert_eq!(answer.commands, [json!(["set_property", "sub-delay", -1.0])]);
        assert_eq!(answer.message, "Subtitle delay -1.0 s");
        tracks.observe(Observed::Delay(-1.0));
        assert_eq!(tracks.delay(), -1.0);
        let delayed = Remembered {
            delay: -1.0,
            embedded: None,
        };
        assert_eq!(tracks.take_remembered(), Some((film.to_path_buf(), delayed)));
        assert_eq!(tracks.take_remembered(), None, "said once");
        tracks.change(Change::Delay(-1.0));
        assert_eq!(tracks.take_remembered(), None, "an unchanged delay is not said again");
        assert_eq!(tracks.change(Change::Delay(0.0)).message, "Subtitle delay off");
        assert_eq!(
            tracks.change(Change::Delay(500.0)).commands,
            [json!(["set_property", "sub-delay", 60.0])]
        );
        tracks.stopped();
        assert!(!tracks.change(Change::Delay(0.5)).ok);
    }

    #[test]
    fn fetched_subtitles_are_added_only_to_the_file_that_plays() {
        let mut tracks = up();
        let added = tracks.change(Change::Add {
            video: PathBuf::from("/lib/Movies/Up/Up.mkv"),
            sidecar: english("/lib/Movies/Up/Up.en.srt"),
        });
        assert!(added.ok);
        assert_eq!(
            added.commands,
            [
                json!(["sub-add", "/lib/Movies/Up/Up.en.srt", "select", "English", "en"]),
                json!(["set_property", "sub-visibility", true]),
            ]
        );
        let elsewhere = tracks.change(Change::Add {
            video: PathBuf::from("/lib/Movies/Cars/Cars.mkv"),
            sidecar: english("/lib/Movies/Cars/Cars.en.srt"),
        });
        assert!(!elsewhere.ok);
    }
}
