pub mod utils;

use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Error};
use colored::Colorize;
use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::header::{HeaderMap, CONTENT_DISPOSITION, CONTENT_LENGTH};
use tokio::sync::{Semaphore, SemaphorePermit};
use zip::ZipArchive;

static RE_FILENAME: Lazy<Regex> = Lazy::new(|| Regex::new(r#"; filename="([^"]+)";"#).unwrap());

pub async fn download_urls(
    urls: Vec<String>,
    absolute_output_path: &Path,
    force: bool,
) -> Vec<Result<PathBuf, Error>> {
    let multi_progress = Arc::new(MultiProgress::new());
    let semaphore = Arc::new(Semaphore::new(num_cpus::get() / 2));
    let tasks: Vec<_> = urls
        .into_iter()
        .map(|url| {
            let mp = Arc::clone(&multi_progress);
            let sem = Arc::clone(&semaphore);
            let path = absolute_output_path.to_path_buf();
            tokio::spawn(async move {
                let permit: SemaphorePermit = sem.acquire().await.unwrap();
                let result = download_file(&path, &url, mp, force).await;
                drop(permit);
                result
            })
        })
        .collect();

    let results: Vec<Result<PathBuf, _>> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .map(|res| res.unwrap())
        .collect();

    results
}

pub async fn extract_zip_files(zip_files: Vec<PathBuf>, overwrite: bool) {
    let multi_progress = Arc::new(MultiProgress::new());
    let mut tasks = Vec::new();
    let semaphore = Arc::new(Semaphore::new(num_cpus::get() / 2));
    for zip_path in zip_files.into_iter() {
        let sem = Arc::clone(&semaphore);
        let progress_clone = Arc::clone(&multi_progress);
        let task = tokio::spawn(async move {
            let permit = sem.acquire().await.unwrap();
            let result = extract_zip_file(zip_path, progress_clone, overwrite).await;
            drop(permit);
            result
        });
        tasks.push(task);
    }

    let results: Vec<Result<(), _>> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .map(|res| res.unwrap())
        .collect();

    for result in results.iter() {
        if let Err(e) = result {
            eprintln!("{}", format!("Error: {}", e).red());
        }
    }
}

async fn extract_zip_file(
    path: PathBuf,
    multi_progress: Arc<MultiProgress>,
    overwrite: bool,
) -> anyhow::Result<()> {
    let extract_to = path
        .parent()
        .context("Failed to get parent dir")?
        .to_path_buf();

    let zip_path = path.clone();
    // Use spawn_blocking to avoid blocking the async runtime
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let file = File::open(&zip_path)
            .with_context(|| format!("Failed to open zip file: {}", zip_path.display()))?;

        let mut archive = ZipArchive::new(file)
            .with_context(|| format!("Failed to read zip archive: {}", zip_path.display()))?;

        let zip_file_name = zip_path
            .file_name()
            .context("Failed to get zip file name")?
            .to_string_lossy()
            .to_string();

        let progress_bar = multi_progress.add(ProgressBar::new(archive.len() as u64));
        progress_bar.set_style(
            ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:50.magenta/blue} {pos:>3}/{len:3} {msg}")?
                .progress_chars("##-"),
        );
        progress_bar.set_message(zip_file_name);

        for i in 0..archive.len() {
            let mut file = archive.by_index(i).with_context(|| {
                format!(
                    "Failed to access file at index {i} in {}",
                    zip_path.display()
                )
            })?;
            let file_path = file
                .enclosed_name()
                .ok_or_else(|| anyhow::anyhow!("Zip file contains unsafe path: {}", file.name()))?;

            let mut output_path = extract_to.join(file_path);
            if let Some(extension) = output_path.extension() {
                if extension == "aiff" {
                    output_path.set_extension("aif");
                }
            }

            if file.is_dir() {
                fs::create_dir_all(&output_path).with_context(|| {
                    format!("Failed to create directory: {}", output_path.display())
                })?;
            } else {
                if let Some(p) = output_path.parent() {
                    if !p.exists() {
                        fs::create_dir_all(p).with_context(|| {
                            format!("Failed to create parent directory: {}", p.display())
                        })?;
                    }
                }
                if output_path.exists() && !overwrite {
                    continue;
                }
                let mut output_file = File::create(&output_path).with_context(|| {
                    format!("Failed to create output file: {}", output_path.display())
                })?;
                std::io::copy(&mut file, &mut output_file).with_context(|| {
                    format!(
                        "Failed to copy data to output file: {}",
                        output_path.display()
                    )
                })?;
            }
            progress_bar.inc(1);
        }
        progress_bar.finish();
        trash::delete(&zip_path).context("Failed to move zip file to trash")?;
        Ok(())
    })
    .await?
}

async fn download_file(
    dir: &Path,
    url: &str,
    multi_progress: Arc<MultiProgress>,
    overwrite: bool,
) -> anyhow::Result<PathBuf> {
    let response = reqwest::get(url).await?;
    let headers = response.headers();
    let mut filename =
        get_filename(headers).with_context(|| format!("Failed to get filename for: {url}"))?;
    let total_size = get_total_size(headers);

    if filename.ends_with(".aiff") {
        filename.pop();
    }

    let path = dir.join(&filename);
    if path.exists() {
        if !overwrite {
            return Err(anyhow!("File already exists: {}", filename));
        } else {
            tokio::fs::remove_file(&path).await?
        }
    }

    let mut file = File::create(&path)?;
    let mut content = response.bytes_stream();

    let progress_bar = multi_progress.add(ProgressBar::new(total_size));
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:50.cyan/blue} {bytes:>10}/{total_bytes:>10} ({bytes_per_sec:>11}) {msg}")?
            .progress_chars("##-"),
    );
    progress_bar.set_message(filename.to_string());

    while let Some(chunk) = content.next().await {
        let chunk = chunk?;
        progress_bar.inc(chunk.len() as u64);
        file.write_all(&chunk)?;
    }

    progress_bar.finish();

    Ok(path)
}

fn get_total_size(headers: &HeaderMap) -> u64 {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn get_filename(headers: &HeaderMap) -> anyhow::Result<String> {
    headers
        .get(CONTENT_DISPOSITION)
        .and_then(|value| std::str::from_utf8(value.as_bytes()).ok())
        .and_then(|content_disposition| RE_FILENAME.captures(content_disposition))
        .and_then(|captures| captures.get(1))
        .map(|filename| filename.as_str().to_string())
        .ok_or_else(|| anyhow::anyhow!("Failed to get filename"))
}
