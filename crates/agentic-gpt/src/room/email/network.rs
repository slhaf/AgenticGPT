use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use lettre::{
    message::Mailbox, transport::smtp::authentication::Credentials, Message, SmtpTransport,
    Transport,
};

use crate::config::{resolve_secret_reference, EmailAccountConfig, EmailTls};

use super::content::MAX_MESSAGE_BYTES;

const NETWORK_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const FETCH_BATCH: usize = 100;
const MAX_MAILBOX_ITEMS: usize = 200_000;
type Session = imap::Session<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>;

pub(super) struct RemoteMailbox {
    session: Session,
    pub validity: u32,
    pub baseline: u32,
    pub uids: Vec<u32>,
}

pub(super) struct RemoteMessage {
    pub uid: u32,
    pub received_at: DateTime<Utc>,
    pub unread: bool,
    pub flagged: bool,
    pub raw: Vec<u8>,
    pub unavailable: Option<&'static str>,
}

pub(super) struct RemoteFlags {
    pub uid: u32,
    pub unread: bool,
    pub flagged: bool,
}

fn connect(account: &EmailAccountConfig) -> Result<Session> {
    connect_with_root(account, None)
}

fn connect_with_root(
    account: &EmailAccountConfig,
    extra_root: Option<rustls::pki_types::CertificateDer<'static>>,
) -> Result<Session> {
    let addresses: Vec<_> = (account.imap_host.as_str(), account.imap_port)
        .to_socket_addrs()
        .map_err(|_| anyhow!("email_imap_dns_failed"))?
        .take(8)
        .collect();
    let mut tcp = addresses
        .iter()
        .find_map(|address| TcpStream::connect_timeout(address, NETWORK_TIMEOUT).ok())
        .ok_or_else(|| anyhow!("email_imap_connection_failed"))?;
    tcp.set_read_timeout(Some(NETWORK_TIMEOUT))?;
    tcp.set_write_timeout(Some(NETWORK_TIMEOUT))?;
    let starttls = account.imap_tls == EmailTls::Starttls;
    if starttls {
        upgrade_starttls(&mut tcp)?;
    }
    let mut roots =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(root) = extra_root {
        roots
            .add(root)
            .map_err(|_| anyhow!("email_imap_tls_root_invalid"))?;
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_root_certificates(roots)
    .with_no_client_auth();
    let server_name = rustls::pki_types::ServerName::try_from(account.imap_host.clone())
        .map_err(|_| anyhow!("email_imap_hostname_invalid"))?;
    let tls = rustls::StreamOwned::new(
        rustls::ClientConnection::new(Arc::new(config), server_name)?,
        tcp,
    );
    let mut client = imap::Client::new(tls);
    if starttls {
        client.greeting_read = true;
    } else {
        client
            .read_greeting()
            .map_err(|_| anyhow!("email_imap_tls_failed"))?;
    }
    let password = resolve_secret_reference(&account.password)
        .map_err(|_| anyhow!("email_password_unavailable"))?;
    client
        .login(&account.username, password)
        .map_err(|_| anyhow!("email_imap_authentication_failed"))
}

fn upgrade_starttls(tcp: &mut TcpStream) -> Result<()> {
    // The IMAP crate's builder does not expose connect/read timeouts for its
    // pre-login upgrade. Only negotiate TLS here; parsing/authentication and all
    // mailbox commands remain in the protocol library.
    let mut reader = BufReader::new(tcp);
    let greeting = read_handshake_line(&mut reader)?;
    if !greeting.to_ascii_uppercase().starts_with("* OK") {
        return Err(anyhow!("email_imap_greeting_failed"));
    }
    reader.get_mut().write_all(b"a0 STARTTLS\r\n")?;
    reader.get_mut().flush()?;
    let mut bytes = greeting.len();
    loop {
        let line = read_handshake_line(&mut reader)?;
        bytes += line.len();
        if bytes > 65536 {
            return Err(anyhow!("email_imap_starttls_failed"));
        }
        let upper = line.to_ascii_uppercase();
        if upper.starts_with("A0 OK") {
            return Ok(());
        }
        if upper.starts_with("A0 ") || upper.starts_with("* BYE") {
            return Err(anyhow!("email_imap_starttls_required"));
        }
    }
}

fn read_handshake_line(reader: &mut BufReader<&mut TcpStream>) -> Result<String> {
    let mut line = String::new();
    Read::take(reader, 16385)
        .read_line(&mut line)
        .map_err(|_| anyhow!("email_imap_starttls_failed"))?;
    if line.len() > 16384 || !line.ends_with('\n') {
        return Err(anyhow!("email_imap_starttls_failed"));
    }
    Ok(line)
}

pub(super) fn inbox(account: &EmailAccountConfig) -> Result<RemoteMailbox> {
    select_inbox(connect(account)?)
}

#[cfg(test)]
pub(super) fn inbox_with_test_root(
    account: &EmailAccountConfig,
    root: rustls::pki_types::CertificateDer<'static>,
) -> Result<RemoteMailbox> {
    select_inbox(connect_with_root(account, Some(root))?)
}

fn select_inbox(mut session: Session) -> Result<RemoteMailbox> {
    let selected = session
        .select("INBOX")
        .map_err(|_| anyhow!("email_imap_inbox_failed"))?;
    let validity = selected
        .uid_validity
        .ok_or_else(|| anyhow!("email_imap_uidvalidity_required"))?;
    let mut uids: Vec<_> = session
        .uid_search("ALL")
        .map_err(|_| anyhow!("email_imap_search_failed"))?
        .into_iter()
        .collect();
    if uids.len() > MAX_MAILBOX_ITEMS {
        return Err(anyhow!("email_mailbox_item_limit_exceeded"));
    }
    uids.sort_unstable();
    // SELECT's UIDNEXT fixes the boundary before SEARCH; arrivals during SEARCH
    // must remain new mail rather than becoming part of the initial import.
    let baseline = selected
        .uid_next
        .map(|uid| uid.saturating_sub(1))
        .ok_or_else(|| anyhow!("email_imap_uidnext_required"))?;
    Ok(RemoteMailbox {
        session,
        validity,
        baseline,
        uids,
    })
}

impl RemoteMailbox {
    pub fn flags(&mut self, uids: &[u32]) -> Result<Vec<RemoteFlags>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let messages = self
            .session
            .uid_fetch(uid_set(uids), "(UID FLAGS)")
            .map_err(|_| anyhow!("email_imap_flags_failed"))?;
        Ok(messages
            .iter()
            .filter_map(|mail| {
                mail.uid.map(|uid| RemoteFlags {
                    uid,
                    unread: !mail.flags().contains(&imap::types::Flag::Seen),
                    flagged: mail.flags().contains(&imap::types::Flag::Flagged),
                })
            })
            .collect())
    }

    pub fn message(&mut self, uid: u32) -> Result<Option<RemoteMessage>> {
        let info = self
            .session
            .uid_fetch(uid.to_string(), "(UID RFC822.SIZE)")
            .map_err(|_| anyhow!("email_imap_fetch_failed"))?;
        let Some(size) = info
            .iter()
            .find(|mail| mail.uid == Some(uid))
            .and_then(|mail| mail.size)
        else {
            return Ok(None); // Expunged between SEARCH and FETCH.
        };
        let too_large = size as usize > MAX_MESSAGE_BYTES;
        drop(info);
        let messages = self
            .session
            .uid_fetch(
                uid.to_string(),
                if too_large {
                    "(UID FLAGS INTERNALDATE BODY.PEEK[HEADER]<0.16384>)"
                } else {
                    "(UID FLAGS INTERNALDATE BODY.PEEK[])"
                },
            )
            .map_err(|_| anyhow!("email_imap_fetch_failed"))?;
        let Some(mail) = messages.iter().find(|mail| mail.uid == Some(uid)) else {
            return Ok(None);
        };
        let raw = if too_large {
            mail.header()
        } else {
            mail.body()
        }
        .ok_or_else(|| anyhow!("email_imap_body_missing"))?;
        if raw.len() > MAX_MESSAGE_BYTES {
            return Err(anyhow!("email_message_too_large"));
        }
        let received_at = mail
            .internal_date()
            .ok_or_else(|| anyhow!("email_imap_received_time_missing"))?
            .with_timezone(&Utc);
        Ok(Some(RemoteMessage {
            uid,
            received_at,
            unread: !mail.flags().contains(&imap::types::Flag::Seen),
            flagged: mail.flags().contains(&imap::types::Flag::Flagged),
            raw: raw.to_vec(),
            unavailable: too_large.then_some("tooLarge"),
        }))
    }
}

fn uid_set(uids: &[u32]) -> String {
    uids.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn operate(
    account: &EmailAccountConfig,
    validity: u32,
    uid: u32,
    action: &str,
) -> Result<()> {
    let mut session = connect(account)?;
    let selected = session
        .select("INBOX")
        .map_err(|_| anyhow!("email_imap_inbox_failed"))?;
    if selected.uid_validity != Some(validity) {
        return Err(anyhow!("email_identity_recovery_required"));
    }
    let found = session
        .uid_search(format!("UID {uid}"))
        .map_err(|_| anyhow!("email_imap_search_failed"))?;
    if !found.contains(&uid) {
        return Err(anyhow!("email_remote_message_unavailable"));
    }
    if action == "trash" {
        if !session
            .capabilities()
            .map_err(|_| anyhow!("email_imap_capabilities_failed"))?
            .has_str("MOVE")
        {
            return Err(anyhow!("email_imap_move_not_supported"));
        }
        let mailboxes = session
            .list(None, Some("*"))
            .map_err(|_| anyhow!("email_trash_mailbox_lookup_failed"))?;
        let trash = mailboxes
            .iter()
            .find(|mailbox| {
                mailbox
                    .attributes()
                    .iter()
                    .any(|attribute| matches!(attribute, imap_proto::NameAttribute::Trash))
            })
            .map(|mailbox| mailbox.name().to_owned())
            .ok_or_else(|| anyhow!("email_trash_mailbox_unavailable"))?;
        drop(mailboxes);
        session
            .uid_mv(uid.to_string(), &trash)
            .map_err(|_| anyhow!("email_trash_failed"))?;
    } else {
        let flags = match action {
            "markRead" => "+FLAGS.SILENT (\\Seen)",
            "markUnread" => "-FLAGS.SILENT (\\Seen)",
            "flag" => "+FLAGS.SILENT (\\Flagged)",
            "unflag" => "-FLAGS.SILENT (\\Flagged)",
            _ => return Err(anyhow!("email_action_invalid")),
        };
        session
            .uid_store(uid.to_string(), flags)
            .map_err(|_| anyhow!("email_operate_failed"))?;
    }
    Ok(())
}

pub(super) fn build_message(
    account: &EmailAccountConfig,
    to: &[String],
    cc: &[String],
    bcc: &[String],
    subject: &str,
    text: &str,
    reply: Option<(String, String)>,
) -> Result<Message> {
    let mut builder = Message::builder().from(
        account
            .from
            .parse::<Mailbox>()
            .map_err(|_| anyhow!("email_from_invalid"))?,
    );
    for recipient in to {
        builder = builder.to(recipient
            .parse()
            .map_err(|_| anyhow!("email_recipient_invalid"))?);
    }
    for recipient in cc {
        builder = builder.cc(recipient
            .parse()
            .map_err(|_| anyhow!("email_recipient_invalid"))?);
    }
    for recipient in bcc {
        builder = builder.bcc(
            recipient
                .parse()
                .map_err(|_| anyhow!("email_recipient_invalid"))?,
        );
    }
    if let Some((message_id, references)) = reply {
        builder = builder.in_reply_to(message_id).references(references);
    }
    builder
        .subject(subject)
        .body(text.to_owned())
        .map_err(|_| anyhow!("email_message_invalid"))
}

pub(super) enum SendOutcome {
    Sent,
    Failed,
    Unknown,
}

pub(super) fn send(account: &EmailAccountConfig, message: &Message) -> Result<SendOutcome> {
    send_with_root(account, message, None)
}

#[cfg(test)]
pub(super) fn send_with_test_root(
    account: &EmailAccountConfig,
    message: &Message,
    root: &[u8],
) -> Result<SendOutcome> {
    send_with_root(account, message, Some(root))
}

fn send_with_root(
    account: &EmailAccountConfig,
    message: &Message,
    extra_root: Option<&[u8]>,
) -> Result<SendOutcome> {
    let mut builder = match account.smtp_tls {
        EmailTls::Implicit => SmtpTransport::relay(&account.smtp_host),
        EmailTls::Starttls => SmtpTransport::starttls_relay(&account.smtp_host),
    }
    .map_err(|_| anyhow!("email_smtp_tls_configuration_failed"))?;
    if let Some(root) = extra_root {
        use lettre::transport::smtp::client::{Certificate, Tls, TlsParameters};
        let certificate = Certificate::from_der(root.to_vec())
            .map_err(|_| anyhow!("email_smtp_tls_root_invalid"))?;
        let parameters = TlsParameters::builder(account.smtp_host.clone())
            .add_root_certificate(certificate)
            .build()
            .map_err(|_| anyhow!("email_smtp_tls_configuration_failed"))?;
        builder = builder.tls(match account.smtp_tls {
            EmailTls::Implicit => Tls::Wrapper(parameters),
            EmailTls::Starttls => Tls::Required(parameters),
        });
    }
    let password = resolve_secret_reference(&account.password)
        .map_err(|_| anyhow!("email_password_unavailable"))?;
    let transport = builder
        .port(account.smtp_port)
        .timeout(Some(NETWORK_TIMEOUT))
        .credentials(Credentials::new(account.username.clone(), password))
        .build();
    Ok(match transport.send(message) {
        Ok(_) => SendOutcome::Sent,
        Err(error) if error.is_response() || error.is_client() || error.is_tls() => {
            SendOutcome::Failed
        }
        // A socket failure may occur after DATA was accepted. Never retry it.
        Err(_) => SendOutcome::Unknown,
    })
}
