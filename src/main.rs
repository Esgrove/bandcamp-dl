use std::path::PathBuf;

use clap::Parser;
use colored::Colorize;

use bandcamp_dl::utils;

#[derive(Parser)]
#[command(author, about, version)]
struct Args {
    /// A single URL or JSON string array of URLs
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
    let output_path = utils::resolve_output_path(args.output.as_deref())?;

    if args.verbose {
        println!(
            "Downloading {} items to {}",
            urls.len(),
            utils::get_relative_path_from_current_working_directory(&output_path).display()
        );
        println!(
            "Using {} concurrent executors (out of {})",
            num_cpus::get_physical(),
            num_cpus::get()
        );
    }

    let file_count_at_start = utils::count_files_in_directory(&output_path)?;

    let results = match bandcamp_dl::download_urls(urls, &output_path, args.force).await {
        Ok(r) => r,
        Err(e) => {
            anyhow::bail!("{e}")
        }
    };

    let mut successful: Vec<PathBuf> = Vec::new();
    for result in results {
        match result {
            Ok(path) => successful.push(path),
            Err(e) => eprintln!("{}", format!("Error: {e}").red()),
        }
    }

    let zip_files = utils::get_all_zip_files(&successful);
    if !zip_files.is_empty() {
        if zip_files.len() > 1 {
            println!("Extracting {} zip files", zip_files.len());
        } else {
            println!("Extracting 1 zip file");
        }
        let num_files = bandcamp_dl::extract_zip_files(zip_files, args.force).await;
        if args.verbose {
            println!("Unzipped {num_files} files");
        }
    }

    // TODO: use count. Get number of downloaded files and calculate total from that.
    utils::remove_images(&output_path, args.verbose)?;

    let file_count_at_end = utils::count_files_in_directory(&output_path)?;
    match file_count_at_end.saturating_sub(file_count_at_start) {
        added if added >= 2 => println!("{}", format!("Added {added} new files").green()),
        1 => println!("{}", "Added 1 new file".green()),
        _ => println!("{}", "No new files added".yellow()),
    }

    Ok(())
}

/// Parse URL input argument string to a list of URLs.
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

        let urls: Vec<String> = parse_urls(&args.urls).expect("Failed to parse URLs");
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
        let args = Args::parse_from(["test", r"https://p4.bcbits.com/download/album/10"]);

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

        let urls: Vec<String> = parse_urls(&args.urls).expect("Failed to parse URLs");
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
