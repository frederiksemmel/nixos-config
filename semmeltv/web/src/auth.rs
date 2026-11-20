use crate::api::{Api, CONTROL_TV, Choice, DOWNLOAD, HEADPHONES, Me, Member, Person};
use crate::layout::back;
use topcoat::Result;
use topcoat::context::{Cx, try_request_context};
use topcoat::router::error::redirect;
use topcoat::router::request::uri;
use topcoat::router::{Body, Layer, LayerFuture, Next, Path, page, path_param};
use topcoat::runtime::link;
use topcoat::view::{View, attributes, component, view};

path_param!(token);

const OPEN: [&str; 5] = [
    "/profiles",
    "/invite/",
    "/build",
    "/_topcoat/assets/",
    "/_topcoat/fonts/",
];
const FOR_ADMINS: [&str; 4] = ["/storage", "/people", "/feedback", "/usage"];
const FOR_DOWNLOADERS: [&str; 1] = ["/downloads"];
const BEFORE_SETUP: [&str; 3] = ["/welcome", "/account", "/_topcoat"];

pub fn me(cx: &Cx) -> Option<&Me> {
    try_request_context::<Me>(cx)
}

pub fn controls_tv(cx: &Cx) -> bool {
    me(cx).is_some_and(|me| me.person.controls_tv())
}

pub fn uses_headphones(cx: &Cx) -> bool {
    me(cx).is_some_and(|me| me.person.uses_headphones())
}

pub fn downloads(cx: &Cx) -> bool {
    me(cx).is_some_and(|me| me.person.downloads())
}

pub fn may_download(cx: &Cx) -> Result<()> {
    if downloads(cx) {
        Ok(())
    } else {
        Err(topcoat::Error::msg("Only someone allowed to download can do that"))
    }
}

pub fn admin(cx: &Cx) -> Result<()> {
    match me(cx) {
        Some(me) if me.person.admin() => Ok(()),
        _ => Err(topcoat::Error::msg("Only an admin can do that")),
    }
}

fn open(path: &str) -> bool {
    path == "/manifest.webmanifest"
        || path == "/sw.js"
        || OPEN.iter().any(|open| {
            if open.ends_with('/') {
                path.starts_with(open)
            } else {
                path == *open
            }
        })
}

fn under(prefixes: &[&str], path: &str) -> bool {
    prefixes
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

fn for_admins(path: &str) -> bool {
    under(&FOR_ADMINS, path)
}

fn for_downloaders(path: &str) -> bool {
    under(&FOR_DOWNLOADERS, path)
}

fn set_up(me: &Me) -> bool {
    me.password || !me.passkeys.is_empty()
}

fn waits_for_setup(path: &str) -> bool {
    !open(path) && !under(&BEFORE_SETUP, path)
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
            let wanted = target
                .path_and_query()
                .map_or(path.clone(), |found| found.as_str().to_string());
            let found = Api::of(cx).me().await;
            match found {
                Ok(Some(me)) => {
                    if (for_admins(&path) && !me.person.admin()) || (for_downloaders(&path) && !me.person.downloads()) {
                        return Err(redirect("/").into());
                    }
                    if !set_up(&me) && waits_for_setup(&path) {
                        return Err(redirect("/welcome").into());
                    }
                    if let Some(token) = path.strip_prefix("/invite/") {
                        let invitee = Api::of(cx).invitee(token).await;
                        let spent = match &invitee {
                            Ok(Ok(person)) => person.id == me.person.id,
                            Ok(Err(_)) => true,
                            Err(_) => false,
                        };
                        if spent {
                            return Err(redirect("/").into());
                        }
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
                    <p class="sign-in-help">"This link signs you in to SemmelTV on this device. Then you choose how you sign in next time: a passkey, so Face ID or your fingerprint does it, a password, or both."</p>
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
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
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

#[page("/welcome")]
async fn welcome_page(cx: &Cx) -> Result<impl View> {
    let Some(me) = me(cx).cloned() else {
        return Err(redirect("/profiles").into());
    };
    let done = set_up(&me);
    Ok(view! {
        <article class="page account welcome">
            <div class="account-head">
                avatar(person: &me.person, size: "large")
                <h1>"Hi " (me.person.name.as_str())</h1>
                if done {
                    <p class="setting-note">"You're set up. Next time you sign in with what you chose here."</p>
                } else {
                    <p class="setting-note">"One more step: choose how you sign in next time. A passkey lets Face ID or your fingerprint do it; a password works on any device. Set up at least one."</p>
                }
            </div>
            <section class="settings" id="welcome-passkey" hidden=(true)>
                <h2>"Passkey"</h2>
                <div class="setting-group">
                    <div class="setting setting-row">
                        <span class="setting-name">(if me.passkeys.is_empty() { "No passkey yet" } else { "Passkey added" })
                            <small>"Face ID, Touch ID or your phone's screen lock signs you in"</small>
                        </span>
                        if me.passkeys.is_empty() {
                            <button type="button" class="pill" id="add-passkey">"Add a passkey"</button>
                        }
                    </div>
                </div>
            </section>
            <section class="settings">
                <h2>"Password"</h2>
                <form class="setting-group password-set" id="password-set" data-then="reload">
                    <input type="text" name="username" autocomplete="username" value=(me.person.name.as_str()) hidden=(true)>
                    <div class="setting">
                        <label class="setting-name" for="new-password">(if me.password { "Password set" } else { "Set a password" })
                            <small>"At least 8 characters"</small>
                        </label>
                        <input class="password-input" id="new-password" type="password" name="password" autocomplete="new-password" minlength="8" required=(true)>
                    </div>
                    <div class="setting setting-row">
                        <span class="setting-name"></span>
                        <button type="submit" class=(if me.password { "pill quiet" } else { "pill" })>(if me.password { "Change password" } else { "Save password" })</button>
                    </div>
                </form>
            </section>
            if done {
                <button type="button" class="pill welcome-done" id="welcome-done">"Start watching"</button>
            }
        </article>
    })
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
                <p class="setting-note">(role_note(&me.person))</p>
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

fn role_note(person: &Person) -> &'static str {
    match (person.admin(), person.controls_tv()) {
        (true, _) => "Admin: everything, including the TV, downloads, storage and people",
        (false, true) => "Watches, plays on the TV and uses its remote",
        (false, false) => "Watches on this phone or laptop; the TV's remote isn't yours on this TV",
    }
}

#[component]
async fn member_row(member: &Member, own: bool) -> Result<impl View> {
    let person = &member.person;
    let mut facts = Vec::new();
    facts.push(match (person.admin(), person.controls_tv()) {
        (true, _) => "Admin".to_string(),
        (false, true) => "Viewer, controls the TV".to_string(),
        (false, false) => "Viewer, can't control the TV".to_string(),
    });
    if !person.admin() {
        facts.push(if person.uses_headphones() {
            "uses headphones".to_string()
        } else {
            "no headphones".to_string()
        });
        facts.push(if person.downloads() {
            "downloads".to_string()
        } else {
            "no downloads".to_string()
        });
    }
    let control = format!("/api/auth/members/{}/capabilities/{CONTROL_TV}", person.id);
    let headphones = format!("/api/auth/members/{}/capabilities/{HEADPHONES}", person.id);
    let download = format!("/api/auth/members/{}/capabilities/{DOWNLOAD}", person.id);
    facts.push(match member.passkeys {
        0 => "no passkey".to_string(),
        1 => "1 passkey".to_string(),
        count => format!("{count} passkeys"),
    });
    facts.push(if member.password {
        "password".to_string()
    } else {
        "no password".to_string()
    });
    match member.sessions {
        0 => facts.push("signed in nowhere".to_string()),
        1 => facts.push("signed in on 1 device".to_string()),
        count => facts.push(format!("signed in on {count} devices")),
    }
    let invite = member
        .invite_expires_at
        .map(|until| format!("Invite link open until {}", day(until)));
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
                if !person.admin() {
                    if person.controls_tv() {
                        <button type="button" class="pill quiet" data-capability=(control) data-method="DELETE">"Take TV control away"</button>
                    } else {
                        <button type="button" class="pill" data-capability=(control) data-method="PUT">"Give TV control"</button>
                    }
                    if person.uses_headphones() {
                        <button type="button" class="pill quiet" data-capability=(headphones) data-method="DELETE">"Take headphones away"</button>
                    } else {
                        <button type="button" class="pill" data-capability=(headphones) data-method="PUT">"Give headphones"</button>
                    }
                    if person.downloads() {
                        <button type="button" class="pill quiet" data-capability=(download) data-method="DELETE">"Stop downloads"</button>
                    } else {
                        <button type="button" class="pill" data-capability=(download) data-method="PUT">"Let download"</button>
                    }
                }
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
            <p class="setting-note">"Everyone signs in with an invite link once, then with a passkey or their password. Viewers browse and watch on their own phone or laptop, while nothing else plays; only those you let control the TV play on it and use its remote, voice and sound, only those you give headphones pair and listen on Bluetooth headphones, and only those you let download find films and seasons and add them to the library. Admins do all that, and stop downloads, remove titles, pair the TV and manage people."</p>
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
    fn every_page_has_its_access() {
        let rows = [
            ("/profiles", "open"),
            ("/invite/abc", "open"),
            ("/_topcoat/assets/style.css", "open"),
            ("/manifest.webmanifest", "open"),
            ("/sw.js", "open"),
            ("/sw.jsx", "signed"),
            ("/", "signed"),
            ("/movies/dune", "signed"),
            ("/_topcoat/runtime/procedures/play", "signed"),
            ("/profilesx", "signed"),
            ("/account", "signed"),
            ("/settings", "signed"),
            ("/storagex", "signed"),
            ("/storage", "admin"),
            ("/downloads", "download"),
            ("/downloadsx", "signed"),
            ("/people", "admin"),
            ("/feedback", "admin"),
            ("/usage", "admin"),
            ("/usagex", "signed"),
            ("/welcome", "signed"),
        ];
        for (path, want) in rows {
            let got = match (open(path), for_admins(path), for_downloaders(path)) {
                (true, _, _) => "open",
                (false, true, _) => "admin",
                (false, false, true) => "download",
                (false, false, false) => "signed",
            };
            assert_eq!(got, want, "{path}");
        }
    }

    #[test]
    fn someone_not_set_up_reaches_only_the_setup_and_sign_in_pages() {
        let rows = [
            ("/", true),
            ("/movies/dune", true),
            ("/settings", true),
            ("/welcome", false),
            ("/account", false),
            ("/profiles", false),
            ("/invite/abc", false),
            ("/build", false),
            ("/_topcoat/runtime/procedures/play", false),
            ("/_topcoat/assets/style.css", false),
            ("/welcomex", true),
        ];
        for (path, sent_to_setup) in rows {
            assert_eq!(waits_for_setup(path), sent_to_setup, "{path}");
        }
    }

    #[test]
    fn a_passkey_or_a_password_is_enough_setup() {
        let person: Person =
            serde_json::from_value(serde_json::json!({"id": 2, "name": "Mami", "role": "viewer", "color": "#3E63DD"}))
                .unwrap();
        let mut me = Me {
            person,
            others: Vec::new(),
            passkeys: Vec::new(),
            password: false,
        };
        assert!(!set_up(&me));
        me.password = true;
        assert!(set_up(&me));
    }

    #[test]
    fn signing_in_returns_to_where_it_was_asked_for() {
        assert_eq!(sign_in_link("/"), "/profiles");
        assert_eq!(sign_in_link("/_topcoat/runtime/shards/cc7d"), "/profiles");
        assert_eq!(
            sign_in_link("/movies/dune?x=1&y=2"),
            "/profiles?next=/movies/dune%3Fx%3D1%26y%3D2"
        );
    }

    #[test]
    fn days_are_written_the_way_people_say_them() {
        assert_eq!(day(0), "1 Jan 1970");
        assert_eq!(day(1_794_614_400), "14 Nov 2026");
    }
}
