//! Notifications on people's phones through Web Push: the browser's push
//! service hands them to the SemmelTV web app on the Home Screen, even while
//! it is closed. The TV signs its requests with a key of its own (VAPID,
//! RFC 8292), made on first use and kept in the `settings` table, and
//! encrypts each message for the one browser it goes to (RFC 8291).

use crate::auth::Role;
use crate::db::{Db, Subscription};
use crate::library::{Episode, Library, Show};
use crate::manifest::Date;
use crate::profile::ProfileId;
use base64::Engine;
use base64::alphabet::URL_SAFE;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{self, EcdsaKeyPair, KeyPair};
use ring::{aead, agreement, hkdf};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::OnceCell;

const KEY_SETTING: &str = "push_key";
const RECORD_SIZE: u32 = 4096;
const KEPT_BY_THE_PUSH_SERVICE: u64 = 12 * 60 * 60;
/// Someone who carries on with a show again within this time, an episode
/// later or after a pause, doesn't notify the others again.
const QUIET_AFTER_CARRYING_ON: u64 = 3 * 60 * 60;
/// Titles that come back after a glitch on the mount carry their old date.
const NEW_FOR_DAYS: u64 = 2;

const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &URL_SAFE,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Notice {
    pub title: String,
    pub body: String,
    pub url: String,
    pub tag: String,
}

struct Vapid {
    pair: EcdsaKeyPair,
    subject: String,
}

impl Vapid {
    fn new(pkcs8: &[u8], subject: &str) -> Result<Self, String> {
        let pair = EcdsaKeyPair::from_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8, &SystemRandom::new())
            .map_err(|error| format!("push key: {error}"))?;
        Ok(Self {
            pair,
            subject: subject.to_string(),
        })
    }

    fn generate() -> Result<Vec<u8>, String> {
        EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
            .map(|document| document.as_ref().to_vec())
            .map_err(|_| "push key: could not make one".to_string())
    }

    fn public_key(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.pair.public_key().as_ref())
    }

    fn authorization(&self, endpoint: &str, now: u64) -> Result<String, String> {
        let url = url::Url::parse(endpoint).map_err(|error| format!("{endpoint}: {error}"))?;
        let header = URL_SAFE_NO_PAD.encode(r#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = serde_json::json!({
            "aud": url.origin().ascii_serialization(),
            "exp": now + KEPT_BY_THE_PUSH_SERVICE,
            "sub": self.subject,
        });
        let claims = URL_SAFE_NO_PAD.encode(claims.to_string());
        let signed = format!("{header}.{claims}");
        let signature = self
            .pair
            .sign(&SystemRandom::new(), signed.as_bytes())
            .map_err(|_| "push key: could not sign".to_string())?;
        Ok(format!(
            "vapid t={signed}.{}, k={}",
            URL_SAFE_NO_PAD.encode(signature.as_ref()),
            self.public_key()
        ))
    }
}

struct Length(usize);

impl hkdf::KeyType for Length {
    fn len(&self) -> usize {
        self.0
    }
}

fn expand(prk: &hkdf::Prk, info: &[u8], length: usize) -> Vec<u8> {
    let mut out = vec![0; length];
    let info = [info];
    prk.expand(&info, Length(length))
        .and_then(|okm| okm.fill(&mut out))
        .expect("HKDF-SHA256 gives up to 8160 bytes");
    out
}

/// The `aes128gcm` body RFC 8291 sends to one browser: a header with the salt
/// and the TV's one-off public key, then the message in a single record.
fn encrypt(plain: &[u8], to: &Subscription, ephemeral: &dyn SecureRandom, salt: [u8; 16]) -> Result<Vec<u8>, String> {
    let theirs = LENIENT.decode(&to.p256dh).map_err(|error| format!("p256dh: {error}"))?;
    let auth = LENIENT.decode(&to.auth).map_err(|error| format!("auth: {error}"))?;
    let private = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, ephemeral)
        .map_err(|_| "push: no key for this message".to_string())?;
    let ours = private
        .compute_public_key()
        .map_err(|_| "push: no key for this message".to_string())?
        .as_ref()
        .to_vec();
    let shared = agreement::agree_ephemeral(
        private,
        &agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, &theirs),
        <[u8]>::to_vec,
    )
    .map_err(|_| "push: the browser's key is not a P-256 point".to_string())?;
    let mut info = b"WebPush: info\0".to_vec();
    info.extend_from_slice(&theirs);
    info.extend_from_slice(&ours);
    let ikm = expand(&hkdf::Salt::new(hkdf::HKDF_SHA256, &auth).extract(&shared), &info, 32);
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &salt).extract(&ikm);
    let cek = expand(&prk, b"Content-Encoding: aes128gcm\0", 16);
    let nonce = expand(&prk, b"Content-Encoding: nonce\0", 12);
    let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &cek).expect("16 bytes"));
    let nonce = aead::Nonce::try_assume_unique_for_key(&nonce).expect("12 bytes");
    let mut record = plain.to_vec();
    record.push(2);
    if record.len() + aead::AES_128_GCM.tag_len() > RECORD_SIZE as usize {
        return Err("push: the message is too long".to_string());
    }
    key.seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut record)
        .map_err(|_| "push: could not encrypt".to_string())?;
    let mut body = salt.to_vec();
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(ours.len() as u8);
    body.extend_from_slice(&ours);
    body.extend_from_slice(&record);
    Ok(body)
}

enum Delivered {
    Sent,
    Gone,
    Failed(String),
}

#[derive(Clone)]
enum Courier {
    Http(ureq::Agent),
    #[cfg(test)]
    Recorded(Arc<Mutex<Vec<(String, Notice)>>>),
}

impl Courier {
    fn deliver(&self, vapid: &Vapid, to: &Subscription, notice: &Notice, now: u64) -> Delivered {
        match self {
            Self::Http(agent) => {
                let sent = (|| {
                    let mut salt = [0; 16];
                    let random = SystemRandom::new();
                    random.fill(&mut salt).map_err(|_| "push: no randomness".to_string())?;
                    let body = encrypt(
                        serde_json::to_string(notice).unwrap_or_default().as_bytes(),
                        to,
                        &random,
                        salt,
                    )?;
                    let authorization = vapid.authorization(&to.endpoint, now)?;
                    agent
                        .post(&to.endpoint)
                        .header("TTL", KEPT_BY_THE_PUSH_SERVICE.to_string())
                        .header("Urgency", "normal")
                        .header("Topic", topic(&notice.tag))
                        .header("Content-Encoding", "aes128gcm")
                        .header("Content-Type", "application/octet-stream")
                        .header("Authorization", authorization)
                        .send(&body[..])
                        .map_err(|error| match error {
                            ureq::Error::StatusCode(404 | 410) => "gone".to_string(),
                            error => format!("{}: {error}", host(&to.endpoint)),
                        })
                })();
                match sent {
                    Ok(_) => Delivered::Sent,
                    Err(error) if error == "gone" => Delivered::Gone,
                    Err(error) => Delivered::Failed(error),
                }
            }
            #[cfg(test)]
            Self::Recorded(sent) => {
                sent.lock().unwrap().push((to.endpoint.clone(), notice.clone()));
                Delivered::Sent
            }
        }
    }
}

/// A push service replaces a waiting message with a newer one of the same
/// topic: at most 32 URL-safe characters.
fn topic(tag: &str) -> String {
    tag.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(32)
        .collect()
}

fn host(endpoint: &str) -> String {
    url::Url::parse(endpoint)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| "push".to_string())
}

#[derive(Clone)]
pub struct Push {
    db: Db,
    subject: String,
    vapid: Arc<OnceCell<Arc<Vapid>>>,
    courier: Courier,
    carried_on: Arc<Mutex<HashMap<(ProfileId, String), u64>>>,
}

impl Push {
    /// `subject` is how a push service reaches whoever runs the TV: its
    /// address on the web.
    pub fn new(db: Db, subject: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(true)
            .build()
            .new_agent();
        Self::with(db, subject, Courier::Http(agent))
    }

    fn with(db: Db, subject: String, courier: Courier) -> Self {
        Self {
            db,
            subject,
            vapid: Arc::default(),
            courier,
            carried_on: Arc::default(),
        }
    }

    #[cfg(test)]
    pub fn recorded(db: Db) -> Self {
        Self::with(db, "https://tv.example".to_string(), Courier::Recorded(Arc::default()))
    }

    /// Who was sent what, by endpoint, when the pushes are only recorded.
    #[cfg(test)]
    pub fn sent(&self) -> Vec<(String, Notice)> {
        match &self.courier {
            Courier::Recorded(sent) => sent.lock().unwrap().clone(),
            Courier::Http(_) => Vec::new(),
        }
    }

    async fn vapid(&self) -> Result<Arc<Vapid>, String> {
        self.vapid
            .get_or_try_init(|| async {
                let kept = self
                    .db
                    .setting(KEY_SETTING)
                    .await
                    .map_err(|error| format!("push key: {error}"))?;
                let pkcs8 = match kept.and_then(|kept| LENIENT.decode(kept).ok()) {
                    Some(pkcs8) => pkcs8,
                    None => {
                        let made = Vapid::generate()?;
                        let encoded = URL_SAFE_NO_PAD.encode(&made);
                        self.db
                            .set_setting(KEY_SETTING, Some(&encoded))
                            .await
                            .map_err(|error| format!("push key: {error}"))?;
                        made
                    }
                };
                Vapid::new(&pkcs8, &self.subject).map(Arc::new)
            })
            .await
            .cloned()
    }

    /// The key a browser subscribes with, so it takes pushes only from this TV.
    pub async fn key(&self) -> Result<String, String> {
        Ok(self.vapid().await?.public_key())
    }

    /// Sends `notice` to every browser `profile` turned notifications on in,
    /// and forgets the ones their push service no longer knows.
    pub async fn notify(&self, profile: ProfileId, notice: &Notice) -> usize {
        let vapid = match self.vapid().await {
            Ok(vapid) => vapid,
            Err(error) => {
                eprintln!("{error}");
                return 0;
            }
        };
        let subscriptions = self.db.subscriptions(profile).await.unwrap_or_else(|error| {
            eprintln!("push: subscriptions of {}: {error}", profile.0);
            Vec::new()
        });
        let mut sent = 0;
        for subscription in subscriptions {
            let (courier, vapid, notice, to) = (
                self.courier.clone(),
                vapid.clone(),
                notice.clone(),
                subscription.clone(),
            );
            let now = crate::watching::now();
            let delivered = tokio::task::spawn_blocking(move || courier.deliver(&vapid, &to, &notice, now))
                .await
                .unwrap_or_else(|error| Delivered::Failed(error.to_string()));
            match delivered {
                Delivered::Sent => sent += 1,
                Delivered::Gone => {
                    eprintln!(
                        "push: {} forgot a subscription of {}",
                        host(&subscription.endpoint),
                        profile.0
                    );
                    if let Err(error) = self.db.unsubscribe(&subscription.endpoint).await {
                        eprintln!("push: {error}");
                    }
                }
                Delivered::Failed(error) => eprintln!("push to {}: {error}", profile.0),
            }
        }
        sent
    }

    /// Tells everyone who watches `show` together with `player` that they
    /// carry on with `episode`, at most once in a few hours per person and show.
    pub async fn carried_on(&self, player: ProfileId, show: &Show, episode: &Episode, now: u64) {
        let partners = match self.db.partners(player, &show.key).await {
            Ok(partners) if !partners.is_empty() => partners,
            Ok(_) => return,
            Err(error) => return eprintln!("push: partners in {}: {error}", show.key),
        };
        {
            let mut carried_on = self.carried_on.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let last = carried_on.entry((player, show.key.clone())).or_insert(0);
            if now < *last + QUIET_AFTER_CARRYING_ON {
                return;
            }
            *last = now;
        }
        let Ok(Some(name)) = self.db.profile(player).await.map(|found| found.map(|found| found.name)) else {
            return;
        };
        let body = match &episode.title {
            Some(title) => format!("{} · {title}", episode.short_label()),
            None => episode.short_label(),
        };
        let notice = Notice {
            title: format!("{name} carries on with {}", show.title),
            body,
            url: format!("/shows/{}", show.key),
            tag: format!("together-{}", show.key),
        };
        for partner in partners {
            self.notify(partner, &notice).await;
        }
    }

    /// Tells each admin what someone else added to the library.
    pub async fn added(&self, additions: Vec<Addition>) {
        if additions.is_empty() {
            return;
        }
        let admins: Vec<_> = match self.db.profiles().await {
            Ok(profiles) => profiles
                .into_iter()
                .filter(|profile| profile.role == Role::Admin)
                .collect(),
            Err(error) => return eprintln!("push: profiles: {error}"),
        };
        for addition in additions {
            for admin in admins.iter().filter(|admin| admin.name != addition.by) {
                self.notify(admin.id, &addition.notice).await;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Addition {
    pub by: String,
    pub notice: Notice,
}

/// What `now` has that `before` hadn't and that someone downloaded in the
/// last days: one addition per film, and one per show and person.
pub fn additions(before: &Library, now: &Library, today: u64) -> Vec<Addition> {
    if before.is_empty() {
        return Vec::new();
    }
    let since = Date::from_unix(today.saturating_sub(NEW_FOR_DAYS * 24 * 60 * 60));
    let films: HashSet<&str> = before.movies.iter().map(|movie| movie.key.as_str()).collect();
    let episodes: HashSet<(&str, u32, u32)> = before
        .shows
        .iter()
        .flat_map(|show| {
            show.episodes()
                .map(move |episode| (show.key.as_str(), episode.season, episode.number))
        })
        .collect();
    let mut found = Vec::new();
    for movie in &now.movies {
        let Some(by) = &movie.downloaded_by else { continue };
        if films.contains(movie.key.as_str()) || movie.added < since {
            continue;
        }
        let title = match movie.year {
            Some(year) => format!("{} ({year})", movie.title),
            None => movie.title.clone(),
        };
        found.push(Addition {
            by: by.clone(),
            notice: Notice {
                title: "New in the library".to_string(),
                body: format!("{by} added {title}"),
                url: format!("/movies/{}", movie.key),
                tag: format!("added-{}", movie.key),
            },
        });
    }
    for show in &now.shows {
        let mut by_person: BTreeMap<&str, Vec<&Episode>> = BTreeMap::new();
        for episode in show.episodes() {
            let Some(by) = &episode.downloaded_by else { continue };
            if episodes.contains(&(show.key.as_str(), episode.season, episode.number)) || episode.added < since {
                continue;
            }
            by_person.entry(by).or_default().push(episode);
        }
        for (by, added) in by_person {
            let what = match added.as_slice() {
                [episode] => format!("{} {}", show.title, episode.short_label()),
                more => format!("{} episodes of {}", more.len(), show.title),
            };
            found.push(Addition {
                by: by.to_string(),
                notice: Notice {
                    title: "New in the library".to_string(),
                    body: format!("{by} added {what}"),
                    url: format!("/shows/{}", show.key),
                    tag: format!("added-{}", show.key),
                },
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(text: &str) -> Vec<u8> {
        LENIENT.decode(text).unwrap()
    }

    /// What a browser does with a push: RFC 8291 from its side, with its own
    /// private key and the auth secret it shared when it subscribed.
    fn decrypt(body: &[u8], private: agreement::EphemeralPrivateKey, auth: &[u8], theirs: &[u8]) -> Vec<u8> {
        let (salt, rest) = body.split_at(16);
        assert_eq!(rest[..4], RECORD_SIZE.to_be_bytes());
        let length = usize::from(rest[4]);
        let (sender, record) = rest[5..].split_at(length);
        let shared = agreement::agree_ephemeral(
            private,
            &agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, sender),
            <[u8]>::to_vec,
        )
        .unwrap();
        let mut info = b"WebPush: info\0".to_vec();
        info.extend_from_slice(theirs);
        info.extend_from_slice(sender);
        let ikm = expand(&hkdf::Salt::new(hkdf::HKDF_SHA256, auth).extract(&shared), &info, 32);
        let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(&ikm);
        let key = aead::LessSafeKey::new(
            aead::UnboundKey::new(&aead::AES_128_GCM, &expand(&prk, b"Content-Encoding: aes128gcm\0", 16)).unwrap(),
        );
        let nonce = aead::Nonce::try_assume_unique_for_key(&expand(&prk, b"Content-Encoding: nonce\0", 12)).unwrap();
        let mut record = record.to_vec();
        let plain = key.open_in_place(nonce, aead::Aad::empty(), &mut record).unwrap();
        assert_eq!(plain.last(), Some(&2));
        plain[..plain.len() - 1].to_vec()
    }

    #[test]
    fn a_browser_reads_the_message_with_its_own_key() {
        let random = SystemRandom::new();
        let browser = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &random).unwrap();
        let theirs = browser.compute_public_key().unwrap().as_ref().to_vec();
        let mut shared = [0; 16];
        random.fill(&mut shared).unwrap();
        let to = Subscription {
            endpoint: "https://push.example.net/push/1".to_string(),
            p256dh: URL_SAFE_NO_PAD.encode(&theirs),
            auth: URL_SAFE_NO_PAD.encode(shared),
        };
        let mut salt = [0; 16];
        random.fill(&mut salt).unwrap();
        let body = encrypt(b"When I grow up, I want to be a watermelon", &to, &random, salt).unwrap();
        assert_eq!(body[..16], salt);
        assert_eq!(body.len(), 16 + 4 + 1 + 65 + 41 + 1 + 16);
        assert_eq!(
            decrypt(&body, browser, &shared, &theirs),
            b"When I grow up, I want to be a watermelon"
        );
    }

    #[test]
    fn the_vapid_header_carries_a_token_for_the_push_service_signed_with_the_tvs_key() {
        let vapid = Vapid::new(&Vapid::generate().unwrap(), "https://tv.example").unwrap();
        let header = vapid
            .authorization("https://web.push.apple.com/QK1h2mJ0", 1_000)
            .unwrap();
        let (token, key) = header
            .strip_prefix("vapid t=")
            .and_then(|rest| rest.split_once(", k="))
            .unwrap();
        assert_eq!(key, vapid.public_key());
        let parts: Vec<&str> = token.split('.').collect();
        let claims: serde_json::Value = serde_json::from_slice(&bytes(parts[1])).unwrap();
        assert_eq!(
            claims,
            serde_json::json!({ "aud": "https://web.push.apple.com", "exp": 1_000 + KEPT_BY_THE_PUSH_SERVICE, "sub": "https://tv.example" })
        );
        let public = signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, bytes(key));
        public
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &bytes(parts[2]))
            .unwrap();
    }

    #[tokio::test]
    async fn the_tv_makes_its_key_once_and_keeps_it() {
        let db = Db::memory().await;
        let push = Push::recorded(db.clone());
        let key = push.key().await.unwrap();
        assert_eq!(bytes(&key).len(), 65);
        let again = Push::recorded(db);
        assert_eq!(again.key().await.unwrap(), key);
    }

    fn library(paths: &[&str]) -> Library {
        crate::import::library_from_paths(paths)
    }

    fn downloaded(mut library: Library, by: &str, added: u64) -> Library {
        let date = Date::from_unix(added);
        for show in &mut library.shows {
            for season in &mut show.seasons {
                for episode in &mut season.episodes {
                    episode.downloaded_by = Some(by.to_string());
                    episode.added = date;
                }
            }
        }
        for movie in &mut library.movies {
            movie.downloaded_by = Some(by.to_string());
            movie.added = date;
        }
        library
    }

    const NOW: u64 = 1_795_000_000;

    #[test]
    fn what_someone_downloaded_is_an_addition_once_per_film_and_per_show() {
        let before = downloaded(
            library(&["Series/Dark/Dark.S01E01.mkv", "Movies/Up (2009)/Up.2009.mkv"]),
            "Frederik",
            NOW - 30 * 86_400,
        );
        let mut now = downloaded(
            library(&[
                "Series/Dark/Dark.S01E01.mkv",
                "Series/Dark/Dark.S01E02.mkv",
                "Series/Dark/Dark.S01E03.mkv",
                "Series/Lost/Lost.S01E01.mkv",
                "Movies/Up (2009)/Up.2009.mkv",
                "Movies/Arrival (2016)/Arrival.2016.mkv",
            ]),
            "Mami",
            NOW - 3_600,
        );
        let bodies = |additions: &[Addition]| -> Vec<String> {
            additions.iter().map(|addition| addition.notice.body.clone()).collect()
        };
        assert_eq!(
            bodies(&additions(&before, &now, NOW)),
            [
                "Mami added Arrival (2016)",
                "Mami added 2 episodes of Dark",
                "Mami added Lost S1 E1",
            ]
        );
        let found = additions(&before, &now, NOW);
        assert_eq!(found[0].by, "Mami");
        assert_eq!(found[0].notice.url, "/movies/arrival-2016");
        assert_eq!(found[1].notice.url, "/shows/dark");

        assert_eq!(additions(&Library::default(), &now, NOW), []);
        assert_eq!(additions(&before, &now, NOW + 3 * 86_400), []);
        for movie in &mut now.movies {
            movie.downloaded_by = None;
        }
        assert_eq!(bodies(&additions(&before, &now, NOW)).len(), 2);
    }

    async fn person(db: &Db, name: &str, role: Role) -> ProfileId {
        let found = db.profiles().await.unwrap();
        let found = found.iter().find(|profile| profile.name == name).unwrap();
        assert_eq!(found.role, role);
        found.id
    }

    fn phone(name: &str) -> Subscription {
        Subscription {
            endpoint: format!("https://push.example/{name}"),
            p256dh: "unused".to_string(),
            auth: "unused".to_string(),
        }
    }

    #[tokio::test]
    async fn everyone_watching_a_show_together_hears_when_one_carries_on_but_not_every_episode() {
        let db = Db::memory().await;
        let library = crate::db::test_library();
        db.set_library(&library);
        let show = &library.shows[0];
        let frederik = person(&db, "Frederik", Role::Admin).await;
        let mami = person(&db, "Mami", Role::Viewer).await;
        let miriam = person(&db, "Miriam", Role::Viewer).await;
        let ramu = person(&db, "Ramu", Role::Viewer).await;
        for (profile, name) in [
            (frederik, "frederik"),
            (mami, "mami"),
            (miriam, "miriam"),
            (ramu, "ramu"),
        ] {
            db.subscribe(profile, &phone(name), 1).await.unwrap();
        }
        db.set_together(frederik, mami, &show.key, true).await.unwrap();
        db.set_together(miriam, mami, &show.key, true).await.unwrap();
        let push = Push::recorded(db);
        let episode = show.episodes().next().unwrap();
        push.carried_on(mami, show, episode, NOW).await;
        push.carried_on(mami, show, episode, NOW + 3_000).await;
        let sent = push.sent();
        assert_eq!(
            sent.iter().map(|(to, _)| to.as_str()).collect::<Vec<_>>(),
            ["https://push.example/frederik", "https://push.example/miriam"]
        );
        assert_eq!(
            sent[0].1,
            Notice {
                title: format!("Mami carries on with {}", show.title),
                body: match &episode.title {
                    Some(title) => format!("{} · {title}", episode.short_label()),
                    None => episode.short_label(),
                },
                url: format!("/shows/{}", show.key),
                tag: format!("together-{}", show.key),
            }
        );
        push.carried_on(mami, show, episode, NOW + QUIET_AFTER_CARRYING_ON)
            .await;
        assert_eq!(push.sent().len(), 4);
    }

    #[tokio::test]
    async fn an_admin_hears_what_others_added_but_not_what_they_added_themselves() {
        let db = Db::memory().await;
        let frederik = person(&db, "Frederik", Role::Admin).await;
        let mami = person(&db, "Mami", Role::Viewer).await;
        db.subscribe(frederik, &phone("frederik"), 1).await.unwrap();
        db.subscribe(mami, &phone("mami"), 1).await.unwrap();
        let push = Push::recorded(db);
        let addition = |by: &str| Addition {
            by: by.to_string(),
            notice: Notice {
                title: "New in the library".to_string(),
                body: format!("{by} added Arrival (2016)"),
                url: "/movies/arrival-2016".to_string(),
                tag: "added-arrival-2016".to_string(),
            },
        };
        push.added(vec![addition("Mami"), addition("Frederik")]).await;
        let sent = push.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "https://push.example/frederik");
        assert_eq!(sent[0].1.body, "Mami added Arrival (2016)");
    }

    #[test]
    fn a_topic_is_at_most_32_url_safe_characters() {
        assert_eq!(
            topic("together-the-lord-of-the-rings-the-rings-of-power"),
            "together-the-lord-of-the-rings-t"
        );
        assert_eq!(topic("added-amélie"), "added-am-lie");
    }
}
