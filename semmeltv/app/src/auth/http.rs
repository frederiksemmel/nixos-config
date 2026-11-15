use super::gate::{self, Gate};
use super::{Auth, Person, Refusal, Resolved, Role};
use crate::profile::ProfileId;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::json;
use webauthn_rs::prelude::{PublicKeyCredential, RegisterPublicKeyCredential};

pub fn routes<S: Clone + Send + Sync + 'static>(gate: Gate) -> Router<S> {
    Router::new()
        .route("/api/auth/choices", get(choices))
        .route("/api/auth/me", get(me))
        .route("/api/auth/password", post(password_sign_in))
        .route("/api/auth/passkey/start", post(start_sign_in))
        .route("/api/auth/passkey/finish", post(finish_sign_in))
        .route("/api/auth/switch", post(switch))
        .route("/api/auth/sign-out", post(sign_out))
        .route("/api/auth/invite/{token}", get(peek_invite).post(redeem))
        .route("/api/auth/account/password", post(set_password))
        .route("/api/auth/passkeys/start", post(start_registration))
        .route("/api/auth/passkeys/finish", post(finish_registration))
        .route("/api/auth/passkeys/{id}", delete(remove_passkey))
        .route("/api/auth/members", get(members).post(create_member))
        .route("/api/auth/members/{id}", delete(delete_member))
        .route("/api/auth/members/{id}/invite", post(invite))
        .route("/api/auth/members/{id}/sign-out", post(sign_out_member))
        .with_state(gate)
}

fn status(refusal: &Refusal) -> StatusCode {
    match refusal {
        Refusal::UnknownProfile | Refusal::InviteUnknown => StatusCode::NOT_FOUND,
        Refusal::InviteUsed | Refusal::InviteExpired => StatusCode::GONE,
        Refusal::Throttled(_) => StatusCode::TOO_MANY_REQUESTS,
        Refusal::NotSignedIn | Refusal::WrongPassword | Refusal::PasskeyFailed => StatusCode::UNAUTHORIZED,
        Refusal::UnknownOrigin => StatusCode::FORBIDDEN,
        Refusal::Store(_) => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::UNPROCESSABLE_ENTITY,
    }
}

fn refused(refusal: Refusal) -> Response {
    let mut body = json!({ "error": refusal.to_string(), "ok": false });
    if let Refusal::NoPasskey(_) = refusal {
        body["password"] = json!(true);
    }
    (status(&refusal), Json(body)).into_response()
}

pub fn label(headers: &HeaderMap) -> String {
    let agent = headers.get(header::USER_AGENT).and_then(|value| value.to_str().ok()).unwrap_or("");
    let device = [
        ("iPad", "iPad"),
        ("iPhone", "iPhone"),
        ("Android", "Android"),
        ("Macintosh", "Mac"),
        ("Windows", "Windows"),
        ("CrOS", "Chromebook"),
        ("Linux", "Linux"),
    ]
    .iter()
    .find(|(needle, _)| agent.contains(needle))
    .map_or("Browser", |(_, name)| name);
    let browser = [("Firefox/", "Firefox"), ("Edg/", "Edge"), ("Chrome/", "Chrome"), ("Safari/", "Safari")]
        .iter()
        .find(|(needle, _)| agent.contains(needle))
        .map(|(_, name)| *name);
    match browser {
        Some(browser) => format!("{device} · {browser}"),
        None => device.to_string(),
    }
}

fn origin(auth: &Auth, headers: &HeaderMap) -> Result<String, Response> {
    let host = || {
        headers
            .get("x-forwarded-host")
            .or_else(|| headers.get(header::HOST))
            .and_then(|value| value.to_str().ok())
            .map(|host| format!("https://{host}"))
    };
    match gate::origin(headers).or_else(host) {
        Some(origin) if auth.allows_origin(&origin) => Ok(origin),
        _ => Err(refused(Refusal::UnknownOrigin)),
    }
}

async fn signed_in(gate: &Gate, resolved: &Resolved, person: Person, token: String) -> Response {
    let mut tokens = vec![token];
    for (known, old) in resolved.active.iter().chain(&resolved.others) {
        if known.id == person.id {
            if let Err(refusal) = gate.auth.sign_out(old).await {
                eprintln!("auth: ending a replaced session: {refusal}");
            }
        } else {
            tokens.push(old.clone());
        }
    }
    tokens.truncate(super::MOST_SESSIONS_ON_A_DEVICE);
    let mut response = Json(json!({ "ok": true, "person": person })).into_response();
    response.headers_mut().append(header::SET_COOKIE, gate::cookie(&tokens));
    response
}

async fn choices(State(gate): State<Gate>, Extension(resolved): Extension<Resolved>, headers: HeaderMap) -> Response {
    let here = gate::origin(&headers)
        .or_else(|| origin(&gate.auth, &headers).ok())
        .unwrap_or_default();
    match gate.auth.choices(&here).await {
        Ok(choices) => {
            let active = resolved.person().map(|person| person.id);
            let on_device = resolved.others.iter().map(|(person, _)| person.id).collect::<Vec<_>>();
            let profiles = choices
                .into_iter()
                .map(|choice| {
                    let mut value = json!(choice);
                    value["active"] = json!(Some(choice.person.id) == active);
                    value["signed_in"] =
                        json!(Some(choice.person.id) == active || on_device.contains(&choice.person.id));
                    value
                })
                .collect::<Vec<_>>();
            Json(json!({ "profiles": profiles })).into_response()
        }
        Err(refusal) => refused(refusal),
    }
}

async fn me(
    State(gate): State<Gate>,
    Extension(person): Extension<Person>,
    Extension(resolved): Extension<Resolved>,
    headers: HeaderMap,
) -> Response {
    let here = origin(&gate.auth, &headers).unwrap_or_default();
    let found = async {
        let passkeys = gate.auth.passkeys(person.id, &here).await?;
        let password = gate.auth.has_password(person.id).await?;
        Ok::<_, Refusal>((passkeys, password))
    };
    match found.await {
        Ok((passkeys, password)) => Json(json!({
            "person": person,
            "others": resolved.others.iter().map(|(person, _)| person).collect::<Vec<_>>(),
            "passkeys": passkeys,
            "password": password,
        }))
        .into_response(),
        Err(refusal) => refused(refusal),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasswordSignIn {
    profile: ProfileId,
    password: String,
}

async fn password_sign_in(
    State(gate): State<Gate>,
    Extension(resolved): Extension<Resolved>,
    headers: HeaderMap,
    Json(body): Json<PasswordSignIn>,
) -> Response {
    let label = label(&headers);
    let signed = async {
        let token = gate.auth.sign_in_with_password(body.profile, &body.password, &label).await?;
        Ok::<_, Refusal>((gate.auth.person(body.profile).await?, token))
    };
    match signed.await {
        Ok((person, token)) => signed_in(&gate, &resolved, person, token).await,
        Err(refusal) => refused(refusal),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartSignIn {
    profile: ProfileId,
}

async fn start_sign_in(State(gate): State<Gate>, headers: HeaderMap, Json(body): Json<StartSignIn>) -> Response {
    let origin = match origin(&gate.auth, &headers) {
        Ok(origin) => origin,
        Err(response) => return response,
    };
    match gate.auth.start_sign_in(&origin, body.profile).await {
        Ok((ceremony, options)) => Json(json!({ "ceremony": ceremony, "options": options })).into_response(),
        Err(refusal) => refused(refusal),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FinishSignIn {
    ceremony: String,
    credential: PublicKeyCredential,
}

async fn finish_sign_in(
    State(gate): State<Gate>,
    Extension(resolved): Extension<Resolved>,
    headers: HeaderMap,
    Json(body): Json<FinishSignIn>,
) -> Response {
    let origin = match origin(&gate.auth, &headers) {
        Ok(origin) => origin,
        Err(response) => return response,
    };
    let label = label(&headers);
    let signed = async {
        let (profile, token) = gate.auth.finish_sign_in(&origin, &body.ceremony, &body.credential, &label).await?;
        Ok::<_, Refusal>((gate.auth.person(profile).await?, token))
    };
    match signed.await {
        Ok((person, token)) => signed_in(&gate, &resolved, person, token).await,
        Err(refusal) => refused(refusal),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Switch {
    profile: ProfileId,
}

async fn switch(State(gate): State<Gate>, Extension(resolved): Extension<Resolved>, Json(body): Json<Switch>) -> Response {
    let Some((active, active_token)) = resolved.active.clone() else {
        return refused(Refusal::NotSignedIn);
    };
    if active.id == body.profile {
        return Json(json!({ "ok": true, "person": active })).into_response();
    }
    let target = resolved.others.iter().find(|(person, _)| person.id == body.profile).cloned();
    let Some((person, token)) = target.filter(|(person, _)| !person.admin()) else {
        let name = gate.auth.person(body.profile).await.map(|person| person.name);
        return match name {
            Ok(name) => (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": format!("Sign in as {name}"), "ok": false, "sign_in": true })),
            )
                .into_response(),
            Err(refusal) => refused(refusal),
        };
    };
    let mut tokens = vec![token];
    if active.admin() {
        if let Err(refusal) = gate.auth.sign_out(&active_token).await {
            eprintln!("auth: signing an admin out on a switch: {refusal}");
        }
    } else {
        tokens.push(active_token);
    }
    tokens.extend(
        resolved
            .others
            .iter()
            .filter(|(other, _)| other.id != person.id)
            .map(|(_, token)| token.clone()),
    );
    let mut response = Json(json!({ "ok": true, "person": person })).into_response();
    response.headers_mut().append(header::SET_COOKIE, gate::cookie(&tokens));
    response
}

async fn sign_out(State(gate): State<Gate>, Extension(resolved): Extension<Resolved>) -> Response {
    if let Some((_, token)) = &resolved.active {
        if let Err(refusal) = gate.auth.sign_out(token).await {
            return refused(refusal);
        }
    }
    let rest = resolved.others.iter().map(|(_, token)| token.clone()).collect::<Vec<_>>();
    let next = resolved.others.first().map(|(person, _)| person.clone());
    let mut response = Json(json!({ "ok": true, "person": next })).into_response();
    response.headers_mut().append(header::SET_COOKIE, gate::cookie(&rest));
    response
}

async fn peek_invite(State(gate): State<Gate>, Path(token): Path<String>) -> Response {
    match gate.auth.peek_invite(&token).await {
        Ok(person) => Json(json!({ "person": person })).into_response(),
        Err(refusal) => refused(refusal),
    }
}

async fn redeem(
    State(gate): State<Gate>,
    Extension(resolved): Extension<Resolved>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Response {
    let label = label(&headers);
    let signed = async {
        let (profile, session) = gate.auth.redeem(&token, &label).await?;
        Ok::<_, Refusal>((gate.auth.person(profile).await?, session))
    };
    match signed.await {
        Ok((person, token)) => signed_in(&gate, &resolved, person, token).await,
        Err(refusal) => refused(refusal),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewPassword {
    password: String,
}

async fn set_password(
    State(gate): State<Gate>,
    Extension(person): Extension<Person>,
    Json(body): Json<NewPassword>,
) -> Response {
    match gate.auth.set_password(person.id, &body.password).await {
        Ok(()) => Json(json!({ "ok": true, "message": "Password saved" })).into_response(),
        Err(refusal) => refused(refusal),
    }
}

async fn start_registration(
    State(gate): State<Gate>,
    Extension(person): Extension<Person>,
    headers: HeaderMap,
) -> Response {
    let origin = match origin(&gate.auth, &headers) {
        Ok(origin) => origin,
        Err(response) => return response,
    };
    match gate.auth.start_registration(&origin, person.id).await {
        Ok((ceremony, options)) => Json(json!({ "ceremony": ceremony, "options": options })).into_response(),
        Err(refusal) => refused(refusal),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FinishRegistration {
    ceremony: String,
    credential: RegisterPublicKeyCredential,
}

async fn finish_registration(
    State(gate): State<Gate>,
    Extension(person): Extension<Person>,
    headers: HeaderMap,
    Json(body): Json<FinishRegistration>,
) -> Response {
    let origin = match origin(&gate.auth, &headers) {
        Ok(origin) => origin,
        Err(response) => return response,
    };
    let name = label(&headers);
    match gate.auth.finish_registration(&origin, person.id, &body.ceremony, &body.credential, &name).await {
        Ok(passkey) => Json(json!({ "ok": true, "message": "Passkey saved", "passkey": passkey })).into_response(),
        Err(refusal) => refused(refusal),
    }
}

async fn remove_passkey(
    State(gate): State<Gate>,
    Extension(person): Extension<Person>,
    Path(id): Path<String>,
) -> Response {
    match gate.auth.remove_passkey(person.id, &id).await {
        Ok(true) => Json(json!({ "ok": true, "message": "Passkey removed" })).into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({ "error": "No such passkey", "ok": false }))).into_response(),
        Err(refusal) => refused(refusal),
    }
}

async fn members(State(gate): State<Gate>) -> Response {
    match gate.auth.members().await {
        Ok(members) => Json(json!({ "members": members })).into_response(),
        Err(refusal) => refused(refusal),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewMember {
    name: String,
    role: Role,
}

async fn create_member(State(gate): State<Gate>, Json(body): Json<NewMember>) -> Response {
    match gate.auth.create_profile(&body.name, body.role).await {
        Ok(person) => Json(json!({ "ok": true, "message": format!("{} added", person.name), "person": person }))
            .into_response(),
        Err(refusal) => refused(refusal),
    }
}

async fn delete_member(
    State(gate): State<Gate>,
    Extension(person): Extension<Person>,
    Path(id): Path<i64>,
) -> Response {
    let removed = async {
        let name = gate.auth.person(ProfileId(id)).await?.name;
        gate.auth.delete_profile(person.id, ProfileId(id)).await?;
        Ok::<_, Refusal>(name)
    };
    match removed.await {
        Ok(name) => Json(json!({ "ok": true, "message": format!("{name} removed") })).into_response(),
        Err(refusal) => refused(refusal),
    }
}

async fn invite(
    State(gate): State<Gate>,
    Extension(person): Extension<Person>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    let origin = match origin(&gate.auth, &headers) {
        Ok(origin) => origin,
        Err(response) => return response,
    };
    let made = async {
        let name = gate.auth.person(ProfileId(id)).await?.name;
        Ok::<_, Refusal>((name, gate.auth.invite(Some(person.id), ProfileId(id)).await?))
    };
    match made.await {
        Ok((name, token)) => Json(json!({
            "ok": true,
            "message": format!("Invite link for {name}"),
            "link": format!("{origin}/invite/{token}"),
            "expires_in_days": super::INVITE_LIFETIME / 86400,
        }))
        .into_response(),
        Err(refusal) => refused(refusal),
    }
}

async fn sign_out_member(State(gate): State<Gate>, Path(id): Path<i64>) -> Response {
    let done = async {
        let name = gate.auth.person(ProfileId(id)).await?.name;
        gate.auth.sign_out_everywhere(ProfileId(id)).await?;
        Ok::<_, Refusal>(name)
    };
    match done.await {
        Ok(name) => Json(json!({ "ok": true, "message": format!("{name} is signed out everywhere") })).into_response(),
        Err(refusal) => refused(refusal),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn a_session_is_named_after_the_device_and_browser() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Mobile/15E148 Safari/604.1",
            ),
        );
        assert_eq!(label(&headers), "iPhone · Safari");
        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static("Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 Chrome/130.0 Mobile Safari/537.36"),
        );
        assert_eq!(label(&headers), "Android · Chrome");
        assert_eq!(label(&HeaderMap::new()), "Browser");
    }

    #[tokio::test]
    async fn the_origin_comes_from_the_request_and_must_be_known() {
        let auth = Auth::new(crate::db::Db::memory().await, &["https://tv.example.org".to_string()]).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("tv.example.org"));
        assert_eq!(origin(&auth, &headers).unwrap(), "https://tv.example.org");
        headers.insert(header::ORIGIN, HeaderValue::from_static("https://evil.example.com"));
        assert!(origin(&auth, &headers).is_err());
    }
}
