use anyhow::{Context, Result};
use futures::future::join_all;
use russh_sftp::client::SftpSession;
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::app::FileEntry;

pub async fn list_directory(sftp: &SftpSession, path: &str) -> Result<Vec<FileEntry>> {
    let entries = sftp
        .read_dir(path)
        .await
        .context("Failed to read directory")?;

    let mut files = Vec::new();

    // Add parent directory entry if not root
    if path != "/" {
        files.push(FileEntry {
            name: String::from(".."),
            path: String::from(".."),
            is_dir: true,
            size: 0,
            modified: None,
            permissions: None,
        });
    }

    // Collect file info first
    let mut file_info: Vec<(String, String)> = Vec::new();

    for entry in entries {
        let filename = entry.file_name();

        // Skip . and .. entries since we handle .. explicitly
        if filename == "." || filename == ".." {
            continue;
        }

        let full_path = if path.ends_with('/') {
            format!("{}{}", path, filename)
        } else {
            format!("{}/{}", path, filename)
        };

        file_info.push((filename.to_string(), full_path));
    }

    // Create futures for all metadata fetches with owned strings
    let metadata_futures: Vec<_> = file_info
        .iter()
        .map(|(_, path)| sftp.metadata(path))
        .collect();

    // Fetch all metadata concurrently (this is the speedup!)
    let metadata_results = join_all(metadata_futures).await;

    // Process results
    for ((filename, full_path), metadata_result) in file_info.into_iter().zip(metadata_results) {
        let metadata = metadata_result.ok();

        let (is_dir, size, modified) = if let Some(meta) = metadata {
            let modified_time = meta.modified().ok().and_then(|t| {
                t.duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|d| d.as_secs() as i64)
            });

            (
                meta.is_dir(),
                meta.len(),
                modified_time,
            )
        } else {
            // Fallback if stat fails - assume it's a file
            (false, 0, None)
        };

        files.push(FileEntry {
            name: filename,
            path: full_path,
            is_dir,
            size,
            modified,
            permissions: None,
        });
    }

    files.sort_by(|a, b| {
        match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        }
    });

    Ok(files)
}

pub async fn download_file(
    sftp: &SftpSession,
    remote_path: &str,
    local_path: &Path,
) -> Result<()> {
    let mut remote_file = sftp
        .open(remote_path)
        .await
        .context("Failed to open remote file")?;

    let mut local_file = File::create(local_path)
        .await
        .context("Failed to create local file")?;

    let mut buffer = vec![0u8; 32768];
    loop {
        let n = remote_file
            .read(&mut buffer)
            .await
            .context("Failed to read from remote file")?;

        if n == 0 {
            break;
        }

        local_file
            .write_all(&buffer[..n])
            .await
            .context("Failed to write to local file")?;
    }

    Ok(())
}

pub async fn upload_file(
    sftp: &SftpSession,
    local_path: &Path,
    remote_path: &str,
) -> Result<()> {
    let mut local_file = File::open(local_path)
        .await
        .context("Failed to open local file")?;

    let mut remote_file = sftp
        .create(remote_path)
        .await
        .context("Failed to create remote file")?;

    let mut buffer = vec![0u8; 32768];
    loop {
        let n = local_file
            .read(&mut buffer)
            .await
            .context("Failed to read from local file")?;

        if n == 0 {
            break;
        }

        remote_file
            .write_all(&buffer[..n])
            .await
            .context("Failed to write to remote file")?;
    }

    Ok(())
}

pub async fn delete_file(sftp: &SftpSession, path: &str) -> Result<()> {
    sftp.remove_file(path)
        .await
        .context("Failed to delete file")?;
    Ok(())
}

pub async fn delete_directory(sftp: &SftpSession, path: &str) -> Result<()> {
    sftp.remove_dir(path)
        .await
        .context("Failed to delete directory")?;
    Ok(())
}

pub async fn create_directory(sftp: &SftpSession, path: &str) -> Result<()> {
    sftp.create_dir(path)
        .await
        .context("Failed to create directory")?;
    Ok(())
}

pub async fn rename(sftp: &SftpSession, old_path: &str, new_path: &str) -> Result<()> {
    sftp.rename(old_path, new_path)
        .await
        .context("Failed to rename file")?;
    Ok(())
}
