use anyhow::{anyhow, Result};
use mailparse::{DispositionType, MailHeaderMap, ParsedMail};
use serde::{Deserialize, Serialize};

pub(super) const MAX_MESSAGE_BYTES: usize = 32 * 1024 * 1024;
pub(super) const TEXT_BUDGET: usize = 24 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    parse_parts(raw, Parts::Import)
}

pub(super) fn image(raw: &[u8]) -> Result<Content> {
    parse_parts(raw, Parts::Image)
}

pub(super) fn attachments(raw: &[u8]) -> Result<Vec<Attachment>> {
    Ok(parse_parts(raw, Parts::Metadata)?.attachments)
}

pub(super) fn attachment(raw: &[u8], id: &str) -> Result<Attachment> {
    parse_parts(raw, Parts::Attachment(id))?
        .attachments
        .into_iter()
        .find(|attachment| attachment.id == id)
        .ok_or_else(|| anyhow!("email_attachment_not_found"))
}

#[derive(Clone, Copy)]
enum Parts<'a> {
    Import,
    Image,
    Metadata,
    Attachment(&'a str),
}

fn parse_parts(raw: &[u8], parts: Parts<'_>) -> Result<Content> {
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
        parts,
        false,
    )?;
    let (body, format) = if !matches!(parts, Parts::Import) {
        (String::new(), "plain")
    } else if let Some(plain) = plain {
        (plain, "plain")
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
    parts: Parts<'_>,
    related_root: bool,
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
        let related_root_index = if mime == "multipart/related" {
            if let Some(start) = mail.ctype.params.get("start") {
                let start = normalize_content_id(start);
                mail.subparts.iter().position(|child| {
                    child
                        .headers
                        .get_first_value("Content-ID")
                        .is_some_and(|id| normalize_content_id(&id) == start)
                })
            } else {
                Some(0)
            }
        } else {
            None
        };
        for (index, child) in mail.subparts.iter().enumerate() {
            let child_is_related_root = related_root_index
                .map(|root_index| root_index == index)
                .unwrap_or(related_root);
            collect(
                child,
                depth + 1,
                count,
                plain,
                html,
                attachments,
                parts,
                child_is_related_root,
            )?;
        }
    } else if cid.is_some() && mail.subparts.is_empty() && !related_root {
        let index = attachments.len() + 1;
        let id = format!("attachment_{index}");
        let decode = matches!(parts, Parts::Import | Parts::Metadata | Parts::Image)
            || matches!(parts, Parts::Attachment(target) if target == id);
        let mut bytes = if decode {
            mail.get_body_raw()
                .map_err(|_| anyhow!("email_attachment_decode_failed"))?
        } else {
            Vec::new()
        };
        let size = bytes.len();
        if matches!(parts, Parts::Metadata) {
            bytes = Vec::new();
        }
        attachments.push(Attachment {
            id,
            name: name
                .map(|s| safe_filename(s))
                .unwrap_or_else(|| format!("attachment-{index}")),
            mime_type: mime,
            size,
            bytes,
            cid: cid.map(|s| s.trim().trim_matches(['<', '>']).to_owned()),
        });
    } else if (!is_attachment || related_root) && mime == "text/plain" {
        if plain.is_none() && matches!(parts, Parts::Import) {
            let body = mail
                .get_body()
                .map_err(|_| anyhow!("email_body_decode_failed"))?;
            let body = normalize_text(&body);
            if !body.is_empty() {
                *plain = Some(body);
            }
        }
    } else if (!is_attachment || related_root) && mime == "text/html" {
        if html.is_none() && matches!(parts, Parts::Import | Parts::Image) {
            *html = Some(
                mail.get_body()
                    .map_err(|_| anyhow!("email_body_decode_failed"))?,
            );
        }
    } else {
        let index = attachments.len() + 1;
        let id = format!("attachment_{index}");
        let decode = matches!(parts, Parts::Import | Parts::Metadata)
            || matches!(parts, Parts::Attachment(target) if target == id);
        let mut bytes = if decode {
            mail.get_body_raw()
                .map_err(|_| anyhow!("email_attachment_decode_failed"))?
        } else {
            Vec::new()
        };
        let size = bytes.len();
        if matches!(parts, Parts::Metadata) {
            bytes = Vec::new();
        }
        attachments.push(Attachment {
            id,
            name: name
                .map(|s| safe_filename(s))
                .unwrap_or_else(|| format!("attachment-{index}")),
            mime_type: mime,
            size,
            bytes,
            cid: cid.map(|s| normalize_content_id(&s).to_owned()),
        });
    }
    Ok(())
}

fn normalize_content_id(content_id: &str) -> &str {
    content_id.trim().trim_matches(['<', '>'])
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
    content
        .html
        .clone()
        .unwrap_or_else(|| format!("<pre>{}</pre>", escape_html(&content.body)))
}

pub(super) fn cid_resources(content: &Content) -> Vec<crate::email_render::EmailCidResource> {
    content
        .attachments
        .iter()
        .filter_map(|attachment| {
            attachment
                .cid
                .as_ref()
                .map(|content_id| crate::email_render::EmailCidResource {
                    content_id: content_id.clone(),
                    content_type: attachment.mime_type.clone(),
                    bytes: attachment.bytes.clone(),
                })
        })
        .collect()
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
