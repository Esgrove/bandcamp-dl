use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use colored::Colorize;

use bandcamp_dl::{browser, utils};

#[derive(Parser)]
#[command(author, about, version)]
struct Args {
    /// A single URL or JSON string array of URLs
    #[arg(required_unless_present_any = ["browser", "browser_setup"], conflicts_with_all = ["browser", "browser_setup"])]
    urls: Option<String>,

    /// Get links from the paired Firefox extension
    #[arg(long)]
    browser: bool,

    /// Create/show pairing settings to paste into the Firefox extension
    #[arg(long, conflicts_with_all = ["browser", "output", "force"])]
    browser_setup: bool,

    /// Pairing file (defaults to the user's configuration directory)
    #[arg(long, value_name = "PATH", conflicts_with = "urls")]
    browser_config: Option<PathBuf>,

    /// Local bridge port to save during setup
    #[arg(long, requires = "browser_setup", conflicts_with_all = ["urls", "browser"], value_parser = clap::value_parser!(u16).range(1..))]
    browser_port: Option<u16>,

    /// Seconds to wait for browser import
    #[arg(long, requires = "browser", conflicts_with_all = ["urls", "browser_setup"], default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..=3600))]
    browser_timeout: u64,

    /// Choose a numbered tab from the terminal list (for noninteractive use)
    #[arg(long, requires = "browser", conflicts_with_all = ["urls", "browser_setup"], value_parser = clap::value_parser!(u32).range(1..))]
    browser_tab: Option<u32>,

    /// Overwrite existing files
    #[arg(short, long)]
    force: bool,

    /// Optional output directory
    #[arg(short, long, value_name = "PATH")]
    output: Option<String>,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if args.browser_setup {
        return browser::setup(args.browser_config.as_deref(), args.browser_port);
    }
    let output_path = utils::resolve_output_path(args.output.as_deref())?;
    let urls = if args.browser {
        browser::import(
            args.browser_config.as_deref(),
            Duration::from_secs(args.browser_timeout),
            args.browser_tab,
        )
        .await?
    } else {
        parse_urls(args.urls.as_deref().expect("clap requires URL input"))?
    };

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

    let results = match bandcamp_dl::download_urls(urls, &output_path, args.force).await {
        Ok(r) => r,
        Err(e) => {
            anyhow::bail!("{e}")
        }
    };

    let mut successful: Vec<PathBuf> = Vec::new();
    let mut failed = 0;
    for result in results {
        match result {
            Ok(path) => successful.push(path),
            Err(e) => {
                failed += 1;
                if args.browser {
                    eprintln!("{}", format!("Error: {}", browser::redact_error(&e)).red());
                } else {
                    eprintln!("{}", format!("Error: {e}").red());
                }
            }
        }
    }

    let zip_files = utils::get_all_zip_files(&successful);
    // Files downloaded directly (single tracks) are output as-is,
    // zips are unpacked below.
    let downloaded_file_count = successful.len() - zip_files.len();

    let mut extracted_file_count = 0;
    if !zip_files.is_empty() {
        if zip_files.len() > 1 {
            println!("Extracting {} zip files", zip_files.len());
        } else {
            println!("Extracting 1 zip file");
        }
        extracted_file_count = bandcamp_dl::extract_zip_files(zip_files, args.force).await;
        if args.verbose {
            println!("Unzipped {extracted_file_count} files");
        }
    }

    let removed_image_count = utils::remove_images(&output_path, args.verbose)?;

    // Count only what this run produced,
    // direct downloads plus files unpacked from zips,
    // minus the cover images removed afterwards.
    // This avoids miscounting from unrelated filesystem changes,
    // or files extracted into subdirectories.
    let added = (downloaded_file_count + extracted_file_count).saturating_sub(removed_image_count);
    match added {
        added if added >= 2 => println!("{}", format!("Added {added} new files").green()),
        1 => println!("{}", "Added 1 new file".green()),
        _ => println!("{}", "No new files added".yellow()),
    }

    if args.browser && failed > 0 {
        anyhow::bail!(
            "{failed} downloads failed. If links expired, refresh the Bandcamp download page and retry."
        );
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

        let urls: Vec<String> =
            parse_urls(args.urls.as_deref().unwrap()).expect("Failed to parse URLs");
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

        let urls: Vec<String> = parse_urls(args.urls.as_deref().unwrap()).unwrap();
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

        let urls: Vec<String> =
            parse_urls(args.urls.as_deref().unwrap()).expect("Failed to parse URLs");
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

    #[test]
    fn browser_arguments() {
        let args = Args::try_parse_from([
            "bcdl",
            "--browser",
            "-o",
            "music",
            "--browser-timeout",
            "10",
            "--browser-tab",
            "2",
            "--force",
        ])
        .unwrap();
        assert!(args.browser);
        assert!(args.force);
        assert_eq!(args.output.as_deref(), Some("music"));
        assert_eq!(args.browser_tab, Some(2));
        for arguments in [
            vec!["bcdl"],
            vec!["bcdl", "--browser", "https://example.com"],
            vec!["bcdl", "--browser", "--browser-setup"],
            vec!["bcdl", "--browser-timeout", "10", "https://example.com"],
            vec!["bcdl", "--browser", "--browser-timeout", "0"],
        ] {
            assert!(Args::try_parse_from(&arguments).is_err(), "{arguments:?}");
        }
        assert!(Args::try_parse_from(["bcdl", "--browser-setup"]).is_ok());
    }
}
