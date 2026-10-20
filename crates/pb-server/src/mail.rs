//! Sending emails: sign-in links, and word that a board was shared with someone.
//!
//! Through an SMTP server given as `PB_SMTP` (e.g. `smtps://user:password@smtp.example.com`,
//! or `smtp://user:password@smtp.example.com:587?tls=required`), from `PB_MAIL_FROM`.
//! For trying the server out without one, `PB_MAIL=log` writes the emails to the log.

use std::sync::{Arc, Mutex};

use lettre::message::Mailbox;
use lettre::{SmtpTransport, Transport};

/// An email to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mail {
    pub to: String,
    pub subject: String,
    pub body: String,
}

// There is only one, made at start.
#[allow(clippy::large_enum_variant)]
pub enum Mailer {
    Smtp {
        transport: SmtpTransport,
        from: Mailbox,
    },
    /// Emails go to the log: for trying the server out on one machine.
    Log,
    /// Emails are kept, for tests.
    #[cfg_attr(not(test), allow(dead_code))]
    Kept(Arc<Mutex<Vec<Mail>>>),
}

impl Mailer {
    /// The mailer the environment asks for; `None` if it asks for none (no accounts then).
    pub fn from_env() -> Result<Option<Self>, String> {
        if std::env::var("PB_MAIL").is_ok_and(|v| v == "log") {
            return Ok(Some(Self::Log));
        }
        let Some(url) = std::env::var("PB_SMTP").ok().filter(|u| !u.is_empty()) else {
            return Ok(None);
        };
        let from = std::env::var("PB_MAIL_FROM")
            .map_err(|_| "PB_SMTP needs PB_MAIL_FROM, the address emails come from")?;
        let from: Mailbox = from
            .parse()
            .map_err(|e| format!("PB_MAIL_FROM is not an address: {e}"))?;
        let transport = SmtpTransport::from_url(&url)
            .map_err(|e| format!("PB_SMTP is not understood: {e}"))?
            .build();
        Ok(Some(Self::Smtp { transport, from }))
    }

    /// Sends `mail`; SMTP's wait happens off the async threads.
    pub async fn send(&self, mail: Mail) -> Result<(), String> {
        match self {
            Self::Smtp { transport, from } => {
                let to: Mailbox = mail
                    .to
                    .parse()
                    .map_err(|e| format!("not an address: {e}"))?;
                let message = lettre::Message::builder()
                    .from(from.clone())
                    .to(to)
                    .subject(mail.subject)
                    .header(lettre::message::header::ContentType::TEXT_PLAIN)
                    .body(mail.body)
                    .map_err(|e| e.to_string())?;
                let transport = transport.clone();
                tokio::task::spawn_blocking(move || transport.send(&message))
                    .await
                    .map_err(|e| e.to_string())?
                    .map(drop)
                    .map_err(|e| e.to_string())
            }
            Self::Log => {
                log::info!("email to {}: {}\n{}", mail.to, mail.subject, mail.body);
                Ok(())
            }
            Self::Kept(kept) => {
                kept.lock().expect("mail lock").push(mail);
                Ok(())
            }
        }
    }
}

/// Russian or English, as the reader's browser or app asks (`Accept-Language`, or the
/// app's language).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Ru,
}

impl Lang {
    pub fn from_header(header: Option<&str>) -> Self {
        let first = header
            .and_then(|h| h.split(',').next())
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        if first.starts_with("ru") {
            Self::Ru
        } else {
            Self::En
        }
    }

    pub fn pick(self, en: &'static str, ru: &'static str) -> &'static str {
        match self {
            Self::En => en,
            Self::Ru => ru,
        }
    }
}

/// `host[:port]` of `scheme://host[:port]`.
pub fn host_of(base: &str) -> &str {
    base.split_once("://").map_or(base, |(_, rest)| rest)
}

/// The email with a sign-in link.
pub fn sign_in(to: &str, server: &str, link: &str, code: &str, lang: Lang) -> Mail {
    let (subject, body) = match lang {
        Lang::Ru => (
            format!("Вход в PaintBoard, код {code}"),
            format!(
                "Чтобы войти в PaintBoard на {server}, откройте ссылку:\n\n{link}\n\n\
                 Проверьте, что в программе показан код {code}. Ссылка работает 15 минут.\n\
                 Если вы не входили, просто удалите это письмо."
            ),
        ),
        Lang::En => (
            format!("Sign in to PaintBoard, code {code}"),
            format!(
                "To sign in to PaintBoard on {server}, open this link:\n\n{link}\n\n\
                 Check that the app shows the code {code}. The link works for 15 minutes.\n\
                 If you did not try to sign in, just delete this email."
            ),
        ),
    };
    Mail {
        to: to.to_owned(),
        subject,
        body,
    }
}

/// The email telling someone a board was shared with them.
pub fn shared(to: &str, by: &str, title: &str, link: &str, lang: Lang) -> Mail {
    let (subject, body) = match lang {
        Lang::Ru => (
            format!("Вам открыли доступ к доске «{title}»"),
            format!(
                "Вам открыли доступ к доске «{title}» в PaintBoard (владелец доски — {by}).\n\n\
                 Войдите в PaintBoard с этим адресом ({to}) — доска будет в списке «Мои доски». \
                 Или откройте её в браузере:\n\n{link}"
            ),
        ),
        Lang::En => (
            format!("{by} shared the board \"{title}\""),
            format!(
                "{by} gave you access to the board \"{title}\" in PaintBoard.\n\n\
                 Sign in to PaintBoard with this address ({to}): the board is in \"My boards\". \
                 Or open it in a browser:\n\n{link}"
            ),
        ),
    };
    Mail {
        to: to.to_owned(),
        subject,
        body,
    }
}
