use anyhow::{anyhow, Context};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{env, fs};

use clap::Parser;
use colored::Colorize;
use futures::stream::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::header::{HeaderMap, CONTENT_DISPOSITION, CONTENT_LENGTH};
use zip::ZipArchive;

static RE_FILENAME: Lazy<Regex> = Lazy::new(|| Regex::new(r#"; filename="([^"]+)";"#).unwrap());

#[derive(Parser)]
#[command(author, about, version)]
struct Args {
    /// JSON string containing an array of URLs
    urls: String,

    /// Overwrite existing files
    #[arg(short, long)]
    force: bool,

    /// Optional output directory
    #[arg(short, long)]
    output: Option<String>,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let urls: Vec<String> = serde_json::from_str(&args.urls).expect("Failed to parse URLs");

    let output = args.output.clone().unwrap_or_default().trim().to_string();
    let output_path = if output.is_empty() {
        env::current_dir().context("Failed to get current working directory")?
    } else {
        PathBuf::from(output)
    };
    if !output_path.exists() {
        anyhow::bail!(
            "Output path does not exist or is not accessible: '{}'",
            dunce::simplified(&output_path).display()
        );
    }

    let absolute_output_path = dunce::canonicalize(output_path)?;

    if args.verbose {
        println!(
            "Downloading {} items to {}",
            urls.len(),
            absolute_output_path.display()
        )
    }

    let file_count_at_start = count_files_in_directory(&absolute_output_path)?;

    let multi_progress = Arc::new(MultiProgress::new());

    let tasks: Vec<_> = urls
        .into_iter()
        .map(|url| {
            let mp = Arc::clone(&multi_progress);
            let path = absolute_output_path.clone();
            tokio::spawn(async move { download_file(&path, &url, mp, args.force).await })
        })
        .collect();

    let results: Vec<Result<PathBuf, _>> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .map(|res| res.unwrap())
        .collect();

    let mut successful: Vec<PathBuf> = Vec::new();
    for result in results.into_iter() {
        match result {
            Ok(path) => {
                if args.verbose {
                    println!("Downloaded {}", path.display());
                }
                successful.push(path);
            }
            Err(e) => {
                eprintln!("{}", format!("Error: {}", e).red());
            }
        }
    }

    let zip_files: Vec<PathBuf> = successful
        .into_iter()
        .filter(|path| path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("zip"))
        .collect();

    if !zip_files.is_empty() {
        println!("Extracting {} zip files", zip_files.len());
        let mut tasks = Vec::new();
        let mut errors = Vec::new();

        for zip_path in zip_files.into_iter() {
            let task = extract_zip_file(zip_path);
            tasks.push(task);
        }

        // Wait for all tasks to complete and gather errors
        for task in tasks {
            if let Err(e) = task.await {
                errors.push(e.to_string());
            }
        }

        if !errors.is_empty() {
            println!("Errors occurred during extraction:");
            for error in errors {
                println!("{}", error);
            }
        }
    }

    let file_count_at_end = count_files_in_directory(&absolute_output_path)?;
    let added_files = file_count_at_end as i64 - file_count_at_start as i64;
    println!("{}", format!("Added {added_files} new files").green());

    Ok(())
}

async fn download_file(
    dir: &Path,
    url: &str,
    multi_progress: Arc<MultiProgress>,
    overwrite: bool,
) -> anyhow::Result<PathBuf> {
    let response = reqwest::get(url).await?;
    let headers = response.headers();
    let mut filename = get_filename(headers)?;
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

    let pb = multi_progress.add(ProgressBar::new(total_size));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:60.cyan/blue} {bytes}/{total_bytes} ({eta}) {msg}")?
            .progress_chars("##-"),
    );
    pb.set_message(filename.to_string());

    while let Some(chunk) = content.next().await {
        let chunk = chunk?;
        pb.inc(chunk.len() as u64);
        file.write_all(&chunk)?;
    }

    pb.finish();

    Ok(path)
}

async fn extract_zip_file(zip_path: PathBuf) -> anyhow::Result<()> {
    let extract_to = zip_path
        .parent()
        .context("Failed to get parent dir")?
        .to_path_buf();

    // Use spawn_blocking to avoid blocking the async runtime
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let file = File::open(&zip_path)
            .with_context(|| format!("Failed to open zip file: {}", zip_path.display()))?;
        let mut archive = ZipArchive::new(file)
            .with_context(|| format!("Failed to read zip archive: {}", zip_path.display()))?;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i).with_context(|| {
                format!(
                    "Failed to access file at index {i} in {}",
                    zip_path.display()
                )
            })?;
            let output_path = extract_to.join(file.name());

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
        }
        Ok(())
    })
    .await??;

    Ok(())
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
        .and_then(|value| value.to_str().ok())
        .and_then(|content_disposition| RE_FILENAME.captures(content_disposition))
        .and_then(|captures| captures.get(1))
        .map(|filename| filename.as_str().to_string())
        .ok_or_else(|| anyhow::anyhow!("Failed to get filename"))
}

fn count_files_in_directory<P: AsRef<Path>>(path: P) -> anyhow::Result<usize> {
    let entries = fs::read_dir(path)?;
    let mut count = 0;
    for entry in entries {
        let entry = entry?;
        if entry.path().is_file() {
            count += 1;
        }
    }
    Ok(count)
}
