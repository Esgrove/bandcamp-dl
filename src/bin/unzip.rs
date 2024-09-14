use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::Result;
use clap::Parser;
use colored::Colorize;

static ZIP_EXTENSION: LazyLock<Option<&std::ffi::OsStr>> =
    LazyLock::new(|| Some(std::ffi::OsStr::new("zip")));

#[derive(Parser)]
#[command(author, about = "Extract all zip files concurrently", version)]
struct Args {
    /// Optional input path
    input: Option<String>,

    /// Overwrite existing files
    #[arg(short, long)]
    force: bool,

    /// Get zip files recursively
    #[arg(short, long)]
    recursive: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let input_path = bandcamp_dl::utils::resolve_path(args.input)?;

    if args.verbose {
        println!(
            "Using {} concurrent executors (out of {})",
            num_cpus::get_physical(),
            num_cpus::get()
        );
    }

    let zip_files = gather_zip_files(&input_path, args.recursive)?;
    if zip_files.is_empty() {
        anyhow::bail!("No zip files found")
    }

    if zip_files.len() > 1 {
        println!("Extracting {} zip files", zip_files.len());
    } else {
        println!("Extracting 1 zip file");
    };

    // TODO: fix for recursive
    // Move to extract and return number of files added in total
    let file_count_at_start = bandcamp_dl::utils::count_files_in_directory(&input_path)?;

    bandcamp_dl::extract_zip_files(zip_files, args.force).await;
    bandcamp_dl::utils::remove_images(&input_path, args.verbose)?;

    if args.verbose {
        let file_count_at_end = bandcamp_dl::utils::count_files_in_directory(&input_path)?;
        let added_files = file_count_at_end.saturating_sub(file_count_at_start);
        println!("{}", format!("Added {added_files} new files").green());
    }

    Ok(())
}

fn gather_zip_files(input_path: &PathBuf, recursive: bool) -> Result<Vec<PathBuf>> {
    let zip_files = if recursive {
        gather_zip_files_recursive(input_path)?
    } else {
        fs::read_dir(input_path)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.extension() == *ZIP_EXTENSION {
                    Some(path)
                } else {
                    None
                }
            })
            .collect()
    };
    Ok(zip_files)
}

fn gather_zip_files_recursive(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut zip_files: Vec<PathBuf> = Vec::new();
    if dir.is_dir() {
        for entry in (fs::read_dir(dir)?).flatten() {
            let path = entry.path();
            if path.is_dir() {
                zip_files.extend(gather_zip_files_recursive(&path)?);
            } else if path.extension() == *ZIP_EXTENSION {
                zip_files.push(path);
            }
        }
    }

    Ok(zip_files)
}
