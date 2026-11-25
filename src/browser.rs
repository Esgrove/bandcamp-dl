//! Temporary, authenticated Firefox-to-CLI link import.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::Context;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify};

const DEFAULT_PORT: u16 = 47653;
const MAX_LINKS: usize = 2000;
const MAX_BODY: usize = 1024 * 1024;
const MAX_TABS: usize = 100;
static URL_IN_ERROR: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\(?https?://[^\s]+").expect("valid URL redaction regex")
});

/// Keep useful download errors without exposing signed links in terminal output.
#[must_use]
pub fn redact_error(error: &anyhow::Error) -> String {
    URL_IN_ERROR
        .replace_all(&error.to_string(), "[signed URL hidden]")
        .into_owned()
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    port: u16,
    token: String,
}

fn config_path(path: Option<&Path>) -> anyhow::Result<PathBuf> {
    path.map_or_else(
        || {
            Ok(dirs::config_dir()
                .context("Cannot locate configuration directory; use --browser-config PATH")?
                .join("bandcamp-dl")
                .join("browser.json"))
        },
        |path| Ok(path.to_path_buf()),
    )
}

fn load_config(path: &Path) -> anyhow::Result<Config> {
    let data = std::fs::read(path)
        .context("Cannot read browser pairing; run bcdl --browser-setup first")?;
    let config: Config = serde_json::from_slice(&data).context("Invalid browser pairing file")?;
    anyhow::ensure!(
        config.port != 0
            && config.token.len() == 64
            && config.token.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid browser pairing settings; remove the file and run --browser-setup again"
    );
    Ok(config)
}

fn random_id() -> anyhow::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).context("Cannot generate browser pairing randomness")?;
    let mut result = String::with_capacity(64);
    for byte in bytes {
        write!(result, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(result)
}

/// Create pairing settings, or show the existing settings without rotating the secret.
pub fn setup(path: Option<&Path>, port: Option<u16>) -> anyhow::Result<()> {
    anyhow::ensure!(port != Some(0), "Browser port must not be zero");
    let path = config_path(path)?;
    let config = if path.exists() {
        let config = load_config(&path)?;
        anyhow::ensure!(
            port.is_none_or(|port| port == config.port),
            "Pairing already exists with another port; remove the pairing file before changing it"
        );
        config
    } else {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .context("Cannot create browser configuration directory")?;
        }
        let config = Config {
            port: port.unwrap_or(DEFAULT_PORT),
            token: random_id()?,
        };
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .context("Cannot create browser pairing file")?;
        file.write_all(serde_json::to_string_pretty(&config)?.as_bytes())?;
        file.sync_all()?;
        config
    };
    eprintln!("Pairing file: {}", path.display());
    eprintln!("Paste this private JSON into the Firefox extension's settings:");
    println!("{}", serde_json::to_string_pretty(&config)?);
    Ok(())
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Tab {
    id: u32,
    title: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TabList {
    request_id: String,
    profile_id: String,
    tabs: Vec<Tab>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkResult {
    request_id: String,
    profile_id: String,
    tab_id: u32,
    urls: Vec<String>,
    error: Option<String>,
}

#[derive(Default)]
struct Session {
    profiles: BTreeMap<String, Vec<Tab>>,
    selected: Option<(String, u32)>,
    result: Option<anyhow::Result<Vec<String>>>,
    received: bool,
    listing: bool,
}

#[derive(Clone)]
struct Bridge {
    config: Arc<Config>,
    request_id: Arc<String>,
    session: Arc<Mutex<Session>>,
    changed: Arc<Notify>,
}

impl Bridge {
    fn new(config: Config) -> anyhow::Result<Self> {
        Ok(Self {
            config: Arc::new(config),
            request_id: Arc::new(random_id()?),
            session: Arc::new(Mutex::new(Session {
                listing: true,
                ..Session::default()
            })),
            changed: Arc::new(Notify::new()),
        })
    }

    fn router(&self) -> Router {
        Router::new()
            .route("/v1/request", get(request))
            .route("/v1/tabs", post(tabs))
            .route("/v1/links", post(links))
            .layer(DefaultBodyLimit::max(MAX_BODY))
            .layer(middleware::from_fn_with_state(self.clone(), authenticate))
            .with_state(self.clone())
    }
}

async fn authenticate(
    State(bridge): State<Bridge>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let expected_host = format!("127.0.0.1:{}", bridge.config.port);
    let host_ok = headers.get("host").and_then(|v| v.to_str().ok()) == Some(expected_host.as_str());
    let token_ok = headers.get("x-bcdl-token").and_then(|v| v.to_str().ok())
        == Some(bridge.config.token.as_str());
    let origin_ok = headers.get("origin").is_none_or(|origin| {
        origin
            .to_str()
            .ok()
            .and_then(|s| reqwest::Url::parse(s).ok())
            .is_some_and(|url| {
                url.scheme() == "moz-extension"
                    && url.host_str().is_some()
                    && matches!(url.path(), "" | "/")
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.port().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
            })
    });
    let unambiguous = ["host", "origin", "x-bcdl-token"]
        .iter()
        .all(|header| headers.get_all(*header).iter().count() <= 1);
    if !host_ok || !token_ok || !origin_ok || !unambiguous {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().expect("valid header"));
    response
}

#[derive(Deserialize)]
struct Provider {
    profile_id: String,
}

async fn request(
    State(bridge): State<Bridge>,
    Query(provider): Query<Provider>,
) -> Json<serde_json::Value> {
    let session = bridge.session.lock().await;
    let selected = session
        .selected
        .as_ref()
        .filter(|(profile, _)| profile == &provider.profile_id);
    let action = if session.received {
        "idle"
    } else if selected.is_some() {
        "collect"
    } else if session.listing {
        "list"
    } else {
        "idle"
    };
    Json(serde_json::json!({
        "version": 1,
        "request_id": *bridge.request_id,
        "action": action,
        "tab_id": selected.map(|(_, tab)| *tab),
    }))
}

fn valid_profile(profile: &str) -> bool {
    !profile.is_empty()
        && profile.len() <= 64
        && profile
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

async fn tabs(State(bridge): State<Bridge>, Json(list): Json<TabList>) -> StatusCode {
    if list.request_id != *bridge.request_id
        || !valid_profile(&list.profile_id)
        || list.tabs.is_empty()
        || list.tabs.len() > MAX_TABS
        || list.tabs.iter().any(|tab| tab.title.len() > 512)
        || list
            .tabs
            .iter()
            .map(|tab| tab.id)
            .collect::<HashSet<_>>()
            .len()
            != list.tabs.len()
    {
        return StatusCode::BAD_REQUEST;
    }
    let mut session = bridge.session.lock().await;
    if !session.listing {
        // A list response may arrive after discovery closes; acknowledge known providers.
        return if session.profiles.contains_key(&list.profile_id) {
            StatusCode::NO_CONTENT
        } else {
            StatusCode::CONFLICT
        };
    }
    if session.profiles.len() >= 16 && !session.profiles.contains_key(&list.profile_id) {
        return StatusCode::TOO_MANY_REQUESTS;
    }
    session.profiles.insert(list.profile_id, list.tabs);
    drop(session);
    bridge.changed.notify_one();
    StatusCode::NO_CONTENT
}

fn validate_urls(urls: Vec<String>) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(
        !urls.is_empty() && urls.len() <= MAX_LINKS,
        "Browser returned an invalid link count"
    );
    let mut unique = HashSet::new();
    let mut result = Vec::new();
    for input in urls {
        anyhow::ensure!(
            input.len() <= 8192 && !input.bytes().any(|b| b.is_ascii_control() || b == b' '),
            "Browser returned an invalid URL length or whitespace"
        );
        let url = reqwest::Url::parse(&input).context("Browser returned an invalid URL")?;
        let host_ok = url.host_str().is_some_and(|host| {
            host.strip_prefix('p')
                .and_then(|s| s.strip_suffix(".bcbits.com"))
                .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        });
        anyhow::ensure!(
            url.scheme() == "https"
                && host_ok
                && url.username().is_empty()
                && url.password().is_none()
                && url.port().is_none()
                && url.fragment().is_none()
                && (url.path().starts_with("/download/album/")
                    || url.path().starts_with("/download/track/")),
            "Browser returned a URL outside supported Bandcamp download hosts/paths"
        );
        if unique.insert(input.clone()) {
            result.push(input);
        }
    }
    Ok(result)
}

async fn links(State(bridge): State<Bridge>, Json(result): Json<LinkResult>) -> StatusCode {
    let mut session = bridge.session.lock().await;
    if result.request_id != *bridge.request_id
        || session.selected.as_ref() != Some(&(result.profile_id, result.tab_id))
        || session.received
    {
        return StatusCode::CONFLICT;
    }
    session.received = true;
    // Error text is intentionally not relayed: browser content may contain signed URLs.
    session.result = Some(if result.error.is_some() {
        Err(anyhow::anyhow!(
            "Cannot collect links from the selected tab. Reload the page, wait for download links, and retry."
        ))
    } else {
        validate_urls(result.urls)
    });
    drop(session);
    bridge.changed.notify_one();
    StatusCode::NO_CONTENT
}

async fn select_and_receive(bridge: &Bridge, choice: Option<u32>) -> anyhow::Result<Vec<String>> {
    loop {
        let notified = bridge.changed.notified();
        if !bridge.session.lock().await.profiles.is_empty() {
            break;
        }
        notified.await;
    }
    // Allow other open Firefox profiles to announce their tabs before selecting.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let candidates: Vec<_> = {
        let mut session = bridge.session.lock().await;
        session.listing = false;
        session
            .profiles
            .iter()
            .flat_map(|(profile, tabs)| tabs.iter().map(|tab| (profile.clone(), tab.clone())))
            .collect()
    };
    let index = if candidates.len() == 1 && choice.is_none() {
        0
    } else {
        eprintln!("Bandcamp download tabs:");
        for (index, (_, tab)) in candidates.iter().enumerate() {
            let title: String = URL_IN_ERROR
                .replace_all(&tab.title, "[URL hidden]")
                .chars()
                .filter(|c| !c.is_control())
                .take(100)
                .collect();
            eprintln!("  {}: {} (tab {})", index + 1, title, tab.id);
        }
        let number = if let Some(choice) = choice {
            choice as usize
        } else {
            anyhow::ensure!(
                std::io::stdin().is_terminal(),
                "Multiple tabs found; rerun with --browser-tab NUMBER"
            );
            eprint!("Select tab number: ");
            std::io::stderr().flush()?;
            let (sender, receiver) = tokio::sync::oneshot::channel();
            // Tokio's blocking stdin worker would prevent shutdown on timeout/Ctrl+C.
            std::thread::Builder::new()
                .name("browser-tab-choice".into())
                .spawn(move || {
                    let mut line = String::new();
                    let result = std::io::stdin().read_line(&mut line).map(|_| line);
                    let _ = sender.send(result);
                })?;
            let line = receiver.await.context("Tab selection input stopped")??;
            line.trim()
                .parse::<usize>()
                .context("Expected a tab number")?
        };
        anyhow::ensure!(
            number > 0 && number <= candidates.len(),
            "Tab number is out of range"
        );
        number - 1
    };
    let (profile, tab) = &candidates[index];
    bridge.session.lock().await.selected = Some((profile.clone(), tab.id));
    loop {
        let notified = bridge.changed.notified();
        let result = bridge.session.lock().await.result.take();
        if let Some(result) = result {
            return result;
        }
        notified.await;
    }
}

/// Import fresh links, closing the local listener before returning to the downloader.
pub async fn import(
    path: Option<&Path>,
    timeout: Duration,
    choice: Option<u32>,
) -> anyhow::Result<Vec<String>> {
    let config = load_config(&config_path(path)?)?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", config.port))
        .await.context("Cannot open browser bridge port; another bcdl may be running. Use --browser-setup to inspect pairing.")?;
    let bridge = Bridge::new(config)?;
    eprintln!(
        "Waiting for Firefox download links ({} seconds). Keep the paired download tab open.",
        timeout.as_secs()
    );
    let (shutdown, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = axum::serve(listener, bridge.router())
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .into_future();
    tokio::pin!(server);
    let result = tokio::select! {
        result = select_and_receive(&bridge, choice) => result,
        () = tokio::time::sleep(timeout) =>
            Err(anyhow::anyhow!("Browser import timed out. Check extension pairing/permissions, reload the download tab, and wait for its links.")),
        result = &mut server => {
            result.context("Browser bridge failed")?;
            return Err(anyhow::anyhow!("Browser bridge stopped before receiving links"));
        },
    };
    let _ = shutdown.send(());
    tokio::time::timeout(Duration::from_secs(2), &mut server)
        .await
        .context("Browser bridge shutdown timed out")?
        .context("Browser bridge shutdown failed")?;
    let result = result?;
    eprintln!("Received {} download links from Firefox.", result.len());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_deduplicates_signed_links() {
        let link = "https://p4.bcbits.com/download/album/example?sig=a%2Bb&x=1".to_string();
        assert_eq!(
            validate_urls(vec![link.clone(), link.clone()]).unwrap(),
            vec![link]
        );
        assert_eq!(
            redact_error(&anyhow::anyhow!(
                "Request failed with status 403 for: https://p4.bcbits.com/download/album/1?sig=secret"
            )),
            "Request failed with status 403 for: [signed URL hidden]"
        );
        assert_eq!(
            redact_error(&anyhow::anyhow!("File already exists: track.aif")),
            "File already exists: track.aif"
        );
        for url in [
            "http://p4.bcbits.com/download/album/1",
            "https://p4.bcbits.com.evil.test/download/album/1",
            "https://evil.test/download/album/1",
            "https://p4.bcbits.com/image/1",
            "https://user@p4.bcbits.com/download/track/1",
            "https://p4.bcbits.com:444/download/track/1",
            "https://p4.bcbits.com/download/track/1?\nsig=secret",
        ] {
            assert!(validate_urls(vec![url.to_string()]).is_err());
        }
        assert!(validate_urls(vec![]).is_err());
        assert!(
            validate_urls(vec![
                "https://p4.bcbits.com/download/track/1".into();
                MAX_LINKS + 1
            ])
            .is_err()
        );
    }

    #[test]
    fn setup_preserves_pairing_and_protects_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("browser.json");
        setup(Some(&path), Some(12345)).unwrap();
        let first = load_config(&path).unwrap();
        setup(Some(&path), None).unwrap();
        assert_eq!(first.token, load_config(&path).unwrap().token);
        assert!(setup(Some(&path), Some(12346)).is_err());
        assert!(setup(Some(&path), Some(0)).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    async fn test_bridge() -> (Bridge, String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let bridge = Bridge::new(Config {
            port,
            token: "a".repeat(64),
        })
        .unwrap();
        let router = bridge.router();
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (bridge, format!("http://127.0.0.1:{port}"), task)
    }

    #[tokio::test]
    async fn bridge_authentication_and_limits() {
        let (bridge, root, server) = test_bridge().await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let endpoint = format!("{root}/v1/request?profile_id=test");
        assert_eq!(
            client.get(&endpoint).send().await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        for (header, value) in [
            ("origin", "https://evil.test"),
            ("host", "evil.test"),
            ("x-bcdl-token", "wrong"),
        ] {
            let response = client
                .get(&endpoint)
                .header("x-bcdl-token", bridge.config.token.as_str())
                .header(header, value)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        let response = client
            .get(&endpoint)
            .header("x-bcdl-token", bridge.config.token.as_str())
            .header("origin", "moz-extension://example")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(
            response
                .headers()
                .get("access-control-allow-origin")
                .is_none()
        );
        let data: serde_json::Value = response.json().await.unwrap();
        assert_eq!(data["action"], "list");
        assert_eq!(data["version"], 1);
        let response = client
            .post(format!("{root}/v1/tabs"))
            .header("x-bcdl-token", bridge.config.token.as_str())
            .header("content-type", "application/json")
            .body(" ".repeat(MAX_BODY + 1))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let response = client
            .post(format!("{root}/v1/tabs"))
            .header("x-bcdl-token", bridge.config.token.as_str())
            .json(&serde_json::json!({
                "request_id": "stale", "profile_id": "test", "tabs": [{"id": 1, "title": "test"}]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        for listing in [true, false] {
            bridge.session.lock().await.listing = listing;
            let response = client
                .post(format!("{root}/v1/tabs"))
                .header("x-bcdl-token", bridge.config.token.as_str())
                .json(&serde_json::json!({
                    "request_id": *bridge.request_id, "profile_id": "test",
                    "tabs": [{"id": 1, "title": "test"}]
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
        }
        server.abort();
    }

    #[tokio::test]
    async fn selection_binds_results_to_profile_tab_and_request() {
        let (bridge, root, server) = test_bridge().await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        bridge.session.lock().await.selected = Some(("profile".into(), 42));
        for (request_id, profile_id, tab_id) in [
            ("stale", "profile", 42),
            (bridge.request_id.as_str(), "other", 42),
            (bridge.request_id.as_str(), "profile", 43),
        ] {
            let response = client
                .post(format!("{root}/v1/links"))
                .header("x-bcdl-token", bridge.config.token.as_str())
                .json(&serde_json::json!({
                    "request_id": request_id, "profile_id": profile_id, "tab_id": tab_id,
                    "urls": ["https://p4.bcbits.com/download/album/example"]
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CONFLICT);
        }
        let body = serde_json::json!({
            "request_id": *bridge.request_id, "profile_id": "profile", "tab_id": 42,
            "urls": ["https://p4.bcbits.com/download/album/example"]
        });
        for expected in [StatusCode::NO_CONTENT, StatusCode::CONFLICT] {
            let response = client
                .post(format!("{root}/v1/links"))
                .header("x-bcdl-token", bridge.config.token.as_str())
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
        }
        assert!(bridge.session.lock().await.result.as_ref().unwrap().is_ok());
        bridge.session.lock().await.result.take();
        let response = client
            .post(format!("{root}/v1/links"))
            .header("x-bcdl-token", bridge.config.token.as_str())
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        server.abort();
    }

    #[tokio::test]
    async fn browser_errors_are_explicit_and_do_not_leak_page_data() {
        let (bridge, root, server) = test_bridge().await;
        bridge.session.lock().await.selected = Some(("profile".into(), 42));
        let response = reqwest::Client::new()
            .post(format!("{root}/v1/links"))
            .header("x-bcdl-token", bridge.config.token.as_str())
            .json(&serde_json::json!({
                "request_id": *bridge.request_id, "profile_id": "profile", "tab_id": 42,
                "urls": [], "error": "secret-signed-url"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let error = bridge
            .session
            .lock()
            .await
            .result
            .take()
            .unwrap()
            .unwrap_err()
            .to_string();
        assert!(error.contains("Cannot collect"));
        assert!(!error.contains("secret"));
        server.abort();
    }

    #[tokio::test]
    async fn full_import_selects_tab_and_closes_listener() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("browser.json");
        let port_reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = port_reservation.local_addr().unwrap().port();
        setup(Some(&path), Some(port)).unwrap();
        let token = load_config(&path).unwrap().token;
        drop(port_reservation);
        let provider = async {
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let root = format!("http://127.0.0.1:{port}/v1");
            loop {
                let response = client
                    .get(format!("{root}/request?profile_id=profile"))
                    .header("x-bcdl-token", &token)
                    .send()
                    .await;
                let Ok(response) = response else {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    continue;
                };
                let request: serde_json::Value = response.json().await.unwrap();
                match request["action"].as_str().unwrap() {
                    "list" => {
                        let response = client
                            .post(format!("{root}/tabs"))
                            .header("x-bcdl-token", &token)
                            .json(&serde_json::json!({
                                "request_id": request["request_id"], "profile_id": "profile",
                                "tabs": [{"id": 1, "title": "First"}, {"id": 2, "title": "Second"}]
                            }))
                            .send()
                            .await
                            .unwrap();
                        assert_eq!(response.status(), StatusCode::NO_CONTENT);
                    }
                    "collect" => {
                        assert_eq!(request["tab_id"], 2);
                        let response = client.post(format!("{root}/links"))
                            .header("x-bcdl-token", &token)
                            .json(&serde_json::json!({
                                "request_id": request["request_id"], "profile_id": "profile", "tab_id": 2,
                                "urls": ["https://p4.bcbits.com/download/track/example"]
                            })).send().await.unwrap();
                        assert_eq!(response.status(), StatusCode::NO_CONTENT);
                        break;
                    }
                    "idle" => {}
                    action => panic!("Unexpected action: {action}"),
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };
        let (urls, ()) = tokio::time::timeout(Duration::from_secs(12), async {
            tokio::join!(
                import(Some(&path), Duration::from_secs(10), Some(2)),
                provider
            )
        })
        .await
        .expect("import/provider exchange must finish");
        assert_eq!(
            urls.unwrap(),
            vec!["https://p4.bcbits.com/download/track/example"]
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .unwrap();
        drop(listener);
        assert!(
            import(Some(&path), Duration::from_millis(30), None)
                .await
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .unwrap();
        assert!(
            import(Some(&path), Duration::from_millis(30), None)
                .await
                .is_err()
        );
        drop(listener);
    }
}
