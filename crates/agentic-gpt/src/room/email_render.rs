//! Bounded, offline email rendering in a disposable worker process.
//!
//! The worker uses litehtml's CPU pixel buffer. Resource loading is limited to
//! validated bytes in data URIs; HTTP, filesystem and stylesheet imports are
//! never serviced. The parent prepares any explicitly requested remote images.

use std::collections::HashSet;
use std::process::Stdio;
use std::time::Duration;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use image::ImageEncoder;
use litehtml::email::prepare_email_html;
use litehtml::pixbuf::PixbufContainer;
use litehtml::{Document, DrawContext, Position};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

const WIDTH: u32 = 1024;
const SLICE_HEIGHT: u32 = 1600;
const MAX_HEIGHT: u32 = 32_000;
const MAX_INLINE_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_HTML_BYTES: usize = 16 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 24 * 1024 * 1024;
// Leave room for the email metadata and the MCP envelope under the shared 8 MiB bound.
const MAX_IMAGE_BASE64_BYTES: usize = 7 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = MAX_IMAGE_BASE64_BYTES + 16 * 1024;
const MAX_INLINE_IMAGES: usize = 128;
const VERSION: u8 = 2;
const WORKER_TIMEOUT: Duration = Duration::from_secs(30);
static RENDER_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
const RENDER_CSS: &str = "html { background: white; } body { font-family: sans-serif; } \
    *, *::before, *::after { animation: none !important; transition: none !important; }";

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImageCursor {
    v: u8,
    mail_id: String,
    revision: String,
    remote: bool,
    next_slice: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkerRequest {
    html: String,
    slice: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkerResponse {
    data: String,
    height: u32,
    blocked_remote_images: usize,
}

/// Render one image slice. `remote` binds the cursor's view; it never enables I/O.
pub(crate) async fn render(
    mail_id: &str,
    html: &str,
    cursor: Option<&str>,
    remote: bool,
) -> Result<Value, String> {
    if html.len() > MAX_HTML_BYTES {
        return Err("email_render_html_too_large".to_owned());
    }
    let revision = format!("{:x}", Sha256::digest(html.as_bytes()));
    let slice = match cursor {
        Some(cursor) => {
            if cursor.len() > 4096 {
                return Err("email_cursor_invalid".to_owned());
            }
            let bytes = URL_SAFE_NO_PAD
                .decode(cursor)
                .map_err(|_| "email_cursor_invalid")?;
            let decoded: ImageCursor =
                serde_json::from_slice(&bytes).map_err(|_| "email_cursor_invalid")?;
            if decoded.v != VERSION
                || decoded.mail_id != mail_id
                || decoded.revision != revision
                || decoded.remote != remote
                || decoded.next_slice == 0
                || decoded.next_slice >= MAX_HEIGHT / SLICE_HEIGHT
            {
                return Err("email_cursor_invalid".to_owned());
            }
            decoded.next_slice
        }
        None => 0,
    };
    let input = serde_json::to_vec(&WorkerRequest {
        html: html.to_owned(),
        slice,
    })
    .map_err(|_| "email_renderer_request_failed")?;
    if input.len() > MAX_REQUEST_BYTES {
        return Err("email_render_html_too_large".to_owned());
    }
    let output = run_worker(&input).await?;
    let has_more = (slice + 1) * SLICE_HEIGHT < output.height;
    let mut result = json!({
        "images": [{"mimeType": "image/png", "data": output.data}],
        "blockedRemoteImages": output.blocked_remote_images,
        "hasMore": has_more,
    });
    if has_more {
        let next = ImageCursor {
            v: VERSION,
            mail_id: mail_id.to_owned(),
            revision,
            remote,
            next_slice: slice + 1,
        };
        let next = serde_json::to_vec(&next).map_err(|_| "email_cursor_encoding_failed")?;
        result["nextCursor"] = Value::String(URL_SAFE_NO_PAD.encode(next));
    }
    Ok(result)
}

struct RenderChild(Option<Child>);

impl Drop for RenderChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.start_kill();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = child.wait().await;
                });
            }
        }
    }
}

async fn run_worker(input: &[u8]) -> Result<WorkerResponse, String> {
    let _permit = RENDER_SLOTS
        .try_acquire()
        .map_err(|_| "email_renderer_busy")?;
    let executable = std::env::current_exe().map_err(|_| "email_renderer_unavailable")?;
    let child = Command::new(executable)
        .arg("__email-render-worker")
        .env("TOKIO_WORKER_THREADS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "email_renderer_unavailable")?;
    let mut guard = RenderChild(Some(child));
    let child = guard.0.as_mut().ok_or("email_renderer_unavailable")?;
    let work = async {
        let mut stdin = child.stdin.take().ok_or("email_renderer_io_failed")?;
        let stdout = child.stdout.take().ok_or("email_renderer_io_failed")?;
        stdin
            .write_all(input)
            .await
            .map_err(|_| "email_renderer_io_failed")?;
        drop(stdin);
        let mut output = Vec::new();
        stdout
            .take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut output)
            .await
            .map_err(|_| "email_renderer_io_failed")?;
        if output.len() > MAX_RESPONSE_BYTES {
            return Err("email_render_image_too_large".to_owned());
        }
        let status = child
            .wait()
            .await
            .map_err(|_| "email_renderer_worker_failed")?;
        if !status.success() {
            return Err("email_renderer_worker_failed".to_owned());
        }
        let value: Value =
            serde_json::from_slice(&output).map_err(|_| "email_renderer_response_invalid")?;
        if let Some(error) = value.get("error").and_then(Value::as_str) {
            return Err(error.to_owned());
        }
        serde_json::from_value(value).map_err(|_| "email_renderer_response_invalid".to_owned())
    };
    let result = tokio::time::timeout(WORKER_TIMEOUT, work)
        .await
        .unwrap_or_else(|_| Err("email_renderer_timeout".to_owned()));
    // Explicitly reap on success, input/output failure and timeout. Drop handles
    // cancellation of the caller, with kill_on_drop as a final fallback.
    if let Some(mut child) = guard.0.take() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    result
}

/// Hidden command entrypoint. Call before normal Agent configuration/startup.
pub(crate) fn worker() -> anyhow::Result<()> {
    use std::io::{Read, Write};

    set_resource_limits()?;
    let mut input = Vec::new();
    std::io::stdin()
        .take(MAX_REQUEST_BYTES as u64 + 1)
        .read_to_end(&mut input)?;
    let result = if input.len() > MAX_REQUEST_BYTES {
        Err("email_render_html_too_large".to_owned())
    } else {
        serde_json::from_slice::<WorkerRequest>(&input)
            .map_err(|_| "email_renderer_request_invalid".to_owned())
            .and_then(render_document)
    };
    let output = match result {
        Ok(output) => serde_json::to_vec(&output)?,
        Err(error) => serde_json::to_vec(&json!({"error": error}))?,
    };
    std::io::stdout().write_all(&output)?;
    Ok(())
}

fn set_resource_limits() -> anyhow::Result<()> {
    for (resource, maximum) in [(libc::RLIMIT_AS, 512 * 1024 * 1024), (libc::RLIMIT_CPU, 20)] {
        let limit = libc::rlimit {
            rlim_cur: maximum,
            rlim_max: maximum,
        };
        // SAFETY: `limit` is a valid rlimit. Limits affect this disposable worker only.
        if unsafe { libc::setrlimit(resource, &limit) } != 0 {
            anyhow::bail!("email_renderer_limit_failed");
        }
    }
    Ok(())
}

fn render_document(request: WorkerRequest) -> Result<WorkerResponse, String> {
    if request.html.len() > MAX_HTML_BYTES || request.slice >= MAX_HEIGHT / SLICE_HEIGHT {
        return Err("email_renderer_request_invalid".to_owned());
    }
    // No CID resolver or URL fetcher: the caller has already supplied allowed
    // resources as data URIs. The built-in container only queues missing URLs.
    let prepared = prepare_email_html(request.html.as_bytes(), None, None);
    let mut container = PixbufContainer::new(WIDTH, SLICE_HEIGHT);
    let mut loaded = HashSet::new();
    let mut blocked = HashSet::new();
    let mut pixels = 0;
    for (uri, bytes) in prepared.images {
        load_inline_image(&mut container, &mut loaded, &mut pixels, uri, bytes)?;
    }
    for _ in 0..4 {
        let height = {
            let mut document = Document::from_html(
                &prepared.html,
                &mut container,
                // A custom master sheet replaces all built-in HTML defaults.
                // EMAIL_MASTER_CSS is only a reset, so passing it here makes
                // div/p/table inline and destroys long-email layout.
                None,
                Some(RENDER_CSS),
            )
            .map_err(|_| "email_render_html_invalid")?;
            let _ = document.render(WIDTH as f32);
            let height = document.height().ceil();
            if !height.is_finite() || height < 0.0 || height > MAX_HEIGHT as f32 {
                return Err("email_render_height_exceeded".to_owned());
            }
            let height = (height as u32).max(1);
            document.draw(
                DrawContext::default(),
                0.0,
                -(request.slice as f32 * SLICE_HEIGHT as f32),
                Some(Position {
                    x: 0.0,
                    y: 0.0,
                    width: WIDTH as f32,
                    height: SLICE_HEIGHT as f32,
                }),
            );
            height
        };
        // Background images and attributes not recognized by the preprocessing
        // helper are discovered by the real layout/drawing callbacks here.
        let mut added = false;
        for (uri, _) in container.take_pending_images() {
            if uri.starts_with("data:") {
                let bytes =
                    litehtml::html::decode_data_uri(&uri).ok_or("email_render_image_invalid")?;
                added |= load_inline_image(&mut container, &mut loaded, &mut pixels, uri, bytes)?;
            } else if !uri.starts_with("cid:") {
                blocked.insert(uri);
            }
        }
        if added {
            container.resize(WIDTH, SLICE_HEIGHT);
            continue;
        }
        if request.slice * SLICE_HEIGHT >= height {
            return Err("email_cursor_invalid".to_owned());
        }
        let slice_height = (height - request.slice * SLICE_HEIGHT).min(SLICE_HEIGHT);
        let mut rgba = container.pixels()[..(WIDTH * slice_height * 4) as usize].to_vec();
        // litehtml's buffer is premultiplied RGBA. Composite onto opaque white
        // before encoding, so transparent email backgrounds remain readable.
        for pixel in rgba.as_chunks_mut::<4>().0 {
            let white = 255 - pixel[3];
            pixel[0] = pixel[0].saturating_add(white);
            pixel[1] = pixel[1].saturating_add(white);
            pixel[2] = pixel[2].saturating_add(white);
            pixel[3] = 255;
        }
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&rgba, WIDTH, slice_height, image::ExtendedColorType::Rgba8)
            .map_err(|_| "email_render_encoding_failed")?;
        let data = STANDARD.encode(png);
        if data.len() > MAX_IMAGE_BASE64_BYTES {
            return Err("email_render_image_too_large".to_owned());
        }
        return Ok(WorkerResponse {
            data,
            height,
            blocked_remote_images: blocked.len(),
        });
    }
    Err("email_render_resources_unstable".to_owned())
}

fn load_inline_image(
    container: &mut PixbufContainer,
    loaded: &mut HashSet<String>,
    pixels: &mut u64,
    uri: String,
    bytes: Vec<u8>,
) -> Result<bool, String> {
    if loaded.contains(&uri) {
        return Ok(false);
    }
    if loaded.len() >= MAX_INLINE_IMAGES {
        return Err("email_render_image_count_exceeded".to_owned());
    }
    let output =
        crate::file_ops::read_image_bytes(&bytes, crate::file_ops::MAX_IMAGE_RESPONSE_BYTES)
            .map_err(|_| "email_render_image_invalid")?
            .ok_or("email_render_image_unsupported")?;
    let image = &output.value["image"];
    let width = image["width"]
        .as_u64()
        .ok_or("email_render_image_invalid")?;
    let height = image["height"]
        .as_u64()
        .ok_or("email_render_image_invalid")?;
    *pixels = pixels
        .checked_add(
            width
                .checked_mul(height)
                .ok_or("email_render_image_too_large")?,
        )
        .filter(|count| *count <= MAX_INLINE_PIXELS)
        .ok_or("email_render_image_too_large")?;
    // GIF validation produces bounded PNG samples. Freeze at the first sample
    // instead of allowing animation to change a later cursor slice.
    let first = output.images.first().ok_or("email_render_image_invalid")?;
    container.load_image_data(&uri, &first.bytes);
    loaded.insert(uri);
    Ok(true)
}
