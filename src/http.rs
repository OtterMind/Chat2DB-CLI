use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::Duration;

use crate::error::{Chat2DbError, Result};

const OFFICIAL_REFERER: &str = "https://chat2db-ai.com/";

pub trait HttpClient {
    fn get(&self, url: &str) -> Result<Vec<u8>>;

    fn download_to(&self, url: &str, path: &Path, progress: DownloadProgress<'_>) -> Result<()> {
        let bytes = self.get(url)?;
        std::fs::write(path, bytes)?;
        progress.finish();
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DownloadProgress<'a> {
    enabled: bool,
    label: &'a str,
}

impl<'a> DownloadProgress<'a> {
    pub fn new(enabled: bool, label: &'a str) -> Self {
        Self { enabled, label }
    }

    fn report(self, downloaded: u64, total: Option<u64>, force: bool) {
        if !self.enabled {
            return;
        }

        match total {
            Some(total) if total > 0 => {
                let percent = downloaded.saturating_mul(100) / total;
                eprint!(
                    "\rDownloading {}: {:>3}% ({}/{})",
                    self.label,
                    percent,
                    format_bytes(downloaded),
                    format_bytes(total)
                );
            }
            _ => {
                eprint!("\rDownloading {}: {}", self.label, format_bytes(downloaded));
            }
        }

        if force {
            eprintln!();
        } else {
            let _ = io::stderr().flush();
        }
    }

    fn finish(self) {
        if self.enabled {
            eprintln!("Downloaded {}.", self.label);
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ReqwestHttpClient;

impl HttpClient for ReqwestHttpClient {
    fn get(&self, url: &str) -> Result<Vec<u8>> {
        let response = send_official_get(url)?;
        let bytes = response
            .bytes()
            .map_err(|source| Chat2DbError::HttpRequest {
                url: url.to_string(),
                source,
            })?;

        Ok(bytes.to_vec())
    }

    fn download_to(&self, url: &str, path: &Path, progress: DownloadProgress<'_>) -> Result<()> {
        let mut response = send_official_get(url)?;
        let total = response.content_length();

        let mut file = File::create(path)?;
        let mut buffer = [0_u8; 64 * 1024];
        let mut downloaded = 0_u64;
        let mut last_percent = None;
        let mut last_chunk_report = 0_u64;

        loop {
            let read = response.read(&mut buffer)?;
            if read == 0 {
                break;
            }

            file.write_all(&buffer[..read])?;
            downloaded += read as u64;

            if should_report_progress(downloaded, total, &mut last_percent, &mut last_chunk_report)
            {
                progress.report(downloaded, total, false);
            }
        }

        file.flush()?;
        progress.report(downloaded, total, true);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TimedReqwestHttpClient {
    timeout: Duration,
}

impl TimedReqwestHttpClient {
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl HttpClient for TimedReqwestHttpClient {
    fn get(&self, url: &str) -> Result<Vec<u8>> {
        let response = send_official_get_with_timeout(url, Some(self.timeout))?;
        let bytes = response
            .bytes()
            .map_err(|source| Chat2DbError::HttpRequest {
                url: url.to_string(),
                source,
            })?;
        Ok(bytes.to_vec())
    }
}

fn should_report_progress(
    downloaded: u64,
    total: Option<u64>,
    last_percent: &mut Option<u64>,
    last_chunk_report: &mut u64,
) -> bool {
    match total {
        Some(total) if total > 0 => {
            let percent = downloaded.saturating_mul(100) / total;
            let should_report = last_percent
                .map(|last| percent >= last.saturating_add(1))
                .unwrap_or(true);
            if should_report {
                *last_percent = Some(percent);
            }
            should_report
        }
        _ => {
            const TEN_MIB: u64 = 10 * 1024 * 1024;
            if downloaded.saturating_sub(*last_chunk_report) >= TEN_MIB {
                *last_chunk_report = downloaded;
                true
            } else {
                false
            }
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    format!("{:.1} MB", bytes as f64 / MIB)
}

fn send_official_get(url: &str) -> Result<reqwest::blocking::Response> {
    send_official_get_with_timeout(url, None)
}

fn send_official_get_with_timeout(
    url: &str,
    timeout: Option<Duration>,
) -> Result<reqwest::blocking::Response> {
    let mut builder = reqwest::blocking::Client::builder()
        .user_agent(concat!("chat2db-cli/", env!("CARGO_PKG_VERSION")));
    if let Some(timeout) = timeout {
        builder = builder.connect_timeout(timeout).timeout(timeout);
    }
    let client = builder
        .build()
        .map_err(|source| Chat2DbError::HttpRequest {
            url: url.to_string(),
            source,
        })?;
    let response = client
        .get(url)
        .header(reqwest::header::REFERER, OFFICIAL_REFERER)
        .send()
        .map_err(|source| Chat2DbError::HttpRequest {
            url: url.to_string(),
            source,
        })?;

    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND && is_known_manifest_url(url) {
        return Err(Chat2DbError::ManifestUnavailable {
            url: url.to_string(),
        });
    }
    if !status.is_success() {
        return Err(Chat2DbError::HttpStatus {
            url: url.to_string(),
            status,
        });
    }

    Ok(response)
}

/// Map a 404 on a recognized manifest endpoint to `manifest_unavailable`
/// instead of a generic HTTP error so agents can branch on a stable code.
fn is_known_manifest_url(url: &str) -> bool {
    is_binary_manifest_url(url) || is_skill_manifest_url(url)
}

fn is_binary_manifest_url(url: &str) -> bool {
    url.starts_with(
        "https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/manifest-cli-",
    ) && url.ends_with(".json")
}

fn is_skill_manifest_url(url: &str) -> bool {
    url == "https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/manifest-skill.json"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_binary_manifest_urls() {
        assert!(is_binary_manifest_url(
            "https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/manifest-cli-darwin-arm64.json"
        ));
        assert!(is_binary_manifest_url(
            "https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/manifest-cli-linux-x86_64.json"
        ));
        assert!(!is_binary_manifest_url(
            "https://cdn.chat2db-ai.com/download/chat2db-cli/0.1.0/foo.tar.gz"
        ));
    }

    #[test]
    fn detects_skill_manifest_url() {
        assert!(is_skill_manifest_url(
            "https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/manifest-skill.json"
        ));
        assert!(!is_skill_manifest_url(
            "https://cdn.chat2db-ai.com/download/chat2db-skill/0.1.0/skill.zip"
        ));
    }
}
