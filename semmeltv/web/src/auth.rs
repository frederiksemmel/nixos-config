use crate::api::{Api, Choice, Me, Member, Person};
use crate::layout::back;
use topcoat::Result;
use topcoat::context::{Cx, try_request_context};
use topcoat::router::error::redirect;
use topcoat::router::request::uri;
use topcoat::router::{Body, Layer, LayerFuture, Next, Path, page, path_param};
use topcoat::runtime::link;
use topcoat::view::{View, attributes, component, view};

path_param!(token);

const OPEN: [&str; 4] = ["/profiles", "/invite/", "/_topcoat/assets/", "/_topcoat/fonts/"];
const FOR_ADMINS: [&str; 3] = ["/storage", "/downloads", "/people"];

pub fn me(cx: &Cx) -> Option<&Me> {
    try_request_context::<Me>(cx)
}

pub fn admin(cx: &Cx) -> Result<()> {
    match me(cx) {
        Some(me) if me.person.admin() => Ok(()),
        _ => Err(topcoat::Error::msg("Only an admin can do that")),
    }
}

fn open(path: &str) -> bool {
    path == "/manifest.webmanifest"
        || OPEN.iter().any(|open| if open.ends_with('/') { path.starts_with(open) } else { path == *open })
}

fn for_admins(path: &str) -> bool {
    FOR_ADMINS.iter().any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

fn sign_in_link(path_and_query: &str) -> String {
    if path_and_query == "/" || path_and_query.is_empty() || path_and_query.starts_with("/_topcoat/") {
        "/profiles".to_string()
    } else {
        format!("/profiles?next={}", encode(path_and_query))
    }
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

pub struct SignedIn;

impl Layer for SignedIn {
    fn path(&self) -> Option<&Path> {
        None
    }

    fn handle<'a>(&'a self, cx: &'a Cx, body: Body, next: Next<'a>) -> LayerFuture<'a> {
        Box::pin(async move {
            let target = uri(cx);
            let path = target.path().to_string();
            let wanted = target.path_and_query().map_or(path.clone(), |found| found.as_str().to_string());
            let found = Api::of(cx).me().await;
            match found {
                Ok(Some(me)) => {
                    if for_admins(&path) && !me.person.admin() {
                        return Err(redirect("/").into());
                    }
                    let cx = cx.with(me);
                    next.run(&cx, body).await
                }
                Ok(None) if open(&path) => next.run(cx, body).await,
                Ok(None) => Err(redirect(sign_in_link(&wanted)).into()),
                Err(error) => {
                    eprintln!("sign-in: {error}");
                    next.run(cx, body).await
                }
            }
        })
    }
}

#[component]
pub async fn avatar(person: &Person, size: &'static str) -> Result<impl View> {
    Ok(view! {
        <span class=(format!("avatar {size}")) style=(format!("--avatar:{}", person.color)) aria-hidden="true">(person.initial())</span>
    })
}

#[component]
pub async fn profile_button(cx: &Cx) -> Result<impl View> {
    let person = me(cx).map(|me| me.person.clone());
    Ok(view! {
        if let Some(person) = person {
            link(href: "/profiles", attrs: attributes! { cx => class="top-link profile-link" aria-label=(format!("{}: switch profile", person.name)) },
                avatar(person: &person, size: "small")
            )
        }
    })
}

#[component]
async fn choice_button(choice: &Choice) -> Result<impl View> {
    let person = &choice.person;
    Ok(view! {
        <li>
            <button type="button" class=(if choice.active { "profile-choice active" } else { "profile-choice" })
                data-profile=(person.id.to_string())
                data-name=(person.name.as_str())
                data-passkey=(choice.passkey.to_string())
                data-password=(choice.password.to_string())
                data-signed-in=(choice.signed_in.to_string())
                data-admin=(person.admin().to_string())
                aria-current=(if choice.active { "true" } else { "false" })>
                avatar(person: person, size: "large")
                <span class="profile-name">(person.name.as_str())</span>
                if choice.active {
                    <small class="profile-state">"Watching now"</small>
                } else if choice.signed_in && !person.admin() {
                    <small class="profile-state">"On this device"</small>
                } else if !choice.password && !choice.passkey {
                    <small class="profile-state">"Not set up yet"</small>
                }
            </button>
        </li>
    })
}

#[page("/profiles")]
async fn profiles_page(cx: &Cx) -> Result<impl View> {
    let choices = Api::of(cx).choices().await;
    let signed_in = me(cx).is_some();
    let admin = me(cx).is_some_and(|me| me.person.admin());
    Ok(view! {
        <article class="page sign-in">
            <h1>"Who's watching?"</h1>
            match choices {
                Ok(choices) => {
                    <ul class="profile-choices">
                        for choice in &choices {
                            choice_button(choice: choice)
                        }
                    </ul>
                },
                Err(error) => <p class="setting-note">(error)</p>,
            }
            <form class="password-form" id="password-form" hidden=(true)>
                <input type="text" name="username" autocomplete="username" hidden=(true)>
                <label class="password-label" for="password-input">"Password for " <b id="password-name"></b></label>
                <input class="password-input" id="password-input" type="password" name="password" autocomplete="current-password" required=(true)>
                <div class="password-actions">
                    <button type="submit" class="pill">"Sign in"</button>
                    <button type="button" class="pill quiet" id="password-passkey" hidden=(true)>"Use passkey"</button>
                </div>
            </form>
            <p class="sign-in-note" id="sign-in-note" role="status"></p>
            if signed_in {
                <nav class="sign-in-links">
                    link(href: "/account", attrs: attributes! { cx => class="pill quiet" }, "Your sign-in")
                    if admin {
                        link(href: "/people", attrs: attributes! { cx => class="pill quiet" }, "People")
                    }
                    <button type="button" class="pill quiet" id="sign-out">"Sign out"</button>
                </nav>
            } else {
                <p class="sign-in-help">"New here? Ask Frederik for an invite link."</p>
            }
        </article>
    })
}

#[page("/invite/{token}")]
async fn invite_page(cx: &Cx) -> Result<impl View> {
    let token = path_param::<Token>(cx).to_string();
    let invitee = Api::of(cx).invitee(&token).await;
    Ok(view! {
        <article class="page sign-in invite">
            match invitee {
                Ok(Ok(person)) => {
                    avatar(person: &person, size: "large")
                    <h1>"Hi " (person.name.as_str())</h1>
                    <p class="sign-in-help">"This link signs you in to SemmelTV on this device. Then you set up a passkey, so Face ID or your fingerprint signs you in next time, and a password for when a passkey isn't at hand."</p>
                    <button type="button" class="pill" id="redeem" data-token=(token.as_str())>"Continue as " (person.name.as_str())</button>
                    <p class="sign-in-note" id="sign-in-note" role="status"></p>
                },
                Ok(Err(problem)) => {
                    <h1>"Invite link"</h1>
                    <p class="sign-in-help">(problem) ". Ask Frederik for a new one."</p>
                    link(href: "/profiles", attrs: attributes! { cx => class="pill quiet" }, "Sign in")
                },
                Err(error) => <p class="setting-note">(error)</p>,
            }
        </article>
    })
}

fn day(seconds: u64) -> String {
    let days = seconds / 86_400;
    let (year, month, day) = civil(days as i64);
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{day} {} {year}", MONTHS[(month - 1) as usize])
}

fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

#[page("/account")]
async fn account_page(cx: &Cx) -> Result<impl View> {
    let Some(me) = me(cx).cloned() else {
        return Err(redirect("/profiles").into());
    };
    let here = me.passkeys.iter().filter(|passkey| passkey.here).count();
    Ok(view! {
        <article class="page account">
            back(href: "/profiles")
            <div class="account-head">
                avatar(person: &me.person, size: "large")
                <h1>(me.person.name.as_str())</h1>
                <p class="setting-note">(if me.person.admin() { "Admin: everything, including downloads, storage and people" } else { "Watches, plays and uses the remote" })</p>
            </div>
            <p class="setting-note">"Safari, the SemmelTV app on your home screen and links opened from inside other apps each sign in on their own. Your passkey and password work in all of them."</p>
            <section class="settings">
                <h2>"Passkeys"</h2>
                <div class="setting-group" id="passkeys">
                    for passkey in &me.passkeys {
                        <div class="setting setting-row">
                            <span class="setting-name">(passkey.name.as_str())
                                <small>
                                    "Added " (day(passkey.created_at))
                                    if !passkey.here { " · for the other address" }
                                </small>
                            </span>
                            <button type="button" class="pill quiet" data-post=(format!("/api/auth/passkeys/{}", passkey.id)) data-method="DELETE" data-done="hide">"Remove"</button>
                        </div>
                    }
                    <div class="setting setting-row">
                        <span class="setting-name">(if here == 0 { "No passkey for this address yet" } else { "Add another device" })
                            <small>"Face ID, Touch ID or your phone's screen lock signs you in"</small>
                        </span>
                        <button type="button" class="pill" id="add-passkey">"Add a passkey"</button>
                    </div>
                </div>
            </section>
            <section class="settings">
                <h2>"Password"</h2>
                <form class="setting-group password-set" id="password-set">
                    <input type="text" name="username" autocomplete="username" value=(me.person.name.as_str()) hidden=(true)>
                    <div class="setting">
                        <label class="setting-name" for="new-password">(if me.password { "Change your password" } else { "Set a password" })
                            <small>"Signs you in where your passkey isn't: another person's device, or when Face ID refuses. At least 8 characters."</small>
                        </label>
                        <input class="password-input" id="new-password" type="password" name="password" autocomplete="new-password" minlength="8" required=(true)>
                    </div>
                    <div class="setting setting-row">
                        <span class="setting-name"></span>
                        <button type="submit" class="pill">"Save password"</button>
                    </div>
                </form>
            </section>
            <section class="settings">
                <div class="setting-group">
                    <div class="setting setting-row">
                        <span class="setting-name">"Sign out" <small>"Only on this device"</small></span>
                        <button type="button" class="pill quiet" id="sign-out">"Sign out"</button>
                    </div>
                </div>
            </section>
        </article>
    })
}

#[component]
async fn member_row(member: &Member, own: bool) -> Result<impl View> {
    let person = &member.person;
    let mut facts = Vec::new();
    facts.push(if person.admin() { "Admin".to_string() } else { "Viewer".to_string() });
    facts.push(match member.passkeys {
        0 => "no passkey".to_string(),
        1 => "1 passkey".to_string(),
        count => format!("{count} passkeys"),
    });
    facts.push(if member.password { "password".to_string() } else { "no password".to_string() });
    match member.sessions {
        0 => facts.push("signed in nowhere".to_string()),
        1 => facts.push("signed in on 1 device".to_string()),
        count => facts.push(format!("signed in on {count} devices")),
    }
    let invite = member.invite_expires_at.map(|until| format!("Invite link open until {}", day(until)));
    Ok(view! {
        <div class="member" data-member=(person.id.to_string())>
            avatar(person: person, size: "small")
            <div class="member-text">
                <b>(person.name.as_str())</b>
                <small>(facts.join(" · "))</small>
                if let Some(invite) = invite {
                    <small>(invite)</small>
                }
                <output class="invite-link" hidden=(true)></output>
            </div>
            <div class="member-actions">
                <button type="button" class="pill" data-invite=(person.id.to_string()) data-name=(person.name.as_str())>"Invite link"</button>
                if member.sessions > 0 {
                    <button type="button" class="pill quiet" data-post=(format!("/api/auth/members/{}/sign-out", person.id)) data-done="hide">"Sign out everywhere"</button>
                }
                if !own {
                    <button type="button" class="pill quiet danger" data-remove=(person.id.to_string()) data-name=(person.name.as_str())>"Remove"</button>
                }
            </div>
        </div>
    })
}

#[page("/people")]
async fn people_page(cx: &Cx) -> Result<impl View> {
    admin(cx)?;
    let own = me(cx).map(|me| me.person.id);
    let members = Api::of(cx).members().await;
    Ok(view! {
        <article class="page people">
            back(href: "/profiles")
            <h1>"People"</h1>
            <p class="setting-note">"Everyone signs in with an invite link once, then with a passkey or their password. Viewers watch, play and use the remote; only admins download, remove titles, pair the TV and manage people."</p>
            match members {
                Ok(members) => {
                    <section class="settings">
                        <div class="setting-group members">
                            for member in &members {
                                member_row(member: member, own: Some(member.person.id) == own)
                            }
                        </div>
                    </section>
                },
                Err(error) => <p class="setting-note">(error)</p>,
            }
            <section class="settings">
                <h2>"Add someone"</h2>
                <form class="setting-group add-member" id="add-member">
                    <div class="setting setting-row">
                        <input class="password-input" name="name" placeholder="Name" maxlength="40" required=(true) autocomplete="off">
                        <select name="role" class="role-select">
                            <option value="viewer" selected=(true)>"Viewer"</option>
                            <option value="admin">"Admin"</option>
                        </select>
                        <button type="submit" class="pill">"Add"</button>
                    </div>
                </form>
            </section>
        </article>
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_pages_and_assets_are_open_and_everything_else_is_not() {
        assert!(open("/profiles"));
        assert!(open("/invite/abc"));
        assert!(open("/_topcoat/assets/style.css"));
        assert!(open("/manifest.webmanifest"));
        assert!(!open("/"));
        assert!(!open("/movies/dune"));
        assert!(!open("/_topcoat/runtime/procedures/play"));
        assert!(!open("/profilesx"));
        assert!(!open("/account"));
    }

    #[test]
    fn storage_downloads_and_people_are_for_admins() {
        assert!(for_admins("/storage"));
        assert!(for_admins("/downloads"));
        assert!(for_admins("/people"));
        assert!(!for_admins("/storagex"));
        assert!(!for_admins("/settings"));
    }

    #[test]
    fn signing_in_returns_to_where_it_was_asked_for() {
        assert_eq!(sign_in_link("/"), "/profiles");
        assert_eq!(sign_in_link("/_topcoat/runtime/shards/cc7d"), "/profiles");
        assert_eq!(sign_in_link("/movies/dune?x=1&y=2"), "/profiles?next=/movies/dune%3Fx%3D1%26y%3D2");
    }

    #[test]
    fn days_are_written_the_way_people_say_them() {
        assert_eq!(day(0), "1 Jan 1970");
        assert_eq!(day(1_794_614_400), "14 Nov 2026");
    }
}
