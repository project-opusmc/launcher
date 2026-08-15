use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;
use url::Url;

const USER_AGENT: &str = concat!("RBW-Client/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadSpec {
    pub url: String,
    pub sha1: String,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadOutcome {
    AlreadyPresent,
    Downloaded,
}

#[derive(Debug, Clone)]
pub struct Downloader {
    client: Client,
}

impl Downloader {
    pub fn new() -> Result<Self, DownloadError> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()?;
        Ok(Self { client })
    }

    pub fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, DownloadError> {
        validate_url(url)?;
        let response = self.client.get(url).send()?.error_for_status()?;
        let limited = response.take(16 * 1024 * 1024);
        Ok(serde_json::from_reader(limited)?)
    }

    pub fn ensure(
        &self,
        spec: &DownloadSpec,
        destination: &Path,
    ) -> Result<DownloadOutcome, DownloadError> {
        validate_spec(spec)?;
        if destination.is_file() && verify_file(destination, spec)? {
            return Ok(DownloadOutcome::AlreadyPresent);
        }

        if destination.exists() {
            fs::remove_file(destination).map_err(|source| DownloadError::RemoveInvalidFile {
                path: destination.to_path_buf(),
                source,
            })?;
        }

        let parent = destination
            .parent()
            .ok_or_else(|| DownloadError::MissingParent(destination.to_path_buf()))?;
        fs::create_dir_all(parent)?;

        let part_path = part_path(destination);
        if part_path.exists() {
            fs::remove_file(&part_path)?;
        }

        let result = self.download_to(spec, &part_path);
        if let Err(error) = result {
            let _ = fs::remove_file(&part_path);
            return Err(error);
        }

        if !verify_file(&part_path, spec)? {
            let _ = fs::remove_file(&part_path);
            return Err(DownloadError::IntegrityMismatch {
                path: destination.to_path_buf(),
            });
        }

        fs::rename(&part_path, destination).map_err(|source| DownloadError::AtomicRename {
            source_path: part_path,
            destination: destination.to_path_buf(),
            source,
        })?;
        Ok(DownloadOutcome::Downloaded)
    }

    fn download_to(&self, spec: &DownloadSpec, destination: &Path) -> Result<(), DownloadError> {
        let response = self.client.get(&spec.url).send()?.error_for_status()?;
        if let (Some(expected), Some(actual)) = (spec.size, response.content_length())
            && expected != actual
        {
            return Err(DownloadError::RemoteSizeMismatch {
                url: spec.url.clone(),
                expected,
                actual,
            });
        }

        let mut source: Box<dyn Read> = if let Some(size) = spec.size {
            Box::new(response.take(size.saturating_add(1)))
        } else {
            Box::new(response.take(512 * 1024 * 1024))
        };
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)?;
        io::copy(&mut source, &mut output)?;
        output.flush()?;
        output.sync_all()?;
        Ok(())
    }
}

pub fn verify_file(path: &Path, spec: &DownloadSpec) -> Result<bool, DownloadError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };

    if let Some(expected_size) = spec.size
        && file.metadata()?.len() != expected_size
    {
        return Ok(false);
    }

    let mut reader = BufReader::new(file);
    let mut hasher = Sha1::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    Ok(actual.eq_ignore_ascii_case(&spec.sha1))
}

fn validate_spec(spec: &DownloadSpec) -> Result<(), DownloadError> {
    validate_url(&spec.url)?;
    if spec.sha1.len() != 40 || !spec.sha1.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DownloadError::InvalidSha1(spec.sha1.clone()));
    }
    Ok(())
}

fn validate_url(raw: &str) -> Result<(), DownloadError> {
    let url = Url::parse(raw)?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err(DownloadError::InsecureUrl(raw.to_owned()));
    }
    Ok(())
}

fn part_path(destination: &Path) -> PathBuf {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    destination.with_file_name(format!(".{file_name}.part-{}", std::process::id()))
}

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("HTTP request failed")]
    Http(#[from] reqwest::Error),
    #[error("invalid URL")]
    Url(#[from] url::ParseError),
    #[error("JSON payload is invalid")]
    Json(#[from] serde_json::Error),
    #[error("filesystem operation failed")]
    Io(#[from] io::Error),
    #[error("refusing non-HTTPS URL: {0}")]
    InsecureUrl(String),
    #[error("invalid SHA-1 digest: {0}")]
    InvalidSha1(String),
    #[error("destination has no parent: {path}", path = .0.display())]
    MissingParent(PathBuf),
    #[error("failed to remove invalid cached file {path}", path = .path.display())]
    RemoveInvalidFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("remote size mismatch for {url}: expected {expected}, got {actual}")]
    RemoteSizeMismatch {
        url: String,
        expected: u64,
        actual: u64,
    },
    #[error("download failed integrity verification for {path}", path = .path.display())]
    IntegrityMismatch { path: PathBuf },
    #[error(
        "failed to atomically move {source_path} to {destination}",
        source_path = .source_path.display(),
        destination = .destination.display()
    )]
    AtomicRename {
        source_path: PathBuf,
        destination: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn verifies_size_and_sha1() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("artifact");
        File::create(&path).unwrap().write_all(b"hello").unwrap();
        let spec = DownloadSpec {
            url: "https://example.invalid/artifact".to_owned(),
            sha1: "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d".to_owned(),
            size: Some(5),
        };
        assert!(verify_file(&path, &spec).unwrap());
    }

    #[test]
    fn rejects_bad_size() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("artifact");
        File::create(&path).unwrap().write_all(b"hello").unwrap();
        let wrong_size = DownloadSpec {
            url: "https://example.invalid/artifact".to_owned(),
            sha1: "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d".to_owned(),
            size: Some(6),
        };
        assert!(!verify_file(&path, &wrong_size).unwrap());
    }

    #[test]
    fn rejects_non_https_urls() {
        assert!(matches!(
            validate_url("http://example.com/file"),
            Err(DownloadError::InsecureUrl(_))
        ));
    }
}
