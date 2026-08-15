use crate::{NativeArchive, safe_join};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use thiserror::Error;
use zip::ZipArchive;

pub fn extract_natives(
    archives: &[NativeArchive],
    destination: &Path,
) -> Result<Vec<PathBuf>, NativeError> {
    if destination.exists() {
        let mut entries = fs::read_dir(destination)?;
        if entries.next().transpose()?.is_some() {
            return Err(NativeError::DestinationNotEmpty(destination.to_path_buf()));
        }
    } else {
        fs::create_dir_all(destination)?;
    }

    let mut extracted = Vec::new();
    for archive in archives {
        let file = File::open(&archive.path).map_err(|source| NativeError::OpenArchive {
            path: archive.path.clone(),
            source,
        })?;
        let mut zip = ZipArchive::new(BufReader::new(file)).map_err(|source| {
            NativeError::InvalidArchive {
                path: archive.path.clone(),
                source,
            }
        })?;

        for index in 0..zip.len() {
            let mut entry = zip
                .by_index(index)
                .map_err(|source| NativeError::InvalidArchive {
                    path: archive.path.clone(),
                    source,
                })?;
            let name = entry.name().replace('\\', "/");
            if entry.is_dir()
                || name.starts_with("META-INF/")
                || archive
                    .excludes
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
            {
                continue;
            }

            let output_path = safe_join(destination, &name)?;
            let parent = output_path
                .parent()
                .ok_or_else(|| NativeError::MissingParent(output_path.clone()))?;
            fs::create_dir_all(parent)?;

            let mut output = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&output_path)
                .map_err(|source| {
                    if source.kind() == io::ErrorKind::AlreadyExists {
                        NativeError::ArchiveCollision(output_path.clone())
                    } else {
                        NativeError::WriteNative {
                            path: output_path.clone(),
                            source,
                        }
                    }
                })?;
            io::copy(&mut entry, &mut output)?;
            output.sync_all()?;
            make_executable(&output_path)?;
            extracted.push(output_path);
        }
    }

    if extracted.is_empty() {
        return Err(NativeError::NoNativesExtracted);
    }
    Ok(extracted)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), io::Error> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    fs::set_permissions(path, permissions)
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), io::Error> {
    Ok(())
}

#[derive(Debug, Error)]
pub enum NativeError {
    #[error("native extraction destination is not empty: {path}", path = .0.display())]
    DestinationNotEmpty(PathBuf),
    #[error("failed to open native archive {path}", path = .path.display())]
    OpenArchive {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid native archive {path}", path = .path.display())]
    InvalidArchive {
        path: PathBuf,
        #[source]
        source: zip::result::ZipError,
    },
    #[error("unsafe native archive path")]
    UnsafePath(#[from] crate::layout::UnsafeRelativePath),
    #[error("native destination has no parent: {path}", path = .0.display())]
    MissingParent(PathBuf),
    #[error("native archives contain a duplicate output path: {path}", path = .0.display())]
    ArchiveCollision(PathBuf),
    #[error("failed to write native library {path}", path = .path.display())]
    WriteNative {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("filesystem operation failed during native extraction")]
    Io(#[from] io::Error),
    #[error("selected native archives did not contain any native files")]
    NoNativesExtracted,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    #[test]
    fn extracts_native_and_skips_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("natives.jar");
        write_zip(
            &archive,
            &[
                ("META-INF/MANIFEST.MF", b"manifest"),
                ("liblwjgl.dylib", b"native"),
            ],
        );
        let destination = temp.path().join("out");
        let extracted = extract_natives(
            &[NativeArchive {
                path: archive,
                excludes: vec!["ignored/".to_owned()],
            }],
            &destination,
        )
        .unwrap();

        assert_eq!(extracted.len(), 1);
        assert_eq!(
            fs::read(destination.join("liblwjgl.dylib")).unwrap(),
            b"native"
        );
        assert!(!destination.join("META-INF").exists());
    }

    #[test]
    fn rejects_archive_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("unsafe.jar");
        write_zip(&archive, &[("../escape.dylib", b"bad")]);

        assert!(
            extract_natives(
                &[NativeArchive {
                    path: archive,
                    excludes: vec![],
                }],
                &temp.path().join("out"),
            )
            .is_err()
        );
        assert!(!temp.path().join("escape.dylib").exists());
    }

    #[test]
    fn rejects_colliding_native_archives() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.jar");
        let second = temp.path().join("second.jar");
        write_zip(&first, &[("same.dylib", b"first")]);
        write_zip(&second, &[("same.dylib", b"second")]);

        assert!(matches!(
            extract_natives(
                &[
                    NativeArchive {
                        path: first,
                        excludes: vec![],
                    },
                    NativeArchive {
                        path: second,
                        excludes: vec![],
                    },
                ],
                &temp.path().join("out"),
            ),
            Err(NativeError::ArchiveCollision(_))
        ));
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut writer = ZipWriter::new(file);
        for (name, contents) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(contents).unwrap();
        }
        writer.finish().unwrap();
    }
}
