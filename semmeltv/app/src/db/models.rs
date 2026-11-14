use crate::subtitles;
use toasty::{Deferred, Json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, toasty::Embed)]
pub enum Role {
    Admin,
    Viewer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, toasty::Embed)]
pub enum Kind {
    Series,
    Movie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, toasty::Embed)]
pub enum Output {
    Tv,
    Headphones,
}

#[derive(Debug, toasty::Model)]
pub struct Profile {
    #[key]
    #[auto]
    pub id: i64,
    #[unique]
    pub name: String,
    pub role: Role,
    pub password_hash: Option<String>,
    pub color: Option<String>,
    pub created_at: i64,

    #[has_many]
    pub passkeys: Deferred<Vec<Passkey>>,
    #[has_many]
    pub sessions: Deferred<Vec<Session>>,
    #[has_many(pair = profile)]
    pub invites: Deferred<Vec<Invite>>,
    #[has_many(pair = creator)]
    pub invites_sent: Deferred<Vec<Invite>>,
    #[has_many]
    pub file_places: Deferred<Vec<FilePlace>>,
    #[has_many]
    pub show_places: Deferred<Vec<ShowPlace>>,
    #[has_many]
    pub watching: Deferred<Vec<Watching>>,
    #[has_many]
    pub dismissals: Deferred<Vec<Dismissal>>,
    #[has_one]
    pub subtitle_settings: Deferred<Option<SubtitleSettings>>,
    #[has_one]
    pub audio_prefs: Deferred<Option<AudioPrefs>>,
}

#[derive(Debug, toasty::Model)]
pub struct Passkey {
    #[key]
    #[auto]
    pub id: i64,
    #[index]
    pub profile_id: i64,
    #[belongs_to(key = profile_id, references = id)]
    pub profile: Deferred<Profile>,
    #[unique]
    pub credential_id: Vec<u8>,
    pub rp_id: String,
    pub name: String,
    pub passkey: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, toasty::Model)]
pub struct Session {
    #[key]
    pub token_hash: Vec<u8>,
    #[index]
    pub profile_id: i64,
    #[belongs_to(key = profile_id, references = id)]
    pub profile: Deferred<Profile>,
    pub created_at: i64,
    pub last_seen_at: i64,
    pub expires_at: i64,
    pub label: String,
}

#[derive(Debug, toasty::Model)]
pub struct Invite {
    #[key]
    pub token_hash: Vec<u8>,
    #[index]
    pub profile_id: i64,
    #[belongs_to(key = profile_id, references = id)]
    pub profile: Deferred<Profile>,
    #[index]
    pub created_by: Option<i64>,
    #[belongs_to(key = created_by, references = id)]
    pub creator: Deferred<Option<Profile>>,
    pub created_at: i64,
    pub expires_at: i64,
    pub used_at: Option<i64>,
}

#[derive(Debug, toasty::Model)]
pub struct File {
    #[key]
    pub path: String,
    pub duration: Option<f64>,
    pub credits: Option<f64>,
    #[default(0.0)]
    pub subtitle_delay: f64,
    pub embedded_subtitles: Option<i64>,
}

#[derive(Debug, toasty::Model)]
#[key(profile_id, path)]
pub struct FilePlace {
    pub profile_id: i64,
    pub path: String,
    #[belongs_to(key = profile_id, references = id)]
    pub profile: Deferred<Profile>,
    pub stopped_at: Option<f64>,
    pub last_played: Option<i64>,
    pub finished: bool,
    pub audio_track: Option<i64>,
    pub subtitle_track: Option<i64>,
    pub subtitles_shown: Option<bool>,
}

#[derive(Debug, toasty::Model)]
#[key(profile_id, show_key)]
pub struct ShowPlace {
    pub profile_id: i64,
    pub show_key: String,
    #[belongs_to(key = profile_id, references = id)]
    pub profile: Deferred<Profile>,
    pub season: i64,
    pub episode: i64,
    pub finished: bool,
}

#[derive(Debug, toasty::Model)]
#[table = "watching"]
#[key(profile_id, kind, title_key)]
pub struct Watching {
    pub profile_id: i64,
    pub kind: Kind,
    pub title_key: String,
    #[belongs_to(key = profile_id, references = id)]
    pub profile: Deferred<Profile>,
    pub last_played: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "dismissed"]
#[key(profile_id, path)]
pub struct Dismissal {
    pub profile_id: i64,
    pub path: String,
    #[belongs_to(key = profile_id, references = id)]
    pub profile: Deferred<Profile>,
    pub at: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "subtitle_settings"]
pub struct SubtitleSettings {
    #[key]
    pub profile_id: i64,
    #[belongs_to(key = profile_id, references = id)]
    pub profile: Deferred<Profile>,
    #[column(type = text)]
    pub settings: Json<subtitles::Settings>,
}

#[derive(Debug, toasty::Model)]
#[table = "audio_prefs"]
pub struct AudioPrefs {
    #[key]
    pub profile_id: i64,
    #[belongs_to(key = profile_id, references = id)]
    pub profile: Deferred<Profile>,
    pub output: Output,
    pub headphones_address: Option<String>,
}

#[derive(Debug, toasty::Model)]
pub struct Setting {
    #[key]
    pub name: String,
    pub value: String,
}

pub fn all() -> toasty::ModelSet {
    toasty::models!(
        Profile,
        Passkey,
        Session,
        Invite,
        File,
        FilePlace,
        ShowPlace,
        Watching,
        Dismissal,
        SubtitleSettings,
        AudioPrefs,
        Setting
    )
}
