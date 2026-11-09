use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(40);
const TORZNAB: &str = "http://torznab.com/schemas/2015/feed";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub name: String,
    pub size: u64,
    pub seeders: u32,
    pub peers: u32,
    pub infohash: String,
    pub magnet: String,
    pub indexer: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Search {
    Movie { title: String, year: Option<u16>, imdb: Option<String> },
    Tv { title: String, season: u32, episode: Option<u32> },
}

pub struct Torznab {
    endpoints: Vec<String>,
    api_key: Option<String>,
    agent: ureq::Agent,
}

impl Torznab {
    pub fn new(endpoints: Vec<String>, api_key: Option<String>) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .build();
        Self {
            endpoints,
            api_key,
            agent: config.new_agent(),
        }
    }

    pub fn search(&self, search: &Search) -> (Vec<Offer>, Vec<String>) {
        let mut offers = Vec::new();
        let mut problems = Vec::new();
        for endpoint in &self.endpoints {
            for query in queries(search) {
                match self.query(endpoint, &query) {
                    Ok(found) => offers.extend(found),
                    Err(problem) => problems.push(problem),
                }
            }
        }
        (offers, problems)
    }

    fn query(&self, endpoint: &str, query: &[(&str, String)]) -> Result<Vec<Offer>, String> {
        let mut request = self.agent.get(endpoint);
        if let Some(key) = &self.api_key {
            request = request.query("apikey", key);
        }
        for (name, value) in query {
            request = request.query(*name, value);
        }
        let mut response = request.call().map_err(|error| format!("{endpoint}: {error}"))?;
        let status = response.status();
        let body = response
            .body_mut()
            .with_config()
            .limit(32 * 1024 * 1024)
            .read_to_string()
            .map_err(|error| format!("{endpoint}: {error}"))?;
        if !status.is_success() {
            return Err(format!("{endpoint}: HTTP {status}: {}", error_text(&body)));
        }
        parse(&body).map_err(|error| format!("{endpoint}: {error}"))
    }
}

fn queries(search: &Search) -> Vec<Vec<(&'static str, String)>> {
    match search {
        Search::Movie { title, year, imdb } => {
            let text = match year {
                Some(year) => format!("{title} {year}"),
                None => title.clone(),
            };
            let mut queries = vec![vec![("t", "movie".to_string()), ("q", text)]];
            if let Some(imdb) = imdb {
                queries.push(vec![("t", "movie".to_string()), ("imdbid", imdb.clone())]);
            }
            queries
        }
        Search::Tv { title, season, episode } => {
            let mut query = vec![
                ("t", "tvsearch".to_string()),
                ("q", title.clone()),
                ("season", season.to_string()),
            ];
            if let Some(episode) = episode {
                query.push(("ep", episode.to_string()));
            }
            vec![query]
        }
    }
}

fn error_text(body: &str) -> String {
    roxmltree::Document::parse(body)
        .ok()
        .and_then(|document| {
            let error = document.root_element();
            (error.has_tag_name("error")).then(|| error.attribute("description").unwrap_or("").to_string())
        })
        .map(|text| text.lines().next().unwrap_or("").to_string())
        .unwrap_or_else(|| body.chars().take(200).collect())
}

fn attribute<'a>(item: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    item.children()
        .filter(|node| node.tag_name().name() == "attr" && node.tag_name().namespace() == Some(TORZNAB))
        .find(|node| node.attribute("name") == Some(name))
        .and_then(|node| node.attribute("value"))
}

fn child<'a>(item: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    item.children()
        .find(|node| node.tag_name().name() == name && node.tag_name().namespace().is_none())
        .and_then(|node| node.text())
}

fn infohash_from_magnet(magnet: &str) -> Option<String> {
    let start = magnet.find("urn:btih:")? + "urn:btih:".len();
    let hash: String = magnet[start..].chars().take_while(char::is_ascii_alphanumeric).collect();
    (hash.len() == 40).then(|| hash.to_lowercase())
}

pub fn parse(xml: &str) -> Result<Vec<Offer>, String> {
    let document = roxmltree::Document::parse(xml).map_err(|error| error.to_string())?;
    let root = document.root_element();
    if root.has_tag_name("error") {
        return Err(root.attribute("description").unwrap_or("error").to_string());
    }
    let offers = document
        .descendants()
        .filter(|node| node.has_tag_name("item"))
        .filter_map(|item| {
            let name = child(item, "title")?.trim().to_string();
            let magnet = attribute(item, "magneturl")
                .or_else(|| child(item, "link").filter(|link| link.starts_with("magnet:")))?
                .to_string();
            let infohash = attribute(item, "infohash")
                .map(str::to_lowercase)
                .or_else(|| infohash_from_magnet(&magnet))?;
            let size = attribute(item, "size")
                .or_else(|| child(item, "size"))
                .and_then(|size| size.parse().ok())?;
            let seeders = attribute(item, "seeders").and_then(|seeders| seeders.parse().ok()).unwrap_or(0);
            let peers = attribute(item, "peers").and_then(|peers| peers.parse().ok()).unwrap_or(seeders);
            let indexer = child(item, "jackettindexer").unwrap_or("").to_string();
            Some(Offer {
                name,
                size,
                seeders,
                peers,
                infohash,
                magnet,
                indexer,
            })
        })
        .collect();
    Ok(offers)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom" xmlns:torznab="http://torznab.com/schemas/2015/feed">
  <channel>
    <item>
      <title>Outlander (2014) S08 (1080p AMZN WEB-DL x265 10bit EAC3 5.1 Silence)</title>
      <guid>magnet:?xt=urn:btih:5459d03debeedb8a0ddc2590def4861e40a93c3c</guid>
      <jackettindexer id="torrentscsv">Torrents.csv</jackettindexer>
      <size>20863404301</size>
      <link>magnet:?xt=urn:btih:5459d03debeedb8a0ddc2590def4861e40a93c3c&amp;dn=Outlander</link>
      <torznab:attr name="seeders" value="253" />
      <torznab:attr name="peers" value="431" />
      <torznab:attr name="infohash" value="5459D03DEBEEDB8A0DDC2590DEF4861E40A93C3C" />
      <torznab:attr name="magneturl" value="magnet:?xt=urn:btih:5459d03debeedb8a0ddc2590def4861e40a93c3c&amp;dn=Outlander" />
    </item>
    <item>
      <title>Outlander.S08E10.1080p.WEB.h264-GRACE</title>
      <size>5045000000</size>
      <link>http://127.0.0.1:9117/dl/knaben/?jackett_apikey=x</link>
      <torznab:attr name="seeders" value="142" />
      <torznab:attr name="magneturl" value="magnet:?xt=urn:btih:ABCDEF0123456789ABCDEF0123456789ABCDEF01&amp;dn=x" />
    </item>
    <item>
      <title>No magnet</title>
      <size>1</size>
      <link>http://127.0.0.1:9117/dl/x</link>
    </item>
  </channel>
</rss>"#;

    #[test]
    fn items_become_offers_with_magnets() {
        let offers = parse(FEED).unwrap();
        assert_eq!(offers.len(), 2);
        assert_eq!(offers[0].infohash, "5459d03debeedb8a0ddc2590def4861e40a93c3c");
        assert_eq!(offers[0].seeders, 253);
        assert_eq!(offers[0].peers, 431);
        assert_eq!(offers[0].size, 20_863_404_301);
        assert_eq!(offers[0].indexer, "Torrents.csv");
        assert_eq!(offers[1].infohash, "abcdef0123456789abcdef0123456789abcdef01");
        assert_eq!(offers[1].peers, 142);
    }

    #[test]
    fn an_error_answer_is_an_error() {
        let error = r#"<?xml version="1.0" encoding="UTF-8"?><error code="100" description="Invalid API Key" />"#;
        assert_eq!(parse(error), Err("Invalid API Key".to_string()));
    }

    #[test]
    fn films_search_by_text_and_imdb_id_and_seasons_by_number() {
        let film = Search::Movie {
            title: "Dune Part Two".to_string(),
            year: Some(2024),
            imdb: Some("tt15239678".to_string()),
        };
        assert_eq!(queries(&film).len(), 2);
        assert_eq!(queries(&film)[0][1], ("q", "Dune Part Two 2024".to_string()));
        let season = Search::Tv {
            title: "Outlander".to_string(),
            season: 8,
            episode: None,
        };
        assert_eq!(
            queries(&season),
            [vec![
                ("t", "tvsearch".to_string()),
                ("q", "Outlander".to_string()),
                ("season", "8".to_string())
            ]]
        );
    }
}
