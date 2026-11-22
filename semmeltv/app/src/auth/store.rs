use crate::profile::ProfileId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
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

/// Something a profile may do beyond browsing and watching on its own
/// device. An admin may do all of them; anyone else only what was granted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    /// Play on the TV and use its remote: playback, volume, the TV's power,
    /// moving what plays between the TV and a phone, and pairing one's own
    /// Bluetooth headphones with the TV and listening on them.
    ControlTv,
    /// Find films and seasons and download them into the library, and watch
    /// how the downloads are going. Stopping a download, the TV's storage
    /// and removing titles stay with the admin.
    Download,
}

impl Capability {
    pub const ALL: [Capability; 2] = [Capability::ControlTv, Capability::Download];

    pub fn name(self) -> &'static str {
        match self {
            Capability::ControlTv => "control-tv",
            Capability::Download => "download",
        }
    }

    pub fn refusal(self) -> &'static str {
        match self {
            Capability::ControlTv => "Only someone allowed to control this TV can do that",
            Capability::Download => "Only someone allowed to download on this TV can do that",
        }
    }

    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|capability| capability.name() == name)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub role: Role,
    pub granted: BTreeSet<Capability>,
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
