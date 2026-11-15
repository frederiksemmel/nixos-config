use super::Db;
use super::models::{self, Invite as StoredInvite, Passkey, Profile as StoredProfile, Session as StoredSession};
use crate::auth::store::{Invite, NewProfile, Profile, Role, Session, StoreError, Stored, StoredPasskey};
use crate::profile::ProfileId;
use toasty_core::driver::operation::TransactionMode;

fn stored<T>(result: toasty::Result<T>) -> Stored<T> {
    result.map_err(|error| {
        if unique_violation(&error) {
            StoreError::Taken
        } else {
            StoreError::Failed(error.to_string())
        }
    })
}

fn unique_violation(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(error) = current {
        if let Some(rusqlite::Error::SqliteFailure(failure, _)) = error.downcast_ref::<rusqlite::Error>() {
            return matches!(
                failure.extended_code,
                rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE | rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
            );
        }
        current = error.source();
    }
    false
}

fn seconds(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn role(role: Role) -> models::Role {
    match role {
        Role::Admin => models::Role::Admin,
        Role::Viewer => models::Role::Viewer,
        Role::Guest => models::Role::Guest,
    }
}

fn profile(stored: StoredProfile) -> Profile {
    Profile {
        id: ProfileId(stored.id),
        name: stored.name,
        role: match stored.role {
            models::Role::Admin => Role::Admin,
            models::Role::Viewer => Role::Viewer,
            models::Role::Guest => Role::Guest,
        },
        color: stored.color,
        password_hash: stored.password_hash,
        created_at: unsigned(stored.created_at),
    }
}

fn passkey(stored: Passkey) -> StoredPasskey {
    StoredPasskey {
        profile: ProfileId(stored.profile_id),
        credential_id: stored.credential_id,
        rp_id: stored.rp_id,
        name: stored.name,
        passkey: stored.passkey,
        created_at: unsigned(stored.created_at),
        last_used_at: stored.last_used_at.map(unsigned),
    }
}

fn session(stored: StoredSession) -> Session {
    Session {
        token_hash: stored.token_hash,
        profile: ProfileId(stored.profile_id),
        created_at: unsigned(stored.created_at),
        last_seen_at: unsigned(stored.last_seen_at),
        expires_at: unsigned(stored.expires_at),
        label: stored.label,
    }
}

fn invite(stored: StoredInvite) -> Invite {
    Invite {
        token_hash: stored.token_hash,
        profile: ProfileId(stored.profile_id),
        created_by: stored.created_by.map(ProfileId),
        created_at: unsigned(stored.created_at),
        expires_at: unsigned(stored.expires_at),
        used_at: stored.used_at.map(unsigned),
    }
}

impl Db {
    pub async fn profiles(&self) -> Stored<Vec<Profile>> {
        let mut db = self.db.clone();
        let rows = stored(
            StoredProfile::all()
                .order_by(StoredProfile::fields().id().asc())
                .exec(&mut db)
                .await,
        )?;
        Ok(rows.into_iter().map(profile).collect())
    }

    pub async fn profile(&self, id: ProfileId) -> Stored<Option<Profile>> {
        let mut db = self.db.clone();
        let row = stored(StoredProfile::filter_by_id(id.0).first().exec(&mut db).await)?;
        Ok(row.map(profile))
    }

    pub async fn create_profile(&self, new: NewProfile, at: u64) -> Stored<Profile> {
        if self.profiles().await?.iter().any(|known| known.name.to_lowercase() == new.name.to_lowercase()) {
            return Err(StoreError::Taken);
        }
        let mut db = self.db.clone();
        let row = stored(
            toasty::create!(StoredProfile {
                name: new.name,
                role: role(new.role),
                color: new.color,
                created_at: seconds(at),
            })
            .exec(&mut db)
            .await,
        )?;
        Ok(profile(row))
    }

    pub async fn delete_profile(&self, id: ProfileId) -> Stored<bool> {
        let mut db = self.db.clone();
        let deleted = async {
            let mut transaction = db.transaction().await?;
            let Some(found) = StoredProfile::filter_by_id(id.0).first().exec(&mut transaction).await? else {
                return Ok(false);
            };
            found.delete().exec(&mut transaction).await?;
            transaction.commit().await?;
            Ok::<_, toasty::Error>(true)
        };
        stored(deleted.await)
    }

    pub async fn set_password_hash(&self, id: ProfileId, hash: Option<&str>) -> Stored<()> {
        let hash = hash.map(str::to_string);
        let mut db = self.db.clone();
        let changed = async {
            let mut transaction = db.transaction().await?;
            let Some(mut found) = StoredProfile::filter_by_id(id.0).first().exec(&mut transaction).await? else {
                return Ok(false);
            };
            toasty::update!(found { password_hash: hash }).exec(&mut transaction).await?;
            transaction.commit().await?;
            Ok::<_, toasty::Error>(true)
        };
        if !stored(changed.await)? {
            return Err(StoreError::Failed(format!("no profile {}", id.0)));
        }
        Ok(())
    }

    pub async fn passkeys(&self, profile: ProfileId) -> Stored<Vec<StoredPasskey>> {
        let mut db = self.db.clone();
        let rows = stored(
            Passkey::filter_by_profile_id(profile.0)
                .order_by(Passkey::fields().id().asc())
                .exec(&mut db)
                .await,
        )?;
        Ok(rows.into_iter().map(passkey).collect())
    }

    pub async fn passkey(&self, credential_id: &[u8]) -> Stored<Option<StoredPasskey>> {
        let mut db = self.db.clone();
        let row = stored(Passkey::filter_by_credential_id(credential_id.to_vec()).first().exec(&mut db).await)?;
        Ok(row.map(passkey))
    }

    pub async fn add_passkey(&self, new: &StoredPasskey) -> Stored<()> {
        let new = new.clone();
        let mut db = self.db.clone();
        stored(
            toasty::create!(Passkey {
                profile_id: new.profile.0,
                credential_id: new.credential_id,
                rp_id: new.rp_id,
                name: new.name,
                passkey: new.passkey,
                created_at: seconds(new.created_at),
                last_used_at: new.last_used_at.map(seconds),
            })
            .exec(&mut db)
            .await,
        )?;
        Ok(())
    }

    pub async fn update_passkey(&self, credential_id: &[u8], json: &str, used_at: u64) -> Stored<()> {
        let (credential_id, json) = (credential_id.to_vec(), json.to_string());
        let mut db = self.db.clone();
        stored(
            toasty::update!(Passkey::filter_by_credential_id(credential_id) {
                passkey: json,
                last_used_at: Some(seconds(used_at)),
            })
            .exec(&mut db)
            .await,
        )?;
        Ok(())
    }

    pub async fn remove_passkey(&self, profile: ProfileId, credential_id: &[u8]) -> Stored<bool> {
        let credential_id = credential_id.to_vec();
        let mut db = self.db.clone();
        let removed = async {
            let fields = Passkey::fields();
            let mine = fields.profile_id().eq(profile.0).and(fields.credential_id().eq(credential_id));
            let mut transaction = db.transaction().await?;
            let found = Passkey::filter(mine.clone()).first().exec(&mut transaction).await?;
            if found.is_some() {
                Passkey::filter(mine).delete().exec(&mut transaction).await?;
            }
            transaction.commit().await?;
            Ok::<_, toasty::Error>(found.is_some())
        };
        stored(removed.await)
    }

    pub async fn create_session(&self, new: &Session) -> Stored<()> {
        let new = new.clone();
        let mut db = self.db.clone();
        stored(
            toasty::create!(StoredSession {
                token_hash: new.token_hash,
                profile_id: new.profile.0,
                created_at: seconds(new.created_at),
                last_seen_at: seconds(new.last_seen_at),
                expires_at: seconds(new.expires_at),
                label: new.label,
            })
            .exec(&mut db)
            .await,
        )?;
        Ok(())
    }

    pub async fn session(&self, token_hash: &[u8]) -> Stored<Option<Session>> {
        let mut db = self.db.clone();
        let row = stored(StoredSession::filter_by_token_hash(token_hash.to_vec()).first().exec(&mut db).await)?;
        Ok(row.map(session))
    }

    pub async fn sessions(&self, profile: ProfileId) -> Stored<Vec<Session>> {
        let mut db = self.db.clone();
        let rows = stored(StoredSession::filter_by_profile_id(profile.0).exec(&mut db).await)?;
        Ok(rows.into_iter().map(session).collect())
    }

    pub async fn touch_session(&self, token_hash: &[u8], last_seen_at: u64, expires_at: u64) -> Stored<()> {
        let token_hash = token_hash.to_vec();
        let mut db = self.db.clone();
        stored(
            toasty::update!(StoredSession::filter_by_token_hash(token_hash) {
                last_seen_at: seconds(last_seen_at),
                expires_at: seconds(expires_at),
            })
            .exec(&mut db)
            .await,
        )?;
        Ok(())
    }

    pub async fn delete_session(&self, token_hash: &[u8]) -> Stored<()> {
        let mut db = self.db.clone();
        stored(StoredSession::filter_by_token_hash(token_hash.to_vec()).delete().exec(&mut db).await)
    }

    pub async fn delete_sessions(&self, profile: ProfileId) -> Stored<()> {
        let mut db = self.db.clone();
        stored(StoredSession::filter_by_profile_id(profile.0).delete().exec(&mut db).await)
    }

    pub async fn delete_expired_sessions(&self, now: u64) -> Stored<()> {
        let mut db = self.db.clone();
        stored(
            StoredSession::filter(StoredSession::fields().expires_at().le(seconds(now)))
                .delete()
                .exec(&mut db)
                .await,
        )
    }

    pub async fn create_invite(&self, new: &Invite) -> Stored<()> {
        let new = new.clone();
        let mut db = self.db.clone();
        stored(
            toasty::create!(StoredInvite {
                token_hash: new.token_hash,
                profile_id: new.profile.0,
                created_by: new.created_by.map(|by| by.0),
                created_at: seconds(new.created_at),
                expires_at: seconds(new.expires_at),
                used_at: new.used_at.map(seconds),
            })
            .exec(&mut db)
            .await,
        )?;
        Ok(())
    }

    pub async fn invite(&self, token_hash: &[u8]) -> Stored<Option<Invite>> {
        let mut db = self.db.clone();
        let row = stored(StoredInvite::filter_by_token_hash(token_hash.to_vec()).first().exec(&mut db).await)?;
        Ok(row.map(invite))
    }

    pub async fn invites(&self) -> Stored<Vec<Invite>> {
        let mut db = self.db.clone();
        let rows = stored(StoredInvite::all().exec(&mut db).await)?;
        Ok(rows.into_iter().map(invite).collect())
    }

    pub async fn use_invite(&self, token_hash: &[u8], at: u64) -> Stored<bool> {
        let token_hash = token_hash.to_vec();
        let mut db = self.db.clone();
        let used = async {
            let mut transaction = db.transaction_builder().mode(TransactionMode::Immediate).begin().await?;
            let fields = StoredInvite::fields();
            let unused = StoredInvite::filter(fields.token_hash().eq(token_hash).and(fields.used_at().is_none()))
                .first()
                .exec(&mut transaction)
                .await?;
            let Some(mut unused) = unused else {
                return Ok(false);
            };
            toasty::update!(unused { used_at: Some(seconds(at)) }).exec(&mut transaction).await?;
            transaction.commit().await?;
            Ok::<_, toasty::Error>(true)
        };
        stored(used.await)
    }

    pub async fn delete_invites(&self, profile: ProfileId) -> Stored<()> {
        let mut db = self.db.clone();
        stored(StoredInvite::filter_by_profile_id(profile.0).delete().exec(&mut db).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Auth;

    async fn auth() -> (Auth, Db) {
        let db = Db::memory().await;
        let auth = Auth::new(db.clone(), &["https://tv.example.org".to_string()]).unwrap();
        (auth, db)
    }

    #[tokio::test]
    async fn the_household_is_there_from_the_start() {
        let (auth, _) = auth().await;
        let people = auth.people().await.unwrap();
        let names = people.iter().map(|person| person.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["Frederik", "Mami", "Miriam", "Ramu", "Guest"]);
        assert!(people[0].admin());
        assert!(people[1..].iter().all(|person| !person.admin()));
        assert_eq!(people[4].role, Role::Guest);
    }

    #[tokio::test]
    async fn invites_sessions_and_passwords_round_trip_through_the_database() {
        let (auth, db) = auth().await;
        let mami = ProfileId(2);
        let invite = auth.invite(Some(ProfileId::FIRST), mami).await.unwrap();
        let (profile, session) = auth.redeem(&invite, "iPhone · Safari").await.unwrap();
        assert_eq!(profile, mami);
        assert!(auth.redeem(&invite, "again").await.is_err());
        assert_eq!(auth.resolve(std::slice::from_ref(&session)).await.unwrap().person().unwrap().name, "Mami");
        assert_eq!(db.sessions(mami).await.unwrap()[0].label, "iPhone · Safari");

        auth.set_password(mami, "correct horse").await.unwrap();
        assert!(auth.sign_in_with_password(mami, "correct horse", "iPad").await.is_ok());
        assert!(auth.sign_in_with_password(mami, "wrong horse", "iPad").await.is_err());
        assert_eq!(db.sessions(mami).await.unwrap().len(), 2);

        auth.sign_out_everywhere(mami).await.unwrap();
        assert_eq!(auth.resolve(&[session]).await.unwrap().active, None);
    }

    #[tokio::test]
    async fn passkeys_are_unique_and_removing_a_profile_takes_its_rows_along() {
        let (auth, db) = auth().await;
        let key = StoredPasskey {
            profile: ProfileId(4),
            credential_id: vec![1, 2, 3],
            rp_id: "tv.example.org".to_string(),
            name: "iPhone".to_string(),
            passkey: "{}".to_string(),
            created_at: 1,
            last_used_at: None,
        };
        db.add_passkey(&key).await.unwrap();
        assert_eq!(db.add_passkey(&StoredPasskey { profile: ProfileId(3), ..key.clone() }).await, Err(StoreError::Taken));
        db.update_passkey(&[1, 2, 3], "{\"x\":1}", 5).await.unwrap();
        assert_eq!(db.passkey(&[1, 2, 3]).await.unwrap().unwrap().last_used_at, Some(5));
        auth.invite(Some(ProfileId::FIRST), ProfileId(4)).await.unwrap();
        auth.delete_profile(ProfileId::FIRST, ProfileId(4)).await.unwrap();
        assert_eq!(db.passkey(&[1, 2, 3]).await.unwrap(), None);
        assert!(db.invites().await.unwrap().is_empty());
        assert_eq!(auth.create_profile("mami", Role::Viewer).await.unwrap_err(), crate::auth::Refusal::NameTaken);
        let new = auth.create_profile("Oma", Role::Viewer).await.unwrap();
        assert_eq!(auth.person(new.id).await.unwrap().name, "Oma");
    }
}
