mod actor;
mod library;
pub mod sound;
mod tending;

use crate::audio::{self, Audio};
use crate::bus::{Bus, Event, Home, NowPlaying};
use crate::cache::Cache;
use crate::command::Command;
use crate::db::Db;
use crate::downloads::remote::Downloads;
use crate::intent::{Control, Pick};
use crate::led::Led;
use crate::library::Library;
use crate::offline::Store;
use crate::profile::ProfileId;
use crate::progress::Progress;
use crate::push::Push;
use crate::screen;
use crate::seek::Direction;
use crate::sidecar;
use crate::storage;
use crate::stream;
use crate::subtitles::{self, Subtitles};
use crate::tv::{self, Television};
use crate::voice::{self, Heard};
use crate::watching;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot};

const ANSWER_TIME: Duration = Duration::from_secs(5);
const TV_PROFILE: &str = "tv_profile";
const TV_TOKEN: &str = "tv_token";

#[derive(Debug)]
pub enum Request {
    Understand(String, ProfileId),
    Play(Pick, ProfileId),
    PlayFile(PathBuf),
    Key(Command),
    Control(Control),
    PlayPause,
    Seek(Seek),
    HideSkip,
    Speed(f64),
    Listen,
    Subtitles(subtitles::Change),
    Storage(storage::Change),
    Dismiss(watching::Kind, String, ProfileId),
    Mark(watching::Kind, String, Option<(u32, u32)>, watching::Mark, ProfileId),
    CatchUp { show: String, to: ProfileId, by: ProfileId },
    TogetherOnTv(String),
    WatchWithOnTv { show: String, with: ProfileId },
    Phone(stream::Report),
    Screen(ScreenCommand, ProfileId),
    TvProfile(ProfileId),
    Dialogue(ProfileId),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ScreenCommand {
    Flip {
        to: ScreenKind,
        device: screen::Device,
        position: Option<f64>,
    },
    Play {
        device: screen::Device,
        pick: Pick,
        takes_over: bool,
    },
    Resume {
        device: screen::Device,
        pick: Pick,
        position: f64,
    },
    Stop {
        device: screen::Device,
    },
    Failed {
        device: screen::Device,
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenKind {
    Tv,
    Phone,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seek {
    To(f64),
    By(f64),
    Step(Direction),
    Segment,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Outcome {
    pub message: String,
    pub ok: bool,
}

impl Outcome {
    pub(crate) fn done(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ok: true,
        }
    }

    pub(crate) fn refused(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ok: false,
        }
    }
}

pub(crate) enum Signal {
    Heard(Heard),
    Tv(crate::link::messages::FromTv),
    Poll { online: bool },
    Library(Result<Library, String>),
    Downloading(Option<PathBuf>),
    Download(actor::Found),
    TvWaking,
    TvShowing,
    TvSilent,
    Audio(audio::Route),
    Tended(tending::Tended),
}

pub(crate) enum Input {
    Request(Request, Option<oneshot::Sender<Outcome>>),
    Signal(Signal),
}

pub struct Setup {
    pub root: PathBuf,
    pub meta: PathBuf,
    pub remote: Option<String>,
    pub store: Store,
    pub storage: Option<storage::Cache>,
    pub cache: Cache,
    pub db: Db,
    pub speech: Option<String>,
    pub wake_word: Option<voice::WakeWord>,
    pub microphone: Option<String>,
    pub led: Option<Led>,
    pub tv_kind: Option<tv::Kind>,
    pub tv_renderer: Option<String>,
    pub handoff: Option<crate::player::renderer::Handoff>,
    pub downloads: Option<Downloads>,
    pub ready: Option<PathBuf>,
    pub headphones: bool,
    pub push_subject: String,
}

#[derive(Default)]
struct Published {
    library: RwLock<Arc<Library>>,
    progress: RwLock<HashMap<ProfileId, Arc<Progress>>>,
    tv_profile: RwLock<ProfileId>,
    storage: storage::Board,
    sidecars: RwLock<HashMap<PathBuf, usize>>,
    scans: AtomicU64,
    dialogue: RwLock<HashMap<ProfileId, Arc<AtomicBool>>>,
}

struct Parts {
    db: Db,
    subtitles: Subtitles,
    television: Television,
    audio: Option<Audio>,
    downloads: Option<Downloads>,
    ready: Option<(PathBuf, PathBuf)>,
    push: Push,
}

#[derive(Clone)]
pub struct Core {
    inbox: mpsc::UnboundedSender<Input>,
    bus: Bus,
    published: Arc<Published>,
    parts: Arc<Parts>,
}

impl Core {
    pub async fn start(setup: Setup) -> Self {
        let Setup {
            root,
            meta,
            remote,
            store,
            storage,
            cache,
            db,
            speech,
            wake_word,
            microphone,
            led,
            tv_kind,
            tv_renderer,
            handoff,
            downloads,
            ready,
            headphones,
            push_subject,
        } = setup;
        let saved = cache.saved_manifest();
        let library = match library::listed(&root, &saved)
            .await
            .filter(|library| !library.is_empty())
        {
            Some(library) => library,
            None => library::listed(&root, &meta.join(crate::manifest::FILE_NAME))
                .await
                .unwrap_or_default(),
        };
        db.set_library(&library);
        let profile = db
            .setting(TV_PROFILE)
            .await
            .unwrap_or_else(|error| {
                eprintln!("tv profile: {error}");
                None
            })
            .and_then(|stored| stored.parse().ok())
            .map(ProfileId)
            .unwrap_or(ProfileId::FIRST);
        let token = db.setting(TV_TOKEN).await.unwrap_or_else(|error| {
            eprintln!("database: {TV_TOKEN}: {error}");
            None
        });
        let runtime = tokio::runtime::Handle::current();
        let keeping = db.clone();
        let television = Television::new(tv::Settings {
            kind: tv_kind,
            renderer: tv_renderer,
            token: tv::Token::new(token, move |token| {
                runtime
                    .block_on(keeping.set_setting(TV_TOKEN, Some(token)))
                    .map_err(|error| error.to_string())
            }),
        });
        let bus = Bus::default();
        let (inbox, received) = mpsc::unbounded_channel();
        let audio = headphones.then(|| {
            let inbox = inbox.clone();
            Audio::spawn(
                bus.clone(),
                Box::new(move |route| {
                    let _ = inbox.send(Input::Signal(Signal::Audio(route)));
                }),
            )
        });
        let subtitles = Subtitles::load(&db, profile).await;
        let core = Self {
            inbox,
            bus,
            published: Arc::default(),
            parts: Arc::new(Parts {
                push: Push::new(db.clone(), push_subject),
                db,
                subtitles,
                television,
                audio,
                downloads,
                ready: ready.clone().map(|file| (root.clone(), file)),
            }),
        };
        core.set_library(Arc::new(library));
        *core.published.tv_profile.write().unwrap() = profile;
        let paths = library::Paths {
            root,
            meta,
            saved,
            remote,
            ready,
        };
        let hardware = actor::Hardware {
            store,
            storage,
            speech,
            wake_word,
            microphone,
            led,
            handoff,
            tv: actor::local_tv(&core),
        };
        let actor = actor::Actor::new(core.clone(), profile, hardware).await;
        let running = tokio::spawn(actor.run(received));
        tokio::spawn(async move {
            if let Err(error) = running.await {
                eprintln!("core: {error}");
                std::process::exit(1);
            }
        });
        library::watch(paths, core.clone());
        crate::cec::spawn(core.clone());
        core
    }

    #[cfg(test)]
    pub(crate) async fn detached(library: Library) -> (Self, mpsc::UnboundedReceiver<Input>) {
        let db = Db::memory().await;
        db.set_library(&library);
        let (inbox, received) = mpsc::unbounded_channel();
        let core = Self {
            inbox,
            bus: Bus::default(),
            published: Arc::default(),
            parts: Arc::new(Parts {
                push: Push::recorded(db.clone()),
                subtitles: Subtitles::load(&db, ProfileId::FIRST).await,
                db,
                television: Television::new(tv::Settings {
                    kind: None,
                    renderer: None,
                    token: tv::Token::memory(),
                }),
                audio: None,
                downloads: None,
                ready: None,
            }),
        };
        core.set_library(Arc::new(library));
        (core, received)
    }

    #[cfg(test)]
    pub(crate) fn answering(
        mut received: mpsc::UnboundedReceiver<Input>,
        answer: impl Fn(&Request) -> Outcome + Send + 'static,
    ) -> std::sync::mpsc::Receiver<String> {
        let (asked, seen) = std::sync::mpsc::channel();
        tokio::spawn(async move {
            while let Some(input) = received.recv().await {
                if let Input::Request(request, reply) = input {
                    let _ = asked.send(format!("{request:?}"));
                    if let Some(reply) = reply {
                        let _ = reply.send(answer(&request));
                    }
                }
            }
        });
        seen
    }

    pub fn send(&self, request: Request) {
        let _ = self.inbox.send(Input::Request(request, None));
    }

    pub async fn ask(&self, request: Request) -> Option<Outcome> {
        let (reply, answer) = oneshot::channel();
        self.inbox.send(Input::Request(request, Some(reply))).ok()?;
        tokio::time::timeout(ANSWER_TIME, answer).await.ok()?.ok()
    }

    pub(crate) fn signal(&self, signal: Signal) {
        let _ = self.inbox.send(Input::Signal(signal));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.bus.subscribe().1
    }

    pub fn bus(&self) -> &Bus {
        &self.bus
    }

    pub fn home(&self) -> Arc<Home> {
        self.bus.home_state()
    }

    pub fn now_playing(&self) -> Option<NowPlaying> {
        self.bus.now_playing()
    }

    pub fn screen(&self) -> screen::View {
        self.bus.screen_state()
    }

    pub fn library(&self) -> Arc<Library> {
        self.published.library.read().unwrap().clone()
    }

    pub fn playable(&self, path: &Path) -> PathBuf {
        match &self.parts.ready {
            Some((root, file)) => crate::ready::Ready::read(file).playable(root, path),
            None => path.to_path_buf(),
        }
    }

    pub async fn progress(&self, profile: ProfileId) -> Arc<Progress> {
        if let Some(progress) = self.published.progress.read().unwrap().get(&profile) {
            return progress.clone();
        }
        let loaded = self.parts.db.progress(profile).await.unwrap_or_else(|error| {
            eprintln!("progress of profile {}: {error}", profile.0);
            Progress::default()
        });
        let mut progress = self.published.progress.write().unwrap();
        progress.entry(profile).or_insert(Arc::new(loaded)).clone()
    }

    pub fn tv_profile(&self) -> ProfileId {
        *self.published.tv_profile.read().unwrap()
    }

    pub async fn profile_name(&self, profile: ProfileId) -> Option<String> {
        self.parts
            .db
            .profile(profile)
            .await
            .ok()
            .flatten()
            .map(|found| found.name)
    }

    pub fn storage(&self) -> Option<storage::Report> {
        self.published.storage.get()
    }

    pub fn subtitles(&self) -> &Subtitles {
        &self.parts.subtitles
    }

    pub fn television(&self) -> &Television {
        &self.parts.television
    }

    pub fn downloads(&self) -> Option<&Downloads> {
        self.parts.downloads.as_ref()
    }

    pub fn push(&self) -> &Push {
        &self.parts.push
    }

    pub fn subtitle_count(&self, video: &Path) -> usize {
        let sidecars = self.published.sidecars.read().unwrap().get(video).copied().unwrap_or(0);
        sidecars + self.parts.subtitles.embedded(video).unwrap_or(0)
    }

    pub fn recount(&self, video: &Path) {
        let count = sidecar::find(video).len();
        self.published
            .sidecars
            .write()
            .unwrap()
            .insert(video.to_path_buf(), count);
    }

    pub(crate) fn db(&self) -> &Db {
        &self.parts.db
    }

    fn audio(&self) -> Option<&Audio> {
        self.parts.audio.as_ref()
    }

    fn set_library(&self, library: Arc<Library>) {
        *self.published.library.write().unwrap() = library.clone();
        let scan = self.published.scans.fetch_add(1, Ordering::SeqCst) + 1;
        let published = self.published.clone();
        let spawned = thread::Builder::new().name("sidecars".to_string()).spawn(move || {
            let mut counts = HashMap::new();
            for path in library.paths() {
                if published.scans.load(Ordering::SeqCst) != scan {
                    return;
                }
                counts.insert(path.clone(), sidecar::find(path).len());
            }
            *published.sidecars.write().unwrap() = counts;
        });
        if let Err(error) = spawned {
            eprintln!("sidecars: {error}");
        }
    }

    fn forget_progress(&self) {
        self.published.progress.write().unwrap().clear();
    }

    fn set_progress(&self, profile: ProfileId, progress: Arc<Progress>) {
        self.published.progress.write().unwrap().insert(profile, progress);
    }

    fn set_tv_profile(&self, profile: ProfileId) {
        *self.published.tv_profile.write().unwrap() = profile;
    }

    #[cfg(test)]
    pub(crate) fn publish_progress(&self, profile: ProfileId, progress: Progress) {
        self.set_progress(profile, Arc::new(progress));
    }
}
