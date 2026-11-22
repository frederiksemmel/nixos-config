use crate::bakery::{Cause, Closed};

const PAGE: &str = include_str!("screen.html");
pub const ROLL: &str = include_str!("../../assets/logo/favicon.svg");
pub const WORDMARK: &str = include_str!("../../assets/logo/semmeltv-dark.svg");

fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

pub fn page(closed: &Closed) -> String {
    let cause = match closed.cause {
        Cause::Planned => "planned",
        Cause::Restarting => "restarting",
        Cause::Down => "down",
    };
    let mut page = String::with_capacity(PAGE.len() + 512);
    let mut rest = PAGE;
    while let Some(start) = rest.find('{') {
        page.push_str(&rest[..start]);
        rest = &rest[start..];
        let name = rest[1..].find('}').map(|end| &rest[1..=end]);
        let value = match name {
            Some("cause") => cause.to_string(),
            Some("headline") => escape(&closed.headline),
            Some("note") => escape(&closed.note),
            Some("late_hidden") => if closed.late.is_some() { "" } else { " hidden" }.to_string(),
            Some("late") => escape(closed.late.as_deref().unwrap_or("")),
            Some("eta") => escape(&closed.eta),
            _ => {
                page.push('{');
                rest = &rest[1..];
                continue;
            }
        };
        page.push_str(&value);
        rest = &rest[name.map_or(0, str::len) + 2..];
    }
    page.push_str(rest);
    page
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bakery::{Bakery, SECOND, Scope, Status};

    fn planned(reason: &str) -> Closed {
        let mut bakery = Bakery::default();
        bakery.plan(0, 120 * SECOND, Some(reason.to_string()));
        match bakery.status(0, Scope::Pages) {
            Status::Closed(closed) => closed,
            Status::Open => unreachable!(),
        }
    }

    #[test]
    fn the_page_shows_the_headline_the_reason_and_the_time_left() {
        let page = page(&planned("neue Streaming-Limits"));
        assert!(page.contains("<h1 id=\"headline\">Die Backstube ist kurz zu</h1>"));
        assert!(page.contains("Wir backen gerade etwas Neues: neue Streaming-Limits."));
        assert!(page.contains("Wieder da in etwa 2 Minuten"));
        assert!(page.contains("id=\"late\" hidden>"));
        assert!(!page.contains("{eta}") && !page.contains("{headline}"));
    }

    #[test]
    fn a_reason_that_looks_like_a_placeholder_stays_as_written() {
        assert!(page(&planned("{eta}")).contains("etwas Neues: {eta}."));
    }

    #[test]
    fn a_reason_cannot_write_markup_into_the_page() {
        let page = page(&planned("<script>alert(1)</script>"));
        assert!(page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!page.contains("<script>alert(1)"));
    }
}
