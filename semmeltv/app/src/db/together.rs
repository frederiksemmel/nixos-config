use super::models::Together;
use super::{Db, Result};
use crate::profile::ProfileId;
use std::collections::BTreeSet;

impl Db {
    fn show_id(&self, key: &str) -> Option<i64> {
        self.index
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .show(key)
    }

    async fn pairs(&self, show_id: i64) -> Result<Vec<(ProfileId, ProfileId)>> {
        let mut db = self.db.clone();
        let rows = Together::filter(Together::fields().show_id().eq(show_id))
            .exec(&mut db)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| (ProfileId(row.profile_id), ProfileId(row.partner_id)))
            .collect())
    }

    /// Everyone in `profile`'s group for `show`, without `profile` itself:
    /// whoever is linked to them, directly or through someone else.
    pub async fn partners(&self, profile: ProfileId, show: &str) -> Result<Vec<ProfileId>> {
        let Some(show_id) = self.show_id(show) else {
            return Ok(Vec::new());
        };
        let mut group = group(&self.pairs(show_id).await?, profile);
        group.remove(&profile);
        Ok(group.into_iter().collect())
    }

    /// Adding someone puts both whole groups together, so everyone in it
    /// watches with everyone. Taking someone out takes them out of the group
    /// for all; taking yourself out leaves it.
    pub async fn set_together(&self, profile: ProfileId, partner: ProfileId, show: &str, on: bool) -> Result<bool> {
        let Some(show_id) = self.show_id(show).filter(|_| !on || profile != partner) else {
            return Ok(false);
        };
        let pairs = self.pairs(show_id).await?;
        let mut db = self.db.clone();
        let mut transaction = db.transaction().await?;
        if on {
            let mut everyone = group(&pairs, profile);
            everyone.extend(group(&pairs, partner));
            for &one in &everyone {
                for &other in &everyone {
                    if one == other || pairs.contains(&(one, other)) {
                        continue;
                    }
                    toasty::create!(Together {
                        profile_id: one.0,
                        show_id,
                        partner_id: other.0,
                    })
                    .exec(&mut transaction)
                    .await?;
                }
            }
        } else {
            let fields = Together::fields();
            Together::filter(
                fields
                    .show_id()
                    .eq(show_id)
                    .and(fields.profile_id().eq(partner.0).or(fields.partner_id().eq(partner.0))),
            )
            .delete()
            .exec(&mut transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(true)
    }
}

fn group(pairs: &[(ProfileId, ProfileId)], profile: ProfileId) -> BTreeSet<ProfileId> {
    let mut group = BTreeSet::from([profile]);
    let mut reached = vec![profile];
    while let Some(one) = reached.pop() {
        for &(from, to) in pairs {
            for (here, there) in [(from, to), (to, from)] {
                if here == one && group.insert(there) {
                    reached.push(there);
                }
            }
        }
    }
    group
}

#[cfg(test)]
mod tests {
    use crate::db::{Db, test_library};
    use crate::profile::ProfileId;

    #[tokio::test]
    async fn adding_someone_to_a_group_makes_everyone_in_it_watch_with_everyone() {
        let db = Db::memory().await;
        db.set_library(&test_library());
        let show = test_library().shows[0].key.clone();
        let (frederik, mami, miriam, ramu) = (ProfileId(1), ProfileId(2), ProfileId(3), ProfileId(4));
        assert!(db.set_together(frederik, mami, &show, true).await.unwrap());
        assert!(db.set_together(frederik, mami, &show, true).await.unwrap());
        assert!(db.set_together(miriam, frederik, &show, true).await.unwrap());
        assert_eq!(db.partners(frederik, &show).await.unwrap(), [mami, miriam]);
        assert_eq!(db.partners(mami, &show).await.unwrap(), [frederik, miriam]);
        assert_eq!(db.partners(miriam, &show).await.unwrap(), [frederik, mami]);
        assert!(db.set_together(ramu, mami, &show, true).await.unwrap());
        assert_eq!(db.partners(ramu, &show).await.unwrap(), [frederik, mami, miriam]);

        assert!(db.set_together(frederik, mami, &show, false).await.unwrap());
        assert_eq!(db.partners(frederik, &show).await.unwrap(), [miriam, ramu]);
        assert_eq!(db.partners(mami, &show).await.unwrap(), []);
        assert!(db.set_together(ramu, ramu, &show, false).await.unwrap());
        assert_eq!(db.partners(frederik, &show).await.unwrap(), [miriam]);
        assert_eq!(db.partners(ramu, &show).await.unwrap(), []);

        assert!(!db.set_together(frederik, frederik, &show, true).await.unwrap());
        assert!(!db.set_together(frederik, mami, "no-such-show", true).await.unwrap());
        assert!(db.delete_profile(miriam).await.unwrap());
        assert_eq!(db.partners(frederik, &show).await.unwrap(), []);
    }
}
