use super::models::{PushNotice, PushSubscription};
use super::{Db, Result};
use crate::profile::ProfileId;

/// Where a browser receives pushes, and the keys to encrypt them for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

impl Db {
    /// Keeps a browser's subscription for `profile`, taking it over from
    /// whoever had it before on that browser. Whether it is new for them:
    /// the same browser checking in again is not.
    pub async fn subscribe(&self, profile: ProfileId, subscription: &Subscription, now: u64) -> Result<bool> {
        let mut db = self.db.clone();
        let mut transaction = db.transaction().await?;
        let known = PushSubscription::filter_by_endpoint(subscription.endpoint.clone())
            .first()
            .exec(&mut transaction)
            .await?
            .is_some_and(|row| {
                row.profile_id == profile.0 && row.p256dh == subscription.p256dh && row.auth == subscription.auth
            });
        PushSubscription::filter_by_endpoint(subscription.endpoint.clone())
            .delete()
            .exec(&mut transaction)
            .await?;
        toasty::create!(PushSubscription {
            profile_id: profile.0,
            endpoint: subscription.endpoint.clone(),
            p256dh: subscription.p256dh.clone(),
            auth: subscription.auth.clone(),
            created_at: i64::try_from(now).unwrap_or(i64::MAX),
        })
        .exec(&mut transaction)
        .await?;
        transaction.commit().await?;
        Ok(!known)
    }

    pub async fn unsubscribe(&self, endpoint: &str) -> Result<()> {
        let mut db = self.db.clone();
        PushSubscription::filter_by_endpoint(endpoint.to_string())
            .delete()
            .exec(&mut db)
            .await?;
        Ok(())
    }

    pub async fn subscriptions(&self, profile: ProfileId) -> Result<Vec<Subscription>> {
        let mut db = self.db.clone();
        let rows = PushSubscription::filter_by_profile_id(profile.0).exec(&mut db).await?;
        Ok(rows
            .into_iter()
            .map(|row| Subscription {
                endpoint: row.endpoint,
                p256dh: row.p256dh,
                auth: row.auth,
            })
            .collect())
    }

    /// When `profile` was last told `about` something, if ever.
    pub async fn last_notice(&self, profile: ProfileId, about: &str) -> Result<Option<u64>> {
        let mut db = self.db.clone();
        let fields = PushNotice::fields();
        let found = PushNotice::filter(
            fields
                .profile_id()
                .eq(profile.0)
                .and(fields.about().eq(about.to_string())),
        )
        .first()
        .exec(&mut db)
        .await?;
        Ok(found.map(|row| u64::try_from(row.sent_at).unwrap_or(0)))
    }

    pub async fn note_notice(&self, profile: ProfileId, about: &str, now: u64) -> Result<()> {
        let mut db = self.db.clone();
        PushNotice::upsert_by_profile_id_and_about(profile.0, about.to_string())
            .sent_at(i64::try_from(now).unwrap_or(i64::MAX))
            .exec(&mut db)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Subscription;
    use crate::db::Db;
    use crate::profile::ProfileId;

    fn phone(endpoint: &str) -> Subscription {
        Subscription {
            endpoint: endpoint.to_string(),
            p256dh: "BKey".to_string(),
            auth: "secret".to_string(),
        }
    }

    #[tokio::test]
    async fn a_browser_receives_pushes_for_the_last_person_who_turned_them_on_there() {
        let db = Db::memory().await;
        let (frederik, mami) = (ProfileId(1), ProfileId(2));
        db.subscribe(frederik, &phone("https://push.example/a"), 10)
            .await
            .unwrap();
        db.subscribe(frederik, &phone("https://push.example/b"), 11)
            .await
            .unwrap();
        assert_eq!(db.subscriptions(frederik).await.unwrap().len(), 2);
        assert!(
            !db.subscribe(frederik, &phone("https://push.example/b"), 12)
                .await
                .unwrap()
        );
        assert!(db.subscribe(mami, &phone("https://push.example/b"), 12).await.unwrap());
        assert_eq!(
            db.subscriptions(frederik).await.unwrap(),
            [phone("https://push.example/a")]
        );
        assert_eq!(db.subscriptions(mami).await.unwrap(), [phone("https://push.example/b")]);
        db.unsubscribe("https://push.example/a").await.unwrap();
        assert_eq!(db.subscriptions(frederik).await.unwrap(), []);
    }
}
