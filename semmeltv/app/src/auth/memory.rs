use super::store::{AuthStore, Invite, NewProfile, Profile, Session, StoreError, Stored, StoredPasskey};
use crate::profile::ProfileId;
use std::sync::Mutex;

#[derive(Default)]
pub struct MemoryStore {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    profiles: Vec<Profile>,
    passkeys: Vec<StoredPasskey>,
    sessions: Vec<Session>,
    invites: Vec<Invite>,
}

impl MemoryStore {
    pub fn with(profiles: impl IntoIterator<Item = NewProfile>) -> Self {
        let store = Self::default();
        for profile in profiles {
            store.create_profile(profile, 0).unwrap();
        }
        store
    }
}

impl AuthStore for MemoryStore {
    fn profiles(&self) -> Stored<Vec<Profile>> {
        Ok(self.state.lock().unwrap().profiles.clone())
    }

    fn profile(&self, id: ProfileId) -> Stored<Option<Profile>> {
        Ok(self.state.lock().unwrap().profiles.iter().find(|profile| profile.id == id).cloned())
    }

    fn create_profile(&self, profile: NewProfile, at: u64) -> Stored<Profile> {
        let mut state = self.state.lock().unwrap();
        if state.profiles.iter().any(|known| known.name.eq_ignore_ascii_case(&profile.name)) {
            return Err(StoreError::Taken);
        }
        let id = ProfileId(state.profiles.iter().map(|known| known.id.0).max().unwrap_or(0) + 1);
        let created = Profile {
            id,
            name: profile.name,
            role: profile.role,
            color: profile.color,
            password_hash: None,
            created_at: at,
        };
        state.profiles.push(created.clone());
        Ok(created)
    }

    fn delete_profile(&self, id: ProfileId) -> Stored<bool> {
        let mut state = self.state.lock().unwrap();
        let before = state.profiles.len();
        state.profiles.retain(|profile| profile.id != id);
        state.passkeys.retain(|passkey| passkey.profile != id);
        state.sessions.retain(|session| session.profile != id);
        state.invites.retain(|invite| invite.profile != id);
        Ok(state.profiles.len() != before)
    }

    fn set_password_hash(&self, id: ProfileId, hash: Option<&str>) -> Stored<()> {
        let mut state = self.state.lock().unwrap();
        let profile = state.profiles.iter_mut().find(|profile| profile.id == id);
        let profile = profile.ok_or_else(|| StoreError::Failed(format!("no profile {}", id.0)))?;
        profile.password_hash = hash.map(str::to_string);
        Ok(())
    }

    fn passkeys(&self, profile: ProfileId) -> Stored<Vec<StoredPasskey>> {
        let state = self.state.lock().unwrap();
        Ok(state.passkeys.iter().filter(|passkey| passkey.profile == profile).cloned().collect())
    }

    fn passkey(&self, credential_id: &[u8]) -> Stored<Option<StoredPasskey>> {
        let state = self.state.lock().unwrap();
        Ok(state.passkeys.iter().find(|passkey| passkey.credential_id == credential_id).cloned())
    }

    fn add_passkey(&self, passkey: &StoredPasskey) -> Stored<()> {
        let mut state = self.state.lock().unwrap();
        if state.passkeys.iter().any(|known| known.credential_id == passkey.credential_id) {
            return Err(StoreError::Taken);
        }
        state.passkeys.push(passkey.clone());
        Ok(())
    }

    fn update_passkey(&self, credential_id: &[u8], passkey: &str, used_at: u64) -> Stored<()> {
        let mut state = self.state.lock().unwrap();
        if let Some(known) = state.passkeys.iter_mut().find(|known| known.credential_id == credential_id) {
            known.passkey = passkey.to_string();
            known.last_used_at = Some(used_at);
        }
        Ok(())
    }

    fn remove_passkey(&self, profile: ProfileId, credential_id: &[u8]) -> Stored<bool> {
        let mut state = self.state.lock().unwrap();
        let before = state.passkeys.len();
        state
            .passkeys
            .retain(|passkey| !(passkey.profile == profile && passkey.credential_id == credential_id));
        Ok(state.passkeys.len() != before)
    }

    fn create_session(&self, session: &Session) -> Stored<()> {
        self.state.lock().unwrap().sessions.push(session.clone());
        Ok(())
    }

    fn session(&self, token_hash: &[u8]) -> Stored<Option<Session>> {
        let state = self.state.lock().unwrap();
        Ok(state.sessions.iter().find(|session| session.token_hash == token_hash).cloned())
    }

    fn sessions(&self, profile: ProfileId) -> Stored<Vec<Session>> {
        let state = self.state.lock().unwrap();
        Ok(state.sessions.iter().filter(|session| session.profile == profile).cloned().collect())
    }

    fn touch_session(&self, token_hash: &[u8], last_seen_at: u64, expires_at: u64) -> Stored<()> {
        let mut state = self.state.lock().unwrap();
        if let Some(session) = state.sessions.iter_mut().find(|session| session.token_hash == token_hash) {
            session.last_seen_at = last_seen_at;
            session.expires_at = expires_at;
        }
        Ok(())
    }

    fn delete_session(&self, token_hash: &[u8]) -> Stored<()> {
        self.state.lock().unwrap().sessions.retain(|session| session.token_hash != token_hash);
        Ok(())
    }

    fn delete_sessions(&self, profile: ProfileId) -> Stored<()> {
        self.state.lock().unwrap().sessions.retain(|session| session.profile != profile);
        Ok(())
    }

    fn delete_expired_sessions(&self, now: u64) -> Stored<()> {
        self.state.lock().unwrap().sessions.retain(|session| session.expires_at > now);
        Ok(())
    }

    fn create_invite(&self, invite: &Invite) -> Stored<()> {
        self.state.lock().unwrap().invites.push(invite.clone());
        Ok(())
    }

    fn invite(&self, token_hash: &[u8]) -> Stored<Option<Invite>> {
        let state = self.state.lock().unwrap();
        Ok(state.invites.iter().find(|invite| invite.token_hash == token_hash).cloned())
    }

    fn invites(&self) -> Stored<Vec<Invite>> {
        Ok(self.state.lock().unwrap().invites.clone())
    }

    fn use_invite(&self, token_hash: &[u8], at: u64) -> Stored<bool> {
        let mut state = self.state.lock().unwrap();
        match state.invites.iter_mut().find(|invite| invite.token_hash == token_hash) {
            Some(invite) if invite.used_at.is_none() => {
                invite.used_at = Some(at);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn delete_invites(&self, profile: ProfileId) -> Stored<()> {
        self.state.lock().unwrap().invites.retain(|invite| invite.profile != profile);
        Ok(())
    }
}
