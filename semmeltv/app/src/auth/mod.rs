pub mod gate;
pub mod http;
#[cfg(test)]
pub mod memory;
mod password;
pub mod store;

use crate::profile::ProfileId;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use store::{AuthStore, Invite, NewProfile, Profile, Session, StoreError, StoredPasskey};
pub use store::Role;
use webauthn_rs::prelude::{
    CreationChallengeResponse, Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, RequestChallengeResponse, Url, Uuid, Webauthn, WebauthnBuilder,
};

pub const SESSION_LIFETIME: u64 = 7 * 24 * 60 * 60;
pub const RENEW_AFTER: u64 = 60 * 60;
pub const INVITE_LIFETIME: u64 = 7 * 24 * 60 * 60;
pub const SHORTEST_PASSWORD: usize = 8;
pub const LONGEST_PASSWORD: usize = 200;
pub const MOST_SESSIONS_ON_A_DEVICE: usize = 6;
const CEREMONY_TIME: Duration = Duration::from_secs(5 * 60);
const MOST_CEREMONIES: usize = 64;
const FREE_FAILURES: u32 = 5;
const LONGEST_LOCKOUT: u64 = 60 * 60;
const LONGEST_NAME: usize = 40;
const USER_HANDLE: u64 = u64::from_be_bytes(*b"SEMMELTV");

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Person {
    pub id: ProfileId,
    pub name: String,
    pub role: Role,
    pub color: String,
}

impl Person {
    pub fn admin(&self) -> bool {
        self.role == Role::Admin
    }
}

impl From<&Profile> for Person {
    fn from(profile: &Profile) -> Self {
        Person {
            id: profile.id,
            name: profile.name.clone(),
            role: profile.role,
            color: profile.color.clone().unwrap_or_else(|| color_for(profile.id)),
        }
    }
}

const COLORS: [&str; 6] = ["#E5484D", "#3E63DD", "#30A46C", "#F76B15", "#8E4EC6", "#12A594"];

fn color_for(id: ProfileId) -> String {
    COLORS[id.0.rem_euclid(COLORS.len() as i64) as usize].to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    UnknownProfile,
    WrongPassword,
    Throttled(u64),
    NoPassword(String),
    NoPasskey(String),
    ShortPassword,
    LongPassword,
    PasskeyFailed,
    PasskeyTaken,
    CeremonyGone,
    UnknownOrigin,
    InviteUnknown,
    InviteUsed,
    InviteExpired,
    NameTaken,
    BadName,
    LastAdmin,
    Yourself,
    NotSignedIn,
    Store(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::UnknownProfile => write!(f, "There is no such profile"),
            Refusal::WrongPassword => write!(f, "That password is wrong"),
            Refusal::Throttled(seconds) => {
                let minutes = seconds.div_ceil(60);
                let unit = if minutes == 1 { "minute" } else { "minutes" };
                write!(f, "Too many wrong passwords. Try again in {minutes} {unit}")
            }
            Refusal::NoPassword(name) => write!(f, "{name} has no password yet"),
            Refusal::NoPasskey(name) => write!(f, "{name} has no passkey for this address yet"),
            Refusal::ShortPassword => write!(f, "A password needs at least {SHORTEST_PASSWORD} characters"),
            Refusal::LongPassword => write!(f, "A password can have at most {LONGEST_PASSWORD} characters"),
            Refusal::PasskeyFailed => write!(f, "The passkey didn't work"),
            Refusal::PasskeyTaken => write!(f, "This passkey belongs to someone else"),
            Refusal::CeremonyGone => write!(f, "That took too long. Try again"),
            Refusal::UnknownOrigin => write!(f, "Passkeys don't work on this address"),
            Refusal::InviteUnknown => write!(f, "This invite link is not valid"),
            Refusal::InviteUsed => write!(f, "This invite link was already used"),
            Refusal::InviteExpired => write!(f, "This invite link has expired"),
            Refusal::NameTaken => write!(f, "That name is taken"),
            Refusal::BadName => write!(f, "A name needs 1 to {LONGEST_NAME} characters"),
            Refusal::LastAdmin => write!(f, "SemmelTV needs at least one admin"),
            Refusal::Yourself => write!(f, "You can't remove your own profile"),
            Refusal::NotSignedIn => write!(f, "Not signed in"),
            Refusal::Store(reason) => write!(f, "The TV couldn't save that: {reason}"),
        }
    }
}

impl From<StoreError> for Refusal {
    fn from(error: StoreError) -> Self {
        Refusal::Store(error.to_string())
    }
}

pub type Outcome<T> = Result<T, Refusal>;

pub fn token() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

fn user_handle(profile: ProfileId) -> Uuid {
    Uuid::from_u64_pair(USER_HANDLE, profile.0 as u64)
}

struct Party {
    origin: String,
    webauthn: Webauthn,
}

enum Step {
    Register(PasskeyRegistration),
    SignIn(PasskeyAuthentication),
}

struct Ceremony {
    origin: String,
    profile: ProfileId,
    step: Step,
    started: Instant,
}

#[derive(Default)]
struct Failures {
    count: u32,
    until: u64,
}

pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

#[derive(Clone)]
pub struct Auth {
    inner: Arc<Inner>,
}

struct Inner {
    store: Arc<dyn AuthStore>,
    parties: Vec<Party>,
    ceremonies: Mutex<HashMap<String, Ceremony>>,
    failures: Mutex<HashMap<ProfileId, Failures>>,
    clock: Clock,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub active: Option<(Person, String)>,
    pub others: Vec<(Person, String)>,
    pub rewrite: bool,
}

impl Resolved {
    pub fn tokens(&self) -> Vec<String> {
        self.active.iter().chain(&self.others).map(|(_, token)| token.clone()).collect()
    }

    pub fn person(&self) -> Option<&Person> {
        self.active.as_ref().map(|(person, _)| person)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PasskeyView {
    pub id: String,
    pub name: String,
    pub here: bool,
    pub created_at: u64,
    pub last_used_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Member {
    #[serde(flatten)]
    pub person: Person,
    pub password: bool,
    pub passkeys: usize,
    pub sessions: usize,
    pub last_seen_at: Option<u64>,
    pub invite_expires_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Choice {
    #[serde(flatten)]
    pub person: Person,
    pub password: bool,
    pub passkey: bool,
}

impl Auth {
    pub fn new(store: Arc<dyn AuthStore>, origins: &[String]) -> Result<Self, String> {
        Self::with_clock(store, origins, Arc::new(now))
    }

    pub fn with_clock(store: Arc<dyn AuthStore>, origins: &[String], clock: Clock) -> Result<Self, String> {
        let mut parties = Vec::new();
        for origin in origins {
            let url = Url::parse(origin).map_err(|error| format!("origin {origin}: {error}"))?;
            let host = url.host_str().ok_or_else(|| format!("origin {origin} has no host"))?.to_string();
            let webauthn = WebauthnBuilder::new(&host, &url)
                .and_then(|builder| builder.rp_name("SemmelTV").build())
                .map_err(|error| format!("origin {origin}: {error}"))?;
            parties.push(Party {
                origin: url.origin().ascii_serialization(),
                webauthn,
            });
        }
        Ok(Self {
            inner: Arc::new(Inner {
                store,
                parties,
                ceremonies: Mutex::new(HashMap::new()),
                failures: Mutex::new(HashMap::new()),
                clock,
            }),
        })
    }

    fn now(&self) -> u64 {
        (self.inner.clock)()
    }

    fn store(&self) -> &dyn AuthStore {
        self.inner.store.as_ref()
    }

    fn party(&self, origin: &str) -> Outcome<&Party> {
        let origin = origin.trim_end_matches('/');
        self.inner
            .parties
            .iter()
            .find(|party| party.origin == origin)
            .ok_or(Refusal::UnknownOrigin)
    }

    pub fn allows_origin(&self, origin: &str) -> bool {
        self.party(origin).is_ok()
    }

    fn profile(&self, id: ProfileId) -> Outcome<Profile> {
        self.store().profile(id)?.ok_or(Refusal::UnknownProfile)
    }

    pub fn person(&self, id: ProfileId) -> Outcome<Person> {
        Ok(Person::from(&self.profile(id)?))
    }

    pub fn people(&self) -> Outcome<Vec<Person>> {
        Ok(self.store().profiles()?.iter().map(Person::from).collect())
    }

    pub fn choices(&self, origin: &str) -> Outcome<Vec<Choice>> {
        let rp_id = self.party(origin).ok().map(|party| rp_id(&party.origin));
        let mut choices = Vec::new();
        for profile in self.store().profiles()? {
            let passkey = match &rp_id {
                Some(rp_id) => self.store().passkeys(profile.id)?.iter().any(|passkey| &passkey.rp_id == rp_id),
                None => false,
            };
            choices.push(Choice {
                person: Person::from(&profile),
                password: profile.password_hash.is_some(),
                passkey,
            });
        }
        Ok(choices)
    }

    fn open_session(&self, profile: ProfileId, label: &str) -> Outcome<String> {
        let token = token();
        let now = self.now();
        self.store().create_session(&Session {
            token_hash: hash(&token),
            profile,
            created_at: now,
            last_seen_at: now,
            expires_at: now + SESSION_LIFETIME,
            label: label.chars().take(120).collect(),
        })?;
        Ok(token)
    }

    pub fn resolve(&self, tokens: &[String]) -> Outcome<Resolved> {
        let now = self.now();
        let mut found: Vec<(Person, String)> = Vec::new();
        let mut rewrite = tokens.len() > MOST_SESSIONS_ON_A_DEVICE;
        let mut renewed = false;
        for token in tokens.iter().take(MOST_SESSIONS_ON_A_DEVICE) {
            let Some(session) = self.store().session(&hash(token))? else {
                rewrite = true;
                continue;
            };
            let profile = self.store().profile(session.profile)?;
            let live = session.expires_at > now;
            let Some(profile) = profile.filter(|_| live) else {
                self.store().delete_session(&session.token_hash)?;
                rewrite = true;
                continue;
            };
            if found.iter().any(|(person, _)| person.id == profile.id) {
                rewrite = true;
                continue;
            }
            if found.is_empty() && now.saturating_sub(session.last_seen_at) >= RENEW_AFTER {
                self.store().touch_session(&session.token_hash, now, now + SESSION_LIFETIME)?;
                renewed = true;
            }
            found.push((Person::from(&profile), token.clone()));
        }
        let mut found = found.into_iter();
        Ok(Resolved {
            active: found.next(),
            others: found.collect(),
            rewrite: rewrite || renewed,
        })
    }

    fn check_password_shape(password: &str) -> Outcome<()> {
        let length = password.chars().count();
        if length < SHORTEST_PASSWORD {
            return Err(Refusal::ShortPassword);
        }
        if length > LONGEST_PASSWORD {
            return Err(Refusal::LongPassword);
        }
        Ok(())
    }

    fn throttled(&self, profile: ProfileId) -> Outcome<()> {
        let failures = self.inner.failures.lock().unwrap();
        match failures.get(&profile) {
            Some(failed) if failed.until > self.now() => Err(Refusal::Throttled(failed.until - self.now())),
            _ => Ok(()),
        }
    }

    fn failed(&self, profile: ProfileId) {
        let now = self.now();
        let mut failures = self.inner.failures.lock().unwrap();
        let failed = failures.entry(profile).or_default();
        failed.count += 1;
        if failed.count >= FREE_FAILURES {
            let doublings = (failed.count - FREE_FAILURES).min(6);
            failed.until = now + (60u64 << doublings).min(LONGEST_LOCKOUT);
        }
    }

    pub fn sign_in_with_password(&self, profile: ProfileId, password: &str, label: &str) -> Outcome<String> {
        let found = self.profile(profile)?;
        self.throttled(profile)?;
        let Some(stored) = found.password_hash.as_deref() else {
            return Err(Refusal::NoPassword(found.name));
        };
        if password.chars().count() > LONGEST_PASSWORD || !password::verify(password, stored) {
            self.failed(profile);
            return Err(Refusal::WrongPassword);
        }
        self.inner.failures.lock().unwrap().remove(&profile);
        self.open_session(profile, label)
    }

    pub fn set_password(&self, profile: ProfileId, password: &str) -> Outcome<()> {
        Self::check_password_shape(password)?;
        self.profile(profile)?;
        let hashed = password::hash(password).map_err(Refusal::Store)?;
        self.store().set_password_hash(profile, Some(&hashed))?;
        Ok(())
    }

    fn begin(&self, origin: &str, profile: ProfileId, step: Step) -> String {
        let id = token();
        let mut ceremonies = self.inner.ceremonies.lock().unwrap();
        ceremonies.retain(|_, ceremony| ceremony.started.elapsed() < CEREMONY_TIME);
        if ceremonies.len() >= MOST_CEREMONIES
            && let Some(oldest) = ceremonies.iter().min_by_key(|(_, ceremony)| ceremony.started).map(|(id, _)| id.clone())
        {
            ceremonies.remove(&oldest);
        }
        ceremonies.insert(
            id.clone(),
            Ceremony {
                origin: origin.trim_end_matches('/').to_string(),
                profile,
                step,
                started: Instant::now(),
            },
        );
        id
    }

    fn take(&self, id: &str, origin: &str) -> Outcome<Ceremony> {
        let ceremony = self.inner.ceremonies.lock().unwrap().remove(id).ok_or(Refusal::CeremonyGone)?;
        if ceremony.started.elapsed() >= CEREMONY_TIME {
            return Err(Refusal::CeremonyGone);
        }
        if ceremony.origin != origin.trim_end_matches('/') {
            return Err(Refusal::UnknownOrigin);
        }
        Ok(ceremony)
    }

    fn passkeys_here(&self, profile: ProfileId, party: &Party) -> Outcome<Vec<(StoredPasskey, Passkey)>> {
        let rp_id = rp_id(&party.origin);
        let mut found = Vec::new();
        for stored in self.store().passkeys(profile)? {
            if stored.rp_id != rp_id {
                continue;
            }
            match serde_json::from_str::<Passkey>(&stored.passkey) {
                Ok(passkey) => found.push((stored, passkey)),
                Err(error) => eprintln!("auth: passkey of profile {} unreadable: {error}", profile.0),
            }
        }
        Ok(found)
    }

    pub fn start_registration(&self, origin: &str, profile: ProfileId) -> Outcome<(String, CreationChallengeResponse)> {
        let party = self.party(origin)?;
        let found = self.profile(profile)?;
        let known = self.passkeys_here(profile, party)?;
        let exclude = known.iter().map(|(_, passkey)| passkey.cred_id().clone()).collect::<Vec<_>>();
        let (options, state) = party
            .webauthn
            .start_passkey_registration(user_handle(profile), &found.name, &found.name, Some(exclude))
            .map_err(|error| {
                eprintln!("auth: starting a passkey registration: {error}");
                Refusal::PasskeyFailed
            })?;
        let id = self.begin(origin, profile, Step::Register(state));
        Ok((id, options))
    }

    pub fn finish_registration(
        &self,
        origin: &str,
        profile: ProfileId,
        ceremony: &str,
        credential: &RegisterPublicKeyCredential,
        name: &str,
    ) -> Outcome<PasskeyView> {
        let party = self.party(origin)?;
        let begun = self.take(ceremony, origin)?;
        let Step::Register(state) = begun.step else {
            return Err(Refusal::CeremonyGone);
        };
        if begun.profile != profile {
            return Err(Refusal::CeremonyGone);
        }
        let passkey = party.webauthn.finish_passkey_registration(credential, &state).map_err(|error| {
            eprintln!("auth: finishing a passkey registration: {error}");
            Refusal::PasskeyFailed
        })?;
        let json = serde_json::to_string(&passkey).map_err(|error| Refusal::Store(error.to_string()))?;
        let name = name.trim().chars().take(LONGEST_NAME).collect::<String>();
        let stored = StoredPasskey {
            profile,
            credential_id: passkey.cred_id().clone(),
            rp_id: rp_id(&party.origin),
            name: if name.is_empty() { "Passkey".to_string() } else { name },
            passkey: json,
            created_at: self.now(),
            last_used_at: None,
        };
        match self.store().add_passkey(&stored) {
            Err(StoreError::Taken) => return Err(Refusal::PasskeyTaken),
            other => other?,
        }
        Ok(view(&stored, &party.origin))
    }

    pub fn start_sign_in(&self, origin: &str, profile: ProfileId) -> Outcome<(String, RequestChallengeResponse)> {
        let party = self.party(origin)?;
        let found = self.profile(profile)?;
        let known = self.passkeys_here(profile, party)?;
        if known.is_empty() {
            return Err(Refusal::NoPasskey(found.name));
        }
        let passkeys = known.into_iter().map(|(_, passkey)| passkey).collect::<Vec<_>>();
        let (options, state) = party.webauthn.start_passkey_authentication(&passkeys).map_err(|error| {
            eprintln!("auth: starting a passkey sign-in: {error}");
            Refusal::PasskeyFailed
        })?;
        let id = self.begin(origin, profile, Step::SignIn(state));
        Ok((id, options))
    }

    pub fn finish_sign_in(
        &self,
        origin: &str,
        ceremony: &str,
        credential: &PublicKeyCredential,
        label: &str,
    ) -> Outcome<(ProfileId, String)> {
        let party = self.party(origin)?;
        let begun = self.take(ceremony, origin)?;
        let Step::SignIn(state) = begun.step else {
            return Err(Refusal::CeremonyGone);
        };
        let result = party.webauthn.finish_passkey_authentication(credential, &state).map_err(|error| {
            eprintln!("auth: finishing a passkey sign-in: {error}");
            Refusal::PasskeyFailed
        })?;
        let stored = self
            .store()
            .passkey(result.cred_id())?
            .filter(|stored| stored.profile == begun.profile)
            .ok_or(Refusal::PasskeyFailed)?;
        let mut passkey: Passkey =
            serde_json::from_str(&stored.passkey).map_err(|error| Refusal::Store(error.to_string()))?;
        passkey.update_credential(&result);
        let json = serde_json::to_string(&passkey).map_err(|error| Refusal::Store(error.to_string()))?;
        self.store().update_passkey(&stored.credential_id, &json, self.now())?;
        let token = self.open_session(begun.profile, label)?;
        Ok((begun.profile, token))
    }

    pub fn has_password(&self, profile: ProfileId) -> Outcome<bool> {
        Ok(self.profile(profile)?.password_hash.is_some())
    }

    pub fn passkeys(&self, profile: ProfileId, origin: &str) -> Outcome<Vec<PasskeyView>> {
        let origin = origin.trim_end_matches('/');
        Ok(self.store().passkeys(profile)?.iter().map(|stored| view(stored, origin)).collect())
    }

    pub fn remove_passkey(&self, profile: ProfileId, id: &str) -> Outcome<bool> {
        let credential_id = URL_SAFE_NO_PAD.decode(id).map_err(|_| Refusal::PasskeyFailed)?;
        Ok(self.store().remove_passkey(profile, &credential_id)?)
    }

    pub fn sign_out(&self, token: &str) -> Outcome<()> {
        Ok(self.store().delete_session(&hash(token))?)
    }

    pub fn sign_out_everywhere(&self, profile: ProfileId) -> Outcome<()> {
        Ok(self.store().delete_sessions(profile)?)
    }

    pub fn invite(&self, by: Option<ProfileId>, profile: ProfileId) -> Outcome<String> {
        self.profile(profile)?;
        let token = token();
        let now = self.now();
        self.store().delete_invites(profile)?;
        self.store().create_invite(&Invite {
            token_hash: hash(&token),
            profile,
            created_by: by,
            created_at: now,
            expires_at: now + INVITE_LIFETIME,
            used_at: None,
        })?;
        Ok(token)
    }

    fn live_invite(&self, token: &str) -> Outcome<Invite> {
        let invite = self.store().invite(&hash(token))?.ok_or(Refusal::InviteUnknown)?;
        if invite.used_at.is_some() {
            return Err(Refusal::InviteUsed);
        }
        if invite.expires_at <= self.now() {
            return Err(Refusal::InviteExpired);
        }
        Ok(invite)
    }

    pub fn peek_invite(&self, token: &str) -> Outcome<Person> {
        let invite = self.live_invite(token)?;
        self.person(invite.profile)
    }

    pub fn redeem(&self, token: &str, label: &str) -> Outcome<(ProfileId, String)> {
        let invite = self.live_invite(token)?;
        self.profile(invite.profile)?;
        if !self.store().use_invite(&invite.token_hash, self.now())? {
            return Err(Refusal::InviteUsed);
        }
        let session = self.open_session(invite.profile, label)?;
        Ok((invite.profile, session))
    }

    pub fn members(&self) -> Outcome<Vec<Member>> {
        let now = self.now();
        let invites = self.store().invites()?;
        let mut members = Vec::new();
        for profile in self.store().profiles()? {
            let sessions = self.store().sessions(profile.id)?;
            let live = sessions.iter().filter(|session| session.expires_at > now).collect::<Vec<_>>();
            members.push(Member {
                person: Person::from(&profile),
                password: profile.password_hash.is_some(),
                passkeys: self.store().passkeys(profile.id)?.len(),
                sessions: live.len(),
                last_seen_at: live.iter().map(|session| session.last_seen_at).max(),
                invite_expires_at: invites
                    .iter()
                    .filter(|invite| invite.profile == profile.id && invite.used_at.is_none() && invite.expires_at > now)
                    .map(|invite| invite.expires_at)
                    .max(),
            });
        }
        Ok(members)
    }

    pub fn create_profile(&self, name: &str, role: Role) -> Outcome<Person> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > LONGEST_NAME {
            return Err(Refusal::BadName);
        }
        let created = self.store().create_profile(
            NewProfile {
                name: name.to_string(),
                role,
                color: None,
            },
            self.now(),
        );
        match created {
            Ok(profile) => Ok(Person::from(&profile)),
            Err(StoreError::Taken) => Err(Refusal::NameTaken),
            Err(error) => Err(error.into()),
        }
    }

    pub fn delete_profile(&self, by: ProfileId, profile: ProfileId) -> Outcome<()> {
        if by == profile {
            return Err(Refusal::Yourself);
        }
        let found = self.profile(profile)?;
        let profiles = self.store().profiles()?;
        let admins = profiles.iter().filter(|known| known.role == Role::Admin).count();
        if found.role == Role::Admin && admins <= 1 {
            return Err(Refusal::LastAdmin);
        }
        self.store().delete_profile(profile)?;
        Ok(())
    }

    pub fn forget_expired(&self) {
        if let Err(error) = self.store().delete_expired_sessions(self.now()) {
            eprintln!("auth: forgetting expired sessions: {error}");
        }
    }
}

fn rp_id(origin: &str) -> String {
    Url::parse(origin)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_default()
}

fn view(stored: &StoredPasskey, origin: &str) -> PasskeyView {
    PasskeyView {
        id: URL_SAFE_NO_PAD.encode(&stored.credential_id),
        name: stored.name.clone(),
        here: stored.rp_id == rp_id(origin),
        created_at: stored.created_at,
        last_used_at: stored.last_used_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::MemoryStore;
    use std::sync::atomic::{AtomicU64, Ordering};

    const ORIGIN: &str = "https://tv.example.org";

    struct Rig {
        auth: Auth,
        store: Arc<MemoryStore>,
        time: Arc<AtomicU64>,
    }

    impl Rig {
        fn new() -> Self {
            let store = Arc::new(MemoryStore::with([
                NewProfile {
                    name: "Frederik".to_string(),
                    role: Role::Admin,
                    color: None,
                },
                NewProfile {
                    name: "Mami".to_string(),
                    role: Role::Viewer,
                    color: None,
                },
            ]));
            let time = Arc::new(AtomicU64::new(1_000_000));
            let clock = time.clone();
            let auth = Auth::with_clock(store.clone(), &[ORIGIN.to_string()], Arc::new(move || clock.load(Ordering::SeqCst)))
                .unwrap();
            Rig { auth, store, time }
        }

        fn pass(&self, seconds: u64) {
            self.time.fetch_add(seconds, Ordering::SeqCst);
        }
    }

    const FREDERIK: ProfileId = ProfileId(1);
    const MAMI: ProfileId = ProfileId(2);

    #[test]
    fn an_invite_signs_in_once_and_only_until_it_expires() {
        let rig = Rig::new();
        let invite = rig.auth.invite(Some(FREDERIK), MAMI).unwrap();
        assert_eq!(rig.auth.peek_invite(&invite).unwrap().name, "Mami");
        let (profile, session) = rig.auth.redeem(&invite, "iPhone").unwrap();
        assert_eq!(profile, MAMI);
        assert_eq!(rig.auth.resolve(&[session]).unwrap().person().unwrap().name, "Mami");
        assert_eq!(rig.auth.redeem(&invite, "iPhone"), Err(Refusal::InviteUsed));

        let late = rig.auth.invite(Some(FREDERIK), MAMI).unwrap();
        rig.pass(INVITE_LIFETIME);
        assert_eq!(rig.auth.redeem(&late, "iPhone"), Err(Refusal::InviteExpired));
        assert_eq!(rig.auth.redeem("made-up", "iPhone"), Err(Refusal::InviteUnknown));
    }

    #[test]
    fn a_new_invite_replaces_the_one_before() {
        let rig = Rig::new();
        let first = rig.auth.invite(Some(FREDERIK), MAMI).unwrap();
        let second = rig.auth.invite(Some(FREDERIK), MAMI).unwrap();
        assert_eq!(rig.auth.redeem(&first, "iPad"), Err(Refusal::InviteUnknown));
        assert!(rig.auth.redeem(&second, "iPad").is_ok());
    }

    #[test]
    fn tokens_are_stored_only_as_hashes() {
        let rig = Rig::new();
        let invite = rig.auth.invite(Some(FREDERIK), MAMI).unwrap();
        let (_, session) = rig.auth.redeem(&invite, "iPhone").unwrap();
        let stored = rig.store.sessions(MAMI).unwrap();
        assert_eq!(stored[0].token_hash, hash(&session));
        assert_ne!(stored[0].token_hash, session.as_bytes());
        assert_eq!(rig.store.invites().unwrap()[0].token_hash, hash(&invite));
    }

    #[test]
    fn a_session_lasts_a_week_after_it_was_last_used() {
        let rig = Rig::new();
        rig.auth.set_password(MAMI, "correct horse").unwrap();
        let session = rig.auth.sign_in_with_password(MAMI, "correct horse", "iPhone").unwrap();

        rig.pass(6 * 24 * 60 * 60);
        let resolved = rig.auth.resolve(std::slice::from_ref(&session)).unwrap();
        assert!(resolved.active.is_some());
        assert!(resolved.rewrite, "a renewed session sends its cookie again");

        rig.pass(6 * 24 * 60 * 60);
        assert!(rig.auth.resolve(std::slice::from_ref(&session)).unwrap().active.is_some());

        rig.pass(10 * 60);
        let again = rig.auth.resolve(std::slice::from_ref(&session)).unwrap();
        assert!(again.active.is_some());
        assert!(!again.rewrite, "a session renews at most once an hour");

        rig.pass(SESSION_LIFETIME);
        let gone = rig.auth.resolve(std::slice::from_ref(&session)).unwrap();
        assert_eq!(gone.active, None);
        assert!(gone.rewrite);
        assert!(rig.store.sessions(MAMI).unwrap().is_empty());
    }

    #[test]
    fn a_device_keeps_several_profiles_with_the_first_active() {
        let rig = Rig::new();
        rig.auth.set_password(MAMI, "correct horse").unwrap();
        rig.auth.set_password(FREDERIK, "battery staple").unwrap();
        let mami = rig.auth.sign_in_with_password(MAMI, "correct horse", "iPad").unwrap();
        let frederik = rig.auth.sign_in_with_password(FREDERIK, "battery staple", "iPad").unwrap();
        let resolved = rig.auth.resolve(&[mami.clone(), "stale".to_string(), frederik.clone()]).unwrap();
        assert_eq!(resolved.person().unwrap().id, MAMI);
        assert_eq!(resolved.others.len(), 1);
        assert_eq!(resolved.tokens(), vec![mami, frederik]);
        assert!(resolved.rewrite, "the stale token leaves the cookie");
    }

    #[test]
    fn wrong_passwords_lock_the_profile_for_a_while() {
        let rig = Rig::new();
        rig.auth.set_password(MAMI, "correct horse").unwrap();
        for _ in 0..FREE_FAILURES {
            assert_eq!(rig.auth.sign_in_with_password(MAMI, "wrong one", "x"), Err(Refusal::WrongPassword));
        }
        assert_eq!(rig.auth.sign_in_with_password(MAMI, "correct horse", "x"), Err(Refusal::Throttled(60)));
        assert!(rig.auth.sign_in_with_password(FREDERIK, "anything", "x") != Err(Refusal::Throttled(60)));
        rig.pass(60);
        assert!(rig.auth.sign_in_with_password(MAMI, "correct horse", "x").is_ok());
        assert_eq!(rig.auth.sign_in_with_password(MAMI, "wrong one", "x"), Err(Refusal::WrongPassword));
    }

    #[test]
    fn passwords_need_a_sensible_length_and_are_hashed() {
        let rig = Rig::new();
        assert_eq!(rig.auth.set_password(MAMI, "short"), Err(Refusal::ShortPassword));
        assert_eq!(rig.auth.set_password(MAMI, &"x".repeat(201)), Err(Refusal::LongPassword));
        assert_eq!(
            rig.auth.sign_in_with_password(MAMI, "anything", "x"),
            Err(Refusal::NoPassword("Mami".to_string()))
        );
        rig.auth.set_password(MAMI, "correct horse").unwrap();
        let stored = rig.store.profile(MAMI).unwrap().unwrap().password_hash.unwrap();
        assert!(stored.starts_with("$argon2id$"));
        assert!(!stored.contains("correct horse"));
    }

    #[test]
    fn signing_out_ends_only_that_session() {
        let rig = Rig::new();
        rig.auth.set_password(MAMI, "correct horse").unwrap();
        let phone = rig.auth.sign_in_with_password(MAMI, "correct horse", "iPhone").unwrap();
        let ipad = rig.auth.sign_in_with_password(MAMI, "correct horse", "iPad").unwrap();
        rig.auth.sign_out(&phone).unwrap();
        assert_eq!(rig.auth.resolve(&[phone]).unwrap().active, None);
        assert!(rig.auth.resolve(std::slice::from_ref(&ipad)).unwrap().active.is_some());
        rig.auth.sign_out_everywhere(MAMI).unwrap();
        assert_eq!(rig.auth.resolve(&[ipad]).unwrap().active, None);
    }

    #[test]
    fn passkeys_only_start_on_a_known_address_for_someone_who_has_one() {
        let rig = Rig::new();
        assert_eq!(
            rig.auth.start_sign_in("https://elsewhere.example.org", MAMI).unwrap_err(),
            Refusal::UnknownOrigin
        );
        assert_eq!(rig.auth.start_sign_in(ORIGIN, MAMI).unwrap_err(), Refusal::NoPasskey("Mami".to_string()));
        let (ceremony, options) = rig.auth.start_registration(ORIGIN, MAMI).unwrap();
        let options = serde_json::to_value(&options).unwrap();
        assert_eq!(options["publicKey"]["rp"]["id"], "tv.example.org");
        assert_eq!(options["publicKey"]["user"]["name"], "Mami");
        assert!(rig.auth.take(&ceremony, "https://elsewhere.example.org").is_err());
        assert!(rig.auth.take(&ceremony, ORIGIN).is_err(), "a ceremony is used up by any attempt");
    }

    #[test]
    fn the_last_admin_and_oneself_stay() {
        let rig = Rig::new();
        assert_eq!(rig.auth.delete_profile(FREDERIK, FREDERIK), Err(Refusal::Yourself));
        assert_eq!(rig.auth.delete_profile(MAMI, FREDERIK), Err(Refusal::LastAdmin));
        let ramu = rig.auth.create_profile(" Ramu ", Role::Viewer).unwrap();
        assert_eq!(ramu.name, "Ramu");
        assert_eq!(rig.auth.create_profile("ramu", Role::Viewer), Err(Refusal::NameTaken));
        assert_eq!(rig.auth.create_profile("", Role::Viewer), Err(Refusal::BadName));
        rig.auth.delete_profile(FREDERIK, ramu.id).unwrap();
        assert_eq!(rig.auth.people().unwrap().len(), 2);
    }

    #[test]
    fn members_show_who_can_sign_in_how() {
        let rig = Rig::new();
        rig.auth.set_password(MAMI, "correct horse").unwrap();
        rig.auth.invite(Some(FREDERIK), FREDERIK).unwrap();
        let members = rig.auth.members().unwrap();
        assert!(!members[0].password);
        assert!(members[0].invite_expires_at.is_some());
        assert!(members[1].password);
        assert_eq!(members[1].invite_expires_at, None);
    }
}
