//! Clubs whose courts lie apart: GotCourts lists ASVZ Zürich at Zurich HB,
//! but its courts are at Irchel and Hönggerberg. Each site shows as a club
//! of its own, at its own place, with the courts whose label names it.

use crate::db;
use crate::gotcourts::Court;

pub struct Site {
    pub club: i64,
    /// For anchors and map pins: "21-irchel".
    pub key: &'static str,
    /// The word in the labels of its courts.
    pub word: &'static str,
    pub name: &'static str,
    pub address: &'static str,
    pub zip: &'static str,
    pub city: &'static str,
    pub latitude: f64,
    pub longitude: f64,
}

pub const SITES: &[Site] = &[
    Site {
        club: 21,
        key: "21-irchel",
        word: "Irchel",
        name: "ASVZ Irchel",
        address: "Winterthurerstrasse 190",
        zip: "8057",
        city: "Zürich",
        latitude: 47.3970,
        longitude: 8.5488,
    },
    Site {
        club: 21,
        key: "21-hoenggerberg",
        word: "Hönggerberg",
        name: "ASVZ Hönggerberg",
        address: "Robert-Gnehm-Weg 15",
        zip: "8049",
        city: "Zürich",
        latitude: 47.4073,
        longitude: 8.5102,
    },
];

/// The site of `club` that `court` lies at, if the club is split.
pub fn site_of(club: i64, court: &Court) -> Option<&'static Site> {
    let label = court.label.to_lowercase();
    SITES
        .iter()
        .find(|site| site.club == club && label.contains(&site.word.to_lowercase()))
}

/// The sites `club` is split into.
pub fn of(club: i64) -> impl Iterator<Item = &'static Site> {
    SITES.iter().filter(move |site| site.club == club)
}

/// The club as it shows at `site`: that name and place.
pub fn at(club: &db::Club, site: Option<&Site>) -> db::Club {
    let mut shown = club.clone();
    if let Some(site) = site {
        shown.name = site.name.into();
        shown.address = site.address.into();
        shown.zip = site.zip.into();
        shown.city = site.city.into();
        shown.latitude = site.latitude;
        shown.longitude = site.longitude;
    }
    shown
}

/// One part of a club as the day page lists it: the courts at no site
/// stay with the club itself, each site is a part of its own.
pub struct Part {
    pub key: String,
    pub club: db::Club,
    pub site: Option<&'static Site>,
}

impl Part {
    pub fn holds(&self, court: &Court) -> bool {
        site_of(self.club.id, court).map(|site| site.key) == self.site.map(|site| site.key)
    }
}

pub fn parts(club: &db::Club) -> Vec<Part> {
    std::iter::once(None)
        .chain(of(club.id).map(Some))
        .map(|site| Part {
            key: site.map_or_else(|| club.id.to_string(), |site| site.key.to_string()),
            club: at(club, site),
            site,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn court(id: i64, label: &str) -> Court {
        Court {
            id,
            sport: "tennis".into(),
            label: label.into(),
            surface: "hard".into(),
            indoor: Some(false),
            cover: "outdoor".into(),
            ball_machine: false,
            order: id,
            interval: 60,
            durations: vec![60],
            opening: 8 * 3600,
            closing: 21 * 3600,
            cells: Vec::new(),
        }
    }

    #[test]
    fn asvz_courts_lie_at_irchel_and_hoenggerberg() {
        assert_eq!(
            site_of(21, &court(115, "SC Irchel Platz 1")).map(|s| s.name),
            Some("ASVZ Irchel")
        );
        assert_eq!(
            site_of(21, &court(128, "SC Hönggerberg 1")).map(|s| s.name),
            Some("ASVZ Hönggerberg")
        );
        assert!(site_of(21, &court(130, "SC Fluntern 1")).is_none());
        assert!(site_of(6, &court(1, "Irchel")).is_none());
    }
}
