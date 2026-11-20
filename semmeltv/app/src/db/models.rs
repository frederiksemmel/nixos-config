use crate::subtitles;
use toasty::{Deferred, Json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, toasty::Embed)]
pub enum Role {
    Admin,
    Viewer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, toasty::Embed)]
pub enum Capability {
    ControlTv,
    /// No longer granted, since controlling the TV covers one's own
    /// headphones; Toasty can't drop an enum variant, so old rows stay and
    /// read as nothing.
    Headphones,
    Download,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, toasty::Embed)]
pub enum Kind {
    Series,
    Movie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, toasty::Embed)]
pub enum VideoKind {
    Episode,
    Movie,
}

#[derive(Debug, Clone, Copy, PartialEq, toasty::Embed)]
pub enum Viewing {
    NotStarted,
    InProgress {
        at: f64,
        audio_track: Option<i64>,
        subtitle_track: Option<i64>,
        subtitles_shown: Option<bool>,
    },
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq, toasty::Embed)]
pub enum Headphones {
    Unpaired,
    Paired { address: String, listening: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, toasty::Embed)]
pub enum Activity {
    SignIn,
    Visit,
    Play,
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
    pub grants: Deferred<Vec<Grant>>,
    #[has_many]
    pub passkeys: Deferred<Vec<Passkey>>,
    #[has_many]
    pub sessions: Deferred<Vec<Session>>,
    #[has_many(pair = profile)]
    pub invites: Deferred<Vec<Invite>>,
    #[has_many(pair = creator)]
    pub invites_sent: Deferred<Vec<Invite>>,
    #[has_many]
    pub video_places: Deferred<Vec<VideoPlace>>,
    #[has_many]
    pub series_places: Deferred<Vec<SeriesPlace>>,
    #[has_many]
    pub started: Deferred<Vec<Started>>,
    #[has_many]
    pub dismissals: Deferred<Vec<Dismissal>>,
    #[has_one]
    pub subtitle_settings: Deferred<Option<SubtitleSettings>>,
    #[has_one]
    pub audio_prefs: Deferred<Option<AudioPrefs>>,
}

#[derive(Debug, toasty::Model)]
#[table = "capabilities"]
#[key(profile_id, capability)]
pub struct Grant {
    pub profile_id: i64,
    pub capability: Capability,
    #[belongs_to(key = profile_id, references = id)]
    pub profile: Deferred<Profile>,
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
#[key(kind, video_id)]
pub struct VideoFact {
    pub kind: VideoKind,
    pub video_id: i64,
    pub duration: Option<f64>,
    pub credits: Option<f64>,
    #[default(0.0)]
    pub subtitle_delay: f64,
    pub embedded_subtitles: Option<i64>,
}

#[derive(Debug, toasty::Model)]
#[key(profile_id, kind, video_id)]
pub struct VideoPlace {
    pub profile_id: i64,
    pub kind: VideoKind,
    pub video_id: i64,
    #[belongs_to(key = profile_id, references = id)]
    pub profile: Deferred<Profile>,
    pub viewing: Viewing,
    pub last_played: Option<i64>,
}

#[derive(Debug, toasty::Model)]
#[key(profile_id, show_id)]
pub struct SeriesPlace {
    pub profile_id: i64,
    pub show_id: i64,
    #[belongs_to(key = profile_id, references = id)]
    pub profile: Deferred<Profile>,
    pub season: i64,
    pub episode: i64,
    pub finished: bool,
}

#[derive(Debug, toasty::Model)]
#[table = "started"]
#[key(profile_id, kind, title_id)]
pub struct Started {
    pub profile_id: i64,
    pub kind: Kind,
    pub title_id: i64,
    #[belongs_to(key = profile_id, references = id)]
    pub profile: Deferred<Profile>,
    pub last_played: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "dismissed_videos"]
#[key(profile_id, kind, video_id)]
pub struct Dismissal {
    pub profile_id: i64,
    pub kind: VideoKind,
    pub video_id: i64,
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
    pub headphones: Headphones,
    pub dialogue: bool,
}

#[derive(Debug, toasty::Model)]
pub struct Setting {
    #[key]
    pub name: String,
    pub value: String,
}

#[derive(Debug, toasty::Model)]
#[table = "usage"]
pub struct Usage {
    #[key]
    #[auto]
    pub id: i64,
    #[index]
    pub at: i64,
    pub activity: Activity,
    pub profile_id: i64,
    pub device: String,
    pub title: String,
    #[index]
    pub play: String,
    pub seconds: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "watch_together"]
#[key(profile_id, show_id, partner_id)]
pub struct Together {
    pub profile_id: i64,
    pub show_id: i64,
    pub partner_id: i64,
}

#[derive(Debug, toasty::Model)]
#[table = "push_subscriptions"]
pub struct PushSubscription {
    #[key]
    #[auto]
    pub id: i64,
    #[index]
    pub profile_id: i64,
    #[unique]
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub created_at: i64,
}

pub fn all() -> toasty::ModelSet {
    toasty::models!(
        Profile,
        Grant,
        Passkey,
        Session,
        Invite,
        VideoFact,
        VideoPlace,
        SeriesPlace,
        Started,
        Dismissal,
        SubtitleSettings,
        AudioPrefs,
        Setting,
        Usage,
        Together,
        PushSubscription
    )
}
