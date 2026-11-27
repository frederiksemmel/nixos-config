use crate::gotcourts::{self, Busy, Court};
use crate::tariffs::Week;
use std::collections::{BTreeMap, HashMap};
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
        /// Where its courts are booked: "gotcourts" or "eversports".
        pub source: String,
        pub name: String,
        /// GotCourts' map name or Eversports' slug.
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
        /// Whose watch it is; none for watches from before there were people,
        /// which are the first admin's.
        pub person_id: Option<i64>,
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
        /// Whom it signs in; none for links from before there were people.
        pub person_id: Option<i64>,
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
        pub person_id: Option<i64>,
        pub p256dh: String,
        pub auth: String,
        pub created_at: i64,
    }

    /// A court's price for each hour of each weekday, asked of GotCourts once a day.
    #[derive(Debug, toasty::Model)]
    pub struct Tariff {
        #[key]
        pub court_id: i64,
        #[index]
        pub club_id: i64,
        pub checked_at: i64,
        #[column(type = text)]
        pub week: Json<crate::tariffs::Week>,
    }

    /// Someone who may use Courts. An admin also sees Users and Feedback.
    #[derive(Debug, Clone, toasty::Model)]
    #[table = "people"]
    pub struct Person {
        #[key]
        #[auto]
        pub id: i64,
        pub name: String,
        pub admin: bool,
        pub created_at: i64,
    }

    /// A browser that is signed in as a person, kept as the SHA-256 of the
    /// token in its cookie.
    #[derive(Debug, Clone, toasty::Model)]
    pub struct Device {
        #[key]
        pub token_hash: String,
        #[index]
        pub person_id: i64,
        pub created_at: i64,
        pub seen_at: i64,
        /// "iPhone", "Mac", …: from the browser's user agent when it signed in.
        pub kind: String,
    }

    /// Free slots sent to others to choose from, and what they chose.
    #[derive(Debug, Clone, toasty::Model)]
    pub struct Ask {
        #[key]
        pub token: String,
        #[index]
        pub person_id: i64,
        pub created_at: i64,
        /// The last date among the options, after which the ask is forgotten.
        pub until: String,
        #[column(type = text)]
        pub options: Json<Vec<crate::share::Shared>>,
        #[column(type = text)]
        pub picks: Json<Vec<crate::ask::Pick>>,
    }

    #[derive(Debug, toasty::Model)]
    pub struct Setting {
        #[key]
        pub name: String,
        pub value: String,
    }

    pub fn all() -> toasty::ModelSet {
        toasty::models!(
            Club,
            Day,
            Closed,
            Watch,
            Told,
            Subscription,
            Setting,
            Report,
            Invite,
            Tariff,
            Person,
            Device,
            Ask
        )
    }
}

pub use models::{Ask, Club, Day, Device, Person, Report, Subscription, Watch};
use models::{Closed, Invite, Setting, Tariff, Told};

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
            .source(club.source.key().to_string())
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

    /// The clubs stored from `source`.
    pub async fn clubs_from(&self, source: crate::source::Source) -> Result<Vec<Club>> {
        let mut db = self.db.clone();
        Ok(Club::all()
            .exec(&mut db)
            .await?
            .into_iter()
            .filter(|club| club.source == source.key())
            .collect())
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
        let ask = Ask::fields();
        Ask::filter(ask.until().lt(date.to_string()))
            .delete()
            .exec(&mut db)
            .await?;
        Ok(())
    }

    pub async fn watches(&self) -> Result<Vec<Watch>> {
        let mut db = self.db.clone();
        let mut watches = Watch::all().exec(&mut db).await?;
        watches.sort_by(|a, b| (&a.date, a.earliest).cmp(&(&b.date, b.earliest)));
        Ok(watches)
    }

    pub async fn watches_of(&self, person: i64) -> Result<Vec<Watch>> {
        let mut watches = self.watches().await?;
        watches.retain(|watch| watch.person_id == Some(person));
        Ok(watches)
    }

    pub async fn watch(&self, id: i64) -> Result<Option<Watch>> {
        let mut db = self.db.clone();
        Ok(Watch::filter_by_id(id).first().exec(&mut db).await?)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn add_watch(
        &self,
        person: i64,
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
            person_id: Some(person),
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

    pub async fn add_invite(&self, token_hash: &str, person: i64, now: i64, expires_at: i64) -> Result<()> {
        let mut db = self.db.clone();
        toasty::create!(Invite {
            token_hash: token_hash.to_string(),
            person_id: Some(person),
            created_at: now,
            expires_at,
        })
        .exec(&mut db)
        .await?;
        Ok(())
    }

    /// Uses up the invite with this hash: whom it signs in, if it was there,
    /// unused and in time.
    pub async fn redeem(&self, token_hash: &str, now: i64) -> Result<Option<i64>> {
        let mut db = self.db.clone();
        let mut transaction = db.transaction().await?;
        let found = Invite::filter_by_token_hash(token_hash.to_string())
            .first()
            .exec(&mut transaction)
            .await?;
        let Some(mut found) = found.filter(|invite| invite.used_at.is_none() && invite.expires_at > now) else {
            return Ok(None);
        };
        let person = found.person_id;
        toasty::update!(found { used_at: Some(now) })
            .exec(&mut transaction)
            .await?;
        transaction.commit().await?;
        Ok(person)
    }

    pub async fn people(&self) -> Result<Vec<Person>> {
        let mut db = self.db.clone();
        let mut people = Person::all().exec(&mut db).await?;
        people.sort_by_key(|person| (!person.admin, person.name.to_lowercase(), person.id));
        Ok(people)
    }

    pub async fn person(&self, id: i64) -> Result<Option<Person>> {
        let mut db = self.db.clone();
        Ok(Person::filter_by_id(id).first().exec(&mut db).await?)
    }

    pub async fn add_person(&self, name: &str, admin: bool, now: i64) -> Result<Person> {
        let mut db = self.db.clone();
        Ok(toasty::create!(Person {
            name: name.to_string(),
            admin,
            created_at: now,
        })
        .exec(&mut db)
        .await?)
    }

    pub async fn set_admin(&self, id: i64, admin: bool) -> Result<()> {
        let mut db = self.db.clone();
        if let Some(mut person) = Person::filter_by_id(id).first().exec(&mut db).await? {
            toasty::update!(person { admin: admin }).exec(&mut db).await?;
        }
        Ok(())
    }

    /// The first admin, made as `name` when nobody is there yet. What was
    /// kept before there were people (watches, notifications, sign-in links)
    /// becomes theirs.
    pub async fn first_admin(&self, name: &str, now: i64) -> Result<Person> {
        let people = self.people().await?;
        let admin = match people.into_iter().filter(|p| p.admin).min_by_key(|p| p.id) {
            Some(admin) => admin,
            None => self.add_person(name, true, now).await?,
        };
        let mut db = self.db.clone();
        for mut watch in Watch::all().exec(&mut db).await? {
            if watch.person_id.is_none() {
                toasty::update!(watch {
                    person_id: Some(admin.id)
                })
                .exec(&mut db)
                .await?;
            }
        }
        for mut subscription in Subscription::all().exec(&mut db).await? {
            if subscription.person_id.is_none() {
                toasty::update!(subscription {
                    person_id: Some(admin.id)
                })
                .exec(&mut db)
                .await?;
            }
        }
        for mut invite in Invite::all().exec(&mut db).await? {
            if invite.person_id.is_none() {
                toasty::update!(invite {
                    person_id: Some(admin.id)
                })
                .exec(&mut db)
                .await?;
            }
        }
        Ok(admin)
    }

    /// Forgets a person with everything that is theirs.
    pub async fn remove_person(&self, id: i64) -> Result<()> {
        for watch in self.watches_of(id).await? {
            self.forget_watch(watch.id).await?;
        }
        for subscription in self.subscriptions_of(id).await? {
            self.unsubscribe(&subscription.endpoint).await?;
        }
        let mut db = self.db.clone();
        for invite in Invite::all().exec(&mut db).await? {
            if invite.person_id == Some(id) {
                Invite::filter_by_token_hash(invite.token_hash)
                    .delete()
                    .exec(&mut db)
                    .await?;
            }
        }
        self.sign_out(id).await?;
        Ask::filter(Ask::fields().person_id().eq(id))
            .delete()
            .exec(&mut db)
            .await?;
        Person::filter_by_id(id).delete().exec(&mut db).await?;
        Ok(())
    }

    pub async fn add_device(&self, token_hash: &str, person: i64, kind: &str, now: i64) -> Result<()> {
        let mut db = self.db.clone();
        toasty::create!(Device {
            token_hash: token_hash.to_string(),
            person_id: person,
            created_at: now,
            seen_at: now,
            kind: kind.to_string(),
        })
        .exec(&mut db)
        .await?;
        Ok(())
    }

    /// The device with this hash, marked as seen now (at most once an hour).
    pub async fn device(&self, token_hash: &str, now: i64) -> Result<Option<Device>> {
        let mut db = self.db.clone();
        let Some(mut device) = Device::filter_by_token_hash(token_hash.to_string())
            .first()
            .exec(&mut db)
            .await?
        else {
            return Ok(None);
        };
        if now - device.seen_at > 3600 {
            toasty::update!(device { seen_at: now }).exec(&mut db).await?;
        }
        Ok(Some(device))
    }

    pub async fn devices(&self) -> Result<Vec<Device>> {
        let mut db = self.db.clone();
        Ok(Device::all().exec(&mut db).await?)
    }

    /// Signs every device of a person out.
    pub async fn sign_out(&self, person: i64) -> Result<()> {
        let mut db = self.db.clone();
        Device::filter(Device::fields().person_id().eq(person))
            .delete()
            .exec(&mut db)
            .await?;
        Ok(())
    }

    pub async fn forget_device(&self, token_hash: &str) -> Result<()> {
        let mut db = self.db.clone();
        Device::filter_by_token_hash(token_hash.to_string())
            .delete()
            .exec(&mut db)
            .await?;
        Ok(())
    }

    pub async fn add_ask(&self, token: &str, person: i64, options: Vec<crate::share::Shared>, now: i64) -> Result<Ask> {
        let mut db = self.db.clone();
        let until = options.iter().map(|o| o.date.clone()).max().unwrap_or_default();
        Ok(toasty::create!(Ask {
            token: token.to_string(),
            person_id: person,
            created_at: now,
            until: until,
            options: options,
            picks: Vec::<crate::ask::Pick>::new(),
        })
        .exec(&mut db)
        .await?)
    }

    pub async fn ask(&self, token: &str) -> Result<Option<Ask>> {
        let mut db = self.db.clone();
        Ok(Ask::filter_by_token(token.to_string()).first().exec(&mut db).await?)
    }

    pub async fn asks_of(&self, person: i64) -> Result<Vec<Ask>> {
        let mut db = self.db.clone();
        let mut asks = Ask::filter(Ask::fields().person_id().eq(person)).exec(&mut db).await?;
        asks.sort_by_key(|ask| std::cmp::Reverse(ask.created_at));
        Ok(asks)
    }

    /// Adds `pick` to the ask, unless it has `most` already: whether it did.
    pub async fn add_pick(&self, token: &str, pick: crate::ask::Pick, most: usize) -> Result<bool> {
        let mut db = self.db.clone();
        let mut transaction = db.transaction().await?;
        let Some(mut ask) = Ask::filter_by_token(token.to_string())
            .first()
            .exec(&mut transaction)
            .await?
        else {
            return Ok(false);
        };
        if ask.picks.0.len() >= most {
            return Ok(false);
        }
        let mut picks = ask.picks.0.clone();
        picks.push(pick);
        toasty::update!(ask { picks: picks }).exec(&mut transaction).await?;
        transaction.commit().await?;
        Ok(true)
    }

    pub async fn subscribe(&self, subscription: Subscription) -> Result<()> {
        let mut db = self.db.clone();
        Subscription::upsert_by_endpoint(subscription.endpoint)
            .person_id(subscription.person_id)
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

    /// Every court's week of prices.
    pub async fn tariffs(&self) -> Result<HashMap<i64, Week>> {
        let mut db = self.db.clone();
        Ok(Tariff::all()
            .exec(&mut db)
            .await?
            .into_iter()
            .map(|tariff| (tariff.court_id, tariff.week.0))
            .collect())
    }

    /// When each club's tariffs were last asked: the oldest of its courts.
    pub async fn tariffs_checked(&self) -> Result<HashMap<i64, i64>> {
        let mut db = self.db.clone();
        let mut checked = HashMap::new();
        for tariff in Tariff::all().exec(&mut db).await? {
            let at = checked.entry(tariff.club_id).or_insert(tariff.checked_at);
            *at = (*at).min(tariff.checked_at);
        }
        Ok(checked)
    }

    /// A club's weeks of prices, in place of what was there.
    pub async fn save_tariffs(&self, club: i64, weeks: &BTreeMap<i64, Week>, now: i64) -> Result<()> {
        let mut db = self.db.clone();
        let mut transaction = db.transaction().await?;
        Tariff::filter(Tariff::fields().club_id().eq(club))
            .delete()
            .exec(&mut transaction)
            .await?;
        for (court, week) in weeks {
            Tariff::upsert_by_court_id(*court)
                .club_id(club)
                .checked_at(now)
                .week(week.clone())
                .exec(&mut transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn subscriptions_of(&self, person: i64) -> Result<Vec<Subscription>> {
        let mut found = self.subscriptions().await?;
        found.retain(|subscription| subscription.person_id == Some(person));
        Ok(found)
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
                source: crate::source::Source::GotCourts,
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
        let stored = db.clubs_from(crate::source::Source::GotCourts).await.unwrap();
        assert_eq!(stored[0].source, "gotcourts");
        assert!(
            db.clubs_from(crate::source::Source::Eversports)
                .await
                .unwrap()
                .is_empty()
        );
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
        db.add_invite("aa", 7, 10, 100).await.unwrap();
        db.add_invite("bb", 7, 10, 100).await.unwrap();
        assert_eq!(db.redeem("aa", 50).await.unwrap(), Some(7));
        assert_eq!(db.redeem("aa", 51).await.unwrap(), None);
        assert_eq!(db.redeem("bb", 100).await.unwrap(), None);
        assert_eq!(db.redeem("cc", 50).await.unwrap(), None);
    }

    fn subscription(endpoint: &str, person: Option<i64>) -> Subscription {
        Subscription {
            endpoint: endpoint.into(),
            person_id: person,
            p256dh: String::new(),
            auth: String::new(),
            created_at: 0,
        }
    }

    #[tokio::test]
    async fn the_first_admin_takes_over_what_was_kept_before_people() {
        let db = Db::memory().await;
        let mut handle = db.db.clone();
        toasty::create!(Watch {
            person_id: None,
            club_id: None,
            date: "2026-12-01".to_string(),
            earliest: 0,
            latest: 3600,
            indoor: false,
            sport: None,
            created_at: 1,
        })
        .exec(&mut handle)
        .await
        .unwrap();
        db.subscribe(subscription("https://push.example/old", None))
            .await
            .unwrap();
        let admin = db.first_admin("Frederik", 5).await.unwrap();
        assert!(admin.admin);
        assert_eq!(admin.name, "Frederik");
        assert_eq!(db.watches_of(admin.id).await.unwrap().len(), 1);
        assert_eq!(db.subscriptions_of(admin.id).await.unwrap().len(), 1);
        let again = db.first_admin("Someone else", 6).await.unwrap();
        assert_eq!(again.id, admin.id);
        assert_eq!(db.people().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_person_goes_with_their_devices_watches_and_asks() {
        let db = Db::memory().await;
        let admin = db.first_admin("Frederik", 1).await.unwrap();
        let anna = db.add_person("Anna", false, 2).await.unwrap();
        db.add_device("dev-a", anna.id, "iPhone", 3).await.unwrap();
        db.add_device("dev-f", admin.id, "Mac", 3).await.unwrap();
        assert_eq!(db.device("dev-a", 4).await.unwrap().unwrap().person_id, anna.id);
        assert_eq!(db.device("dev-a", 3000).await.unwrap().unwrap().seen_at, 3);
        assert_eq!(db.device("dev-a", 4000).await.unwrap().unwrap().seen_at, 4000);
        assert_eq!(db.device("dev-a", 4001).await.unwrap().unwrap().seen_at, 4000);
        db.add_watch(anna.id, None, "2026-12-01", 0, 3600, false, None, 5)
            .await
            .unwrap();
        db.add_watch(admin.id, None, "2026-12-01", 0, 3600, false, None, 5)
            .await
            .unwrap();
        db.subscribe(subscription("https://push.example/anna", Some(anna.id)))
            .await
            .unwrap();
        let slot = crate::share::Shared {
            club: 20,
            court: 1,
            date: "2026-12-01".into(),
            start: 3600,
            end: 7200,
        };
        db.add_ask("t-anna", anna.id, vec![slot.clone()], 6).await.unwrap();
        db.add_ask("t-admin", admin.id, vec![slot], 6).await.unwrap();
        assert_eq!(db.watches_of(anna.id).await.unwrap().len(), 1);
        db.remove_person(anna.id).await.unwrap();
        assert!(db.person(anna.id).await.unwrap().is_none());
        assert!(db.device("dev-a", 5000).await.unwrap().is_none());
        assert!(db.device("dev-f", 5000).await.unwrap().is_some());
        assert!(db.watches_of(anna.id).await.unwrap().is_empty());
        assert_eq!(db.watches().await.unwrap().len(), 1);
        assert!(db.subscriptions_of(anna.id).await.unwrap().is_empty());
        assert!(db.ask("t-anna").await.unwrap().is_none());
        assert!(db.ask("t-admin").await.unwrap().is_some());
        db.sign_out(admin.id).await.unwrap();
        assert!(db.device("dev-f", 5000).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn an_ask_takes_picks_up_to_a_limit_and_goes_after_its_last_day() {
        let db = Db::memory().await;
        let slot = |date: &str| crate::share::Shared {
            club: 20,
            court: 1,
            date: date.into(),
            start: 3600,
            end: 7200,
        };
        let ask = db
            .add_ask("tok", 1, vec![slot("2026-12-01"), slot("2026-12-03")], 6)
            .await
            .unwrap();
        assert_eq!(ask.until, "2026-12-03");
        let pick = |name: &str| crate::ask::Pick {
            name: name.into(),
            option: 1,
            at: 7,
        };
        assert!(db.add_pick("tok", pick("Anna"), 2).await.unwrap());
        assert!(db.add_pick("tok", pick("Ben"), 2).await.unwrap());
        assert!(!db.add_pick("tok", pick("Cleo"), 2).await.unwrap());
        assert!(!db.add_pick("nope", pick("Dan"), 2).await.unwrap());
        let kept = db.ask("tok").await.unwrap().unwrap();
        assert_eq!(kept.picks.0.len(), 2);
        assert_eq!(db.asks_of(1).await.unwrap().len(), 1);
        db.forget_before("2026-12-03").await.unwrap();
        assert!(db.ask("tok").await.unwrap().is_some());
        db.forget_before("2026-12-04").await.unwrap();
        assert!(db.ask("tok").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_club_s_tariffs_replace_its_old_ones() {
        let db = Db::memory().await;
        let mut week = Week(vec![vec![None; 24]; 7]);
        week.0[0][8] = Some(7500);
        let weeks = BTreeMap::from([(1, week.clone()), (2, week.clone())]);
        db.save_tariffs(20, &weeks, 5).await.unwrap();
        db.save_tariffs(21, &BTreeMap::from([(3, week.clone())]), 9)
            .await
            .unwrap();
        db.save_tariffs(20, &BTreeMap::from([(2, week.clone())]), 7)
            .await
            .unwrap();
        let stored = db.tariffs().await.unwrap();
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[&2], week);
        assert_eq!(db.tariffs_checked().await.unwrap()[&20], 7);
    }

    #[tokio::test]
    async fn a_watch_tells_once_and_goes_with_its_date() {
        let db = Db::memory().await;
        let watch = db
            .add_watch(1, None, "2026-12-01", 0, 86400, false, None, 1)
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
