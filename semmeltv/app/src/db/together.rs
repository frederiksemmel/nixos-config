use super::models::Together;
use super::{Db, Result};
use crate::profile::ProfileId;

impl Db {
    fn show_id(&self, key: &str) -> Option<i64> {
        self.index
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .show(key)
    }

    pub async fn partners(&self, profile: ProfileId, show: &str) -> Result<Vec<ProfileId>> {
        let Some(show_id) = self.show_id(show) else {
            return Ok(Vec::new());
        };
        let mut db = self.db.clone();
        let fields = Together::fields();
        let rows = Together::filter(fields.profile_id().eq(profile.0).and(fields.show_id().eq(show_id)))
            .order_by(fields.partner_id().asc())
            .exec(&mut db)
            .await?;
        Ok(rows.into_iter().map(|row| ProfileId(row.partner_id)).collect())
    }

    pub async fn set_together(&self, profile: ProfileId, partner: ProfileId, show: &str, on: bool) -> Result<bool> {
        let Some(show_id) = self.show_id(show).filter(|_| profile != partner) else {
            return Ok(false);
        };
        let mut db = self.db.clone();
        let mut transaction = db.transaction().await?;
        for (one, other) in [(profile, partner), (partner, profile)] {
            let fields = Together::fields();
            Together::filter(
                fields
                    .profile_id()
                    .eq(one.0)
                    .and(fields.show_id().eq(show_id))
                    .and(fields.partner_id().eq(other.0)),
            )
            .delete()
            .exec(&mut transaction)
            .await?;
            if on {
                toasty::create!(Together {
                    profile_id: one.0,
                    show_id,
                    partner_id: other.0,
                })
                .exec(&mut transaction)
                .await?;
            }
        }
        transaction.commit().await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::{Db, test_library};
    use crate::profile::ProfileId;

    #[tokio::test]
    async fn watching_together_pairs_both_people_on_one_show_until_either_ends_it() {
        let db = Db::memory().await;
        db.set_library(&test_library());
        let show = test_library().shows[0].key.clone();
        let (frederik, mami, miriam) = (ProfileId(1), ProfileId(2), ProfileId(3));
        assert!(db.set_together(frederik, mami, &show, true).await.unwrap());
        assert!(db.set_together(frederik, mami, &show, true).await.unwrap());
        assert!(db.set_together(miriam, frederik, &show, true).await.unwrap());
        assert_eq!(db.partners(frederik, &show).await.unwrap(), [mami, miriam]);
        assert_eq!(db.partners(mami, &show).await.unwrap(), [frederik]);
        assert!(db.set_together(mami, frederik, &show, false).await.unwrap());
        assert_eq!(db.partners(frederik, &show).await.unwrap(), [miriam]);
        assert_eq!(db.partners(mami, &show).await.unwrap(), []);
        assert!(!db.set_together(frederik, frederik, &show, true).await.unwrap());
        assert!(!db.set_together(frederik, mami, "no-such-show", true).await.unwrap());
        assert!(db.delete_profile(miriam).await.unwrap());
        assert_eq!(db.partners(frederik, &show).await.unwrap(), []);
    }
}
