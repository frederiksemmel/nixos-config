use crate::gotcourts::{self, Busy, Court};
use std::fs;
use std::path::Path;
use std::time::Duration;
use toasty::Json;
use toasty_driver_sqlite::{JournalMode, Sqlite, Synchronous};

static MIGRATIONS: toasty::migration::MigrationSet = toasty::embed_migrations!("toasty");

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

mod models {
    use super::*;

    #[derive(Debug, Clone, toasty::Model)]
    pub struct Club {
        #[key]
        pub id: i64,
        pub name: String,
        pub canonical: String,
        pub address: String,
        pub zip: String,
        pub city: String,
        pub latitude: f64,
        pub longitude: f64,
        pub phone: String,
        pub web: String,
        pub kind: String,
        #[column(type = text)]
        pub courts: Json<Vec<Court>>,
        pub checked_at: i64,
    }

    /// What GotCourts said about one club on one date, the last time it was asked.
    #[derive(Debug, toasty::Model)]
    #[key(club_id, date)]
    pub struct Day {
        pub club_id: i64,
        pub date: String,
        pub fetched_at: i64,
        #[column(type = text)]
        pub courts: Json<Vec<Court>>,
        #[column(type = text)]
        pub busy: Json<Vec<Busy>>,
    }

    /// A club GotCourts doesn't book for, asked again after a day.
    #[derive(Debug, toasty::Model)]
    #[table = "closed_clubs"]
    pub struct Closed {
        #[key]
        pub club_id: i64,
        pub why: String,
        pub at: i64,
    }

    #[derive(Debug, toasty::Model)]
    pub struct Watch {
        #[key]
        #[auto]
        pub id: i64,
        pub club_id: Option<i64>,
        #[index]
        pub date: String,
        pub earliest: i64,
        pub latest: i64,
        pub indoor: bool,
        pub sport: Option<String>,
        pub created_at: i64,
    }

    /// One batch of comments sent from feedback mode, and what became of it.
    #[derive(Debug, toasty::Model)]
    #[table = "feedback"]
    pub struct Report {
        #[key]
        #[auto]
        pub id: i64,
        pub created_at: i64,
        pub page: String,
        #[column(type = text)]
        pub batch: Json<serde_json::Value>,
        #[index]
        pub state: String,
        pub outcome: Option<String>,
        pub answer: Option<String>,
        pub commit: Option<String>,
        pub marked_at: i64,
        #[column(type = text)]
        pub thread: Json<Vec<crate::feedback::Said>>,
    }

    /// A one-time link that signs a device in, kept as the SHA-256 of its token.
    #[derive(Debug, toasty::Model)]
    pub struct Invite {
        #[key]
        pub token_hash: String,
        pub created_at: i64,
        pub expires_at: i64,
        pub used_at: Option<i64>,
    }

    /// A free slot a watch already told about, so it is told once.
    #[derive(Debug, toasty::Model)]
    #[table = "told"]
    #[key(watch_id, court_id, start)]
    pub struct Told {
        pub watch_id: i64,
        pub court_id: i64,
        pub start: i64,
        pub date: String,
    }

    #[derive(Debug, toasty::Model)]
    pub struct Subscription {
        #[key]
        pub endpoint: String,
        pub p256dh: String,
        pub auth: String,
        pub created_at: i64,
    }

    #[derive(Debug, toasty::Model)]
    pub struct Setting {
        #[key]
        pub name: String,
        pub value: String,
    }

    pub fn all() -> toasty::ModelSet {
        toasty::models!(Club, Day, Closed, Watch, Told, Subscription, Setting, Report, Invite)
    }
}

use models::{Closed, Invite, Setting, Told};
pub use models::{Club, Day, Report, Subscription, Watch};

#[derive(Clone)]
pub struct Db {
    db: toasty::Db,
}

impl Db {
    pub async fn open(file: &Path) -> Result<Self> {
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        let driver = Sqlite::open(file)
            .journal_mode(JournalMode::Wal)
            .synchronous(Synchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        Self::ready(driver).await
    }

    #[cfg(test)]
    pub async fn memory() -> Self {
        Self::ready(Sqlite::in_memory()).await.unwrap()
    }

    async fn ready(driver: Sqlite) -> Result<Self> {
        let db = toasty::Db::builder().models(models::all()).build(driver).await?;
        MIGRATIONS.apply(&db).await?;
        Ok(Self { db })
    }

    pub async fn save_day(&self, day: &gotcourts::Day, date: &str, now: i64) -> Result<()> {
        let mut db = self.db.clone();
        let club = &day.club;
        Club::upsert_by_id(club.id)
            .name(club.name.clone())
            .canonical(club.canonical.clone())
            .address(club.address.clone())
            .zip(club.zip.clone())
            .city(club.city.clone())
            .latitude(club.latitude)
            .longitude(club.longitude)
            .phone(club.phone.clone())
            .web(club.web.clone())
            .kind(club.kind.clone())
            .courts(day.courts.clone())
            .checked_at(now)
            .exec(&mut db)
            .await?;
        Day::upsert_by_club_id_and_date(club.id, date.to_string())
            .fetched_at(now)
            .courts(day.courts.clone())
            .busy(day.busy.clone())
            .exec(&mut db)
            .await?;
        Closed::filter_by_club_id(club.id).delete().exec(&mut db).await?;
        Ok(())
    }

    pub async fn close(&self, club: i64, why: &str, now: i64) -> Result<()> {
        let mut db = self.db.clone();
        Closed::upsert_by_club_id(club)
            .why(why.to_string())
            .at(now)
            .exec(&mut db)
            .await?;
        Ok(())
    }

    /// When GotCourts last said it doesn't book for `club`.
    pub async fn closed_at(&self, club: i64) -> Result<Option<i64>> {
        let mut db = self.db.clone();
        let found = Closed::filter_by_club_id(club).first().exec(&mut db).await?;
        Ok(found.map(|closed| closed.at))
    }

    /// The clubs among `only` that GotCourts books for.
    pub async fn clubs(&self, only: &[i64]) -> Result<Vec<Club>> {
        let mut db = self.db.clone();
        let closed: Vec<i64> = Closed::all()
            .exec(&mut db)
            .await?
            .into_iter()
            .map(|c| c.club_id)
            .collect();
        let mut clubs: Vec<Club> = Club::all()
            .exec(&mut db)
            .await?
            .into_iter()
            .filter(|club| !closed.contains(&club.id) && only.contains(&club.id))
            .collect();
        clubs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(clubs)
    }

    pub async fn club(&self, id: i64) -> Result<Option<Club>> {
        let mut db = self.db.clone();
        Ok(Club::filter_by_id(id).first().exec(&mut db).await?)
    }

    pub async fn days_on(&self, date: &str) -> Result<Vec<Day>> {
        let mut db = self.db.clone();
        let fields = Day::fields();
        Ok(Day::filter(fields.date().eq(date.to_string())).exec(&mut db).await?)
    }

    pub async fn days_of(&self, club: i64) -> Result<Vec<Day>> {
        let mut db = self.db.clone();
        let fields = Day::fields();
        Ok(Day::filter(fields.club_id().eq(club)).exec(&mut db).await?)
    }

    pub async fn day(&self, club: i64, date: &str) -> Result<Option<Day>> {
        let mut db = self.db.clone();
        Ok(Day::filter_by_club_id_and_date(club, date.to_string())
            .first()
            .exec(&mut db)
            .await?)
    }

    pub async fn forget_before(&self, date: &str) -> Result<()> {
        let mut db = self.db.clone();
        let day = Day::fields();
        Day::filter(day.date().lt(date.to_string()))
            .delete()
            .exec(&mut db)
            .await?;
        let watch = Watch::fields();
        let old: Vec<i64> = Watch::filter(watch.date().lt(date.to_string()))
            .exec(&mut db)
            .await?
            .into_iter()
            .map(|w| w.id)
            .collect();
        for id in old {
            self.forget_watch(id).await?;
        }
        Ok(())
    }

    pub async fn watches(&self) -> Result<Vec<Watch>> {
        let mut db = self.db.clone();
        let mut watches = Watch::all().exec(&mut db).await?;
        watches.sort_by(|a, b| (&a.date, a.earliest).cmp(&(&b.date, b.earliest)));
        Ok(watches)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn add_watch(
        &self,
        club: Option<i64>,
        date: &str,
        earliest: i64,
        latest: i64,
        indoor: bool,
        sport: Option<String>,
        now: i64,
    ) -> Result<Watch> {
        let mut db = self.db.clone();
        Ok(toasty::create!(Watch {
            club_id: club,
            date: date.to_string(),
            earliest,
            latest,
            indoor,
            sport,
            created_at: now,
        })
        .exec(&mut db)
        .await?)
    }

    pub async fn forget_watch(&self, id: i64) -> Result<()> {
        let mut db = self.db.clone();
        Told::filter(Told::fields().watch_id().eq(id))
            .delete()
            .exec(&mut db)
            .await?;
        Watch::filter_by_id(id).delete().exec(&mut db).await?;
        Ok(())
    }

    /// The (court, start) pairs `watch` told about before.
    pub async fn told(&self, watch: i64) -> Result<Vec<(i64, i64)>> {
        let mut db = self.db.clone();
        Ok(Told::filter(Told::fields().watch_id().eq(watch))
            .exec(&mut db)
            .await?
            .into_iter()
            .map(|told| (told.court_id, told.start))
            .collect())
    }

    pub async fn tell(&self, watch: i64, date: &str, court: i64, start: i64) -> Result<()> {
        let mut db = self.db.clone();
        Told::upsert_by_watch_id_and_court_id_and_start(watch, court, start)
            .date(date.to_string())
            .exec(&mut db)
            .await?;
        Ok(())
    }

    /// Forgets that `watch` told about a slot that is taken again, so it
    /// tells once more when it frees up.
    pub async fn untell(&self, watch: i64, court: i64, start: i64) -> Result<()> {
        let mut db = self.db.clone();
        Told::filter_by_watch_id_and_court_id_and_start(watch, court, start)
            .delete()
            .exec(&mut db)
            .await?;
        Ok(())
    }

    pub async fn add_report(&self, page: &str, batch: serde_json::Value, now: i64) -> Result<Report> {
        let mut db = self.db.clone();
        Ok(toasty::create!(Report {
            created_at: now,
            page: page.to_string(),
            batch: batch,
            state: "new".to_string(),
            marked_at: now,
            thread: Vec::<crate::feedback::Said>::new(),
        })
        .exec(&mut db)
        .await?)
    }

    pub async fn reports(&self) -> Result<Vec<Report>> {
        let mut db = self.db.clone();
        let mut reports = Report::all().exec(&mut db).await?;
        reports.sort_by_key(|report| std::cmp::Reverse(report.id));
        Ok(reports)
    }

    pub async fn report(&self, id: i64) -> Result<Option<Report>> {
        let mut db = self.db.clone();
        Ok(Report::filter_by_id(id).first().exec(&mut db).await?)
    }

    pub async fn save_report(&self, report: Report) -> Result<()> {
        let mut db = self.db.clone();
        let mut found = report;
        let (state, outcome, answer, commit, marked_at, thread) = (
            found.state.clone(),
            found.outcome.clone(),
            found.answer.clone(),
            found.commit.clone(),
            found.marked_at,
            found.thread.0.clone(),
        );
        toasty::update!(found {
            state: state,
            outcome: outcome,
            answer: answer,
            commit: commit,
            marked_at: marked_at,
            thread: thread,
        })
        .exec(&mut db)
        .await?;
        Ok(())
    }

    pub async fn add_invite(&self, token_hash: &str, now: i64, expires_at: i64) -> Result<()> {
        let mut db = self.db.clone();
        toasty::create!(Invite {
            token_hash: token_hash.to_string(),
            created_at: now,
            expires_at,
        })
        .exec(&mut db)
        .await?;
        Ok(())
    }

    /// Uses up the invite with this hash: whether it was there, unused and in time.
    pub async fn redeem(&self, token_hash: &str, now: i64) -> Result<bool> {
        let mut db = self.db.clone();
        let mut transaction = db.transaction().await?;
        let found = Invite::filter_by_token_hash(token_hash.to_string())
            .first()
            .exec(&mut transaction)
            .await?;
        let Some(mut found) = found.filter(|invite| invite.used_at.is_none() && invite.expires_at > now) else {
            return Ok(false);
        };
        toasty::update!(found { used_at: Some(now) })
            .exec(&mut transaction)
            .await?;
        transaction.commit().await?;
        Ok(true)
    }

    pub async fn subscribe(&self, subscription: Subscription) -> Result<()> {
        let mut db = self.db.clone();
        Subscription::upsert_by_endpoint(subscription.endpoint)
            .p256dh(subscription.p256dh)
            .auth(subscription.auth)
            .created_at(subscription.created_at)
            .exec(&mut db)
            .await?;
        Ok(())
    }

    pub async fn unsubscribe(&self, endpoint: &str) -> Result<()> {
        let mut db = self.db.clone();
        Subscription::filter_by_endpoint(endpoint.to_string())
            .delete()
            .exec(&mut db)
            .await?;
        Ok(())
    }

    pub async fn subscriptions(&self) -> Result<Vec<Subscription>> {
        let mut db = self.db.clone();
        Ok(Subscription::all().exec(&mut db).await?)
    }

    pub async fn setting(&self, name: &str) -> Result<Option<String>> {
        let mut db = self.db.clone();
        let found = Setting::filter_by_name(name.to_string()).first().exec(&mut db).await?;
        Ok(found.map(|setting| setting.value))
    }

    pub async fn set_setting(&self, name: &str, value: &str) -> Result<()> {
        let mut db = self.db.clone();
        Setting::upsert_by_name(name.to_string())
            .value(value.to_string())
            .exec(&mut db)
            .await?;
        Ok(())
    }
}

pub async fn toasty_cli(file: &Path, arguments: Vec<String>) -> Result<()> {
    let config = toasty_cli::Config::new().migration(toasty_cli::MigrationConfig::new().path("toasty"));
    let db = toasty::Db::builder()
        .models(models::all())
        .build(Sqlite::open(file))
        .await?;
    toasty_cli::ToastyCli::with_config(db, config)
        .parse_from(std::iter::once("toasty".to_string()).chain(arguments))
        .await
        .map_err(|error| toasty::Error::from_args(format_args!("{error:#}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(busy: Vec<Busy>) -> gotcourts::Day {
        gotcourts::Day {
            club: gotcourts::Club {
                id: 20,
                name: "Lengg".into(),
                canonical: "lengg".into(),
                address: "Lengg 1".into(),
                zip: "8008".into(),
                city: "Zürich".into(),
                latitude: 47.35,
                longitude: 8.57,
                phone: String::new(),
                web: String::new(),
                kind: "public".into(),
            },
            courts: vec![],
            busy,
        }
    }

    #[tokio::test]
    async fn a_day_fetched_again_replaces_what_was_taken() {
        let db = Db::memory().await;
        let busy = Busy {
            court: Some(1),
            start: 3600,
            end: 7200,
            note: None,
        };
        db.save_day(&day(vec![busy.clone()]), "2026-12-01", 1).await.unwrap();
        db.save_day(&day(vec![]), "2026-12-01", 2).await.unwrap();
        let stored = db.day(20, "2026-12-01").await.unwrap().unwrap();
        assert!(stored.busy.0.is_empty());
        assert_eq!(stored.fetched_at, 2);
        assert_eq!(db.clubs(&[20]).await.unwrap().len(), 1);
        assert!(db.clubs(&[21]).await.unwrap().is_empty());
        db.close(20, "unknown club", 3).await.unwrap();
        assert!(db.clubs(&[20]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_report_keeps_its_batch_and_takes_a_status_and_thread() {
        let db = Db::memory().await;
        let batch = serde_json::json!({"page": "/", "items": [{"n": 1, "comment": "bigger"}]});
        let report = db.add_report("/", batch.clone(), 5).await.unwrap();
        assert_eq!(report.state, "new");
        let mut found = db.report(report.id).await.unwrap().unwrap();
        found.state = "done".into();
        found.outcome = Some("fixed".into());
        found.thread.0.push(crate::feedback::Said {
            at: 6,
            by: "claude".into(),
            text: "done".into(),
        });
        db.save_report(found).await.unwrap();
        let again = db.report(report.id).await.unwrap().unwrap();
        assert_eq!(
            (again.state.as_str(), again.outcome.as_deref()),
            ("done", Some("fixed"))
        );
        assert_eq!(again.batch.0, batch);
        assert_eq!(again.thread.0.len(), 1);
    }

    #[tokio::test]
    async fn an_invite_works_once_and_only_in_time() {
        let db = Db::memory().await;
        db.add_invite("aa", 10, 100).await.unwrap();
        db.add_invite("bb", 10, 100).await.unwrap();
        assert!(db.redeem("aa", 50).await.unwrap());
        assert!(!db.redeem("aa", 51).await.unwrap());
        assert!(!db.redeem("bb", 100).await.unwrap());
        assert!(!db.redeem("cc", 50).await.unwrap());
    }

    #[tokio::test]
    async fn a_watch_tells_once_and_goes_with_its_date() {
        let db = Db::memory().await;
        let watch = db
            .add_watch(None, "2026-12-01", 0, 86400, false, None, 1)
            .await
            .unwrap();
        db.tell(watch.id, "2026-12-01", 5, 3600).await.unwrap();
        db.tell(watch.id, "2026-12-01", 5, 3600).await.unwrap();
        assert_eq!(db.told(watch.id).await.unwrap(), [(5, 3600)]);
        db.untell(watch.id, 5, 3600).await.unwrap();
        assert!(db.told(watch.id).await.unwrap().is_empty());
        db.forget_before("2026-12-02").await.unwrap();
        assert!(db.watches().await.unwrap().is_empty());
    }
}
