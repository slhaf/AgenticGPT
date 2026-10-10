use super::browser_distribution_verify::{
    parse_inrelease_metadata, parse_pinned_repository_key, select_chatgpt_package,
    verify_cleartext_with_key, AuthenticatedPackagesIndex,
};
use super::*;
use parking_lot::Mutex;
use pgp::{
    composed::{Deserializable, SignedPublicKey},
    types::KeyDetails,
};
use std::io::Cursor;
use tar::Builder;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use xz2::write::XzEncoder;

const TEST_PUBLIC_KEY: &str = r#"-----BEGIN PGP PUBLIC KEY BLOCK-----
Version: GnuPG v2

mQENBFdqP+gBCACoG81sddF9ZZx6TsN7lenDxs53wqJt1bawToXZl6qJ+hO8tMNy
/aUgaqqi8pC5FX3HlSRQhjRFDQXq4jR3+jecFK3wfwmxTKXMKhN52zhSIWClED47
56B3wPITuwAG2WYFccClhrtWX8j+wBK5IpLVVAXQnORZLOP7fzUZkO1KDu3bP4D6
f1LQyatIzgS08dnlp6WvsJDrkUeGY6R5smpx9f+PqBVfUVpmckbMOR+BYPJtplZT
0lez4qWsmdWbqU7ZhdvnhMGcTbBbk9WbfR1IJptigeuK8vTU24Jp2FQj6iIBV9OD
jUfr11EJ6W3TvHaWuddd0hfX9DnyH9rghrDFABEBAAG0FnN0ZXZlLmJpa29AZXhh
bXBsZS5uZXSJATcEEwEIACEFAldqP+gCGwMFCwkIBwIGFQgJCgsCBBYCAwECHgEC
F4AACgkQqkPx3Mf+0bd5kggAphS7UDycKadfaRH5JENmKXeI+UUd+E0iERwv7eXq
RcgjNK1oHQSXN+ejDEXzZv2fcCRB7rWEvEXL0pCtPveyzDAQJdhZTRVgmfCXTr1m
9pJfVC3B20jgx6ZxZO8jKDL+bqvufWJczWDT0iHP0Jv04SqASLRs2JRPy+a+w3GJ
+DzG8orfAKiIE1Qycovr8Ol+jdo9ZV9blRA8/j4eqZYg4b7AOf8/mDyXsx3xzSPV
uwkDSluhaOrsV8N0suZ51rfdpapv6VJsXlyQbceJwwgSt2A1n2Sw3ZINwpO7BODy
wO6J44751+qY4cmap4NItyqGQTT6TUEL9ANfrZFmPWmFWLkBDQRXaj/oAQgA255C
UJxFEKLVwEoSgwZqXd94AhjGUbMY6NXdFj5cCq0JmWZrbpT/5OblTrymiH1iLmI0
ymo+/s8vh6NtB98dhr1syH3asNQfXZRfF+u5X5hLDNPF4sUelsl4+EUef0Hbc9U+
e+8F8A9TMxELSqQ8Ul3Hu42hc+/ugkc1G/8++Sv/f60TqWcUR2GmuiAvkuS1WmdA
TMhwPr7vMfssV0X0mboz32//b/UfuOyctso5FM+bRaKrEJDQ2WDg57yqnaqsKEga
jW0jElpAVIn792W6YWKOk4auYSpO5f7BVs40Z+bxKGxiH87z9fnmlYAsQwPOOxZw
WaCSrReeheK6c6emAQARAQABiQEfBBgBCAAJBQJXaj/oAhsMAAoJEKpD8dzH/tG3
baoH/0KI3pIUiIYiLESGXqF+s/W2BmGNwdkYldcyFwkXz84VXoG0B3k7nrwT2DOJ
AEeToavzd3J+aZ4PmxBRAMtDhah0wsMXrwCI8y9Stmm6PIssnu9IP9+jgr4IkKIR
UB/Wn6nzgseaNd7vN4JChCyLSvF+vLd3D56Wzq+hBjybaE+zcEusVLdKYDm2i0YC
pkBkmSuC18lLxhNC8oSCCvVOiyw+TqGHhLnrpA4nGi0MLjAR3OgJ5d/TclYgkLcp
yOupg9GplQsAZUFfQPrY80SJuN9ijBp4xtA1U+WCGKh4ySv1+odpRjPX3eOGUFKZ
sJRKpZupoGWfVN78wm1nPLBKTvM=
=6N/A
-----END PGP PUBLIC KEY BLOCK-----"#;

const TEST_SIGNED_CLEARTEXT: &str = r#"-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA256

You are scrupulously honest, frank, and straightforward.  Therefore you
have few friends.
-----BEGIN PGP SIGNATURE-----
Version: GnuPG v2

iQE0BAEBCAAeBQJXakWmFxxzdGV2ZS5iaWtvQGV4YW1wbGUubmV0AAoJEKpD8dzH
/tG3OiIH/18NlMSXXRFRrxXq9OZySzJxgLI7BjGilRTqb4ALeFzNjmCwu3Y+Gkdg
t7NjYjSe0erWiKYDEmALICwcpmSmXHA//gol3QkHJKIlKQGXJP1qLvIde5+lnK8K
YVwLKLBQBQtlGMkMXPdUEn9PgzSoBFoFIqrzQmAdLO3yijSdm0Mzl9wyIhtbUXk+
VgX2d/6DRIwcKcFoX2QbFlM/z1kdrS6cOYFbJWavEpLDz9ON8Q8a8uqcBiqRlSpW
eGOMMsysJs+44+qX6uE3hu2KJE9xvHwhSjJOxqtw8dN3KZ1+8IkxsDrvDAhn+Klf
Hbtj647f/iTOF88o1ihO7goDi93Bpv4=
=xAv4
-----END PGP SIGNATURE-----"#;

#[test]
fn pinned_repository_key_has_exact_fingerprint() {
    let key = parse_pinned_repository_key().unwrap();
    assert_eq!(
        format!("{:X}", key.fingerprint()),
        PINNED_REPOSITORY_FINGERPRINT
    );
}

#[test]
fn cleartext_signature_verifies_and_tampering_fails() {
    let (key, _) = SignedPublicKey::from_string(TEST_PUBLIC_KEY).unwrap();
    let authenticated = verify_cleartext_with_key(TEST_SIGNED_CLEARTEXT.as_bytes(), &key).unwrap();
    assert!(authenticated.contains("scrupulously honest"));

    let tampered = TEST_SIGNED_CLEARTEXT.replace("honest", "dishonest");
    assert_eq!(
        verify_cleartext_with_key(tampered.as_bytes(), &key)
            .unwrap_err()
            .to_string(),
        "browser_runtime_acquisition_inrelease_signature_invalid"
    );
}

#[test]
fn authenticated_inrelease_metadata_selects_exact_target() {
    let amd64 = "a".repeat(64);
    let arm64 = "b".repeat(64);
    let body = format!(
            "Suite: stable\nCodename: stable\nDate: Tue, 1 Jul 2003 10:52:37 +0200\n\
             SHA256:\n {amd64} 42 main/binary-amd64/Packages\n {arm64} 43 main/binary-arm64/Packages\n"
        );
    let index = parse_inrelease_metadata(&body, "amd64").unwrap();
    assert_eq!(
        index,
        AuthenticatedPackagesIndex {
            path: "main/binary-amd64/Packages".into(),
            sha256: amd64,
            size: 42,
        }
    );
    let fetch_path = packages_fetch_path(&index).unwrap();
    assert_eq!(fetch_path, "dists/stable/main/binary-amd64/Packages");
}

type RecordedFetchResponse = (String, u16, Vec<u8>);
type RecordedPackageResponse = (u16, Vec<u8>);
type SharedFetchResponses = Arc<Mutex<Vec<RecordedFetchResponse>>>;
type SharedPackageResponse = Arc<Mutex<Option<RecordedPackageResponse>>>;

#[derive(Clone, Default)]
struct TestFetcher {
    responses: SharedFetchResponses,
    package: SharedPackageResponse,
}

impl BrowserDistributionFetcher for TestFetcher {
    fn fetch_bytes(&self, path: String, _max_bytes: u64) -> BrowserDistributionFuture {
        let response = self
            .responses
            .lock()
            .iter()
            .find(|(candidate, _, _)| candidate == &path)
            .map(|(_, status, body)| BrowserDistributionResponse {
                status: *status,
                body: body.clone(),
            });
        Box::pin(
            async move { response.ok_or_else(|| error("browser_runtime_acquisition_http_failed")) },
        )
    }

    fn stream_file(
        &self,
        _path: String,
        destination: tokio::fs::File,
        _expected_size: u64,
        _expected_sha256: String,
    ) -> BrowserDistributionFuture {
        let package = self.package.lock().clone();
        Box::pin(async move {
            let Some((status, body)) = package else {
                return Err(error("browser_runtime_acquisition_http_failed"));
            };
            let mut output = destination;
            output
                .write_all(&body)
                .await
                .map_err(|_| error("browser_runtime_acquisition_temp_failed"))?;
            output
                .sync_all()
                .await
                .map_err(|_| error("browser_runtime_acquisition_temp_failed"))?;
            Ok(BrowserDistributionResponse {
                status,
                body: Vec::new(),
            })
        })
    }
}

async fn local_http_response(
    status: u16,
    body: &[u8],
    declared_length: Option<usize>,
    location: Option<&str>,
) -> (reqwest::Response, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let body = body.to_vec();
    let declared_length = declared_length.unwrap_or(body.len());
    let location = location.map(str::to_owned);
    let task = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).await;
            let reason = if status == 200 { "OK" } else { "Found" };
            let mut response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: {declared_length}\r\n\
                     Connection: close\r\n"
            );
            if let Some(location) = location {
                response.push_str(&format!("Location: {location}\r\n"));
            }
            response.push_str("\r\n");
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.write_all(&body).await;
        }
    });
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client
        .get(format!("http://{address}/fixture"))
        .send()
        .await
        .unwrap();
    (response, task)
}

#[test]
fn fixed_repository_url_rejects_untrusted_path_forms() {
    let url = fixed_repository_url(INRELEASE_PATH).unwrap();
    assert_eq!(url.scheme(), "https");
    assert_eq!(url.host_str(), Some(OPENAI_REPOSITORY_HOST));
    for path in [
        "../evil",
        "pool/main/c/chatgpt/../evil.deb",
        "https://evil.example/package.deb",
        "pool/main/c/chatgpt/package.deb?redirect=1",
        "pool/main/c/chatgpt/package.deb#fragment",
    ] {
        assert_eq!(
            fixed_repository_url(path).unwrap_err().to_string(),
            "browser_runtime_acquisition_url_invalid"
        );
    }
}

#[tokio::test]
async fn local_http_status_redirect_and_body_bounds_are_rejected() {
    let (redirect, task) = local_http_response(
        302,
        b"redirect",
        None,
        Some("http://127.0.0.1:9/should-not-follow"),
    )
    .await;
    assert_eq!(
        fetch_http_response(redirect, MAX_INRELEASE_BYTES)
            .await
            .unwrap_err()
            .to_string(),
        "browser_runtime_acquisition_http_status"
    );
    task.await.unwrap();

    let body = vec![b'x'; 4];
    let (oversized, task) = local_http_response(200, &body, None, None).await;
    assert_eq!(
        fetch_http_response(oversized, 3)
            .await
            .unwrap_err()
            .to_string(),
        "browser_runtime_acquisition_response_size_limit"
    );
    task.await.unwrap();
}

#[tokio::test]
async fn streamed_package_checks_size_hash_and_temp_cleanup() {
    let root = temp();
    let body = b"small package bytes";
    let destination = root.join("package.deb");
    let (guard, destination_file) = AcquisitionTempFile::claim(destination.clone()).unwrap();
    let (response, task) = local_http_response(200, body, None, None).await;
    stream_http_response(response, destination_file, body.len() as u64, sha256(body))
        .await
        .unwrap();
    task.await.unwrap();
    assert_eq!(fs::read(&destination).unwrap(), body);
    drop(guard);
    assert!(!destination.exists());

    let mismatch = root.join("mismatch.deb");
    let (guard, mismatch_file) = AcquisitionTempFile::claim(mismatch.clone()).unwrap();
    let (response, task) = local_http_response(200, body, None, None).await;
    assert_eq!(
        stream_http_response(response, mismatch_file, body.len() as u64, "a".repeat(64),)
            .await
            .unwrap_err()
            .to_string(),
        "browser_runtime_acquisition_download_hash_mismatch"
    );
    task.await.unwrap();
    drop(guard);
    assert!(!mismatch.exists());

    let overshoot = root.join("overshoot.deb");
    let (guard, overshoot_file) = AcquisitionTempFile::claim(overshoot.clone()).unwrap();
    let (response, task) = local_http_response(200, body, None, None).await;
    assert_eq!(
        stream_http_response(
            response,
            overshoot_file,
            body.len() as u64 - 1,
            sha256(body),
        )
        .await
        .unwrap_err()
        .to_string(),
        "browser_runtime_acquisition_download_oversize"
    );
    task.await.unwrap();
    drop(guard);
    assert!(!overshoot.exists());

    let short = root.join("short.deb");
    let (guard, short_file) = AcquisitionTempFile::claim(short.clone()).unwrap();
    let (response, task) = local_http_response(200, body, None, None).await;
    assert_eq!(
        stream_http_response(response, short_file, body.len() as u64 + 1, sha256(body),)
            .await
            .unwrap_err()
            .to_string(),
        "browser_runtime_acquisition_download_size_mismatch"
    );
    task.await.unwrap();
    drop(guard);
    assert!(!short.exists());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn provisioning_lock_rechecks_cache_after_waiting() {
    let root = temp();
    let cache = root.join("cache");
    let codex_home = root.join("codex");
    fs::create_dir_all(cache.join(".locks")).unwrap();
    let lock_path = cache
        .join(".locks")
        .join(format!("{}.provision.lock", current_target()));
    let held = Lock::acquire(&lock_path).unwrap();
    let fetcher = Arc::new(TestFetcher::default());
    let waiting_cache = cache.clone();
    let waiting_codex_home = codex_home.clone();
    let waiting_target = current_target().to_owned();
    let waiting_fetcher = fetcher.clone();
    let waiting = tokio::spawn(async move {
        provision_managed_browser_runtime_with_fetcher(
            &waiting_cache,
            &waiting_codex_home,
            &waiting_target,
            waiting_fetcher,
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let bytes = deb();
    let package = package(&root, &bytes);
    let expected = materialize_browser_runtime_at(&cache, package).unwrap();
    drop(held);

    let actual = waiting.await.unwrap().unwrap();
    assert_eq!(actual, expected);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn acquisition_temp_cleanup_handles_materializer_success_and_failure() {
    let root = temp();
    let cache = root.join("cache");
    let bytes = deb();
    let input = root.join("input.deb");
    fs::write(&input, &bytes).unwrap();
    let successful = root.join("successful.part");
    let (guard, successful_file) = AcquisitionTempFile::claim(successful.clone()).unwrap();
    drop(successful_file);
    fs::write(&successful, &bytes).unwrap();
    let package = VerifiedBrowserPackage {
        deb_path: input.clone(),
        deb_sha256: sha256(&bytes),
        app_version: "1.2.3".into(),
        channel: "prod".into(),
        target: current_target().into(),
        codex_home: root.join("codex"),
    };
    assert!(materialize_browser_runtime_at(&cache, package).is_ok());
    drop(guard);
    assert!(!successful.exists());

    let failed = root.join("failed.part");
    let (guard, failed_file) = AcquisitionTempFile::claim(failed.clone()).unwrap();
    drop(failed_file);
    fs::write(&failed, &bytes).unwrap();
    let package = VerifiedBrowserPackage {
        deb_path: failed.clone(),
        deb_sha256: "b".repeat(64),
        app_version: "1.2.3".into(),
        channel: "prod".into(),
        target: current_target().into(),
        codex_home: root.join("codex"),
    };
    assert_eq!(
        materialize_browser_runtime_at(&cache, package)
            .unwrap_err()
            .to_string(),
        "browser_runtime_package_hash_mismatch"
    );
    drop(guard);
    assert!(!failed.exists());
    let _ = fs::remove_dir_all(root);
}

fn package_paragraph(
    version: &str,
    architecture: &str,
    filename: &str,
    size: &str,
    digest: &str,
) -> String {
    format!(
        "Package: chatgpt\nVersion: {version}\nArchitecture: {architecture}\n\
             Filename: {filename}\nSize: {size}\nSHA256: {digest}\n"
    )
}

#[test]
#[ignore = "manual real-package materialization smoke"]
fn real_package_materializes_from_explicit_fixture_path() {
    let deb_path = std::env::var_os("AGENTIC_BROWSER_REAL_DEB")
        .map(PathBuf::from)
        .expect("AGENTIC_BROWSER_REAL_DEB must point to a verified official .deb");
    let deb_sha256 = std::env::var("AGENTIC_BROWSER_REAL_DEB_SHA256")
        .expect("AGENTIC_BROWSER_REAL_DEB_SHA256 must contain the verified package digest");
    let app_version = std::env::var("AGENTIC_BROWSER_REAL_APP_VERSION")
        .expect("AGENTIC_BROWSER_REAL_APP_VERSION must contain the verified package version");
    let scratch = temp();
    let cache_root = std::env::var_os("AGENTIC_BROWSER_REAL_CACHE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| scratch.join("cache"));
    let package = VerifiedBrowserPackage {
        deb_path,
        deb_sha256,
        app_version: app_version.clone(),
        channel: "prod".into(),
        target: current_target().into(),
        codex_home: scratch.join("codex-home"),
    };
    let descriptor = materialize_browser_runtime_at(&cache_root, package).unwrap();
    assert_eq!(descriptor.app_version, app_version);
    assert_eq!(descriptor.channel, "prod");
    assert!(descriptor.node_repl_path.is_file());
    assert!(descriptor.browser_client_path.is_file());
    assert!(descriptor.docs_root.is_dir());
    let chrome_root = descriptor
        .browser_service_path
        .parent()
        .and_then(Path::parent)
        .expect("browser service remains inside the selected chrome bundle");
    assert!(descriptor
        .trusted_code_paths
        .iter()
        .any(|path| path == chrome_root));
    let _ = fs::remove_dir_all(scratch);
}

fn current_target() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "linux-arm64",
        _ => "linux-x64",
    }
}

fn encoded_tar(build: impl FnOnce(&mut Builder<XzEncoder<&mut Vec<u8>>>)) -> Vec<u8> {
    let mut compressed = Vec::new();
    {
        let encoder = XzEncoder::new(&mut compressed, 1);
        let mut tar = Builder::new(encoder);
        build(&mut tar);
        tar.into_inner().unwrap().finish().unwrap();
    }
    compressed
}

fn append_file(tar: &mut Builder<XzEncoder<&mut Vec<u8>>>, name: &str, data: &[u8], mode: u32) {
    let mut header = tar::Header::new_gnu();
    header.set_path(name).unwrap();
    header.set_size(data.len() as u64);
    header.set_mode(mode);
    header.set_cksum();
    tar.append(&header, Cursor::new(data)).unwrap();
}

fn append_directory(tar: &mut Builder<XzEncoder<&mut Vec<u8>>>, name: &str, mode: u32) {
    let mut header = tar::Header::new_gnu();
    header.set_path(name).unwrap();
    header.set_entry_type(EntryType::Directory);
    header.set_size(0);
    header.set_mode(mode);
    header.set_cksum();
    tar.append(&header, std::io::empty()).unwrap();
}

fn append_special(tar: &mut Builder<XzEncoder<&mut Vec<u8>>>, name: &str, entry_type: EntryType) {
    let mut header = tar::Header::new_gnu();
    header.set_path(name).unwrap();
    header.set_entry_type(entry_type);
    header.set_size(0);
    header.set_mode(0o777);
    if entry_type == EntryType::symlink() || entry_type == EntryType::hard_link() {
        header.set_link_name("outside").unwrap();
    }
    header.set_cksum();
    tar.append(&header, std::io::empty()).unwrap();
}

fn tar_fixture() -> Vec<u8> {
    encoded_tar(|tar| {
        append_directory(tar, "usr/lib/chatgpt/resources/cua_node/lib", 0o555);
        for (name, data, mode) in [
                (
                    "usr/lib/chatgpt/resources/cua_node/bin/node",
                    b"node".as_slice(),
                    0o6755,
                ),
                (
                    "usr/lib/chatgpt/resources/cua_node/bin/node_repl",
                    b"repl".as_slice(),
                    0o2755,
                ),
                (
                    "usr/lib/chatgpt/resources/cua_node/lib/node_modules/package.json",
                    b"{}".as_slice(),
                    0o644,
                ),
                (
                    "usr/lib/chatgpt/resources/cua_node/shared.txt",
                    b"cua-shared".as_slice(),
                    0o644,
                ),
                (
                    "usr/lib/chatgpt/resources/plugins/openai-bundled/plugins/chrome/scripts/browser-client.mjs",
                    b"client".as_slice(),
                    0o644,
                ),
                (
                    "usr/lib/chatgpt/resources/plugins/openai-bundled/plugins/chrome/scripts/browser-service.mjs",
                    b"service".as_slice(),
                    0o644,
                ),
                (
                    "usr/lib/chatgpt/resources/plugins/openai-bundled/plugins/chrome/docs/index.md",
                    b"docs".as_slice(),
                    0o644,
                ),
                (
                    "usr/lib/chatgpt/resources/plugins/openai-bundled/plugins/chrome/shared.txt",
                    b"chrome-shared".as_slice(),
                    0o644,
                ),
                ("usr/share/unrelated.txt", b"ignored".as_slice(), 0o644),
            ] {
                append_file(tar, name, data, mode);
            }
    })
}
fn ar_member(name: &str, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let header = format!(
        "{:<16}{:<12}{:<6}{:<6}{:<8}{:>10}`\n",
        name,
        0,
        0,
        0,
        0,
        body.len()
    );
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(b'\n');
    }
    out
}
fn deb() -> Vec<u8> {
    let mut out = AR_MAGIC.to_vec();
    out.extend(ar_member("debian-binary", b"2.0\n"));
    out.extend(ar_member("control.tar.xz", b"not used"));
    out.extend(ar_member("data.tar.xz", &tar_fixture()));
    out
}
fn package(root: &Path, bytes: &[u8]) -> VerifiedBrowserPackage {
    let path = root.join("input.deb");
    fs::write(&path, bytes).unwrap();
    VerifiedBrowserPackage {
        deb_path: path,
        deb_sha256: sha256(bytes),
        app_version: "1.2.3".into(),
        channel: "prod".into(),
        target: current_target().into(),
        codex_home: root.join("codex"),
    }
}
fn temp() -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "browser-distribution-{}-{}",
        std::process::id(),
        unique()
    ));
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn package_consumers_reject_invalid_path_components_and_hashes() {
    let digest = "a".repeat(64);
    let unsafe_filename = package_paragraph(
        "1.2.3",
        "amd64",
        "pool/main/c/chatgpt/../escape.deb",
        "3",
        &digest,
    );
    assert_eq!(
        select_chatgpt_package(unsafe_filename.as_bytes(), "amd64")
            .unwrap_err()
            .to_string(),
        "browser_runtime_acquisition_package_filename_invalid"
    );

    let root = temp();
    let mut package = package(&root, &deb());
    package.deb_sha256 = "A".repeat(64);
    assert_eq!(
        materialize_browser_runtime_at(&root.join("cache"), package)
            .unwrap_err()
            .to_string(),
        "browser_runtime_package_hash_invalid"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn synthetic_deb_materializes_only_selected_resources() {
    let root = temp();
    let p = package(&root, &deb());
    let descriptor = materialize_browser_runtime_at(&root.join("cache"), p).unwrap();
    assert_eq!(descriptor.codex_cli_path, None);
    assert!(descriptor.node_path.exists());
    assert!(descriptor.browser_client_path.exists());
    assert_eq!(
        descriptor.docs_root,
        descriptor
            .browser_client_path
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("docs")
    );
    let cua_root = descriptor.node_path.parent().unwrap().parent().unwrap();
    let chrome_root = descriptor
        .browser_client_path
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    assert_eq!(
        descriptor.node_module_dirs,
        vec![cua_root.join("lib/node_modules")]
    );
    assert_eq!(descriptor.trusted_code_paths[0], root.join("codex"));
    assert!(descriptor
        .trusted_code_paths
        .iter()
        .any(|path| path == chrome_root));
    assert_eq!(
        fs::read(cua_root.join("shared.txt")).unwrap(),
        b"cua-shared"
    );
    assert_eq!(
        fs::read(chrome_root.join("shared.txt")).unwrap(),
        b"chrome-shared"
    );
    assert!(!cua_root.join("usr/share/unrelated.txt").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let node_mode = fs::metadata(&descriptor.node_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(node_mode & 0o777, 0o755);
        assert_eq!(node_mode & 0o7000, 0);
        let lib_mode = fs::metadata(cua_root.join("lib"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(lib_mode & 0o777, 0o555);
    }
    assert!(discover_managed_browser_runtime(
        &root.join("cache"),
        current_target(),
        &root.join("codex")
    )
    .is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hash_mismatch_is_rejected_before_archive_processing() {
    let root = temp();
    let mut p = package(&root, &deb());
    p.deb_sha256 = "0".repeat(64);
    assert_eq!(
        materialize_browser_runtime_at(&root.join("cache"), p)
            .unwrap_err()
            .to_string(),
        "browser_runtime_package_hash_mismatch"
    );
    assert!(!root.join("cache/.staging").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn duplicate_data_member_is_rejected() {
    let root = temp();
    let data = tar_fixture();
    let mut bytes = AR_MAGIC.to_vec();
    bytes.extend(ar_member("debian-binary", b"2.0\n"));
    bytes.extend(ar_member("data.tar.xz", &data));
    bytes.extend(ar_member("data.tar.xz", &data));
    let mut p = package(&root, &bytes);
    p.deb_sha256 = sha256(&bytes);
    assert_eq!(
        materialize_browser_runtime_at(&root.join("cache"), p)
            .unwrap_err()
            .to_string(),
        "browser_runtime_package_duplicate"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ar_parser_rejects_malformed_truncated_duplicate_and_unsupported_members() {
    let root = temp();
    let path = root.join("probe.deb");

    let mut odd = AR_MAGIC.to_vec();
    odd.extend(ar_member("debian-binary", b"2.0\n"));
    odd.extend(ar_member("control.tar.xz", b"x"));
    odd.extend(ar_member("data.tar.xz", &tar_fixture()));
    fs::write(&path, &odd).unwrap();
    assert!(ar_data(&path).is_ok());

    let mut unsupported = AR_MAGIC.to_vec();
    unsupported.extend(ar_member("debian-binary", b"2.0\n"));
    unsupported.extend(ar_member("data.tar.gz", b"x"));
    fs::write(&path, &unsupported).unwrap();
    assert_eq!(
        ar_data(&path).unwrap_err().to_string(),
        "browser_runtime_package_compression_unsupported"
    );

    let mut duplicate_binary = AR_MAGIC.to_vec();
    duplicate_binary.extend(ar_member("debian-binary", b"2.0\n"));
    duplicate_binary.extend(ar_member("debian-binary", b"2.0\n"));
    duplicate_binary.extend(ar_member("data.tar.xz", &tar_fixture()));
    fs::write(&path, &duplicate_binary).unwrap();
    assert_eq!(
        ar_data(&path).unwrap_err().to_string(),
        "browser_runtime_package_duplicate"
    );

    let mut truncated = odd;
    truncated.pop();
    fs::write(&path, &truncated).unwrap();
    assert_eq!(
        ar_data(&path).unwrap_err().to_string(),
        "browser_runtime_package_format"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn selected_links_special_types_and_duplicate_files_are_rejected() {
    for entry_type in [
        EntryType::symlink(),
        EntryType::hard_link(),
        EntryType::fifo(),
    ] {
        let root = temp();
        let tar = encoded_tar(|builder| {
            append_special(builder, "usr/lib/chatgpt/resources/cua_node", entry_type);
        });
        let mut bytes = AR_MAGIC.to_vec();
        bytes.extend(ar_member("debian-binary", b"2.0\n"));
        bytes.extend(ar_member("data.tar.xz", &tar));
        let p = package(&root, &bytes);
        assert_eq!(
            materialize_browser_runtime_at(&root.join("cache"), p)
                .unwrap_err()
                .to_string(),
            "browser_runtime_package_entry_type_rejected"
        );
        let _ = fs::remove_dir_all(root);
    }

    let root = temp();
    let duplicate = encoded_tar(|builder| {
        append_file(
            builder,
            "usr/lib/chatgpt/resources/cua_node/bin/node",
            b"first",
            0o755,
        );
        append_file(
            builder,
            "usr/lib/chatgpt/resources/cua_node/bin/node",
            b"second",
            0o755,
        );
    });
    let mut bytes = AR_MAGIC.to_vec();
    bytes.extend(ar_member("debian-binary", b"2.0\n"));
    bytes.extend(ar_member("data.tar.xz", &duplicate));
    let p = package(&root, &bytes);
    assert_eq!(
        materialize_browser_runtime_at(&root.join("cache"), p)
            .unwrap_err()
            .to_string(),
        "browser_runtime_package_duplicate"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn identical_install_is_idempotent_and_corrupt_artifact_repairs() {
    let root = temp();
    let cache = root.join("cache");
    let bytes = deb();
    let p = package(&root, &bytes);
    let first = materialize_browser_runtime_at(&cache, p.clone()).unwrap();
    let artifact = first
        .node_path
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let before = fs::read(artifact.join("runtime.json")).unwrap();
    assert_eq!(
        materialize_browser_runtime_at(&cache, p.clone()).unwrap(),
        first
    );
    assert_eq!(fs::read(artifact.join("runtime.json")).unwrap(), before);
    fs::remove_file(&first.browser_client_path).unwrap();
    assert!(materialize_browser_runtime_at(&cache, p).is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn active_manifest_corruption_fails_closed() {
    let root = temp();
    let cache = root.join("cache");
    let bytes = deb();
    let p = package(&root, &bytes);
    materialize_browser_runtime_at(&cache, p).unwrap();
    let active = cache
        .join("active")
        .join(format!("{}.json", current_target()));
    fs::write(&active, b"{}\n").unwrap();
    assert_eq!(
        discover_managed_browser_runtime(&cache, current_target(), &root.join("codex"))
            .unwrap_err()
            .to_string(),
        "browser_runtime_cache_active_invalid"
    );

    fs::write(
        &active,
        serde_json::json!({
            "target": if current_target() == "linux-x64" { "linux-arm64" } else { "linux-x64" },
            "appVersion": "1.2.3",
            "packageSha256": "0".repeat(64)
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(
        discover_managed_browser_runtime(&cache, current_target(), &root.join("codex"))
            .unwrap_err()
            .to_string(),
        "browser_runtime_cache_active_invalid"
    );

    fs::write(
        &active,
        serde_json::json!({
            "target": current_target(),
            "appVersion": "../escape",
            "packageSha256": "0".repeat(64)
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(
        discover_managed_browser_runtime(&cache, current_target(), &root.join("codex"))
            .unwrap_err()
            .to_string(),
        "browser_runtime_cache_active_invalid"
    );

    fs::write(&active, vec![b'x'; (MAX_METADATA_BYTES + 1) as usize]).unwrap();
    assert_eq!(
        discover_managed_browser_runtime(&cache, current_target(), &root.join("codex"))
            .unwrap_err()
            .to_string(),
        "browser_runtime_cache_active_invalid"
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn symlinked_critical_parent_in_cached_artifact_fails_closed() {
    use std::os::unix::fs::symlink;

    let root = temp();
    let cache = root.join("cache");
    let p = package(&root, &deb());
    let descriptor = materialize_browser_runtime_at(&cache, p).unwrap();
    let artifact = descriptor
        .node_path
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let bin = artifact.join("cua_node/bin");
    let outside = root.join("outside-bin");
    fs::rename(&bin, &outside).unwrap();
    symlink(&outside, &bin).unwrap();
    assert_eq!(
        discover_managed_browser_runtime(&cache, current_target(), &root.join("codex"))
            .unwrap_err()
            .to_string(),
        "browser_runtime_cache_active_invalid"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn target_locks_serialize_without_global_lock() {
    let root = temp();
    let first_path = root.join("same.lock");
    let other_path = root.join("other.lock");
    let first = Lock::acquire(&first_path).unwrap();
    assert!(Lock::acquire(&other_path).is_ok());
    let blocked = std::thread::spawn(move || Lock::acquire(&first_path).is_ok());
    std::thread::sleep(std::time::Duration::from_millis(10));
    drop(first);
    assert!(blocked.join().unwrap());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn unix_lock_reuses_existing_path_with_stale_owner_metadata() {
    let root = temp();
    let path = root.join("dead.lock");
    fs::write(&path, b"pid=4294967295\n").unwrap();
    let lock = Lock::acquire(&path).unwrap();
    assert!(path.exists());
    drop(lock);
    assert!(!path.exists());
    let _ = fs::remove_dir_all(root);
}
