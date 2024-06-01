use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use futures::stream::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::header::{HeaderMap, CONTENT_DISPOSITION, CONTENT_LENGTH};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let urls = vec![
        "https://p4.bcbits.com/download/track/1bc74434a1bc66251cfd38da92c20f53e/aiff-lossless/3982707245?id=3982707245&sig=d100c233fb7c8158a445faf708f48eae&sitem_id=291330142&token=1717843604_7522f6b45552b71956bc7a5e619ac0113277908b",
        "https://p4.bcbits.com/download/track/19f4de2807fae9aa36dc0d10464dc491e/aiff-lossless/2330900126?id=2330900126&sig=9a7ffa5d154c7e3aaa28119a1438034a&sitem_id=291330178&token=1717843604_a74301d258e4690243b0b2e91545201b9d3d4abd",
        "https://p4.bcbits.com/download/track/1ddd7ddac5fd3c6d01eb8b77de9a92608/aiff-lossless/356708740?id=356708740&sig=8381d568e494d30b65883177c2bf6164&sitem_id=291330405&token=1717843603_4071fe1c2281ee99cc5080c2517920667720771f",
    ];

    let multi_progress = Arc::new(MultiProgress::new());

    let tasks: Vec<_> = urls
        .into_iter()
        .map(|url| {
            let mp = Arc::clone(&multi_progress);
            tokio::spawn(async move {
                if let Err(e) = download_file(url, mp).await {
                    eprintln!("Error downloading {}: {}", url, e);
                }
            })
        })
        .collect();

    futures::future::join_all(tasks).await;

    Ok(())
}

async fn download_file(
    url: &str,
    multi_progress: Arc<MultiProgress>,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = reqwest::get(url).await?;
    let headers = response.headers();
    let filename = get_filename(headers, url);
    let total_size = headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let path = Path::new(&filename);
    if path.exists() {
        println!("File already exists: {}", filename);
        return Ok(());
    }

    println!("Downloading {} ({} bytes)...", filename, total_size);

    let mut file = File::create(&filename)?;
    let mut content = response.bytes_stream();

    let pb = multi_progress.add(ProgressBar::new(total_size));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{bar:40.cyan/blue} {bytes}/{total_bytes} ({eta})")?
            .progress_chars("##-"),
    );

    while let Some(chunk) = content.next().await {
        let chunk = chunk?;
        pb.inc(chunk.len() as u64);
        file.write_all(&chunk)?;
    }

    pb.finish_with_message("Download complete");
    println!("File downloaded: {}", filename);
    Ok(())
}

fn get_filename(headers: &HeaderMap, url: &str) -> String {
    if let Some(content_disposition) = headers.get(CONTENT_DISPOSITION) {
        if let Ok(content_disposition) = content_disposition.to_str() {
            if let Some(filename) = content_disposition.split("filename=").nth(1) {
                return filename.trim_matches('"').to_string();
            }
        }
    }
    url.split('/')
        .last()
        .unwrap_or("downloaded_file")
        .to_string()
}
