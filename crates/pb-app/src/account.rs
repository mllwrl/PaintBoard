//! The account on the sharing server: signing in with a link sent by email, the boards of
//! the account, and what the owner of a shared board can do (let people in by address,
//! make new links, delete it). The server's side is `pb-server`'s `accounts`.
//!
//! Requests run in the background; their answers come back through `send` as [`Reply`]s,
//! which the app hands to [`Accounts::receive`].

use std::time::Duration;

use pb_core::Role;
use pb_platform::Instant;
use pb_platform::net::MaybeSend;
use serde_json::Value;

/// How often the server is asked whether the link in the email was followed.
const POLL_EVERY: Duration = Duration::from_secs(2);

/// How long that is asked: the link works for 15 minutes.
const POLL_FOR: Duration = Duration::from_secs(15 * 60);

/// Signed in on a server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub server: String,
    pub email: String,
    pub session: String,
}

/// A sign-in waiting for the link in the email to be followed.
#[derive(Clone, Debug)]
pub struct SigningIn {
    pub server: String,
    pub email: String,
    /// The code the email shows too; `None` until the server answers.
    pub code: Option<String>,
    request: Option<String>,
    next_poll: Option<Instant>,
    until: Instant,
}

/// A board in the account's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardEntry {
    pub board: String,
    pub title: String,
    /// `owner`, `edit`, `comment` or `view`.
    pub role: String,
}

/// Someone let in to a board by address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub email: String,
    /// `owner`, `edit`, `comment` or `view`.
    pub role: String,
}

/// An answer from the server.
#[derive(Debug)]
pub enum Reply {
    Started(Result<String, String>),
    Polled(Result<String, String>),
    Boards(Result<String, String>),
    Members(String, Result<String, String>),
    /// A member was let in, changed or removed on this board.
    MemberSet(String, Result<String, String>),
    Renewed(String, Role, Result<String, String>),
    BoardDeleted(String, Result<String, String>),
    AccountDeleted(Result<String, String>),
}

/// What the app should do after [`Accounts::receive`].
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The link for `Role` on this board changed to this token.
    NewLink(String, Role, String),
    /// This board is gone from the server.
    BoardGone(String),
}

#[cfg(not(target_arch = "wasm32"))]
type Send = std::sync::Arc<dyn Fn(Reply) + std::marker::Send + Sync>;
#[cfg(target_arch = "wasm32")]
type Send = std::rc::Rc<dyn Fn(Reply)>;

pub struct Accounts {
    /// Signed in on the server in use, if so.
    pub account: Option<Account>,
    pub signing_in: Option<SigningIn>,
    /// The account's boards, as last asked.
    pub boards: Option<Vec<BoardEntry>>,
    /// The members of the open board, when this account owns it.
    pub members: Option<(String, Vec<Member>)>,
    /// What went wrong last.
    pub error: Option<String>,
    send: Send,
}

impl Accounts {
    pub fn new(send: impl Fn(Reply) + MaybeSend + 'static) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let send: Send = std::sync::Arc::new(send);
        #[cfg(target_arch = "wasm32")]
        let send: Send = std::rc::Rc::new(send);
        Self {
            account: None,
            signing_in: None,
            boards: None,
            members: None,
            error: None,
            send,
        }
    }

    /// Uses the account kept for `server` (as typed in the sharing window), if any.
    pub fn use_server(&mut self, server: &str) {
        let Some(server) = crate::sharing::server_address(server) else {
            self.account = None;
            return;
        };
        if self.account.as_ref().is_some_and(|a| a.server == server) {
            return;
        }
        self.account = crate::settings::account(&server).map(|(email, session)| Account {
            server: server.clone(),
            email,
            session,
        });
        self.boards = None;
        self.members = None;
    }

    /// The session to join `server`'s boards with, if signed in there.
    pub fn session_for(&self, server: &str) -> Option<&str> {
        self.account
            .as_ref()
            .filter(|a| a.server == server)
            .map(|a| a.session.as_str())
    }

    fn request(
        &self,
        method: &'static str,
        path: &str,
        body: Option<String>,
        reply: impl FnOnce(Result<String, String>) -> Reply + MaybeSend + 'static,
    ) {
        let Some(account) = &self.account else {
            return;
        };
        let send = self.send.clone();
        let auth = format!("Bearer {}", account.session);
        pb_platform::net::fetch(
            method,
            &format!("{}{path}", account.server),
            &[("authorization", &auth)],
            body.as_deref(),
            move |result| send(reply(result)),
        );
    }

    /// Asks `server` to email `email` a link to sign in.
    pub fn start(&mut self, server: &str, email: &str) {
        let Some(server) = crate::sharing::server_address(server) else {
            self.error = Some(crate::i18n::tr("That is not a server address.").to_owned());
            return;
        };
        let email = email.trim().to_owned();
        let lang = match crate::i18n::lang() {
            crate::i18n::Lang::Ru => "ru",
            _ => "en",
        };
        let body = serde_json::json!({ "email": email, "lang": lang }).to_string();
        let send = self.send.clone();
        pb_platform::net::fetch(
            "POST",
            &format!("{server}/auth/start"),
            &[],
            Some(&body),
            move |result| send(Reply::Started(result)),
        );
        crate::settings::save_server(&server);
        self.error = None;
        self.signing_in = Some(SigningIn {
            server,
            email,
            code: None,
            request: None,
            next_poll: None,
            until: Instant::now() + POLL_FOR,
        });
    }

    pub fn cancel(&mut self) {
        self.signing_in = None;
    }

    /// When [`Accounts::tick`] should run next.
    pub fn deadline(&self) -> Option<Instant> {
        self.signing_in.as_ref().and_then(|s| s.next_poll)
    }

    /// Asks whether the sign-in is done, when due.
    pub fn tick(&mut self, now: Instant) {
        let Some(signing) = &mut self.signing_in else {
            return;
        };
        if signing.next_poll.is_none_or(|at| at > now) {
            return;
        }
        if now > signing.until {
            self.signing_in = None;
            self.error = Some(
                crate::i18n::tr("The link in the email has expired: sign in again.").to_owned(),
            );
            return;
        }
        // One question at a time: the next is planned when this one is answered.
        signing.next_poll = None;
        let Some(request) = &signing.request else {
            return;
        };
        let body = serde_json::json!({ "request": request }).to_string();
        let send = self.send.clone();
        pb_platform::net::fetch(
            "POST",
            &format!("{}/auth/poll", signing.server),
            &[],
            Some(&body),
            move |result| send(Reply::Polled(result)),
        );
    }

    pub fn sign_out(&mut self) {
        if let Some(account) = self.account.take() {
            let auth = format!("Bearer {}", account.session);
            pb_platform::net::fetch(
                "POST",
                &format!("{}/auth/logout", account.server),
                &[("authorization", &auth)],
                None,
                |_| {},
            );
            crate::settings::save_account(&account.server, None);
        }
        self.boards = None;
        self.members = None;
    }

    pub fn refresh_boards(&self) {
        self.request("GET", "/me", None, Reply::Boards);
    }

    /// Asks for the members of `board`; the answer comes only if this account owns it.
    pub fn load_members(&self, board: &str) {
        let board = board.to_owned();
        let path = format!("/boards/{board}/members");
        self.request("GET", &path, None, move |r| Reply::Members(board, r));
    }

    /// Lets `email` in to `board` as `role` (`edit`, `comment`, `view`, or `none` to
    /// remove them).
    pub fn set_member(&mut self, board: &str, email: &str, role: &str) {
        let lang = match crate::i18n::lang() {
            crate::i18n::Lang::Ru => "ru",
            _ => "en",
        };
        let body = serde_json::json!({ "email": email.trim(), "role": role, "lang": lang });
        let board = board.to_owned();
        let path = format!("/boards/{board}/members");
        self.error = None;
        self.request("POST", &path, Some(body.to_string()), move |r| {
            Reply::MemberSet(board, r)
        });
    }

    pub fn renew_link(&self, board: &str, role: Role) {
        let name = match role {
            Role::View => "view",
            Role::Comment => "comment",
            Role::Edit => "edit",
        };
        let body = serde_json::json!({ "role": name }).to_string();
        let board = board.to_owned();
        let path = format!("/boards/{board}/links");
        self.request("POST", &path, Some(body), move |r| {
            Reply::Renewed(board, role, r)
        });
    }

    pub fn delete_board(&self, board: &str) {
        let board = board.to_owned();
        let path = format!("/boards/{board}");
        self.request("DELETE", &path, None, move |r| {
            Reply::BoardDeleted(board, r)
        });
    }

    pub fn delete_account(&self) {
        self.request("DELETE", "/me", None, Reply::AccountDeleted);
    }

    /// Takes in an answer from the server.
    pub fn receive(&mut self, reply: Reply) -> Option<Outcome> {
        match reply {
            Reply::Started(result) => {
                let signing = self.signing_in.as_mut()?;
                match result.and_then(|body| parse(&body)) {
                    Ok(answer) => {
                        signing.request = text(&answer, "request");
                        signing.code = text(&answer, "code");
                        signing.next_poll = Some(Instant::now() + POLL_EVERY);
                    }
                    Err(err) => {
                        self.signing_in = None;
                        self.error = Some(explain(&err));
                    }
                }
            }
            Reply::Polled(result) => {
                let signing = self.signing_in.as_mut()?;
                match result.and_then(|body| parse(&body)) {
                    Ok(answer) => match (text(&answer, "session"), text(&answer, "email")) {
                        (Some(session), Some(email)) => {
                            let account = Account {
                                server: signing.server.clone(),
                                email,
                                session,
                            };
                            crate::settings::save_account(
                                &account.server,
                                Some((&account.email, &account.session)),
                            );
                            self.account = Some(account);
                            self.signing_in = None;
                            self.refresh_boards();
                        }
                        _ => signing.next_poll = Some(Instant::now() + POLL_EVERY),
                    },
                    // Gone: expired or used.
                    Err(err) if err.starts_with("410") => {
                        self.signing_in = None;
                        self.error = Some(
                            crate::i18n::tr("The link in the email has expired: sign in again.")
                                .to_owned(),
                        );
                    }
                    // The network may be back by the next try.
                    Err(_) => signing.next_poll = Some(Instant::now() + POLL_EVERY),
                }
            }
            Reply::Boards(result) => match result.and_then(|body| parse(&body)) {
                Ok(answer) => {
                    let boards = answer["boards"].as_array().cloned().unwrap_or_default();
                    self.boards = Some(
                        boards
                            .iter()
                            .filter_map(|b| {
                                Some(BoardEntry {
                                    board: text(b, "board")?,
                                    title: text(b, "title").unwrap_or_default(),
                                    role: text(b, "role")?,
                                })
                            })
                            .collect(),
                    );
                }
                Err(err) => self.failed(&err),
            },
            Reply::Members(board, result) => {
                // Not the owner: nothing to show.
                if let Ok(answer) = result.and_then(|body| parse(&body)) {
                    let members = answer["members"].as_array().cloned().unwrap_or_default();
                    let members = members
                        .iter()
                        .filter_map(|m| {
                            Some(Member {
                                email: text(m, "email")?,
                                role: text(m, "role")?,
                            })
                        })
                        .collect();
                    self.members = Some((board, members));
                }
            }
            Reply::MemberSet(board, result) => match result {
                Ok(_) => self.load_members(&board),
                Err(err) => self.failed(&err),
            },
            Reply::Renewed(board, role, result) => match result.and_then(|body| parse(&body)) {
                Ok(answer) => return Some(Outcome::NewLink(board, role, text(&answer, "token")?)),
                Err(err) => self.failed(&err),
            },
            Reply::BoardDeleted(board, result) => match result {
                Ok(_) => {
                    self.members = None;
                    self.refresh_boards();
                    return Some(Outcome::BoardGone(board));
                }
                Err(err) => self.failed(&err),
            },
            Reply::AccountDeleted(result) => match result {
                Ok(_) => {
                    if let Some(account) = self.account.take() {
                        crate::settings::save_account(&account.server, None);
                    }
                    self.boards = None;
                    self.members = None;
                }
                Err(err) => self.failed(&err),
            },
        }
        None
    }

    fn failed(&mut self, err: &str) {
        // The session is no longer good: signed out here too.
        if err.starts_with("401")
            && let Some(account) = self.account.take()
        {
            crate::settings::save_account(&account.server, None);
            self.boards = None;
            self.members = None;
        }
        self.error = Some(explain(err));
    }
}

fn parse(body: &str) -> Result<Value, String> {
    serde_json::from_str(body)
        .map_err(|_| crate::i18n::tr("The server's answer is not understood.").to_owned())
}

fn text(value: &Value, key: &str) -> Option<String> {
    value[key].as_str().map(str::to_owned)
}

/// The server's complaint as the user should read it.
fn explain(err: &str) -> String {
    use crate::i18n::{tr, trf};
    let status = err.get(..3).unwrap_or_default();
    match status {
        "404" if err.contains("no accounts") => {
            tr("This server has no accounts: share by links instead.").to_owned()
        }
        "401" => tr("Signed out: sign in again.").to_owned(),
        "403" => tr("Only the board's owner can do that.").to_owned(),
        "429" => tr("Too many attempts: try again later.").to_owned(),
        "400" if err.contains("email address") => tr("That is not an email address.").to_owned(),
        "502" => tr("The server could not send the email.").to_owned(),
        _ => trf("The server did not do it: {}", &[&err]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complaints_read_well() {
        assert_eq!(
            explain("401 Unauthorized: sign in first"),
            "Signed out: sign in again."
        );
        assert!(explain("404 Not Found: this server has no accounts").contains("no accounts"));
        assert!(explain("connection refused").contains("connection refused"));
    }
}
