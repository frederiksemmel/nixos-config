use super::models::PushSubscription;
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
    /// whoever had it before on that browser.
    pub async fn subscribe(&self, profile: ProfileId, subscription: &Subscription, now: u64) -> Result<()> {
        let mut db = self.db.clone();
        let mut transaction = db.transaction().await?;
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
        Ok(())
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
        db.subscribe(mami, &phone("https://push.example/b"), 12).await.unwrap();
        assert_eq!(
            db.subscriptions(frederik).await.unwrap(),
            [phone("https://push.example/a")]
        );
        assert_eq!(db.subscriptions(mami).await.unwrap(), [phone("https://push.example/b")]);
        db.unsubscribe("https://push.example/a").await.unwrap();
        assert_eq!(db.subscriptions(frederik).await.unwrap(), []);
    }
}
