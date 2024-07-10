use std::fs;
use std::path::PathBuf;

use clap::Parser;
use colored::Colorize;

#[derive(Parser)]
#[command(author, about = "Extract all zip files concurrently", version)]
struct Args {
    /// Optional input path
    input: Option<String>,

    /// Overwrite existing files
    #[arg(short, long)]
    force: bool,

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

    let zip_files: Vec<PathBuf> = fs::read_dir(&input_path)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension() == Some(std::ffi::OsStr::new("zip")) {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    if zip_files.is_empty() {
        anyhow::bail!("No zip files found")
    }

    if zip_files.len() > 1 {
        println!("Extracting {} zip files", zip_files.len());
    } else {
        println!("Extracting 1 zip file");
    };

    let file_count_at_start = bandcamp_dl::utils::count_files_in_directory(&input_path)?;

    bandcamp_dl::extract_zip_files(zip_files, args.force).await;
    bandcamp_dl::utils::remove_images(&input_path, args.verbose)?;

    if args.verbose {
        let file_count_at_end = bandcamp_dl::utils::count_files_in_directory(&input_path)?;
        let added_files = file_count_at_end as i64 - file_count_at_start as i64;
        println!("{}", format!("Added {added_files} new files").green());
    }

    Ok(())
}
