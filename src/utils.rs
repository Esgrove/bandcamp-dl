use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::Context;

/// Resolves the provided path to a directory or file to an absolute path.
///
/// If `path` is `None` or an empty string, the current working directory is used.
/// The function verifies that the provided path exists and is accessible.
pub fn resolve_path(path: Option<String>) -> anyhow::Result<PathBuf> {
    let input_path = path.unwrap_or_default().trim().to_string();
    let filepath = if input_path.is_empty() {
        env::current_dir().context("Failed to get current working directory")?
    } else {
        PathBuf::from(input_path)
    };
    if !filepath.exists() {
        anyhow::bail!(
            "Input path does not exist or is not accessible: '{}'",
            filepath.display()
        );
    }
    dunce::canonicalize(filepath).context("Failed to get absolute path")
}

/// Convert the given path to be relative to the current working directory.
/// Returns the original path if the relative path cannot be created,
/// or the path is the same.
#[must_use]
pub fn get_relative_path_from_current_working_directory(path: &Path) -> PathBuf {
    env::current_dir().map_or_else(
        |_| path.to_path_buf(),
        |current_dir| {
            if current_dir == path {
                current_dir
            } else {
                path.strip_prefix(&current_dir)
                    .unwrap_or(path)
                    .to_path_buf()
            }
        },
    )
}

/// Move all JPEG and PNG files in given dir to trash.
pub fn remove_images_from_dir(path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let entries = fs::read_dir(path)?;
    let mut removed = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if let Some(extension) = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_lowercase)
        {
            match extension.as_str() {
                "jpg" | "jpeg" | "png" => {
                    trash::delete(&path).with_context(|| {
                        format!("Failed to move image to trash: {}", path.display())
                    })?;
                    removed.push(path.clone());
                }
                _ => (),
            }
        }
    }
    Ok(removed)
}

/// Return the number of files in given directory.
pub fn count_files_in_directory<P: AsRef<Path>>(path: P) -> anyhow::Result<usize> {
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

/// Get all zip files from the given paths.
pub fn get_all_zip_files(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|path| path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("zip"))
        .map(PathBuf::from)
        .collect()
}

/// Get the absolute path for the given output path.
/// Uses current working directory if nothing was given.
pub fn resolve_output_path(path: Option<&str>) -> anyhow::Result<PathBuf> {
    let output = path.unwrap_or_default().trim().to_string();
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
    Ok(absolute_output_path)
}

/// Remove all images from given directory.
/// Print removed files with the verbose flag.
pub fn remove_images(directory: &Path, verbose: bool) -> anyhow::Result<usize> {
    let removed = remove_images_from_dir(directory)?;
    if !removed.is_empty() && verbose {
        println!("Removed images ({}):", removed.len());
        for file in &removed {
            println!(
                "  {}",
                get_relative_path_from_current_working_directory(file).display()
            );
        }
    }
    Ok(removed.len())
}

/// Get filename string for given Path.
pub fn get_filename_from_path(path: &Path) -> anyhow::Result<String> {
    let file_name = path
        .file_name()
        .context("Failed to get zip file name")?
        .to_string_lossy()
        .replace('\u{FFFD}', "");
    Ok(file_name)
}
