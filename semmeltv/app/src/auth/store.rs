use crate::profile::ProfileId;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Viewer,
    Guest,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Viewer => "viewer",
            Role::Guest => "guest",
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
