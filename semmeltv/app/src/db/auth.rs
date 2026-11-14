use super::Db;
use super::models::{self, Invite as StoredInvite, Passkey, Profile as StoredProfile, Session as StoredSession};
use crate::auth::store::{AuthStore, Invite, NewProfile, Profile, Role, Session, StoreError, Stored, StoredPasskey};
use crate::profile::ProfileId;
use toasty_core::driver::operation::TransactionMode;

fn stored<T>(result: super::Result<T>) -> Stored<T> {
    result.map_err(|error| {
        if unique_violation(error.as_ref()) {
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
    }
}

fn profile(stored: StoredProfile) -> Profile {
    Profile {
        id: ProfileId(stored.id),
        name: stored.name,
        role: match stored.role {
            models::Role::Admin => Role::Admin,
            models::Role::Viewer => Role::Viewer,
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

impl AuthStore for Db {
    fn profiles(&self) -> Stored<Vec<Profile>> {
        let rows = stored(self.wait(|mut db| async move {
            StoredProfile::all()
                .order_by(StoredProfile::fields().id().asc())
                .exec(&mut db)
                .await
        }))?;
        Ok(rows.into_iter().map(profile).collect())
    }

    fn profile(&self, id: ProfileId) -> Stored<Option<Profile>> {
        let row = stored(self.wait(move |mut db| async move {
            StoredProfile::filter_by_id(id.0).first().exec(&mut db).await
        }))?;
        Ok(row.map(profile))
    }

    fn create_profile(&self, new: NewProfile, at: u64) -> Stored<Profile> {
        if self.profiles()?.iter().any(|known| known.name.to_lowercase() == new.name.to_lowercase()) {
            return Err(StoreError::Taken);
        }
        let row = stored(self.wait(move |mut db| async move {
            toasty::create!(StoredProfile {
                name: new.name,
                role: role(new.role),
                color: new.color,
                created_at: seconds(at),
            })
            .exec(&mut db)
            .await
        }))?;
        Ok(profile(row))
    }

    fn delete_profile(&self, id: ProfileId) -> Stored<bool> {
        stored(self.wait(move |mut db| async move {
            let mut transaction = db.transaction().await?;
            let Some(found) = StoredProfile::filter_by_id(id.0).first().exec(&mut transaction).await? else {
                return Ok(false);
            };
            found.delete().exec(&mut transaction).await?;
            transaction.commit().await?;
            Ok(true)
        }))
    }

    fn set_password_hash(&self, id: ProfileId, hash: Option<&str>) -> Stored<()> {
        let hash = hash.map(str::to_string);
        let found = stored(self.wait(move |mut db| async move {
            let mut transaction = db.transaction().await?;
            let Some(mut found) = StoredProfile::filter_by_id(id.0).first().exec(&mut transaction).await? else {
                return Ok(false);
            };
            toasty::update!(found { password_hash: hash }).exec(&mut transaction).await?;
            transaction.commit().await?;
            Ok(true)
        }))?;
        if !found {
            return Err(StoreError::Failed(format!("no profile {}", id.0)));
        }
        Ok(())
    }

    fn passkeys(&self, profile: ProfileId) -> Stored<Vec<StoredPasskey>> {
        let rows = stored(self.wait(move |mut db| async move {
            Passkey::filter_by_profile_id(profile.0)
                .order_by(Passkey::fields().id().asc())
                .exec(&mut db)
                .await
        }))?;
        Ok(rows.into_iter().map(passkey).collect())
    }

    fn passkey(&self, credential_id: &[u8]) -> Stored<Option<StoredPasskey>> {
        let credential_id = credential_id.to_vec();
        let row = stored(self.wait(move |mut db| async move {
            Passkey::filter_by_credential_id(credential_id).first().exec(&mut db).await
        }))?;
        Ok(row.map(passkey))
    }

    fn add_passkey(&self, new: &StoredPasskey) -> Stored<()> {
        let new = new.clone();
        stored(self.wait(move |mut db| async move {
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
            .await?;
            Ok(())
        }))
    }

    fn update_passkey(&self, credential_id: &[u8], json: &str, used_at: u64) -> Stored<()> {
        let (credential_id, json) = (credential_id.to_vec(), json.to_string());
        stored(self.wait(move |mut db| async move {
            toasty::update!(Passkey::filter_by_credential_id(credential_id) {
                passkey: json,
                last_used_at: Some(seconds(used_at)),
            })
            .exec(&mut db)
            .await?;
            Ok(())
        }))
    }

    fn remove_passkey(&self, profile: ProfileId, credential_id: &[u8]) -> Stored<bool> {
        let credential_id = credential_id.to_vec();
        stored(self.wait(move |mut db| async move {
            let fields = Passkey::fields();
            let mine = fields.profile_id().eq(profile.0).and(fields.credential_id().eq(credential_id));
            let mut transaction = db.transaction().await?;
            let found = Passkey::filter(mine.clone()).first().exec(&mut transaction).await?;
            if found.is_some() {
                Passkey::filter(mine).delete().exec(&mut transaction).await?;
            }
            transaction.commit().await?;
            Ok(found.is_some())
        }))
    }

    fn create_session(&self, new: &Session) -> Stored<()> {
        let new = new.clone();
        stored(self.wait(move |mut db| async move {
            toasty::create!(StoredSession {
                token_hash: new.token_hash,
                profile_id: new.profile.0,
                created_at: seconds(new.created_at),
                last_seen_at: seconds(new.last_seen_at),
                expires_at: seconds(new.expires_at),
                label: new.label,
            })
            .exec(&mut db)
            .await?;
            Ok(())
        }))
    }

    fn session(&self, token_hash: &[u8]) -> Stored<Option<Session>> {
        let token_hash = token_hash.to_vec();
        let row = stored(self.wait(move |mut db| async move {
            StoredSession::filter_by_token_hash(token_hash).first().exec(&mut db).await
        }))?;
        Ok(row.map(session))
    }

    fn sessions(&self, profile: ProfileId) -> Stored<Vec<Session>> {
        let rows = stored(self.wait(move |mut db| async move {
            StoredSession::filter_by_profile_id(profile.0).exec(&mut db).await
        }))?;
        Ok(rows.into_iter().map(session).collect())
    }

    fn touch_session(&self, token_hash: &[u8], last_seen_at: u64, expires_at: u64) -> Stored<()> {
        let token_hash = token_hash.to_vec();
        stored(self.wait(move |mut db| async move {
            toasty::update!(StoredSession::filter_by_token_hash(token_hash) {
                last_seen_at: seconds(last_seen_at),
                expires_at: seconds(expires_at),
            })
            .exec(&mut db)
            .await?;
            Ok(())
        }))
    }

    fn delete_session(&self, token_hash: &[u8]) -> Stored<()> {
        let token_hash = token_hash.to_vec();
        stored(self.wait(move |mut db| async move {
            StoredSession::filter_by_token_hash(token_hash).delete().exec(&mut db).await
        }))
    }

    fn delete_sessions(&self, profile: ProfileId) -> Stored<()> {
        stored(self.wait(move |mut db| async move {
            StoredSession::filter_by_profile_id(profile.0).delete().exec(&mut db).await
        }))
    }

    fn delete_expired_sessions(&self, now: u64) -> Stored<()> {
        stored(self.wait(move |mut db| async move {
            StoredSession::filter(StoredSession::fields().expires_at().le(seconds(now)))
                .delete()
                .exec(&mut db)
                .await
        }))
    }

    fn create_invite(&self, new: &Invite) -> Stored<()> {
        let new = new.clone();
        stored(self.wait(move |mut db| async move {
            toasty::create!(StoredInvite {
                token_hash: new.token_hash,
                profile_id: new.profile.0,
                created_by: new.created_by.map(|by| by.0),
                created_at: seconds(new.created_at),
                expires_at: seconds(new.expires_at),
                used_at: new.used_at.map(seconds),
            })
            .exec(&mut db)
            .await?;
            Ok(())
        }))
    }

    fn invite(&self, token_hash: &[u8]) -> Stored<Option<Invite>> {
        let token_hash = token_hash.to_vec();
        let row = stored(self.wait(move |mut db| async move {
            StoredInvite::filter_by_token_hash(token_hash).first().exec(&mut db).await
        }))?;
        Ok(row.map(invite))
    }

    fn invites(&self) -> Stored<Vec<Invite>> {
        let rows = stored(self.wait(|mut db| async move { StoredInvite::all().exec(&mut db).await }))?;
        Ok(rows.into_iter().map(invite).collect())
    }

    fn use_invite(&self, token_hash: &[u8], at: u64) -> Stored<bool> {
        let token_hash = token_hash.to_vec();
        stored(self.wait(move |mut db| async move {
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
            Ok(true)
        }))
    }

    fn delete_invites(&self, profile: ProfileId) -> Stored<()> {
        stored(self.wait(move |mut db| async move {
            StoredInvite::filter_by_profile_id(profile.0).delete().exec(&mut db).await
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Auth;
    use std::path::Path;
    use std::sync::Arc;

    fn auth() -> (Auth, Db) {
        let db = Db::memory(Path::new("/library"));
        let auth = Auth::new(Arc::new(db.clone()), &["https://tv.example.org".to_string()]).unwrap();
        (auth, db)
    }

    #[test]
    fn the_household_is_there_from_the_start() {
        let (auth, _) = auth();
        let people = auth.people().unwrap();
        let names = people.iter().map(|person| person.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["Frederik", "Mami", "Miriam", "Ramu"]);
        assert!(people[0].admin());
        assert!(people[1..].iter().all(|person| !person.admin()));
    }

    #[test]
    fn invites_sessions_and_passwords_round_trip_through_the_database() {
        let (auth, db) = auth();
        let mami = ProfileId(2);
        let invite = auth.invite(Some(ProfileId::FIRST), mami).unwrap();
        let (profile, session) = auth.redeem(&invite, "iPhone · Safari").unwrap();
        assert_eq!(profile, mami);
        assert!(auth.redeem(&invite, "again").is_err());
        assert_eq!(auth.resolve(std::slice::from_ref(&session)).unwrap().person().unwrap().name, "Mami");
        assert_eq!(db.sessions(mami).unwrap()[0].label, "iPhone · Safari");

        auth.set_password(mami, "correct horse").unwrap();
        assert!(auth.sign_in_with_password(mami, "correct horse", "iPad").is_ok());
        assert!(auth.sign_in_with_password(mami, "wrong horse", "iPad").is_err());
        assert_eq!(db.sessions(mami).unwrap().len(), 2);

        auth.sign_out_everywhere(mami).unwrap();
        assert_eq!(auth.resolve(&[session]).unwrap().active, None);
    }

    #[test]
    fn passkeys_are_unique_and_removing_a_profile_takes_its_rows_along() {
        let (auth, db) = auth();
        let key = StoredPasskey {
            profile: ProfileId(4),
            credential_id: vec![1, 2, 3],
            rp_id: "tv.example.org".to_string(),
            name: "iPhone".to_string(),
            passkey: "{}".to_string(),
            created_at: 1,
            last_used_at: None,
        };
        db.add_passkey(&key).unwrap();
        assert_eq!(db.add_passkey(&StoredPasskey { profile: ProfileId(3), ..key.clone() }), Err(StoreError::Taken));
        db.update_passkey(&[1, 2, 3], "{\"x\":1}", 5).unwrap();
        assert_eq!(db.passkey(&[1, 2, 3]).unwrap().unwrap().last_used_at, Some(5));
        auth.invite(Some(ProfileId::FIRST), ProfileId(4)).unwrap();
        auth.delete_profile(ProfileId::FIRST, ProfileId(4)).unwrap();
        assert_eq!(db.passkey(&[1, 2, 3]).unwrap(), None);
        assert!(db.invites().unwrap().is_empty());
        assert_eq!(auth.create_profile("mami", Role::Viewer).unwrap_err(), crate::auth::Refusal::NameTaken);
        let new = auth.create_profile("Oma", Role::Viewer).unwrap();
        assert_eq!(auth.person(new.id).unwrap().name, "Oma");
    }
}
