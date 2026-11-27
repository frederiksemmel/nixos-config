//! Which clubs to ask about, from GotCourts' own map: every club it books
//! for inside the city of Zurich with tennis, padel or squash courts, and the
//! padel and squash around it (none inside the city books on GotCourts).
//! Looked up again once a day. A few clubs are missing from the map itself;
//! COURTS_CLUBS adds those.
//!
//! The map answers for a circle around a point. One club in Hottingen
//! (Sportanlage Sonnenberg) breaks every circle it lies in, so a circle that
//! fails is read again as small circles of 450 m; the ones that still fail
//! are kept as holes and shown on the pages.

use crate::db::Db;
use crate::gotcourts::{GotCourts, Listed};
use crate::time;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

const SETTING: &str = "discovered";
/// 1: the app's names for the clubs.
const VERSION: u32 = 1;
const AGAIN_AFTER: i64 = 24 * 60 * 60;
const STEP_KM: f64 = 2.5;
const FINE_STEP_KM: f64 = 0.6;
const FINE_KM: f64 = 0.45;
const BETWEEN_REQUESTS: Duration = Duration::from_millis(300);

/// Where to look: a centre and how far around it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub latitude: f64,
    pub longitude: f64,
    pub km: f64,
}

impl Area {
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split(',').map(|part| part.trim().parse::<f64>().ok());
        Some(Self {
            latitude: parts.next()??,
            longitude: parts.next()??,
            km: parts.next()??,
        })
    }

    fn distance(&self, latitude: f64, longitude: f64) -> f64 {
        let dy = (latitude - self.latitude) * 111.2;
        let dx = (longitude - self.longitude) * 111.2 * self.latitude.to_radians().cos();
        (dx * dx + dy * dy).sqrt()
    }

    /// Centres of circles of `STEP_KM` × 0.8 that together cover the area.
    fn tiles(&self) -> Vec<(f64, f64)> {
        self.grid(STEP_KM)
    }

    /// Centres `step` km apart covering the area; circles of radius
    /// `step` × 0.75 around them leave no gap (the corners are at 0.71).
    fn grid(&self, step: f64) -> Vec<(f64, f64)> {
        let reach = (self.km / step).ceil() as i64 + 1;
        let lat_step = step / 111.2;
        let lng_step = step / (111.2 * self.latitude.to_radians().cos());
        let mut tiles = Vec::new();
        for i in -reach..=reach {
            for j in -reach..=reach {
                let (lat, lng) = (
                    self.latitude + i as f64 * lat_step,
                    self.longitude + j as f64 * lng_step,
                );
                if self.distance(lat, lng) <= self.km + step {
                    tiles.push((lat, lng));
                }
            }
        }
        tiles
    }
}

/// A circle the map couldn't answer for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hole {
    pub latitude: f64,
    pub longitude: f64,
    pub km: f64,
}

/// What the last look at the map found.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Discovered {
    pub at: i64,
    pub clubs: Vec<i64>,
    pub names: BTreeMap<i64, String>,
    pub listed: usize,
    pub booking: usize,
    pub holes: Vec<Hole>,
    /// Map name → club number, so known clubs aren't looked up again.
    pub ids: BTreeMap<String, i64>,
    /// Club number → the app's name for it (`clb_…`), which app links take.
    #[serde(default)]
    pub apps: BTreeMap<i64, String>,
    /// What a look found: a look older than `VERSION` is done again.
    #[serde(default)]
    pub version: u32,
}

/// The clubs to ask about now, shared by the pollers and the pages: what
/// GotCourts' map found, and the venues booked on Eversports.
#[derive(Clone, Default)]
pub struct Clubs {
    found: Arc<RwLock<Discovered>>,
    eversports: Arc<RwLock<BTreeSet<i64>>>,
}

impl Clubs {
    /// Every club, from every source.
    pub fn ids(&self) -> Vec<i64> {
        let mut ids = self.gotcourts();
        ids.extend(self.eversports());
        ids
    }

    /// The clubs to ask GotCourts about.
    pub fn gotcourts(&self) -> Vec<i64> {
        self.found
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clubs
            .clone()
    }

    pub fn eversports(&self) -> Vec<i64> {
        self.eversports
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .copied()
            .collect()
    }

    /// Takes up a venue booked on Eversports.
    pub fn add_eversports(&self, id: i64) {
        self.eversports
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id);
    }

    /// Club number → GotCourts' app name for it, for the clubs on its map.
    pub fn apps(&self) -> BTreeMap<i64, String> {
        self.found
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .apps
            .clone()
    }

    pub fn state(&self) -> Discovered {
        self.found
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn set(&self, discovered: Discovered) {
        *self.found.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = discovered;
    }
}

pub struct Finder {
    pub db: Db,
    pub gotcourts: GotCourts,
    pub clubs: Clubs,
    pub area: Area,
    pub extra: Vec<i64>,
    pub exclude: Vec<i64>,
}

fn in_zurich(city: &str) -> bool {
    let city = city
        .trim_start_matches(|c: char| c.is_ascii_digit() || c.is_whitespace())
        .to_lowercase();
    city.starts_with("zürich") || city.starts_with("zurich")
}

impl Finder {
    /// Takes up what was found before, and looks again if that is a day old.
    pub async fn ready(&self) {
        let kept: Option<Discovered> = self
            .db
            .setting(SETTING)
            .await
            .ok()
            .flatten()
            .and_then(|text| serde_json::from_str(&text).ok());
        if let Some(kept) = kept {
            self.clubs.set(Discovered {
                clubs: self.with_extra(&kept.clubs),
                ..kept
            });
        }
        self.refresh().await;
    }

    fn with_extra(&self, clubs: &[i64]) -> Vec<i64> {
        let set: BTreeSet<i64> = clubs
            .iter()
            .chain(&self.extra)
            .copied()
            .filter(|id| !self.exclude.contains(id))
            .collect();
        set.into_iter().collect()
    }

    /// Looks at the map again when the last look is a day old.
    pub async fn refresh(&self) {
        let known = self.clubs.state();
        if !known.clubs.is_empty() && known.version >= VERSION && time::now() - known.at < AGAIN_AFTER {
            return;
        }
        match self.look(known.ids).await {
            Ok(found) => {
                eprintln!(
                    "discover: {} clubs on the map, {} book on GotCourts, {} to ask about, {} holes",
                    found.listed,
                    found.booking,
                    found.clubs.len(),
                    found.holes.len()
                );
                if let Ok(text) = serde_json::to_string(&found)
                    && let Err(error) = self.db.set_setting(SETTING, &text).await
                {
                    eprintln!("discover: saving: {error}");
                }
                self.clubs.set(found);
            }
            Err(error) => eprintln!("discover: {error}; keeping the clubs found before"),
        }
    }

    /// Reads one circle of the map; whether the map answered.
    async fn circle(&self, latitude: f64, longitude: f64, km: f64, found: &mut BTreeMap<String, Listed>) -> bool {
        tokio::time::sleep(BETWEEN_REQUESTS).await;
        match self.gotcourts.listed(latitude, longitude, km).await {
            Ok(listed) => {
                for club in listed {
                    found.insert(club.canonical.clone(), club);
                }
                true
            }
            Err(_) => false,
        }
    }

    /// Which of this app's sports a club has courts for, today or in last
    /// summer's season (many outdoor clubs have no courts in winter).
    async fn sports(&self, id: i64) -> BTreeSet<String> {
        let year = time::today()[..4].parse::<i32>().unwrap_or(2026);
        for date in [time::today(), format!("{year}-07-01"), format!("{}-07-01", year - 1)] {
            tokio::time::sleep(BETWEEN_REQUESTS).await;
            if let Ok(day) = self.gotcourts.day(id, &date).await
                && !day.courts.is_empty()
            {
                return day.courts.iter().map(|court| court.sport.clone()).collect();
            }
        }
        BTreeSet::new()
    }

    async fn look(&self, mut ids: BTreeMap<String, i64>) -> Result<Discovered, String> {
        let mut found = BTreeMap::new();
        let mut holes = Vec::new();
        let radius = STEP_KM * 0.8;
        let mut failed = Vec::new();
        for (lat, lng) in self.area.tiles() {
            if !self.circle(lat, lng, radius, &mut found).await {
                failed.push((lat, lng));
            }
        }
        for (lat, lng) in failed {
            let fine = Area {
                latitude: lat,
                longitude: lng,
                km: radius,
            };
            for (a, b) in fine.grid(FINE_STEP_KM) {
                if !self.circle(a, b, FINE_KM, &mut found).await {
                    holes.push(Hole {
                        latitude: a,
                        longitude: b,
                        km: FINE_KM,
                    });
                }
            }
        }
        if found.is_empty() {
            return Err("the map listed no club at all".into());
        }
        let near: Vec<&Listed> = found
            .values()
            .filter(|club| self.area.distance(club.latitude, club.longitude) <= self.area.km)
            .collect();
        let booking: Vec<&&Listed> = near.iter().filter(|club| club.books).collect();
        let mut clubs = Vec::new();
        let mut names = BTreeMap::new();
        let mut apps = BTreeMap::new();
        for club in &booking {
            let id = match ids.get(&club.canonical) {
                Some(id) => *id,
                None => {
                    tokio::time::sleep(BETWEEN_REQUESTS).await;
                    match self.gotcourts.id_of(&club.canonical).await {
                        Ok(id) => {
                            ids.insert(club.canonical.clone(), id);
                            id
                        }
                        Err(error) => {
                            eprintln!("discover: {} ({}): {error}", club.name, club.canonical);
                            continue;
                        }
                    }
                }
            };
            if self.exclude.contains(&id) {
                continue;
            }
            let sports = self.sports(id).await;
            let wanted = if in_zurich(&club.city) {
                !sports.is_empty()
            } else {
                sports.contains("padel") || sports.contains("squash")
            };
            if wanted {
                clubs.push(id);
                names.insert(id, club.name.clone());
                if !club.app.is_empty() {
                    apps.insert(id, club.app.clone());
                }
            }
        }
        Ok(Discovered {
            at: time::now(),
            clubs: self.with_extra(&clubs),
            names,
            listed: near.len(),
            booking: booking.len(),
            holes,
            ids,
            apps,
            version: VERSION,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tiles_cover_the_area_and_the_city_is_told_apart() {
        let area = Area::parse("47.3779, 8.5403, 16").unwrap();
        let tiles = area.tiles();
        assert!(tiles.len() > 100 && tiles.len() < 250, "{}", tiles.len());
        for (lat, lng) in [(47.38, 8.54), (47.25, 8.54), (47.37, 8.33)] {
            let nearest = tiles
                .iter()
                .map(|(a, b)| {
                    Area {
                        latitude: *a,
                        longitude: *b,
                        km: 0.0,
                    }
                    .distance(lat, lng)
                })
                .fold(f64::MAX, f64::min);
            assert!(nearest <= STEP_KM * 0.8, "{lat},{lng} is {nearest} km from a tile");
        }
        assert!(in_zurich("Zürich") && in_zurich("Zürich-Affoltern") && in_zurich("8045 Zürich"));
        assert!(!in_zurich("Zollikon") && !in_zurich("8702 Zollikon"));
        assert_eq!(Area::parse("1,2"), None);
    }
}
