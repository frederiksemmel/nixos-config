use crate::profile::ProfileId;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Viewer,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Viewer => "viewer",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub role: Role,
    pub color: Option<String>,
    pub password_hash: Option<String>,
    pub created_at: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewProfile {
    pub name: String,
    pub role: Role,
    pub color: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredPasskey {
    pub profile: ProfileId,
    pub credential_id: Vec<u8>,
    pub rp_id: String,
    pub name: String,
    pub passkey: String,
    pub created_at: u64,
    pub last_used_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub token_hash: Vec<u8>,
    pub profile: ProfileId,
    pub created_at: u64,
    pub last_seen_at: u64,
    pub expires_at: u64,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Invite {
    pub token_hash: Vec<u8>,
    pub profile: ProfileId,
    pub created_by: Option<ProfileId>,
    pub created_at: u64,
    pub expires_at: u64,
    pub used_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    Taken,
    Failed(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Taken => write!(f, "already taken"),
            StoreError::Failed(reason) => write!(f, "{reason}"),
        }
    }
}

pub type Stored<T> = Result<T, StoreError>;

pub trait AuthStore: Send + Sync {
    fn profiles(&self) -> Stored<Vec<Profile>>;
    fn profile(&self, id: ProfileId) -> Stored<Option<Profile>>;
    fn create_profile(&self, profile: NewProfile, at: u64) -> Stored<Profile>;
    fn delete_profile(&self, id: ProfileId) -> Stored<bool>;
    fn set_password_hash(&self, id: ProfileId, hash: Option<&str>) -> Stored<()>;

    fn passkeys(&self, profile: ProfileId) -> Stored<Vec<StoredPasskey>>;
    fn passkey(&self, credential_id: &[u8]) -> Stored<Option<StoredPasskey>>;
    fn add_passkey(&self, passkey: &StoredPasskey) -> Stored<()>;
    fn update_passkey(&self, credential_id: &[u8], passkey: &str, used_at: u64) -> Stored<()>;
    fn remove_passkey(&self, profile: ProfileId, credential_id: &[u8]) -> Stored<bool>;

    fn create_session(&self, session: &Session) -> Stored<()>;
    fn session(&self, token_hash: &[u8]) -> Stored<Option<Session>>;
    fn sessions(&self, profile: ProfileId) -> Stored<Vec<Session>>;
    fn touch_session(&self, token_hash: &[u8], last_seen_at: u64, expires_at: u64) -> Stored<()>;
    fn delete_session(&self, token_hash: &[u8]) -> Stored<()>;
    fn delete_sessions(&self, profile: ProfileId) -> Stored<()>;
    fn delete_expired_sessions(&self, now: u64) -> Stored<()>;

    fn create_invite(&self, invite: &Invite) -> Stored<()>;
    fn invite(&self, token_hash: &[u8]) -> Stored<Option<Invite>>;
    fn invites(&self) -> Stored<Vec<Invite>>;
    fn use_invite(&self, token_hash: &[u8], at: u64) -> Stored<bool>;
    fn delete_invites(&self, profile: ProfileId) -> Stored<()>;
}
