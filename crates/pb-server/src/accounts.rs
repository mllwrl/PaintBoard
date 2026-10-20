//! Accounts over HTTP: signing in with a link sent by email, the boards of an account,
//! and what a board's owner can do.
//!
//! - `POST /auth/start` `{"email","lang"}` sends the email and answers `{"request","code"}`;
//! - `GET /auth/<link>` (from the email) asks to confirm, `POST /auth/<link>` confirms;
//! - `POST /auth/poll` `{"request"}`: 202 while waiting, then `{"session","email"}` once;
//! - `POST /auth/logout`; `GET /me` (the address and boards); `DELETE /me` (the account
//!   and the boards it owns);
//! - owners: `GET`/`POST /boards/<id>/members` (`{"email","role"}`, role `none` removes),
//!   `POST /boards/<id>/links` `{"role"}` (a new link, the old one stops working),
//!   `DELETE /boards/<id>`.
//!
//! Requests on behalf of an account carry `Authorization: Bearer <session>`.

// Handlers answer early with a ready response; its size does not matter here.
#![allow(clippy::result_large_err)]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::json;

use crate::mail::{self, Lang};
use crate::server::{Shared, allow_other_sites, cors, too_many};
use crate::store::{LoginState, email_address, role_name, role_named};

pub fn routes() -> Router<Arc<Shared>> {
    Router::new()
        .route("/auth/start", post(start).options(allow_other_sites))
        .route("/auth/poll", post(poll).options(allow_other_sites))
        .route("/auth/logout", post(logout).options(allow_other_sites))
        .route("/auth/{link}", get(confirm_page).post(confirm))
        .route(
            "/me",
            get(me).delete(delete_account).options(allow_other_sites),
        )
        .route(
            "/boards/{board}/members",
            get(members).post(set_member).options(allow_other_sites),
        )
        .route(
            "/boards/{board}/links",
            post(renew_link).options(allow_other_sites),
        )
        .route(
            "/boards/{board}",
            axum::routing::delete(delete_board).options(allow_other_sites),
        )
}

/// The session in `Authorization: Bearer …`, if any.
pub fn session_of(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn fail(status: StatusCode, why: &'static str) -> Response {
    cors((status, why).into_response())
}

fn no_accounts() -> Response {
    fail(StatusCode::NOT_FOUND, "this server has no accounts")
}

fn store_failed(err: rusqlite::Error) -> Response {
    log::error!("accounts: {err}");
    fail(
        StatusCode::INTERNAL_SERVER_ERROR,
        "the server could not do it",
    )
}

/// The signed-in account's address, or the answer to give when there is none.
fn signed_in(shared: &Shared, headers: &HeaderMap) -> Result<String, Response> {
    if shared.mailer.is_none() {
        return Err(no_accounts());
    }
    let session =
        session_of(headers).ok_or_else(|| fail(StatusCode::UNAUTHORIZED, "sign in first"))?;
    shared
        .store()
        .session_user(session)
        .map_err(store_failed)?
        .ok_or_else(|| fail(StatusCode::UNAUTHORIZED, "signed out: sign in again"))
}

/// The signed-in owner of `board`, or the answer to give otherwise.
fn owner_of(shared: &Shared, headers: &HeaderMap, board: &str) -> Result<String, Response> {
    let email = signed_in(shared, headers)?;
    let membership = shared
        .store()
        .membership(board, &email)
        .map_err(store_failed)?;
    if membership.is_some_and(|m| m.owner) {
        Ok(email)
    } else {
        Err(fail(
            StatusCode::FORBIDDEN,
            "only the board's owner can do that",
        ))
    }
}

/// Where the server is seen from outside, for links in emails: `PB_PUBLIC_URL`, or (only
/// when emails go to the log, to try things out) the address the request came to.
fn public_base(shared: &Shared, headers: &HeaderMap) -> String {
    if let Some(url) = &shared.public_url {
        return url.clone();
    }
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    format!("http://{host}")
}

fn lang_of(asked: &str, headers: &HeaderMap) -> Lang {
    match asked {
        "ru" => Lang::Ru,
        "en" => Lang::En,
        _ => Lang::from_header(
            headers
                .get(axum::http::header::ACCEPT_LANGUAGE)
                .and_then(|h| h.to_str().ok()),
        ),
    }
}

fn parse<T: for<'a> Deserialize<'a>>(body: &str) -> Result<T, Response> {
    serde_json::from_str(body)
        .map_err(|_| fail(StatusCode::BAD_REQUEST, "the request is not understood"))
}

#[derive(Deserialize)]
struct Start {
    email: String,
    #[serde(default)]
    lang: String,
}

async fn start(
    State(shared): State<Arc<Shared>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(mailer) = &shared.mailer else {
        return no_accounts();
    };
    let start: Start = match parse(&body) {
        Ok(start) => start,
        Err(answer) => return answer,
    };
    let Some(email) = email_address(&start.email) else {
        return fail(StatusCode::BAD_REQUEST, "that is not an email address");
    };
    if !shared.limits.login(shared.limits.client(peer, &headers)) {
        return cors(too_many(
            &shared,
            "too many sign-ins from here, try again later",
        ));
    }
    let login = match shared.store().start_login(&email) {
        Ok(Some(login)) => login,
        Ok(None) => {
            return cors(too_many(
                &shared,
                "too many sign-ins are waiting for this address: use one of the emails",
            ));
        }
        Err(err) => return store_failed(err),
    };
    let base = public_base(&shared, &headers);
    let server = crate::mail::host_of(&base);
    let link = format!("{base}/auth/{}", login.link);
    let mail = mail::sign_in(
        &email,
        server,
        &link,
        &login.code,
        lang_of(&start.lang, &headers),
    );
    if let Err(err) = mailer.send(mail).await {
        log::error!("sign-in email to {email} not sent: {err}");
        return fail(
            StatusCode::BAD_GATEWAY,
            "the server could not send the email",
        );
    }
    crate::metrics::bump(&shared.metrics.sign_ins);
    cors(axum::Json(json!({ "request": login.request, "code": login.code })).into_response())
}

/// A small page in the browser the email's link opens.
fn page(lang: Lang, title: &str, text: &str, button: Option<&str>) -> Response {
    let form = button.map_or(String::new(), |b| {
        format!(r#"<form method="post"><button type="submit">{b}</button></form>"#)
    });
    let html = format!(
        r#"<!doctype html><html lang="{lang}"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>PaintBoard</title>
<style>body{{font-family:system-ui,sans-serif;max-width:32rem;margin:4rem auto;padding:0 1rem;color:#1f2937}}
button{{font-size:1rem;padding:.6rem 1.4rem;border-radius:.5rem;border:0;background:#2563eb;color:#fff;cursor:pointer}}
.code{{font-size:2rem;letter-spacing:.3rem;font-weight:700}}</style></head>
<body><h1>{title}</h1><p>{text}</p>{form}</body></html>"#,
        lang = lang.pick("en", "ru"),
    );
    Html(html).into_response()
}

async fn confirm_page(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Path(link): Path<String>,
) -> Response {
    let lang = lang_of("", &headers);
    if shared.mailer.is_none() {
        return no_accounts();
    }
    match shared.store().login_for_link(&link) {
        Ok(Some((email, code))) => {
            // Addresses and codes are plain (see `email_address`): nothing to escape.
            let text = match lang {
                Lang::Ru => format!(
                    "Войти как <b>{email}</b>? Проверьте, что в программе показан тот же код:</p><p class=\"code\">{code}"
                ),
                Lang::En => format!(
                    "Sign in as <b>{email}</b>? Check that the app shows the same code:</p><p class=\"code\">{code}"
                ),
            };
            page(
                lang,
                lang.pick("Sign in to PaintBoard", "Вход в PaintBoard"),
                &text,
                Some(lang.pick("Sign in", "Войти")),
            )
        }
        Ok(None) => expired(lang),
        Err(err) => store_failed(err),
    }
}

fn expired(lang: Lang) -> Response {
    let mut answer = page(
        lang,
        lang.pick("The link has expired", "Ссылка больше не работает"),
        lang.pick(
            "It works for 15 minutes and once. Start signing in again in the app.",
            "Она работает 15 минут и один раз. Начните вход в программе заново.",
        ),
        None,
    );
    *answer.status_mut() = StatusCode::GONE;
    answer
}

async fn confirm(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Path(link): Path<String>,
) -> Response {
    let lang = lang_of("", &headers);
    if shared.mailer.is_none() {
        return no_accounts();
    }
    match shared.store().confirm_login(&link) {
        Ok(true) => page(
            lang,
            lang.pick("Signed in", "Вход выполнен"),
            lang.pick(
                "Go back to PaintBoard: it is signed in now. This page can be closed.",
                "Вернитесь в PaintBoard: вход уже выполнен. Эту страницу можно закрыть.",
            ),
            None,
        ),
        Ok(false) => expired(lang),
        Err(err) => store_failed(err),
    }
}

#[derive(Deserialize)]
struct Poll {
    request: String,
}

async fn poll(State(shared): State<Arc<Shared>>, body: String) -> Response {
    if shared.mailer.is_none() {
        return no_accounts();
    }
    let poll: Poll = match parse(&body) {
        Ok(poll) => poll,
        Err(answer) => return answer,
    };
    let state = shared.store().finish_login(&poll.request);
    match state {
        Ok(LoginState::Waiting) => {
            cors((StatusCode::ACCEPTED, axum::Json(json!({ "waiting": true }))).into_response())
        }
        Ok(LoginState::Done { session, email }) => {
            log::info!("{email} signed in");
            cors(axum::Json(json!({ "session": session, "email": email })).into_response())
        }
        Ok(LoginState::Gone) => fail(StatusCode::GONE, "the sign-in expired: start again"),
        Err(err) => store_failed(err),
    }
}

async fn logout(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if let Some(session) = session_of(&headers)
        && let Err(err) = shared.store().end_session(session)
    {
        return store_failed(err);
    }
    cors(StatusCode::NO_CONTENT.into_response())
}

async fn me(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    let email = match signed_in(&shared, &headers) {
        Ok(email) => email,
        Err(answer) => return answer,
    };
    let boards = match shared.store().boards_of(&email) {
        Ok(boards) => boards,
        Err(err) => return store_failed(err),
    };
    let boards: Vec<_> = boards
        .into_iter()
        .map(|b| {
            json!({
                "board": b.board,
                "title": b.title.unwrap_or_default(),
                "role": b.membership.name(),
                "updated": b.updated,
            })
        })
        .collect();
    cors(axum::Json(json!({ "email": email, "boards": boards })).into_response())
}

async fn delete_account(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    let email = match signed_in(&shared, &headers) {
        Ok(email) => email,
        Err(answer) => return answer,
    };
    let gone = shared.store().delete_account(&email);
    match gone {
        Ok(boards) => {
            for board in &boards {
                shared.close_board(board);
            }
            log::info!("account {email} deleted with {} boards", boards.len());
            cors(StatusCode::NO_CONTENT.into_response())
        }
        Err(err) => store_failed(err),
    }
}

async fn members(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Path(board): Path<String>,
) -> Response {
    if let Err(answer) = owner_of(&shared, &headers, &board) {
        return answer;
    }
    match shared.store().members(&board) {
        Ok(members) => {
            let members: Vec<_> = members
                .into_iter()
                .map(|(email, m)| json!({ "email": email, "role": m.name() }))
                .collect();
            cors(axum::Json(json!({ "members": members })).into_response())
        }
        Err(err) => store_failed(err),
    }
}

#[derive(Deserialize)]
struct SetMember {
    email: String,
    role: String,
    #[serde(default)]
    lang: String,
}

async fn set_member(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Path(board): Path<String>,
    body: String,
) -> Response {
    let owner = match owner_of(&shared, &headers, &board) {
        Ok(owner) => owner,
        Err(answer) => return answer,
    };
    let asked: SetMember = match parse(&body) {
        Ok(asked) => asked,
        Err(answer) => return answer,
    };
    let Some(email) = email_address(&asked.email) else {
        return fail(StatusCode::BAD_REQUEST, "that is not an email address");
    };
    let role = match asked.role.as_str() {
        "none" => None,
        name => match role_named(name) {
            Some(role) => Some(role),
            None => return fail(StatusCode::BAD_REQUEST, "no such role"),
        },
    };
    let (was, title) = {
        let store = shared.store();
        let was = store.membership(&board, &email);
        let set = store.set_member(&board, &email, role);
        let title = store.title(&board);
        match (was, set, title) {
            (Ok(was), Ok(()), Ok(title)) => (was, title),
            (Err(err), ..) | (_, Err(err), _) | (.., Err(err)) => return store_failed(err),
        }
    };
    // Someone new hears of it by email; changes of role do not need one.
    if was.is_none()
        && role.is_some()
        && email != owner
        && let Some(mailer) = &shared.mailer
    {
        let base = public_base(&shared, &headers);
        let title = title.unwrap_or_else(|| board.clone());
        let link = format!("{base}/board/{board}");
        let mail = mail::shared(
            &email,
            &owner,
            &title,
            &link,
            lang_of(&asked.lang, &headers),
        );
        if let Err(err) = mailer.send(mail).await {
            log::warn!("could not tell {email} about board {board}: {err}");
        }
    }
    cors(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
struct RenewLink {
    role: String,
}

async fn renew_link(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Path(board): Path<String>,
    body: String,
) -> Response {
    if let Err(answer) = owner_of(&shared, &headers, &board) {
        return answer;
    }
    let asked: RenewLink = match parse(&body) {
        Ok(asked) => asked,
        Err(answer) => return answer,
    };
    let Some(role) = role_named(&asked.role) else {
        return fail(StatusCode::BAD_REQUEST, "no such role");
    };
    let renewed = shared.store().renew_link(&board, role);
    match renewed {
        Ok(token) => {
            cors(axum::Json(json!({ "role": role_name(role), "token": token })).into_response())
        }
        Err(err) => store_failed(err),
    }
}

async fn delete_board(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Path(board): Path<String>,
) -> Response {
    if let Err(answer) = owner_of(&shared, &headers, &board) {
        return answer;
    }
    let deleted = shared.store().delete_board(&board);
    match deleted {
        Ok(()) => {
            shared.close_board(&board);
            log::info!("board {board} deleted by its owner");
            cors(StatusCode::NO_CONTENT.into_response())
        }
        Err(err) => store_failed(err),
    }
}
