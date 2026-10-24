use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use image::ImageEncoder;
use serde_json::{json, Value};
use uuid::Uuid;

struct IsolatedHome {
    root: PathBuf,
}

impl IsolatedHome {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("agentic-email-render-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let home = Self { root };
        for directory in ["home", "config", "data", "cache", "state", "runtime", "tmp"] {
            fs::create_dir(home.root.join(directory)).unwrap();
        }
        home
    }

    fn worker(&self, request: Value) -> Value {
        let mut child = Command::new(env!("CARGO_BIN_EXE_agentic-gpt"))
            .arg("__email-render-worker")
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_RUNTIME_DIR", self.root.join("runtime"))
            .env("TMPDIR", self.root.join("tmp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut worker = WorkerGuard {
            child,
            reader: Some(thread::spawn(move || {
                let mut bytes = Vec::new();
                stdout.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                Ok(bytes)
            })),
        };
        let input = serde_json::to_vec(&request).unwrap();
        let mut stdin = worker.child.stdin.take().unwrap();
        stdin.write_all(&input).unwrap();
        drop(stdin);
        let deadline = Instant::now() + Duration::from_secs(40);
        let status = loop {
            if let Some(status) = worker.child.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "email renderer did not terminate"
            );
            thread::sleep(Duration::from_millis(20));
        };
        assert!(status.success(), "email renderer exited with {status}");
        let output = worker.reader.take().unwrap().join().unwrap().unwrap();
        assert!(output.len() <= 8 * 1024 * 1024, "unbounded renderer output");
        let output: Value = serde_json::from_slice(&output).unwrap();
        assert!(output.get("error").is_none(), "renderer failed: {output}");
        output
    }
}

impl Drop for IsolatedHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct WorkerGuard {
    child: Child,
    reader: Option<JoinHandle<std::io::Result<Vec<u8>>>>,
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn solid_png(color: [u8; 4]) -> Vec<u8> {
    let pixels: Vec<u8> = (0..16).flat_map(|_| color).collect();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&pixels, 4, 4, image::ExtendedColorType::Rgba8)
        .unwrap();
    png
}

fn decode_output(output: &Value) -> image::RgbaImage {
    let bytes = STANDARD.decode(output["data"].as_str().unwrap()).unwrap();
    image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .unwrap()
        .to_rgba8()
}

// Protect the executable boundary and actual rendering pipeline, including
// images discovered by the layout engine. No other test exercises this worker.
#[test]
fn worker_renders_email_slices_without_fetching_http_or_file_images() {
    let home = IsolatedHome::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let remote = format!("http://{}/must-not-request", listener.local_addr().unwrap());
    let inline_color = [23, 181, 67, 255];
    let file_color = [241, 17, 229, 255];
    let local_png = home.root.join("local-marker.png");
    fs::write(&local_png, solid_png(file_color)).unwrap();
    let file_url = url::Url::from_file_path(&local_png).unwrap();
    let inline = STANDARD.encode(solid_png(inline_color));
    let srcset_color = [211, 121, 17, 255];
    let srcset = STANDARD.encode(solid_png(srcset_color));
    let background_color = [173, 29, 113, 255];
    let background = STANDARD.encode(solid_png(background_color));
    let remote_color = [47, 91, 229, 255];
    let background_urls: Vec<_> = (0..9)
        .map(|index| format!("{remote}?background={index}&kind=image"))
        .collect();
    let html = format!(
        r#"<!doctype html><html><head><style>
        .sheet-image {{ background-image:url('{stylesheet_background}'); }}
        </style></head><body background="{body_background}">
        <table style="border-collapse:collapse;width:100%">
          <tr><td style="padding:12px;background:#e4edf7">邮件摘要：中文测试</td><td>账单金额 42</td></tr>
          <tr><td><img src="data:image/png;base64,{inline}" width="32" height="32"></td><td>内联图片</td></tr>
        </table>
        <img src="{remote}" width="32" height="32">
        <div class="sheet-image" style="width:32px;height:32px">stylesheet image</div>
        <img srcset="data:image/png;base64,{srcset} 1x" width="32" height="32">
        <div style="width:32px;height:32px;background-image:url('data:image/png;base64,{background}')"></div>
        <table background="{}" style="width:100px;height:32px"><tr background="{}"><th background="{}">legacy</th><td background="{}">images</td></tr></table>
        <div style="width:32px;height:32px;background-image:url('{}')"></div>
        <div style="width:32px;height:32px;background-image:url('{}')"></div>
        <img src="file:///etc/passwd" width="32" height="32">
        <img src="{file_url}" width="32" height="32">
        <div style="height:12000px;background:#ddeeff">长邮件后续内容</div>
        <div style="width:32px;height:32px;background-image:url('{}')">后续背景</div>
        </body></html>"#,
        background_urls[0].replace('&', "&amp;"),
        background_urls[1].replace('&', "&amp;"),
        background_urls[2].replace('&', "&amp;"),
        background_urls[3].replace('&', "&amp;"),
        background_urls[4].replace('&', "&amp;"),
        background_urls[5].replace('&', "&amp;"),
        background_urls[6].replace('&', "&amp;"),
        body_background = background_urls[7].replace('&', "&amp;"),
        stylesheet_background = background_urls[8],
    );
    let discovered = home.worker(json!({"html": html, "slice": 0, "discover": true}));
    let sources = discovered["sources"].as_array().unwrap();
    for source in std::iter::once(&remote).chain(&background_urls) {
        assert!(
            sources.contains(&json!(source)),
            "layout missed image {source}: {sources:?}"
        );
    }
    // Supply only parent-approved bytes; the worker must resolve the original
    // URI without fetching it or rewriting the document's image attributes.
    let resources = json!({ (remote.clone()): STANDARD.encode(solid_png(remote_color)) });
    let first_output = home.worker(json!({"html": html, "slice": 0, "resources": resources}));
    let second_output = home.worker(json!({"html": html, "slice": 1, "resources": resources}));
    let first = decode_output(&first_output);
    let second = decode_output(&second_output);
    assert!(first.width() > 0 && first.height() > 0);
    assert!(second.width() > 0 && second.height() > 0);
    assert_eq!(first.width(), second.width());
    assert!(first_output["height"].as_u64().unwrap() > u64::from(first.height()));
    for color in [inline_color, srcset_color, background_color, remote_color] {
        assert!(
            first.pixels().any(|pixel| pixel.0 == color),
            "image was not painted: {color:?}"
        );
    }
    assert_ne!(first.as_raw(), second.as_raw(), "slice offset was ignored");
    for (output, image) in [(&first_output, &first), (&second_output, &second)] {
        assert!(output["blockedRemoteImages"].as_u64().unwrap() >= 3);
        assert!(image.pixels().any(|pixel| pixel.0 != [255, 255, 255, 255]));
        assert!(
            !image.pixels().any(|pixel| pixel.0 == file_color),
            "renderer loaded an image from a file URL"
        );
    }
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
        "renderer connected to a remote image URL"
    );
}
