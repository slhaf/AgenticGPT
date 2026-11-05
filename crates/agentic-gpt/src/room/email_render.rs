//! HTML email rendering in a short-lived Obscura worker.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use obscura_browser::page::CidResource;
use obscura_browser::{BrowserContext, CaptureRegion, Page};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use uuid::Uuid;

const WIDTH: f32 = 1024.0;
const VIEWPORT_HEIGHT: f32 = 768.0;
const SLICE_HEIGHT: f32 = 1600.0;
const MAX_IMAGE_BASE64_BYTES: usize = 7 * 1024 * 1024;
const VERSION: u8 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EmailCidResource {
    pub content_id: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImageCursor {
    v: u8,
    mail_id: String,
    revision: String,
    cache_id: String,
    next_slice: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CacheManifest {
    mail_id: String,
    revision: String,
    slices: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkerRequest {
    html: String,
    cid_resources: Vec<WorkerCidResource>,
    cache_dir: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkerCidResource {
    content_id: String,
    content_type: String,
    data: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkerResponse {
    slices: u32,
}

/// Render the page once in a disposable worker and retain its PNG slices for
/// cursor continuation. The HTML and CID bodies are supplied unchanged.
pub(crate) async fn render(
    cache_root: &Path,
    mail_id: &str,
    html: &str,
    cid_resources: &[EmailCidResource],
    cursor: Option<&str>,
) -> Result<Value, String> {
    let revision = content_revision(html, cid_resources);
    if let Some(cursor) = cursor {
        let cursor = decode_cursor(cursor, mail_id, &revision)?;
        let directory = cache_directory(cache_root, &cursor.cache_id)?;
        let manifest: CacheManifest = read_json(&directory.join("manifest.json"))?;
        if manifest.mail_id != mail_id
            || manifest.revision != revision
            || cursor.next_slice >= manifest.slices
        {
            return Err("email_cursor_invalid".to_owned());
        }
        let mut cache_guard = RenderCacheGuard::new(directory.clone());
        let image = read_slice(&directory, cursor.next_slice)?;
        let has_more = cursor.next_slice + 1 < manifest.slices;
        let mut result = image_result(image, has_more);
        if has_more {
            result["nextCursor"] = json!(encode_cursor(&ImageCursor {
                next_slice: cursor.next_slice + 1,
                ..cursor
            })?);
            cache_guard.keep();
        }
        return Ok(result);
    }

    let cache_id = Uuid::new_v4().simple().to_string();
    let directory = create_cache_directory(cache_root, &cache_id)?;
    let mut cache_guard = RenderCacheGuard::new(directory.clone());
    let request = WorkerRequest {
        html: html.to_owned(),
        cid_resources: cid_resources
            .iter()
            .map(|resource| WorkerCidResource {
                content_id: resource.content_id.clone(),
                content_type: resource.content_type.clone(),
                data: STANDARD.encode(&resource.bytes),
            })
            .collect(),
        cache_dir: directory.clone(),
    };
    let output = run_worker(&request).await?;
    if output.slices == 0 {
        return Err("email_renderer_response_invalid".to_owned());
    }
    write_json(
        &directory.join("manifest.json"),
        &CacheManifest {
            mail_id: mail_id.to_owned(),
            revision: revision.clone(),
            slices: output.slices,
        },
    )?;
    let image = read_slice(&directory, 0)?;
    let has_more = output.slices > 1;
    let mut result = image_result(image, has_more);
    if has_more {
        result["nextCursor"] = json!(encode_cursor(&ImageCursor {
            v: VERSION,
            mail_id: mail_id.to_owned(),
            revision,
            cache_id,
            next_slice: 1,
        })?);
        cache_guard.keep();
    }
    Ok(result)
}

fn image_result(image: Vec<u8>, has_more: bool) -> Value {
    json!({
        "images": [{"mimeType": "image/png", "data": STANDARD.encode(image)}],
        "hasMore": has_more,
    })
}

fn encode_cursor(cursor: &ImageCursor) -> Result<String, String> {
    let bytes = serde_json::to_vec(cursor).map_err(|_| "email_cursor_invalid")?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn content_revision(html: &str, resources: &[EmailCidResource]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(html.as_bytes());
    let mut resources = resources.iter().collect::<Vec<_>>();
    resources.sort_by(|left, right| left.content_id.cmp(&right.content_id));
    for resource in resources {
        hasher.update(resource.content_id.as_bytes());
        hasher.update(resource.content_type.as_bytes());
        hasher.update(&resource.bytes);
    }
    format!("{:x}", hasher.finalize())
}

fn decode_cursor(cursor: &str, mail_id: &str, revision: &str) -> Result<ImageCursor, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| "email_cursor_invalid")?;
    let decoded: ImageCursor =
        serde_json::from_slice(&bytes).map_err(|_| "email_cursor_invalid")?;
    if decoded.v != VERSION
        || decoded.mail_id != mail_id
        || decoded.revision != revision
        || decoded.next_slice == 0
        || Uuid::parse_str(&decoded.cache_id).is_err()
    {
        return Err("email_cursor_invalid".to_owned());
    }
    Ok(decoded)
}

fn cache_directory(cache_root: &Path, cache_id: &str) -> Result<PathBuf, String> {
    let uuid = Uuid::parse_str(cache_id).map_err(|_| "email_cursor_invalid")?;
    let directory = cache_root.join(uuid.simple().to_string());
    if !directory.is_dir() {
        return Err("email_cursor_invalid".to_owned());
    }
    Ok(directory)
}

fn create_cache_directory(cache_root: &Path, cache_id: &str) -> Result<PathBuf, String> {
    let directory = cache_root.join(cache_id);
    std::fs::create_dir(&directory).map_err(|_| "email_renderer_cache_unavailable")?;
    Ok(directory)
}

struct RenderCacheGuard {
    directory: PathBuf,
    remove: bool,
}

impl RenderCacheGuard {
    fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            remove: true,
        }
    }

    fn keep(&mut self) {
        self.remove = false;
    }
}

impl Drop for RenderCacheGuard {
    fn drop(&mut self) {
        if self.remove {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
}

fn write_cache_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "email_renderer_cache_unavailable".to_owned())?;
    file.write_all(bytes)
        .map_err(|_| "email_renderer_cache_unavailable".to_owned())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|_| "email_renderer_cache_unavailable")?;
    write_cache_file(path, &bytes)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes = std::fs::read(path).map_err(|_| "email_cursor_invalid")?;
    serde_json::from_slice(&bytes).map_err(|_| "email_cursor_invalid".to_owned())
}

fn slice_path(directory: &Path, slice: u32) -> PathBuf {
    directory.join(format!("slice-{slice}.png"))
}

fn read_slice(directory: &Path, slice: u32) -> Result<Vec<u8>, String> {
    let path = slice_path(directory, slice);
    let image = std::fs::read(path).map_err(|_| "email_cursor_invalid")?;
    let base64_len = image.len().div_ceil(3) * 4;
    if base64_len > MAX_IMAGE_BASE64_BYTES {
        return Err("email_render_image_too_large".to_owned());
    }
    Ok(image)
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

async fn run_worker(request: &WorkerRequest) -> Result<WorkerResponse, String> {
    let input = serde_json::to_vec(request).map_err(|_| "email_renderer_request_failed")?;
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
        let mut stdout = child.stdout.take().ok_or("email_renderer_io_failed")?;
        stdin
            .write_all(&input)
            .await
            .map_err(|_| "email_renderer_io_failed")?;
        drop(stdin);
        let mut output = Vec::new();
        stdout
            .read_to_end(&mut output)
            .await
            .map_err(|_| "email_renderer_io_failed")?;
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
    let result = work.await;
    if let Some(mut child) = guard.0.take() {
        let _ = child.wait().await;
    }
    result
}

/// Hidden command entrypoint. The current-thread runtime keeps Obscura page,
/// V8 and their destruction on the same thread.
pub(crate) fn worker() -> anyhow::Result<()> {
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    let result = serde_json::from_slice::<WorkerRequest>(&input)
        .map_err(|_| "email_renderer_request_invalid".to_owned())
        .and_then(|request| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| "email_renderer_runtime_unavailable".to_owned())?
                .block_on(render_document(request))
        });
    let output = match result {
        Ok(output) => serde_json::to_vec(&output)?,
        Err(error) => serde_json::to_vec(&json!({"error": error}))?,
    };
    std::io::stdout().write_all(&output)?;
    Ok(())
}

async fn render_document(request: WorkerRequest) -> Result<WorkerResponse, String> {
    let cid_resources = request
        .cid_resources
        .into_iter()
        .map(|resource| {
            Ok(CidResource {
                content_id: resource.content_id,
                content_type: resource.content_type,
                bytes: STANDARD
                    .decode(resource.data)
                    .map_err(|_| "email_renderer_cid_invalid")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut context = BrowserContext::for_email_rendering("email-render".to_owned());
    context.set_allow_file_access(true);
    let mut page = Page::new("email-render".to_owned(), Arc::new(context));
    page.set_viewport((WIDTH, VIEWPORT_HEIGHT));
    page.navigate_html(request.html.as_bytes(), None, cid_resources)
        .await
        .map_err(|error| error.to_string())?;

    // Navigation has its own readiness behavior. Drain renderer resources to
    // completion as well, without imposing a screenshot-specific wait cutoff.
    page.spawn_pending_render_resources();
    while page.has_pending_render_resources() {
        page.drain_render_resource_results();
        if page.has_pending_render_resources() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    page.drain_render_resource_results();

    let (_, measured_height) = page
        .prepared_content_size()
        .ok_or_else(|| "email_renderer_layout_unavailable".to_owned())?;
    let height = measured_height.ceil().max(1.0);
    if !height.is_finite() {
        return Err("email_renderer_layout_unavailable".to_owned());
    }
    let animation_time = page.live_animation_sample().time;
    let mut y = 0.0;
    let mut slice = 0u32;
    while y < height {
        let mut slice_height = SLICE_HEIGHT.min(height - y);
        loop {
            let png = page
                .screenshot_region_at_animation_time(
                    CaptureRegion::new(0.0, y, WIDTH, slice_height, 1.0),
                    animation_time,
                )
                .map_err(|_| "email_renderer_screenshot_failed".to_owned())?;
            if png.len().div_ceil(3) * 4 <= MAX_IMAGE_BASE64_BYTES {
                write_cache_file(&slice_path(&request.cache_dir, slice), &png)?;
                break;
            }
            if slice_height <= 1.0 {
                return Err("email_render_image_too_large".to_owned());
            }
            slice_height = (slice_height / 2.0).floor().max(1.0);
        }
        y += slice_height;
        slice += 1;
    }
    Ok(WorkerResponse { slices: slice })
}
