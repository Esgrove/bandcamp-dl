use bandcamp_dl::count_files_in_directory;
use colored::Colorize;
use std::fs;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let current_dir = std::env::current_dir()?;

    let file_count_at_start = count_files_in_directory(&current_dir)?;

    let zip_files: Vec<_> = fs::read_dir(&current_dir)?
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

    for zip_file in zip_files.iter() {
        println!("{:?}", zip_file);
    }

    if !zip_files.is_empty() {
        if zip_files.len() > 1 {
            println!("Extracting {} zip files", zip_files.len());
        } else {
            println!("Extracting 1 zip file");
        };
        bandcamp_dl::extract_zip_files(zip_files, true).await;
    }

    let file_count_at_end = count_files_in_directory(&current_dir)?;
    let added_files = file_count_at_end as i64 - file_count_at_start as i64;
    println!("{}", format!("Added {added_files} new files").green());

    Ok(())
}
