use crate::details::Ratings;
use crate::library::{Episode, Show};
use crate::manifest::Tenths;
use crate::{HeatCell, HeatFocus, HeatRow, HeatmapData, RatingsData};
use slint::{Color, ModelRc, VecModel};
use std::cmp::Reverse;

pub struct Band {
    pub from: u8,
    pub label: &'static str,
    pub fill: (u8, u8, u8),
    pub ink: (u8, u8, u8),
}

const DARK: (u8, u8, u8) = (0x12, 0x14, 0x1a);
const LIGHT: (u8, u8, u8) = (0xff, 0xff, 0xff);

pub const BANDS: [Band; 6] = [
    Band {
        from: 90,
        label: "Awesome",
        fill: (0x1e, 0x84, 0x49),
        ink: LIGHT,
    },
    Band {
        from: 80,
        label: "Great",
        fill: (0x58, 0xc4, 0x6e),
        ink: DARK,
    },
    Band {
        from: 70,
        label: "Good",
        fill: (0xf2, 0xd0, 0x4a),
        ink: DARK,
    },
    Band {
        from: 60,
        label: "Regular",
        fill: (0xef, 0x8f, 0x3a),
        ink: DARK,
    },
    Band {
        from: 50,
        label: "Bad",
        fill: (0xd9, 0x44, 0x3c),
        ink: LIGHT,
    },
    Band {
        from: 0,
        label: "Garbage",
        fill: (0x7d, 0x3c, 0x98),
        ink: LIGHT,
    },
];

pub fn band(rating: Tenths) -> &'static Band {
    BANDS
        .iter()
        .find(|band| rating.0 >= band.from)
        .unwrap_or(&BANDS[BANDS.len() - 1])
}

fn regular(show: &Show) -> impl Iterator<Item = &Episode> {
    show.seasons
        .iter()
        .filter(|season| season.number > 0)
        .flat_map(|season| &season.episodes)
}

fn rated(show: &Show) -> Vec<(Tenths, u32, u32)> {
    regular(show)
        .filter_map(|episode| Some((episode.ratings.imdb?, episode.season, episode.number)))
        .collect()
}

pub fn order(show: &Show) -> Vec<(u32, u32)> {
    let mut rated = rated(show);
    rated.sort_by_key(|(_, season, number)| (*season, *number));
    rated.into_iter().map(|(_, season, number)| (season, number)).collect()
}

fn ranking(show: &Show) -> Vec<(u32, u32)> {
    let mut rated = rated(show);
    rated.sort_by_key(|(rating, season, number)| (Reverse(*rating), *season, *number));
    rated.into_iter().map(|(_, season, number)| (season, number)).collect()
}

pub fn entry(order: &[(u32, u32)], season: u32, number: u32) -> usize {
    order
        .iter()
        .position(|place| *place == (season, number))
        .or_else(|| order.iter().position(|place| place.0 == season))
        .unwrap_or(0)
}

pub fn vertical(order: &[(u32, u32)], position: usize, up: bool) -> Option<usize> {
    let (season, number) = *order.get(position)?;
    let target = if up {
        order.iter().map(|place| place.0).filter(|s| *s < season).max()?
    } else {
        order.iter().map(|place| place.0).filter(|s| *s > season).min()?
    };
    order
        .iter()
        .enumerate()
        .filter(|(_, place)| place.0 == target)
        .min_by_key(|(_, place)| (place.1.abs_diff(number), place.1))
        .map(|(index, _)| index)
}

fn color((red, green, blue): (u8, u8, u8)) -> Color {
    Color::from_rgb_u8(red, green, blue)
}

pub fn data(show: &Show) -> HeatmapData {
    let seasons: Vec<_> = show.seasons.iter().filter(|season| season.number > 0).collect();
    let columns = regular(show).map(|episode| episode.number).max().unwrap_or(0);
    let rows: Vec<HeatRow> = seasons
        .iter()
        .map(|season| {
            let cells: Vec<HeatCell> = (1..=columns)
                .map(|number| {
                    let rating = season
                        .episodes
                        .iter()
                        .find(|episode| episode.number == number)
                        .and_then(|episode| episode.ratings.imdb);
                    match rating {
                        Some(rating) => HeatCell {
                            text: rating.to_string().into(),
                            fill: color(band(rating).fill),
                            ink: color(band(rating).ink),
                            rated: true,
                            season: season.number as i32,
                            episode: number as i32,
                        },
                        None => HeatCell {
                            season: season.number as i32,
                            episode: number as i32,
                            ..HeatCell::default()
                        },
                    }
                })
                .collect();
            HeatRow {
                title: format!("S{}", season.number).into(),
                cells: ModelRc::new(VecModel::from(cells)),
            }
        })
        .collect();
    let legend: Vec<HeatCell> = BANDS
        .iter()
        .map(|band| HeatCell {
            text: match band.from {
                0 => format!("{} · below 5", band.label),
                from => format!("{} · {}+", band.label, Tenths(from)),
            }
            .into(),
            fill: color(band.fill),
            ink: color(band.ink),
            rated: true,
            ..HeatCell::default()
        })
        .collect();
    HeatmapData {
        rows: ModelRc::new(VecModel::from(rows)),
        columns: columns as i32,
        rated: order(show).len() as i32,
        legend: ModelRc::new(VecModel::from(legend)),
        hint: "◀ ▶ episodes · ▲ ▼ seasons".into(),
    }
}

pub fn focus(show: &Show, position: Option<usize>) -> HeatFocus {
    let places = order(show);
    let Some(((season, number), episode)) = position
        .and_then(|position| places.get(position))
        .and_then(|&place| Some((place, show.episode(place.0, place.1)?)))
    else {
        return HeatFocus::default();
    };
    let rank = ranking(show)
        .iter()
        .position(|place| *place == (season, number))
        .unwrap_or(0);
    HeatFocus {
        active: true,
        season: season as i32,
        episode: number as i32,
        title: episode.title.clone().unwrap_or_else(|| episode.label()).into(),
        place: format!("Season {season} · Episode {number}").into(),
        rank: match rank {
            0 => format!("Best of {}", places.len()),
            rank => format!("#{} of {}", rank + 1, places.len()),
        }
        .into(),
        ratings: RatingsData::from(Ratings::from(&episode.ratings)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Library;
    use crate::manifest::Manifest;
    use slint::Model;

    fn show() -> Show {
        let text = r#"[[show]]
title = "Dark"
episodes = [
  { season = 0, episode = 1, path = "s.mkv", added = 2024-01-01, ratings = { imdb = 9.9 } },
  { season = 1, episode = 1, path = "a.mkv", added = 2024-01-01, ratings = { imdb = 8.1 } },
  { season = 1, episode = 2, path = "b.mkv", added = 2024-01-01, ratings = { imdb = 9.2 } },
  { season = 1, episode = 4, path = "d.mkv", added = 2024-01-01 },
  { season = 2, episode = 1, title = "Beginnings", path = "e.mkv", added = 2024-01-01, ratings = { imdb = 8.1, rt = 90 } },
  { season = 2, episode = 2, path = "f.mkv", added = 2024-01-01, ratings = { imdb = 4.2 } },
]
"#;
        let manifest = Manifest::parse(text).unwrap();
        Library::from_manifest("/lib".as_ref(), &manifest, |_| true)
            .shows
            .remove(0)
    }

    #[test]
    fn the_walk_runs_by_season_and_episode_and_the_rank_best_first_both_without_specials_and_unrated() {
        assert_eq!(ranking(&show()), [(1, 2), (1, 1), (2, 1), (2, 2)]);
        assert_eq!(order(&show()), [(1, 1), (1, 2), (2, 1), (2, 2)]);
    }

    #[test]
    fn up_and_down_move_to_the_nearest_rated_episode_of_the_next_season() {
        let order = [(1, 1), (1, 2), (1, 5), (2, 1), (2, 3), (4, 2)];
        assert_eq!(vertical(&order, 2, false), Some(4));
        assert_eq!(vertical(&order, 1, false), Some(3));
        assert_eq!(vertical(&order, 4, false), Some(5));
        assert_eq!(vertical(&order, 5, true), Some(3));
        assert_eq!(vertical(&order, 0, true), None);
        assert_eq!(vertical(&order, 5, false), None);
    }

    #[test]
    fn entering_lands_on_the_episode_else_its_season_else_the_start() {
        let order = [(1, 1), (1, 2), (2, 1), (2, 3)];
        assert_eq!(entry(&order, 2, 3), 3);
        assert_eq!(entry(&order, 2, 2), 2);
        assert_eq!(entry(&order, 3, 1), 0);
    }

    #[test]
    fn bands_follow_the_rating() {
        let labels: Vec<_> = [95, 90, 89, 80, 75, 61, 50, 49, 0]
            .into_iter()
            .map(|tenths| band(Tenths(tenths)).label)
            .collect();
        assert_eq!(
            labels,
            [
                "Awesome", "Awesome", "Great", "Great", "Good", "Regular", "Bad", "Garbage", "Garbage"
            ]
        );
    }

    #[test]
    fn the_grid_has_a_row_per_season_and_a_cell_per_episode_number() {
        let data = data(&show());
        assert_eq!((data.columns, data.rated), (4, 4));
        let first = data.rows.row_data(0).unwrap();
        assert_eq!(first.title, "S1");
        let texts: Vec<_> = first.cells.iter().map(|cell| cell.text.to_string()).collect();
        assert_eq!(texts, ["8.1", "9.2", "", ""]);
        assert!(!first.cells.row_data(3).unwrap().rated);
        assert_eq!(data.rows.row_count(), 2);
    }

    #[test]
    fn the_focus_names_the_episode_and_its_rank() {
        let show = show();
        let best = focus(&show, Some(1));
        assert_eq!((best.season, best.episode, best.rank.as_str()), (1, 2, "Best of 4"));
        let third = focus(&show, Some(2));
        assert_eq!(third.title, "Beginnings");
        assert_eq!(third.place, "Season 2 · Episode 1");
        assert_eq!(third.rank, "#3 of 4");
        assert_eq!((third.ratings.imdb.as_str(), third.ratings.rt.as_str()), ("8.1", "90%"));
        assert!(!focus(&show, None).active);
    }
}
