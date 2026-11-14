use super::Db;
use super::schema::{invites, passkeys, profiles, sessions};
use crate::auth::store::{AuthStore, Invite, NewProfile, Profile, Role, Session, StoreError, Stored, StoredPasskey};
use crate::profile::ProfileId;
use diesel::prelude::*;
use diesel::result::{DatabaseErrorKind, Error as DieselError};

type ProfileRow = (i64, String, String, Option<String>, Option<String>, i64);
type PasskeyRow = (i64, Vec<u8>, String, String, String, i64, Option<i64>);
type SessionRow = (Vec<u8>, i64, i64, i64, i64, Option<String>);
type InviteRow = (Vec<u8>, i64, Option<i64>, i64, i64, Option<i64>);

fn stored<T>(result: super::Result<T>) -> Stored<T> {
    result.map_err(|error| match error.downcast_ref::<DieselError>() {
        Some(DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation, _)) => StoreError::Taken,
        _ => StoreError::Failed(error.to_string()),
    })
}

fn seconds(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn profile((id, name, role, password_hash, color, created_at): ProfileRow) -> Profile {
    Profile {
        id: ProfileId(id),
        name,
        role: Role::parse(&role).unwrap_or(Role::Viewer),
        color,
        password_hash,
        created_at: unsigned(created_at),
    }
}

fn passkey((profile_id, credential_id, rp_id, name, passkey, created_at, last_used_at): PasskeyRow) -> StoredPasskey {
    StoredPasskey {
        profile: ProfileId(profile_id),
        credential_id,
        rp_id,
        name,
        passkey,
        created_at: unsigned(created_at),
        last_used_at: last_used_at.map(unsigned),
    }
}

fn session((token_hash, profile_id, created_at, last_seen_at, expires_at, label): SessionRow) -> Session {
    Session {
        token_hash,
        profile: ProfileId(profile_id),
        created_at: unsigned(created_at),
        last_seen_at: unsigned(last_seen_at),
        expires_at: unsigned(expires_at),
        label: label.unwrap_or_default(),
    }
}

fn invite((token_hash, profile_id, created_by, created_at, expires_at, used_at): InviteRow) -> Invite {
    Invite {
        token_hash,
        profile: ProfileId(profile_id),
        created_by: created_by.map(ProfileId),
        created_at: unsigned(created_at),
        expires_at: unsigned(expires_at),
        used_at: used_at.map(unsigned),
    }
}

const PROFILE: (
    profiles::id,
    profiles::name,
    profiles::role,
    profiles::password_hash,
    profiles::color,
    profiles::created_at,
) = (
    profiles::id,
    profiles::name,
    profiles::role,
    profiles::password_hash,
    profiles::color,
    profiles::created_at,
);

const PASSKEY: (
    passkeys::profile_id,
    passkeys::credential_id,
    passkeys::rp_id,
    passkeys::name,
    passkeys::passkey,
    passkeys::created_at,
    passkeys::last_used_at,
) = (
    passkeys::profile_id,
    passkeys::credential_id,
    passkeys::rp_id,
    passkeys::name,
    passkeys::passkey,
    passkeys::created_at,
    passkeys::last_used_at,
);

const SESSION: (
    sessions::token_hash,
    sessions::profile_id,
    sessions::created_at,
    sessions::last_seen_at,
    sessions::expires_at,
    sessions::label,
) = (
    sessions::token_hash,
    sessions::profile_id,
    sessions::created_at,
    sessions::last_seen_at,
    sessions::expires_at,
    sessions::label,
);

const INVITE: (
    invites::token_hash,
    invites::profile_id,
    invites::created_by,
    invites::created_at,
    invites::expires_at,
    invites::used_at,
) = (
    invites::token_hash,
    invites::profile_id,
    invites::created_by,
    invites::created_at,
    invites::expires_at,
    invites::used_at,
);

impl AuthStore for Db {
    fn profiles(&self) -> Stored<Vec<Profile>> {
        let rows = stored(self.run(|connection| {
            profiles::table.select(PROFILE).order(profiles::id).load::<ProfileRow>(connection)
        }))?;
        Ok(rows.into_iter().map(profile).collect())
    }

    fn profile(&self, id: ProfileId) -> Stored<Option<Profile>> {
        let row = stored(self.run(|connection| {
            profiles::table.find(id.0).select(PROFILE).first::<ProfileRow>(connection).optional()
        }))?;
        Ok(row.map(profile))
    }

    fn create_profile(&self, new: NewProfile, at: u64) -> Stored<Profile> {
        if self.profiles()?.iter().any(|known| known.name.to_lowercase() == new.name.to_lowercase()) {
            return Err(StoreError::Taken);
        }
        let row = stored(self.run(|connection| {
            diesel::insert_into(profiles::table)
                .values((
                    profiles::name.eq(&new.name),
                    profiles::role.eq(new.role.name()),
                    profiles::color.eq(&new.color),
                    profiles::created_at.eq(seconds(at)),
                ))
                .returning(PROFILE)
                .get_result::<ProfileRow>(connection)
        }))?;
        Ok(profile(row))
    }

    fn delete_profile(&self, id: ProfileId) -> Stored<bool> {
        let deleted = stored(self.run(|connection| diesel::delete(profiles::table.find(id.0)).execute(connection)))?;
        Ok(deleted > 0)
    }

    fn set_password_hash(&self, id: ProfileId, hash: Option<&str>) -> Stored<()> {
        let changed = stored(self.run(|connection| {
            diesel::update(profiles::table.find(id.0))
                .set(profiles::password_hash.eq(hash))
                .execute(connection)
        }))?;
        if changed == 0 {
            return Err(StoreError::Failed(format!("no profile {}", id.0)));
        }
        Ok(())
    }

    fn passkeys(&self, profile: ProfileId) -> Stored<Vec<StoredPasskey>> {
        let rows = stored(self.run(|connection| {
            passkeys::table
                .filter(passkeys::profile_id.eq(profile.0))
                .order(passkeys::id)
                .select(PASSKEY)
                .load::<PasskeyRow>(connection)
        }))?;
        Ok(rows.into_iter().map(passkey).collect())
    }

    fn passkey(&self, credential_id: &[u8]) -> Stored<Option<StoredPasskey>> {
        let row = stored(self.run(|connection| {
            passkeys::table
                .filter(passkeys::credential_id.eq(credential_id))
                .select(PASSKEY)
                .first::<PasskeyRow>(connection)
                .optional()
        }))?;
        Ok(row.map(passkey))
    }

    fn add_passkey(&self, passkey: &StoredPasskey) -> Stored<()> {
        stored(self.run(|connection| {
            diesel::insert_into(passkeys::table)
                .values((
                    passkeys::profile_id.eq(passkey.profile.0),
                    passkeys::credential_id.eq(&passkey.credential_id),
                    passkeys::rp_id.eq(&passkey.rp_id),
                    passkeys::name.eq(&passkey.name),
                    passkeys::passkey.eq(&passkey.passkey),
                    passkeys::created_at.eq(seconds(passkey.created_at)),
                    passkeys::last_used_at.eq(passkey.last_used_at.map(seconds)),
                ))
                .execute(connection)
                .map(drop)
        }))
    }

    fn update_passkey(&self, credential_id: &[u8], passkey: &str, used_at: u64) -> Stored<()> {
        stored(self.run(|connection| {
            diesel::update(passkeys::table.filter(passkeys::credential_id.eq(credential_id)))
                .set((passkeys::passkey.eq(passkey), passkeys::last_used_at.eq(seconds(used_at))))
                .execute(connection)
                .map(drop)
        }))
    }

    fn remove_passkey(&self, profile: ProfileId, credential_id: &[u8]) -> Stored<bool> {
        let deleted = stored(self.run(|connection| {
            diesel::delete(
                passkeys::table
                    .filter(passkeys::profile_id.eq(profile.0))
                    .filter(passkeys::credential_id.eq(credential_id)),
            )
            .execute(connection)
        }))?;
        Ok(deleted > 0)
    }

    fn create_session(&self, session: &Session) -> Stored<()> {
        stored(self.run(|connection| {
            diesel::insert_into(sessions::table)
                .values((
                    sessions::token_hash.eq(&session.token_hash),
                    sessions::profile_id.eq(session.profile.0),
                    sessions::created_at.eq(seconds(session.created_at)),
                    sessions::last_seen_at.eq(seconds(session.last_seen_at)),
                    sessions::expires_at.eq(seconds(session.expires_at)),
                    sessions::label.eq(&session.label),
                ))
                .execute(connection)
                .map(drop)
        }))
    }

    fn session(&self, token_hash: &[u8]) -> Stored<Option<Session>> {
        let row = stored(self.run(|connection| {
            sessions::table
                .find(token_hash)
                .select(SESSION)
                .first::<SessionRow>(connection)
                .optional()
        }))?;
        Ok(row.map(session))
    }

    fn sessions(&self, profile: ProfileId) -> Stored<Vec<Session>> {
        let rows = stored(self.run(|connection| {
            sessions::table
                .filter(sessions::profile_id.eq(profile.0))
                .select(SESSION)
                .load::<SessionRow>(connection)
        }))?;
        Ok(rows.into_iter().map(session).collect())
    }

    fn touch_session(&self, token_hash: &[u8], last_seen_at: u64, expires_at: u64) -> Stored<()> {
        stored(self.run(|connection| {
            diesel::update(sessions::table.find(token_hash))
                .set((
                    sessions::last_seen_at.eq(seconds(last_seen_at)),
                    sessions::expires_at.eq(seconds(expires_at)),
                ))
                .execute(connection)
                .map(drop)
        }))
    }

    fn delete_session(&self, token_hash: &[u8]) -> Stored<()> {
        stored(self.run(|connection| diesel::delete(sessions::table.find(token_hash)).execute(connection).map(drop)))
    }

    fn delete_sessions(&self, profile: ProfileId) -> Stored<()> {
        stored(self.run(|connection| {
            diesel::delete(sessions::table.filter(sessions::profile_id.eq(profile.0)))
                .execute(connection)
                .map(drop)
        }))
    }

    fn delete_expired_sessions(&self, now: u64) -> Stored<()> {
        stored(self.run(|connection| {
            diesel::delete(sessions::table.filter(sessions::expires_at.le(seconds(now))))
                .execute(connection)
                .map(drop)
        }))
    }

    fn create_invite(&self, invite: &Invite) -> Stored<()> {
        stored(self.run(|connection| {
            diesel::insert_into(invites::table)
                .values((
                    invites::token_hash.eq(&invite.token_hash),
                    invites::profile_id.eq(invite.profile.0),
                    invites::created_by.eq(invite.created_by.map(|by| by.0)),
                    invites::created_at.eq(seconds(invite.created_at)),
                    invites::expires_at.eq(seconds(invite.expires_at)),
                    invites::used_at.eq(invite.used_at.map(seconds)),
                ))
                .execute(connection)
                .map(drop)
        }))
    }

    fn invite(&self, token_hash: &[u8]) -> Stored<Option<Invite>> {
        let row = stored(self.run(|connection| {
            invites::table
                .find(token_hash)
                .select(INVITE)
                .first::<InviteRow>(connection)
                .optional()
        }))?;
        Ok(row.map(invite))
    }

    fn invites(&self) -> Stored<Vec<Invite>> {
        let rows = stored(self.run(|connection| invites::table.select(INVITE).load::<InviteRow>(connection)))?;
        Ok(rows.into_iter().map(invite).collect())
    }

    fn use_invite(&self, token_hash: &[u8], at: u64) -> Stored<bool> {
        let used = stored(self.run(|connection| {
            diesel::update(invites::table.find(token_hash).filter(invites::used_at.is_null()))
                .set(invites::used_at.eq(seconds(at)))
                .execute(connection)
        }))?;
        Ok(used > 0)
    }

    fn delete_invites(&self, profile: ProfileId) -> Stored<()> {
        stored(self.run(|connection| {
            diesel::delete(invites::table.filter(invites::profile_id.eq(profile.0)))
                .execute(connection)
                .map(drop)
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
