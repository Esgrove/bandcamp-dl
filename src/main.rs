use anyhow::Context;
use std::env;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use futures::stream::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::header::{HeaderMap, CONTENT_DISPOSITION, CONTENT_LENGTH};
use tokio::fs;
use urlencoding::decode;

static RE_FILENAME: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)filename\*?=(?:UTF-8''|["']?)([^;"']+)"#).unwrap());

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

    let multi_progress = Arc::new(MultiProgress::new());

    let tasks: Vec<_> = urls
        .into_iter()
        .map(|url| {
            let mp = Arc::clone(&multi_progress);
            let path = absolute_output_path.clone();
            tokio::spawn(async move {
                if let Err(e) = download_file(&path, &url, mp, args.force).await {
                    eprintln!("Error downloading {}: {}", url, e);
                }
            })
        })
        .collect();

    futures::future::join_all(tasks).await;

    Ok(())
}

async fn download_file(
    dir: &Path,
    url: &str,
    multi_progress: Arc<MultiProgress>,
    overwrite: bool,
) -> Result<(), Box<dyn std::error::Error>> {
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
            println!("File already exists: {}", filename);
            return Ok(());
        } else {
            fs::remove_file(&path).await?
        }
    }

    let mut file = File::create(&filename)?;
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

    Ok(())
}

fn get_total_size(headers: &HeaderMap) -> u64 {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn get_filename(headers: &HeaderMap) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(content_disposition) = headers.get(CONTENT_DISPOSITION) {
        let content_disposition = content_disposition.to_str()?;
        if let Some(captures) = RE_FILENAME.captures(content_disposition) {
            let filename = &captures[1];
            return Ok(decode(filename)?.to_string());
        }
    }
    Err(Box::from("Failed to get filename"))
}
