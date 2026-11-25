pub mod browser;
pub mod utils;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Error, anyhow};
use colored::Colorize;
use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use regex::Regex;
use reqwest::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, HeaderMap};
use reqwest::{Client, StatusCode};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::{Semaphore, SemaphorePermit};
use zip::ZipArchive;

/// Regex to match filename in `CONTENT_DISPOSITION` header
static RE_FILENAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"; filename="([^"]+)";"#).expect("Filename regex failed"));

const PROGRESS_BAR_CHARS: &str = "=>-";
/// Maximum time without receiving any data before a download is considered stalled.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Delays before retrying a transient download failure.
#[cfg(not(test))]
const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];
#[cfg(test)]
const RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(10), Duration::from_millis(10)];
const PROGRESS_BAR_DOWNLOAD_TEMPLATE: &str = "[{elapsed_precise}] {bar:40.cyan/blue} [{percent:>3}%] {bytes:>10}/{total_bytes:>10} ({bytes_per_sec:>11}) {msg}";
const PROGRESS_BAR_UNZIP_TEMPLATE: &str =
    "[{elapsed_precise}] {bar:40.magenta/blue} {pos:>3}/{len:3} {msg}";

/// Download given URLs concurrently.
/// Returns a list of results with the file paths for successful downloads.
pub async fn download_urls(
    urls: Vec<String>,
    absolute_output_path: &Path,
    force: bool,
) -> anyhow::Result<Vec<Result<PathBuf, Error>>> {
    let client = Client::builder()
        .connect_timeout(Duration::new(5, 0))
        .read_timeout(READ_TIMEOUT)
        .build()
        .context("Failed to create client")?;

    let multi_progress = Arc::new(MultiProgress::new());
    let semaphore = create_semaphore_for_num_physical_cpus();
    let tasks: Vec<_> = urls
        .into_iter()
        .map(|url| {
            let client = client.clone();
            let progress = Arc::clone(&multi_progress);
            let sem = Arc::clone(&semaphore);
            let path = absolute_output_path.to_path_buf();
            tokio::spawn(async move {
                let permit: SemaphorePermit = sem
                    .acquire()
                    .await
                    .expect("Failed to acquire permit for download");
                let result = download_file(&client, &path, &url, progress, force).await;
                drop(permit);
                result
            })
        })
        .collect();

    let results: Vec<Result<PathBuf, _>> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .map(|res| res.expect("Download future failed"))
        .collect();

    Ok(results)
}

/// Extract all zip files concurrently.
pub async fn extract_zip_files(zip_files: Vec<PathBuf>, overwrite: bool) -> usize {
    let multi_progress = Arc::new(MultiProgress::new());
    let mut tasks = Vec::new();
    let semaphore = create_semaphore_for_num_physical_cpus();
    for zip_path in zip_files {
        let sem = Arc::clone(&semaphore);
        let progress = Arc::clone(&multi_progress);
        tasks.push(tokio::spawn(async move {
            let permit = sem
                .acquire()
                .await
                .expect("Failed to acquire permit for unzip");
            let result = extract_zip_file(zip_path, progress, overwrite).await;
            drop(permit);
            result
        }));
    }

    let total_unzipped_files: usize = futures::future::join_all(tasks)
        .await
        .into_iter()
        .map(|res| res.expect("Unzip future failed"))
        .map(|result| {
            result.unwrap_or_else(|e| {
                eprintln!("{}", format!("Error: {e}").red());
                0
            })
        })
        .sum();

    total_unzipped_files
}

/// Extract a single zip file with its own progress bar.
async fn extract_zip_file(
    path: PathBuf,
    multi_progress: Arc<MultiProgress>,
    overwrite: bool,
) -> anyhow::Result<usize> {
    let extract_to = path
        .parent()
        .context("Failed to get parent dir")?
        .to_path_buf();

    let zip_path = path.clone();
    // Use spawn_blocking to avoid blocking the async runtime
    tokio::task::spawn_blocking(move || -> anyhow::Result<usize> {
        let file = std::fs::File::open(&zip_path)
            .with_context(|| format!("Failed to open zip file: {}", zip_path.display()))?;

        let mut archive = ZipArchive::new(file)
            .with_context(|| format!("Failed to read zip archive: {}", zip_path.display()))?;

        let zip_file_name = utils::get_filename_from_path(&zip_path)?;
        let total_entries = archive.len();

        let progress_bar = multi_progress.add(ProgressBar::new(total_entries as u64));
        progress_bar.set_style(
            ProgressStyle::default_bar()
                .template(PROGRESS_BAR_UNZIP_TEMPLATE)?
                .progress_chars(PROGRESS_BAR_CHARS),
        );
        progress_bar.set_message(zip_file_name);

        // Count only files this run actually writes to disk.
        // The reported total ignores directory entries and files skipped as already present.
        let mut extracted_files = 0usize;
        for i in 0..archive.len() {
            progress_bar.inc(1);
            let mut file = archive.by_index(i).with_context(|| {
                format!(
                    "Failed to access file at index {i} in {}",
                    zip_path.display()
                )
            })?;

            let Some(file_path) = file.enclosed_name() else {
                continue;
            };

            // Sanitize each component of the path
            let sanitized_path: PathBuf = file_path
                .components()
                .map(|component| {
                    if let std::path::Component::Normal(name) = component {
                        PathBuf::from(utils::sanitize_filename(&name.to_string_lossy()))
                    } else {
                        PathBuf::from(component.as_os_str())
                    }
                })
                .collect();

            let mut output_path = extract_to.join(sanitized_path);

            if let Some(extension) = output_path.extension()
                && extension == "aiff"
            {
                output_path.set_extension("aif");
            }

            if file.is_dir() {
                std::fs::create_dir_all(&output_path).with_context(|| {
                    format!("Failed to create directory: {}", output_path.display())
                })?;
            } else {
                if let Some(p) = output_path.parent()
                    && !p.exists()
                {
                    std::fs::create_dir_all(p).with_context(|| {
                        format!("Failed to create parent directory: {}", p.display())
                    })?;
                }
                if output_path.exists() && !overwrite {
                    continue;
                }
                let mut output_file = std::fs::File::create(&output_path).with_context(|| {
                    format!("Failed to create output file: {}", output_path.display())
                })?;
                std::io::copy(&mut file, &mut output_file).with_context(|| {
                    format!(
                        "Failed to copy data to output file: {}",
                        output_path.display()
                    )
                })?;
                extracted_files += 1;
            }
        }
        progress_bar.finish();
        trash::delete(&zip_path).context("Failed to move zip file to trash")?;
        Ok(extracted_files)
    })
    .await?
}

/// A failed download attempt, and whether trying again may help.
struct Failure {
    error: Error,
    retryable: bool,
}

impl Failure {
    fn fatal(error: impl Into<Error>) -> Self {
        Self {
            error: error.into(),
            retryable: false,
        }
    }

    fn from_reqwest(error: reqwest::Error) -> Self {
        let retryable = !(error.is_builder() || error.is_redirect() || error.is_status());
        Self {
            error: error.into(),
            retryable,
        }
    }
}

/// Download a single file with its own progress bar.
/// Transient network failures are retried a bounded number of times.
async fn download_file(
    client: &Client,
    dir: &Path,
    url: &str,
    multi_progress: Arc<MultiProgress>,
    overwrite: bool,
) -> anyhow::Result<PathBuf> {
    let mut retries = RETRY_DELAYS.iter();
    loop {
        match download_attempt(client, dir, url, &multi_progress, overwrite).await {
            Ok(path) => return Ok(path),
            Err(failure) if failure.retryable => match retries.next() {
                Some(delay) => tokio::time::sleep(*delay).await,
                None => return Err(failure.error),
            },
            Err(failure) => return Err(failure.error),
        }
    }
}

/// One download attempt: stream into `<name>.part`, then rename into place.
/// A failed attempt never leaves a partial file behind under either name.
async fn download_attempt(
    client: &Client,
    dir: &Path,
    url: &str,
    multi_progress: &MultiProgress,
    overwrite: bool,
) -> Result<PathBuf, Failure> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(Failure::from_reqwest)?;
    let status = response.status();
    if !status.is_success() {
        return Err(Failure {
            error: anyhow!("Request failed with status {status} for: {url}"),
            retryable: status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error(),
        });
    }
    let headers = response.headers();
    let total_bytes = response
        .content_length()
        .unwrap_or_else(|| get_content_length_bytes(headers));
    let mut filename = get_filename(headers)
        .with_context(|| format!("Failed to get filename for: {url}"))
        .map_err(Failure::fatal)?;

    // Bandcamp file extensions are always in lowercase
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    if filename.ends_with(".aiff") {
        // -> ".aif"
        filename.pop();
    }

    let path = dir.join(&filename);
    if path.exists() && !overwrite {
        return Err(Failure::fatal(anyhow!("File already exists: {filename}")));
    }

    let part_path = dir.join(format!("{filename}.part"));
    let progress_bar = multi_progress.add(ProgressBar::new(total_bytes));
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template(PROGRESS_BAR_DOWNLOAD_TEMPLATE)
            .map_err(Failure::fatal)?
            .progress_chars(PROGRESS_BAR_CHARS),
    );
    progress_bar.set_message(filename.clone());

    let result = write_part_file(response, &part_path, total_bytes, &progress_bar).await;
    let result = match result {
        Ok(()) => replace_file(&part_path, &path, overwrite)
            .await
            .map_err(Failure::fatal),
        Err(failure) => Err(failure),
    };
    match result {
        Ok(()) => {
            progress_bar.finish();
            Ok(path)
        }
        Err(failure) => {
            progress_bar.finish_and_clear();
            let _ = tokio::fs::remove_file(&part_path).await;
            Err(failure)
        }
    }
}

async fn write_part_file(
    response: reqwest::Response,
    part_path: &Path,
    total_bytes: u64,
    progress_bar: &ProgressBar,
) -> Result<(), Failure> {
    let file = tokio::fs::File::create(part_path)
        .await
        .map_err(Failure::fatal)?;
    let mut writer = BufWriter::new(file);
    let mut content = response.bytes_stream();
    let mut written: u64 = 0;
    while let Some(chunk) = content.next().await {
        let chunk = chunk.map_err(Failure::from_reqwest)?;
        written += chunk.len() as u64;
        progress_bar.inc(chunk.len() as u64);
        writer.write_all(&chunk).await.map_err(Failure::fatal)?;
    }
    writer.flush().await.map_err(Failure::fatal)?;
    writer
        .into_inner()
        .sync_all()
        .await
        .map_err(Failure::fatal)?;
    if total_bytes > 0 && written != total_bytes {
        return Err(Failure {
            error: anyhow!("Incomplete download: received {written} of {total_bytes} bytes"),
            retryable: true,
        });
    }
    Ok(())
}

/// Move a finished download into place.
/// An existing file is only removed once its replacement is complete.
async fn replace_file(part_path: &Path, path: &Path, overwrite: bool) -> anyhow::Result<()> {
    if path.exists() {
        anyhow::ensure!(
            overwrite,
            "File already exists: {}",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        // Windows cannot rename over an existing file.
        tokio::fs::remove_file(path).await?;
    }
    tokio::fs::rename(part_path, path).await?;
    Ok(())
}

/// Get total file size from headers.
/// Returns zero in case of failure.
fn get_content_length_bytes(headers: &HeaderMap) -> u64 {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Get full filename from headers.
fn get_filename(headers: &HeaderMap) -> anyhow::Result<String> {
    headers
        .get(CONTENT_DISPOSITION)
        .and_then(|value| std::str::from_utf8(value.as_bytes()).ok())
        .and_then(|content_disposition| RE_FILENAME.captures(content_disposition))
        .and_then(|captures| captures.get(1))
        .map(|filename| filename.as_str().to_string())
        .ok_or_else(|| anyhow::anyhow!("Failed to get filename"))
}

#[inline]
/// Create a Semaphore with half the number of logical CPU cores available.
fn create_semaphore_for_num_physical_cpus() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(num_cpus::get_physical()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::body::{Body, Bytes};
    use axum::extract::{Path as UrlPath, State};
    use axum::response::Response;
    use axum::routing::get;

    const BODY_LEN: usize = 100;

    /// Fails the first `fails` requests mid-body, then serves the full file.
    async fn flaky(
        State(count): State<Arc<AtomicUsize>>,
        UrlPath(fails): UrlPath<usize>,
    ) -> Response {
        let attempt = count.fetch_add(1, Ordering::SeqCst);
        let chunks: Vec<Result<Bytes, std::io::Error>> = if attempt < fails {
            vec![
                Ok(Bytes::from(vec![b'x'; 10])),
                Err(std::io::Error::other("connection dropped")),
            ]
        } else {
            vec![Ok(Bytes::from(vec![b'x'; BODY_LEN]))]
        };
        Response::builder()
            .header(CONTENT_LENGTH, BODY_LEN)
            .header(
                CONTENT_DISPOSITION,
                r#"attachment; filename="track.flac"; filename*=UTF-8''track.flac"#,
            )
            .body(Body::from_stream(futures::stream::iter(chunks)))
            .unwrap()
    }

    async fn missing(State(count): State<Arc<AtomicUsize>>) -> axum::http::StatusCode {
        count.fetch_add(1, Ordering::SeqCst);
        axum::http::StatusCode::NOT_FOUND
    }

    async fn serve() -> (String, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let router = axum::Router::new()
            .route("/flaky/{fails}", get(flaky))
            .route("/missing", get(missing))
            .with_state(Arc::clone(&count));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (root, count)
    }

    async fn download(url: &str, dir: &Path) -> anyhow::Result<PathBuf> {
        let client = Client::builder().no_proxy().build().unwrap();
        let progress = Arc::new(MultiProgress::with_draw_target(
            indicatif::ProgressDrawTarget::hidden(),
        ));
        download_file(&client, dir, url, progress, false).await
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn transient_failures_are_retried_without_leftovers() {
        let (root, count) = serve().await;
        let dir = tempfile::tempdir().unwrap();
        let path = download(&format!("{root}/flaky/2"), dir.path())
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 3);
        assert_eq!(std::fs::read(&path).unwrap().len(), BODY_LEN);
        assert_eq!(files(dir.path()), vec!["track.flac"]);
    }

    #[tokio::test]
    async fn exhausted_retries_leave_no_partial_file() {
        let (root, count) = serve().await;
        let dir = tempfile::tempdir().unwrap();
        assert!(
            download(&format!("{root}/flaky/9"), dir.path())
                .await
                .is_err()
        );
        assert_eq!(count.load(Ordering::SeqCst), 1 + RETRY_DELAYS.len());
        assert_eq!(files(dir.path()), Vec::<String>::new());
        // The next run is not blocked by a leftover "existing" file.
        let (root, _) = serve().await;
        assert!(
            download(&format!("{root}/flaky/0"), dir.path())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn client_errors_are_not_retried() {
        let (root, count) = serve().await;
        let dir = tempfile::tempdir().unwrap();
        let error = download(&format!("{root}/missing"), dir.path())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("404"), "{error}");
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(files(dir.path()), Vec::<String>::new());
    }

    #[tokio::test]
    async fn existing_file_is_kept_until_replacement_completes() {
        let (root, _) = serve().await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("track.flac"), b"old").unwrap();
        assert!(
            download(&format!("{root}/flaky/0"), dir.path())
                .await
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
        assert_eq!(
            std::fs::read(dir.path().join("track.flac")).unwrap(),
            b"old"
        );
    }
}
