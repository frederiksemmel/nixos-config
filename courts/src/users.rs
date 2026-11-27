//! Users, for admins: who may use Courts, adding someone with a sign-in
//! link to send them, a new link, signing a person's devices out, making
//! someone an admin, and removing them with what is theirs.

use crate::auth::{self, Me};
use crate::db::{Db, Person};
use crate::pages::nav;
use crate::time;
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::content::Form;
use topcoat::router::error::{SeeOther, see_other};
use topcoat::router::{page, path_param, query_params, route};
use topcoat::view::{View, view};

const LONGEST_NAME: usize = 40;

path_param!(person_id);

#[query_params]
struct UsersQuery {
    person: Option<i64>,
    invite: Option<String>,
}

/// A person as the list shows them.
struct Row {
    person: Person,
    devices: usize,
    seen: Option<i64>,
    me: bool,
}

/// "2 devices · seen 3 h ago", or that they haven't signed in.
fn devices_line(row: &Row) -> String {
    match (row.devices, row.seen) {
        (0, _) => "Not signed in on any device".into(),
        (n, seen) => {
            let devices = if n == 1 {
                "1 device".to_string()
            } else {
                format!("{n} devices")
            };
            seen.map_or(devices.clone(), |at| format!("{devices} · seen {}", time::ago(at)))
        }
    }
}

fn token_ok(token: &str) -> bool {
    token.len() == 32 && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[page("/users")]
async fn users_page(cx: &Cx) -> Result<impl View> {
    let me = auth::person(cx)?;
    let query = query_params::<UsersQuery>(cx).map_err(|error| topcoat::Error::msg(error.to_string()))?;
    let db = app_context::<Db>(cx).clone();
    let people = db.people().await.map_err(topcoat::Error::msg)?;
    let devices = db.devices().await.map_err(topcoat::Error::msg)?;
    let rows: Vec<Row> = people
        .into_iter()
        .map(|person| {
            let theirs: Vec<_> = devices.iter().filter(|d| d.person_id == person.id).collect();
            Row {
                devices: theirs.len(),
                seen: theirs.iter().map(|d| d.seen_at).max(),
                me: person.id == me.id,
                person,
            }
        })
        .collect();
    let origin = auth::origin(cx);
    let invite = query.invite.clone().filter(|token| token_ok(token)).and_then(|token| {
        let person = rows.iter().find(|row| Some(row.person.id) == query.person)?;
        Some((person.person.name.clone(), format!("{origin}/invite/{token}"), token))
    });
    let admins = rows.iter().filter(|row| row.person.admin).count();
    Ok(view! {
        nav(current: "users")
        <article class="page users">
            <h1>"Users"</h1>
            <p class="lede">"Who can use Courts. Each person signs in through a link you send them and has their own watches and notifications. Admins also see Users and Feedback."</p>
            if let Some((name, url, token)) = &invite {
                <section class="invite" id="invite">
                    <img src=(format!("/invites/{token}/qr.svg")) width="240" height="240" alt=(format!("QR code of {name}'s sign-in link"))>
                    <div class="invite-text">
                        <p><strong>(format!("Send this to {name}."))</strong>(format!(" It works once, within a week, and keeps that device signed in as {name}. For another device of theirs, make another link."))</p>
                        <p class="invite-link"><code>(url.as_str())</code></p>
                        <div class="row">
                            <button type="button" class="pill" data-share=(url.as_str())>"Share link"</button>
                            <button type="button" class="pill quiet" data-copy-link=(url.as_str())>"Copy"</button>
                        </div>
                    </div>
                </section>
            }
            <ul class="people">
                for row in &rows {
                    <li class="person">
                        <div class="person-who">
                            <p class="person-name">
                                <strong>(row.person.name.as_str())</strong>
                                if row.me {
                                    <span class="quiet">" (you)"</span>
                                }
                                if row.person.admin {
                                    <span class="tag">"Admin"</span>
                                }
                            </p>
                            <p class="meta">(devices_line(row))</p>
                        </div>
                        <div class="person-actions">
                            <form method="post" action=(format!("/users/{}/invite", row.person.id))>
                                <button class="pill quiet" type="submit">"Sign-in link"</button>
                            </form>
                            if !row.me && row.devices > 0 {
                                <form method="post" action=(format!("/users/{}/sign-out", row.person.id))>
                                    <button class="pill quiet" type="submit">"Sign out"</button>
                                </form>
                            }
                            if !row.me && !(row.person.admin && admins <= 1) {
                                <form method="post" action=(format!("/users/{}/admin", row.person.id))>
                                    <button class="pill quiet" type="submit">(if row.person.admin { "Drop admin" } else { "Make admin" })</button>
                                </form>
                                <details class="confirm">
                                    <summary class="pill quiet">"Remove"</summary>
                                    <form method="post" action=(format!("/users/{}/remove", row.person.id))>
                                        <p>(format!("{} is signed out everywhere and their watches go too.", row.person.name))</p>
                                        <button class="pill danger" type="submit">(format!("Remove {}", row.person.name))</button>
                                    </form>
                                </details>
                            }
                        </div>
                    </li>
                }
            </ul>
            <h2>"Add someone"</h2>
            <form class="new-person" method="post" action="/users/add">
                <label>"Name"
                    <input type="text" name="name" required=(true) maxlength="40" autocomplete="off" enterkeyhint="done">
                </label>
                <label class="check"><input type="checkbox" name="admin" value="1"> "Admin"</label>
                <button class="pill" type="submit">"Add and make a sign-in link"</button>
            </form>
        </article>
    })
}

fn tidy(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(LONGEST_NAME)
        .collect()
}

#[derive(serde::Deserialize)]
struct NewPerson {
    name: String,
    admin: Option<String>,
}

async fn invite_for(cx: &Cx, person: i64) -> Result<SeeOther> {
    let (token, _) = auth::invite(cx, person).await?;
    Ok(see_other(format!("/users?person={person}&invite={token}#invite")))
}

#[route(POST "/users/add")]
async fn add_person(cx: &Cx, Form(new): Form<NewPerson>) -> Result<SeeOther> {
    let name = tidy(&new.name);
    if name.is_empty() {
        return Ok(see_other("/users"));
    }
    let person = app_context::<Db>(cx)
        .add_person(&name, new.admin.is_some(), time::now())
        .await
        .map_err(topcoat::Error::msg)?;
    invite_for(cx, person.id).await
}

/// The person the path names, when it isn't the admin acting.
async fn other(cx: &Cx, me: &Me) -> Result<Option<Person>> {
    let Ok(id) = path_param::<PersonId>(cx).parse::<i64>() else {
        return Ok(None);
    };
    let found = app_context::<Db>(cx).person(id).await.map_err(topcoat::Error::msg)?;
    Ok(found.filter(|person| person.id != me.id))
}

#[route(POST "/users/{person_id}/invite")]
async fn person_invite(cx: &Cx) -> Result<SeeOther> {
    let Ok(id) = path_param::<PersonId>(cx).parse::<i64>() else {
        return Ok(see_other("/users"));
    };
    match app_context::<Db>(cx).person(id).await.map_err(topcoat::Error::msg)? {
        Some(person) => invite_for(cx, person.id).await,
        None => Ok(see_other("/users")),
    }
}

#[route(POST "/users/{person_id}/sign-out")]
async fn person_sign_out(cx: &Cx) -> Result<SeeOther> {
    let me = auth::person(cx)?;
    if let Some(person) = other(cx, &me).await? {
        app_context::<Db>(cx)
            .sign_out(person.id)
            .await
            .map_err(topcoat::Error::msg)?;
    }
    Ok(see_other("/users"))
}

#[route(POST "/users/{person_id}/admin")]
async fn person_admin(cx: &Cx) -> Result<SeeOther> {
    let me = auth::person(cx)?;
    if let Some(person) = other(cx, &me).await? {
        app_context::<Db>(cx)
            .set_admin(person.id, !person.admin)
            .await
            .map_err(topcoat::Error::msg)?;
    }
    Ok(see_other("/users"))
}

#[route(POST "/users/{person_id}/remove")]
async fn person_remove(cx: &Cx) -> Result<SeeOther> {
    let me = auth::person(cx)?;
    if let Some(person) = other(cx, &me).await? {
        app_context::<Db>(cx)
            .remove_person(person.id)
            .await
            .map_err(topcoat::Error::msg)?;
    }
    Ok(see_other("/users"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_tidied_and_kept_short() {
        assert_eq!(tidy("  Anna \t Muster "), "Anna Muster");
        assert_eq!(tidy(&"y".repeat(80)).len(), 40);
        assert_eq!(tidy("   "), "");
        assert!(token_ok("abcdefghijklmnopqrstuvwxyz012345"));
        assert!(!token_ok("abc"));
        assert!(!token_ok("abcdefghijklmnopqrstuvwxyz01234<"));
    }
}
