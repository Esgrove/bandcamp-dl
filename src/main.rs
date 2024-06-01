use std::env;
use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use colored::Colorize;

use bandcamp_dl::utils::{
    count_files_in_directory, get_all_zip_files, get_relative_path_from_current_working_directory,
    remove_images_from_dir,
};

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
            get_relative_path_from_current_working_directory(&absolute_output_path).display()
        )
    }

    let file_count_at_start = count_files_in_directory(&absolute_output_path)?;

    let results = bandcamp_dl::download_urls(urls, &absolute_output_path, args.force).await;
    let mut successful: Vec<PathBuf> = Vec::new();
    for result in results.into_iter() {
        match result {
            Ok(path) => {
                successful.push(path);
            }
            Err(e) => {
                eprintln!("{}", format!("Error: {}", e).red());
            }
        }
    }

    let zip_files = get_all_zip_files(&successful);
    if !zip_files.is_empty() {
        if zip_files.len() > 1 {
            println!("Extracting {} zip files", zip_files.len());
        } else {
            println!("Extracting 1 zip file");
        };
        bandcamp_dl::extract_zip_files(zip_files, args.force).await;
    }

    let removed = remove_images_from_dir(&absolute_output_path)?;
    if !removed.is_empty() && args.verbose {
        println!("Removed images ({}):", removed.len());
        for file in removed.iter() {
            println!(
                "  {}",
                get_relative_path_from_current_working_directory(file).display()
            );
        }
    }

    let file_count_at_end = count_files_in_directory(&absolute_output_path)?;
    let added_files = file_count_at_end as i64 - file_count_at_start as i64;
    println!("{}", format!("Added {added_files} new files").green());

    Ok(())
}
