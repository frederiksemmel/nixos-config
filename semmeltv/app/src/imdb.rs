use crate::manifest::Tenths;
use flate2::read::GzDecoder;
use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader};

const EPISODES: &str = "https://datasets.imdbws.com/title.episode.tsv.gz";
const RATINGS: &str = "https://datasets.imdbws.com/title.ratings.tsv.gz";

pub type ShowRatings = HashMap<String, HashMap<(u32, u32), Tenths>>;

type Places = HashMap<String, (String, u32, u32)>;

fn places(shows: &BTreeSet<String>, episodes: impl BufRead) -> Places {
    let mut places = Places::new();
    for line in episodes.lines().map_while(Result::ok) {
        let mut fields = line.split('\t');
        let (Some(id), Some(parent), Some(season), Some(episode)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if !shows.contains(parent) {
            continue;
        }
        if let (Ok(season), Ok(episode)) = (season.parse(), episode.parse()) {
            places.insert(id.to_string(), (parent.to_string(), season, episode));
        }
    }
    places
}

fn rate(places: &Places, ratings: impl BufRead) -> ShowRatings {
    let mut found = ShowRatings::new();
    for line in ratings.lines().map_while(Result::ok) {
        let mut fields = line.split('\t');
        let (Some(id), Some(average)) = (fields.next(), fields.next()) else {
            continue;
        };
        let (Some((show, season, episode)), Some(rating)) = (places.get(id), Tenths::parse(average)) else {
            continue;
        };
        found
            .entry(show.clone())
            .or_default()
            .insert((*season, *episode), rating);
    }
    found
}

fn download(agent: &ureq::Agent, url: &str) -> Result<impl BufRead, String> {
    let response = agent.get(url).call().map_err(|error| format!("{url}: {error}"))?;
    Ok(BufReader::new(GzDecoder::new(response.into_body().into_reader())))
}

pub fn fetch(shows: &BTreeSet<String>) -> Result<ShowRatings, String> {
    let agent = ureq::Agent::new_with_defaults();
    let places = places(shows, download(&agent, EPISODES)?);
    Ok(rate(&places, download(&agent, RATINGS)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPISODES_TSV: &str = "tconst\tparentTconst\tseasonNumber\tepisodeNumber
tt0000001\ttt5753856\t1\t1
tt0000002\ttt5753856\t1\t2
tt0000003\ttt5753856\t\\N\t\\N
tt0000004\ttt9999999\t1\t1
tt0000005\ttt5753856\t2\t1
";

    const RATINGS_TSV: &str = "tconst\taverageRating\tnumVotes
tt0000001\t8.1\t31000
tt0000002\t7.96\t29000
tt0000003\t9.0\t10
tt0000004\t5.0\t100
tt5753856\t8.7\t500000
";

    #[test]
    fn episodes_of_the_wanted_shows_get_their_rating_by_season_and_number() {
        let shows = BTreeSet::from(["tt5753856".to_string()]);
        let found = rate(&places(&shows, EPISODES_TSV.as_bytes()), RATINGS_TSV.as_bytes());
        let dark = &found["tt5753856"];
        assert_eq!(dark.get(&(1, 1)), Some(&Tenths(81)));
        assert_eq!(dark.get(&(1, 2)), Some(&Tenths(80)));
        assert_eq!(dark.get(&(2, 1)), None);
        assert_eq!(dark.len(), 2);
        assert_eq!(found.len(), 1);
    }
}
