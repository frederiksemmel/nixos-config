//! What a bake-off sheet says and offers, for every screen: the phone draws
//! it as it comes, and the TV takes its words. The oven itself, which runs on
//! each screen's own clock, is the only part a screen decides.

use super::bake::GAME;
use super::{Ending, RESTING, Side, View, minutes};
use crate::overlay;
use serde::Serialize;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Sheet {
    pub title: String,
    pub detail: String,
    /// A line under the buttons, such as how long is left to answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The buttons in order; the first is the one to press.
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Action {
    pub label: &'static str,
    pub strong: bool,
    pub does: Does,
}

/// What a button does on the screen that shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Does {
    /// Bake against the challenger.
    Accept,
    /// Stop watching instead of baking.
    DropOff,
    /// The challenger goes away before the bake.
    GiveUp,
    /// Start watching from the place won.
    Watch,
    /// Back to the paused film.
    Resume,
    /// Another practice bake.
    Again,
    Close,
}

const fn action(label: &'static str, strong: bool, does: Does) -> Action {
    Action { label, strong, does }
}

impl Sheet {
    fn new(title: impl Into<String>, detail: impl Into<String>, actions: Vec<Action>) -> Self {
        Self {
            title: title.into(),
            detail: detail.into(),
            note: None,
            actions,
        }
    }

    /// The sheet for `side` of a bake-off seen as `view`.
    pub fn of(view: &View, side: Side, practice: bool) -> Self {
        match view {
            View::Asked {
                challenger,
                defender,
                answer_within_ms,
                ..
            } => match side {
                Side::Defender => Self {
                    note: Some(format!(
                        "Answer within {}",
                        overlay::clock(Duration::from_millis(*answer_within_ms).as_secs_f64())
                    )),
                    ..Self::new(
                        format!("{GAME}!"),
                        format!(
                            "{challenger} fordert dich zum {GAME} heraus. Pull three Semmeln out of the oven when they're golden: the better baker keeps watching."
                        ),
                        vec![
                            action("Bake", true, Does::Accept),
                            action("Stop watching", false, Does::DropOff),
                        ],
                    )
                },
                Side::Challenger => Self::new(
                    format!("Waiting for {defender}…"),
                    "They can bake against you or stop watching.",
                    vec![action("Give up", false, Does::GiveUp)],
                ),
            },
            View::Baking { pressed: true, .. } => Self::new("Out of the oven", "Waiting for the other baker…", vec![]),
            View::Baking {
                challenger, defender, ..
            } => Self::new(GAME, format!("{challenger} against {defender}"), vec![]),
            View::Done {
                challenger,
                defender,
                winner,
                ending,
                ..
            } => done(challenger, defender, side, *winner, *ending, practice),
        }
    }
}

fn done(challenger: &str, defender: &str, side: Side, winner: Option<Side>, ending: Ending, practice: bool) -> Sheet {
    let other = match side {
        Side::Challenger => defender,
        Side::Defender => challenger,
    };
    let won = winner == Some(side);
    if practice {
        let title = if won {
            format!("You beat the {other}")
        } else {
            format!("The {other} bakes better")
        };
        return Sheet::new(
            title,
            "Just practice: nobody's place was at stake.",
            vec![
                action("Bake again", true, Does::Again),
                action("Close", false, Does::Close),
            ],
        );
    }
    let resting = format!(
        "Your dough rests for {} before you can challenge again.",
        minutes(RESTING)
    );
    let (title, detail) = match (winner, ending) {
        (None, _) => ("Nobody baked".to_string(), format!("{challenger} went away.")),
        (_, Ending::Baked) if won => (
            match side {
                Side::Challenger => "The oven is yours",
                Side::Defender => "You keep the oven",
            }
            .to_string(),
            "Fresh out of the oven: enjoy the film.".to_string(),
        ),
        (_, Ending::Baked) => (format!("{other} bakes better"), resting),
        (_, Ending::DroppedOff | Ending::SeatGone) if won => {
            ("The oven is yours".to_string(), format!("{other} stopped watching."))
        }
        (_, Ending::DroppedOff | Ending::SeatGone) => ("You handed over the oven".to_string(), String::new()),
        (_, Ending::NoAnswer) if won => ("The oven is yours".to_string(), format!("{other} didn't answer.")),
        (_, Ending::NoAnswer) => ("You didn't answer in time".to_string(), resting),
        (_, Ending::Withdrawn) if won => ("You keep the oven".to_string(), format!("{other} left the bake-off.")),
        (_, Ending::Withdrawn) => ("You left the bake-off".to_string(), resting),
    };
    let next = match side {
        Side::Defender if winner != Some(Side::Challenger) => action("Back to the film", true, Does::Resume),
        Side::Challenger if won => action("Watch", true, Does::Watch),
        _ => action("Close", false, Does::Close),
    };
    Sheet::new(title, detail, vec![next])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done_view(winner: Option<Side>, ending: Ending) -> View {
        View::Done {
            challenger: "Ramu".to_string(),
            defender: "Mami".to_string(),
            winner,
            ending,
            baked: [None, None],
        }
    }

    fn said(view: &View, side: Side) -> (String, String, Vec<&'static str>) {
        let sheet = Sheet::of(view, side, false);
        let labels = sheet.actions.iter().map(|action| action.label).collect();
        (sheet.title, sheet.detail, labels)
    }

    fn owned(title: &str, detail: &str, labels: &[&'static str]) -> (String, String, Vec<&'static str>) {
        (title.to_string(), detail.to_string(), labels.to_vec())
    }

    #[test]
    fn whoever_is_challenged_is_asked_to_bake_with_the_time_left_to_answer() {
        let asked = View::Asked {
            challenger: "Ramu".to_string(),
            defender: "Mami".to_string(),
            title: "Dune".to_string(),
            answer_within_ms: 59_400,
        };
        let sheet = Sheet::of(&asked, Side::Defender, false);
        assert_eq!(sheet.title, "Semmelstechen!");
        assert!(
            sheet.detail.starts_with("Ramu fordert dich zum Semmelstechen heraus."),
            "{}",
            sheet.detail
        );
        assert_eq!(sheet.note.as_deref(), Some("Answer within 0:59"));
        assert_eq!(
            sheet.actions,
            vec![
                action("Bake", true, Does::Accept),
                action("Stop watching", false, Does::DropOff)
            ]
        );
        assert_eq!(
            said(&asked, Side::Challenger),
            owned(
                "Waiting for Mami…",
                "They can bake against you or stop watching.",
                &["Give up"]
            )
        );
    }

    #[test]
    fn the_oven_names_both_bakers_until_this_side_has_pressed() {
        let baking = |pressed| View::Baking {
            challenger: "Ramu".to_string(),
            defender: "Mami".to_string(),
            starts_in_ms: 3000,
            golden_ms: [900, 1200, 1500],
            length_ms: 6000,
            bias_ms: Default::default(),
            pressed,
        };
        assert_eq!(
            said(&baking(false), Side::Challenger),
            owned("Semmelstechen", "Ramu against Mami", &[])
        );
        assert_eq!(
            said(&baking(true), Side::Defender),
            owned("Out of the oven", "Waiting for the other baker…", &[])
        );
    }

    #[test]
    fn a_baked_result_tells_each_side_what_it_means_and_what_comes_next() {
        let challenger_won = done_view(Some(Side::Challenger), Ending::Baked);
        assert_eq!(
            said(&challenger_won, Side::Challenger),
            owned(
                "The oven is yours",
                "Fresh out of the oven: enjoy the film.",
                &["Watch"]
            )
        );
        assert_eq!(
            said(&challenger_won, Side::Defender),
            owned(
                "Ramu bakes better",
                "Your dough rests for 10 minutes before you can challenge again.",
                &["Close"]
            )
        );
        let defender_won = done_view(Some(Side::Defender), Ending::Baked);
        assert_eq!(
            said(&defender_won, Side::Defender),
            owned(
                "You keep the oven",
                "Fresh out of the oven: enjoy the film.",
                &["Back to the film"]
            )
        );
        assert_eq!(
            said(&defender_won, Side::Challenger),
            owned(
                "Mami bakes better",
                "Your dough rests for 10 minutes before you can challenge again.",
                &["Close"]
            )
        );
    }

    #[test]
    fn a_bake_off_that_ends_without_baking_says_who_left_or_didnt_answer() {
        let cases = [
            (
                done_view(Some(Side::Challenger), Ending::DroppedOff),
                owned("The oven is yours", "Mami stopped watching.", &["Watch"]),
                owned("You handed over the oven", "", &["Close"]),
            ),
            (
                done_view(Some(Side::Challenger), Ending::SeatGone),
                owned("The oven is yours", "Mami stopped watching.", &["Watch"]),
                owned("You handed over the oven", "", &["Close"]),
            ),
            (
                done_view(Some(Side::Challenger), Ending::NoAnswer),
                owned("The oven is yours", "Mami didn't answer.", &["Watch"]),
                owned(
                    "You didn't answer in time",
                    "Your dough rests for 10 minutes before you can challenge again.",
                    &["Close"],
                ),
            ),
            (
                done_view(Some(Side::Defender), Ending::Withdrawn),
                owned(
                    "You left the bake-off",
                    "Your dough rests for 10 minutes before you can challenge again.",
                    &["Close"],
                ),
                owned("You keep the oven", "Ramu left the bake-off.", &["Back to the film"]),
            ),
            (
                done_view(None, Ending::Withdrawn),
                owned("Nobody baked", "Ramu went away.", &["Close"]),
                owned("Nobody baked", "Ramu went away.", &["Back to the film"]),
            ),
        ];
        for (view, challenger, defender) in cases {
            assert_eq!(said(&view, Side::Challenger), challenger, "{view:?}");
            assert_eq!(said(&view, Side::Defender), defender, "{view:?}");
        }
    }

    #[test]
    fn a_practice_bake_against_the_master_offers_another_round() {
        let view = View::Done {
            challenger: "Ramu".to_string(),
            defender: "Bäckermeister".to_string(),
            winner: Some(Side::Defender),
            ending: Ending::Baked,
            baked: [None, None],
        };
        let sheet = Sheet::of(&view, Side::Challenger, true);
        assert_eq!(sheet.title, "The Bäckermeister bakes better");
        assert_eq!(sheet.detail, "Just practice: nobody's place was at stake.");
        assert_eq!(
            sheet.actions,
            vec![
                action("Bake again", true, Does::Again),
                action("Close", false, Does::Close)
            ]
        );
    }

    #[test]
    fn a_sheet_reaches_the_phone_as_words_and_named_buttons() {
        let sheet = Sheet::of(
            &done_view(Some(Side::Challenger), Ending::Baked),
            Side::Challenger,
            false,
        );
        assert_eq!(
            serde_json::to_value(&sheet).unwrap(),
            serde_json::json!({
                "title": "The oven is yours",
                "detail": "Fresh out of the oven: enjoy the film.",
                "actions": [{ "label": "Watch", "strong": true, "does": "watch" }]
            })
        );
    }
}
