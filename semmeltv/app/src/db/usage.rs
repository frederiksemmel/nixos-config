use super::models::{Activity as StoredActivity, Usage};
use super::{Db, Result};
use crate::profile::ProfileId;
use crate::usage::{Activity, Event};

fn seconds(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

impl Db {
    pub async fn note_usage(&self, at: u64, activity: Activity, profile: ProfileId, device: &str) -> Result<()> {
        let activity = match activity {
            Activity::SignIn => StoredActivity::SignIn,
            Activity::Visit => StoredActivity::Visit,
            Activity::Play => StoredActivity::Play,
            Activity::Control => StoredActivity::Control,
        };
        self.note(at, activity, profile, device, "").await
    }

    /// One thing someone did to the TV through the app, such as "Paused".
    pub async fn note_control(&self, at: u64, profile: ProfileId, device: &str, action: &str) -> Result<()> {
        self.note(at, StoredActivity::Control, profile, device, action).await
    }

    async fn note(
        &self,
        at: u64,
        activity: StoredActivity,
        profile: ProfileId,
        device: &str,
        title: &str,
    ) -> Result<()> {
        let mut db = self.db.clone();
        toasty::create!(Usage {
            at: seconds(at),
            activity,
            profile_id: profile.0,
            device: device.to_string(),
            title: title.to_string(),
            play: String::new(),
            seconds: 0,
        })
        .exec(&mut db)
        .await?;
        Ok(())
    }

    /// Keeps one row per play, found again by `play`, so a phone's reports and
    /// the TV's saves only move its watched time forward.
    pub async fn note_play(
        &self,
        play: &str,
        at: u64,
        profile: ProfileId,
        device: &str,
        title: &str,
        watched: f64,
    ) -> Result<()> {
        let watched = if watched.is_finite() {
            watched.max(0.0) as i64
        } else {
            0
        };
        let mut db = self.db.clone();
        let found = Usage::filter_by_play(play.to_string()).first().exec(&mut db).await?;
        match found {
            Some(row) if row.seconds >= watched => {}
            Some(_) => {
                toasty::update!(Usage::filter_by_play(play.to_string()) { seconds: watched })
                    .exec(&mut db)
                    .await?;
            }
            None => {
                toasty::create!(Usage {
                    at: seconds(at),
                    activity: StoredActivity::Play,
                    profile_id: profile.0,
                    device: device.to_string(),
                    title: title.to_string(),
                    play: play.to_string(),
                    seconds: watched,
                })
                .exec(&mut db)
                .await?;
            }
        }
        Ok(())
    }

    pub async fn usage_since(&self, since: u64) -> Result<Vec<Event>> {
        let mut db = self.db.clone();
        let rows = Usage::filter(Usage::fields().at().ge(seconds(since)))
            .order_by(Usage::fields().at().asc())
            .exec(&mut db)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| Event {
                at: unsigned(row.at),
                activity: match row.activity {
                    StoredActivity::SignIn => Activity::SignIn,
                    StoredActivity::Visit => Activity::Visit,
                    StoredActivity::Play => Activity::Play,
                    StoredActivity::Control => Activity::Control,
                },
                profile: ProfileId(row.profile_id),
                device: row.device,
                title: row.title,
                seconds: unsigned(row.seconds),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_play_keeps_one_row_whose_time_only_grows() {
        let db = Db::memory().await;
        let frederik = ProfileId(1);
        db.note_play("phone-a", 100, frederik, "iPhone", "Dark · S01E01", 0.0)
            .await
            .unwrap();
        db.note_play("phone-a", 160, frederik, "iPhone", "Dark · S01E01", 60.4)
            .await
            .unwrap();
        db.note_play("phone-a", 170, frederik, "iPhone", "Dark · S01E01", 12.0)
            .await
            .unwrap();
        db.note_play("tv-b", 200, frederik, "TV", "Arrival", 30.0)
            .await
            .unwrap();
        db.note_usage(50, Activity::SignIn, frederik, "iPhone").await.unwrap();
        db.note_control(300, frederik, "iPhone · Safari", "Paused")
            .await
            .unwrap();
        let events = db.usage_since(0).await.unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| (event.at, event.activity, event.title.as_str(), event.seconds))
                .collect::<Vec<_>>(),
            vec![
                (50, Activity::SignIn, "", 0),
                (100, Activity::Play, "Dark · S01E01", 60),
                (200, Activity::Play, "Arrival", 30),
                (300, Activity::Control, "Paused", 0),
            ]
        );
        assert_eq!(events[3].device, "iPhone · Safari");
        assert_eq!(db.usage_since(150).await.unwrap().len(), 2);
    }
}
