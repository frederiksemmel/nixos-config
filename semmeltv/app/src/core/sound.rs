use super::{Core, Request};
use crate::audio::{self, Audio};
use crate::db::{self, Db};
use crate::profile::ProfileId;
use std::time::Duration;
use tokio::task::spawn_blocking;

const PAIRING_TIME: Duration = Duration::from_secs(120);
const PAIRING_POLL: Duration = Duration::from_millis(500);

pub enum Refusal {
    NoHeadphones,
    Busy,
    Failed(String),
}

pub type Heard<T> = Result<T, Refusal>;

pub(super) async fn claimed(db: &Db, except: Option<ProfileId>) -> Vec<String> {
    let profiles = db.profiles().await.unwrap_or_else(|error| {
        eprintln!("headphones: {error}");
        Vec::new()
    });
    let mut claimed = Vec::new();
    for profile in profiles.iter().filter(|profile| Some(profile.id) != except) {
        if let Ok(prefs) = db.audio_prefs(profile.id).await
            && let Some(headphones) = prefs.headphones()
        {
            claimed.push(headphones.to_string());
        }
    }
    claimed
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Heard<T> {
    spawn_blocking(work)
        .await
        .map_err(|error| Refusal::Failed(error.to_string()))
}

async fn keep_paired(audio: Audio, db: Db, profile: ProfileId) {
    let deadline = tokio::time::Instant::now() + PAIRING_TIME;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(PAIRING_POLL).await;
        let view = audio.view();
        match view.pairing.as_ref().map(|pairing| pairing.state) {
            Some(audio::Stage::Scanning | audio::Stage::Pairing) => continue,
            Some(audio::Stage::Paired) => {
                let Some(headphones) = view.headphones else { return };
                let saved = async {
                    let prefs = db.audio_prefs(profile).await?.pairing(headphones.address);
                    db.save_audio_prefs(profile, &prefs).await?;
                    audio.set_kept(claimed(&db, None).await);
                    Ok::<_, db::Error>(())
                };
                if let Err(error) = saved.await {
                    eprintln!("headphones: remembering the pair: {error}");
                }
                return;
            }
            _ => return,
        }
    }
}

impl Core {
    fn headphones(&self) -> Heard<Audio> {
        self.audio().cloned().ok_or(Refusal::NoHeadphones)
    }

    pub fn sound(&self) -> Heard<audio::View> {
        Ok(self.headphones()?.view())
    }

    pub async fn choose_output(&self, output: audio::Output) -> Heard<audio::View> {
        let audio = self.headphones()?;
        let view = blocking(move || audio.choose(output)).await?;
        let profile = self.tv_profile();
        let prefs = self.db().audio_prefs(profile).await.unwrap_or_default().choosing(match output {
            audio::Output::Tv => db::Output::Tv,
            audio::Output::Headphones => db::Output::Headphones,
        });
        if let Err(error) = self.db().save_audio_prefs(profile, &prefs).await {
            eprintln!("headphones: {error}");
        }
        Ok(view)
    }

    pub async fn pair_headphones(&self, profile: ProfileId) -> Heard<audio::View> {
        let audio = self.headphones()?;
        self.ask(Request::TvProfile(profile)).await;
        let pairing = audio.clone();
        let view = blocking(move || pairing.pair())
            .await?
            .map_err(|audio::Busy| Refusal::Busy)?;
        tokio::spawn(keep_paired(audio, self.db().clone(), profile));
        Ok(view)
    }

    pub async fn cancel_pairing(&self) -> Heard<audio::View> {
        let audio = self.headphones()?;
        blocking(move || audio.cancel_pairing()).await
    }

    pub async fn forget_headphones(&self, profile: ProfileId) -> Heard<audio::View> {
        let audio = self.headphones()?;
        self.ask(Request::TvProfile(profile)).await;
        let db = self.db();
        audio.set_kept(claimed(db, Some(profile)).await);
        let forgetting = audio.clone();
        let view = blocking(move || forgetting.forget()).await?;
        if let Err(error) = db.save_audio_prefs(profile, &db::AudioPrefs::Unpaired).await {
            eprintln!("headphones: {error}");
        }
        audio.set_kept(claimed(db, None).await);
        Ok(view)
    }

    pub async fn delay_sound(&self, by: i32) -> Heard<audio::View> {
        let audio = self.headphones()?;
        blocking(move || audio.delay_sound(by)).await?.map_err(Refusal::Failed)
    }

    pub async fn change_volume(&self, change: audio::VolumeChange) -> Heard<Option<Result<String, String>>> {
        match self.audio().cloned() {
            Some(audio) => blocking(move || audio.change_volume(change)).await,
            None => Ok(None),
        }
    }
}
