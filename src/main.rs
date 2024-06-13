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
    #[arg(short, long, name = "PATH")]
    output: Option<String>,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let urls = parse_urls(&args.urls)?;

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
    match added_files {
        x if x <= 0 => println!("{}", "No new files added".yellow()),
        1 => println!("{}", "Added 1 new file".green()),
        _ => println!("{}", format!("Added {} new files", added_files).green()),
    }

    Ok(())
}

fn parse_urls(urls: &str) -> anyhow::Result<Vec<String>> {
    let urls: Vec<String> = match serde_json::from_str(urls) {
        Ok(urls) => urls,
        Err(_) => {
            if urls.starts_with("https://") {
                vec![urls.to_string()]
            } else {
                anyhow::bail!("Failed to parse URLs from input string")
            }
        }
    };
    Ok(urls)
}

#[cfg(test)]
mod test_cli {
    use super::*;

    #[test]
    fn argument_url_deserialization() {
        let args = Args::parse_from([
            "test",
            r#"[
                "https://p4.bcbits.com/download/track/1b37d456848ecb79c2",
                "https://p4.bcbits.com/download/track/156807d37379c36a35",
                "https://p4.bcbits.com/download/track/1d231057b370614747",
                "https://p4.bcbits.com/download/track/1d23267dc839a60375",
                "https://p4.bcbits.com/download/track/159d2c2882254493a0",
                "https://p4.bcbits.com/download/track/1c6e680dfda072ca82",
                "https://p4.bcbits.com/download/track/11f650a2b8db1ef52f",
                "https://p4.bcbits.com/download/album/178dd6dd97f4418b69",
                "https://p4.bcbits.com/download/album/1e19efdce8d9084a55",
                "https://p4.bcbits.com/download/track/1f20390aef121b1671"
            ]"#,
        ]);

        let urls: Vec<String> = serde_json::from_str(&args.urls).expect("Failed to parse URLs");
        assert_eq!(urls.len(), 10);
        assert_eq!(
            urls,
            vec![
                "https://p4.bcbits.com/download/track/1b37d456848ecb79c2",
                "https://p4.bcbits.com/download/track/156807d37379c36a35",
                "https://p4.bcbits.com/download/track/1d231057b370614747",
                "https://p4.bcbits.com/download/track/1d23267dc839a60375",
                "https://p4.bcbits.com/download/track/159d2c2882254493a0",
                "https://p4.bcbits.com/download/track/1c6e680dfda072ca82",
                "https://p4.bcbits.com/download/track/11f650a2b8db1ef52f",
                "https://p4.bcbits.com/download/album/178dd6dd97f4418b69",
                "https://p4.bcbits.com/download/album/1e19efdce8d9084a55",
                "https://p4.bcbits.com/download/track/1f20390aef121b1671"
            ]
        );
    }

    #[test]
    fn argument_single_url_string() {
        let args = Args::parse_from(["test", r#"https://p4.bcbits.com/download/album/10"#]);

        let urls: Vec<String> = parse_urls(&args.urls).unwrap();
        assert_eq!(urls.len(), 1);
        assert_eq!(urls, vec!["https://p4.bcbits.com/download/album/10"]);
    }

    #[test]
    fn full_arguments() {
        let args = Args::parse_from([
            "test",
            r#"["https://p4.bcbits.com/download/track/1", "https://p4.bcbits.com/download/track/2"]"#,
            "--force",
            "--output",
            "output_path",
            "--verbose",
        ]);

        let urls: Vec<String> = serde_json::from_str(&args.urls).expect("Failed to parse URLs");
        assert_eq!(
            urls,
            vec![
                "https://p4.bcbits.com/download/track/1",
                "https://p4.bcbits.com/download/track/2"
            ]
        );
        assert!(args.force);
        assert!(args.verbose);
        assert_eq!(args.output.as_deref(), Some("output_path"));
    }
}
