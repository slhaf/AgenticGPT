use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use mailparse::{DispositionType, MailHeaderMap, ParsedMail};
use serde::Serialize;

pub(super) const MAX_MESSAGE_BYTES: usize = 32 * 1024 * 1024;
pub(super) const TEXT_BUDGET: usize = 24 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Attachment {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: usize,
    #[serde(skip)]
    pub bytes: Vec<u8>,
    #[serde(skip)]
    pub cid: Option<String>,
}

pub(super) struct Content {
    pub from: String,
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub message_id: Option<String>,
    pub references: Option<String>,
    pub body: String,
    pub format: &'static str,
    pub html: Option<String>,
    pub attachments: Vec<Attachment>,
}

pub(super) fn normalize_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|c| !c.is_control() || matches!(*c, '\n' | '\t'))
        .collect::<String>()
        .trim()
        .to_owned()
}

pub(super) fn parse(raw: &[u8]) -> Result<Content> {
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err(anyhow!("email_message_too_large"));
    }
    let mail = mailparse::parse_mail(raw).map_err(|_| anyhow!("email_mime_invalid"))?;
    let mut plain = None;
    let mut html = None;
    let mut attachments = Vec::new();
    let mut count = 0;
    collect(
        &mail,
        0,
        &mut count,
        &mut plain,
        &mut html,
        &mut attachments,
    )?;
    let (body, format) = if let Some(plain) = plain.filter(|s| !s.trim().is_empty()) {
        (normalize_text(&plain), "plain")
    } else if let Some(html) = &html {
        let converter = htmd::HtmlToMarkdown::builder()
            .skip_tags(vec![
                "head", "script", "style", "template", "iframe", "object", "embed",
            ])
            .add_handler(
                vec!["img"],
                |_: &dyn htmd::element_handler::Handlers, element: htmd::Element<'_>| {
                    let alt = element
                        .attrs
                        .iter()
                        .find(|attribute| attribute.name.local.as_ref() == "alt")
                        .map(|attribute| attribute.value.to_string())
                        .unwrap_or_default();
                    let mut safe = String::new();
                    for character in normalize_text(&alt).chars().take(240) {
                        if matches!(
                            character,
                            '\\' | '[' | ']' | '(' | ')' | '!' | '<' | '>' | '*' | '_' | '`'
                        ) {
                            safe.push('\\');
                        }
                        safe.push(character);
                    }
                    Some(
                        format!(
                            "[Image{}; use image view]",
                            if safe.is_empty() {
                                String::new()
                            } else {
                                format!(": {safe}")
                            }
                        )
                        .into(),
                    )
                },
            )
            .build();
        // A failed text projection must leave the original HTML available to
        // image reads, and must never stall unrelated later mail in the inbox.
        match converter.convert(html) {
            Ok(markdown) if !markdown.trim().is_empty() || html.trim().is_empty() => {
                (normalize_text(&markdown), "markdown")
            }
            _ => (String::new(), "conversionFailed"),
        }
    } else {
        (String::new(), "plain")
    };
    Ok(Content {
        from: header(&mail, "From"),
        to: header(&mail, "To"),
        cc: header(&mail, "Cc"),
        subject: header(&mail, "Subject"),
        message_id: mail.headers.get_first_value("Message-ID"),
        references: mail.headers.get_first_value("References"),
        body,
        format,
        html,
        attachments,
    })
}

pub(super) fn unavailable_metadata(raw: &[u8]) -> Content {
    let headers = mailparse::parse_headers(raw)
        .ok()
        .map(|(headers, _)| headers)
        .unwrap_or_default();
    let get = |name| {
        headers
            .get_first_value(name)
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control())
            .take(4096)
            .collect::<String>()
    };
    Content {
        from: get("From"),
        to: get("To"),
        cc: get("Cc"),
        subject: get("Subject"),
        message_id: headers.get_first_value("Message-ID"),
        references: headers.get_first_value("References"),
        body: String::new(),
        format: "plain",
        html: None,
        attachments: Vec::new(),
    }
}

pub(super) fn addresses(header: &str) -> Vec<String> {
    if header.trim().is_empty() {
        return Vec::new();
    }
    match mailparse::addrparse(header) {
        Ok(addresses) => addresses
            .iter()
            .flat_map(|address| match address {
                mailparse::MailAddr::Single(single) => vec![single.to_string()],
                mailparse::MailAddr::Group(group) => {
                    group.addrs.iter().map(ToString::to_string).collect()
                }
            })
            .collect(),
        // Preserve malformed provider data as one display value; never split
        // on commas, since quoted names may legitimately contain them.
        Err(_) => vec![header.to_owned()],
    }
}

fn header(mail: &ParsedMail<'_>, name: &str) -> String {
    mail.headers
        .get_first_value(name)
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(4096)
        .collect()
}

fn collect(
    mail: &ParsedMail<'_>,
    depth: usize,
    count: &mut usize,
    plain: &mut Option<String>,
    html: &mut Option<String>,
    attachments: &mut Vec<Attachment>,
) -> Result<()> {
    *count += 1;
    if depth > 30 || *count > 1000 {
        return Err(anyhow!("email_mime_complexity_exceeded"));
    }
    let disposition = mail.get_content_disposition();
    let name = disposition
        .params
        .get("filename")
        .or_else(|| mail.ctype.params.get("name"));
    let cid = mail.headers.get_first_value("Content-ID");
    let is_attachment = disposition.disposition == DispositionType::Attachment || name.is_some();
    let mime = mail.ctype.mimetype.to_ascii_lowercase();
    if !is_attachment && !mail.subparts.is_empty() {
        for child in &mail.subparts {
            collect(child, depth + 1, count, plain, html, attachments)?;
        }
    } else if !is_attachment && mime == "text/plain" {
        if plain.is_none() {
            *plain = Some(
                mail.get_body()
                    .map_err(|_| anyhow!("email_body_decode_failed"))?,
            );
        }
    } else if !is_attachment && mime == "text/html" {
        if html.is_none() {
            *html = Some(
                mail.get_body()
                    .map_err(|_| anyhow!("email_body_decode_failed"))?,
            );
        }
    } else {
        let bytes = mail
            .get_body_raw()
            .map_err(|_| anyhow!("email_attachment_decode_failed"))?;
        let index = attachments.len() + 1;
        attachments.push(Attachment {
            id: format!("attachment_{index}"),
            name: name
                .map(|s| safe_filename(s))
                .unwrap_or_else(|| format!("attachment-{index}")),
            mime_type: mime,
            size: bytes.len(),
            bytes,
            cid: cid.map(|s| s.trim().trim_matches(['<', '>']).to_owned()),
        });
    }
    Ok(())
}

pub(super) fn safe_filename(name: &str) -> String {
    let name = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let name: String = name
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\'))
        .take(180)
        .collect();
    if name.is_empty() || matches!(name.as_str(), "." | "..") {
        "attachment".to_owned()
    } else {
        name
    }
}

pub(super) fn render_html(content: &Content) -> String {
    let mut html = content
        .html
        .clone()
        .unwrap_or_else(|| format!("<pre>{}</pre>", escape_html(&content.body)));
    for attachment in &content.attachments {
        if let Some(cid) = &attachment.cid {
            // SVG and HTML are deliberately excluded from active inline resources.
            if matches!(
                attachment.mime_type.as_str(),
                "image/png" | "image/jpeg" | "image/gif" | "image/webp"
            ) {
                html = html.replace(
                    &format!("cid:{cid}"),
                    &format!(
                        "data:{};base64,{}",
                        attachment.mime_type,
                        STANDARD.encode(&attachment.bytes)
                    ),
                );
            }
        }
    }
    html
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(super) fn end_boundary(body: &str, start: usize, budget: usize) -> usize {
    let mut end = start.saturating_add(budget).min(body.len());
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    if end < body.len() {
        if let Some(paragraph) = body[start..end].rfind("\n\n") {
            if paragraph >= budget / 2 {
                return start + paragraph + 2;
            }
        }
    }
    end
}
