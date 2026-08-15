use serde::{Deserialize, Serialize};
use std::env;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

pub const OPUS_HOME_ENV: &str = "OPUS_HOME";
pub const RBW_HOME_ENV: &str = "RBW_HOME";
/// Root override for the intentionally separate QA launcher installation.
///
/// This is deliberately distinct from `RBW_HOME`: an offline QA build must
/// never pick up a Premium install, account settings, or game directory just
/// because the Premium override is present in the environment.
pub const OPUS_QA_HOME_ENV: &str = "OPUS_QA_HOME";
pub const RBW_QA_HOME_ENV: &str = "RBW_QA_HOME";
/// Root override for the disposable in-game UI preview edition.
///
/// The UI preview is a third, fully isolated lane: it must not reuse either
/// the Premium root or the ordinary offline QA/Demo root while Forge UI work
/// is being proven against a real game session.
pub const OPUS_UI_PREVIEW_HOME_ENV: &str = "OPUS_UI_PREVIEW_HOME";
pub const RBW_UI_PREVIEW_HOME_ENV: &str = "RBW_UI_PREVIEW_HOME";
pub const OPUS_JAVA_HOME_ENV: &str = "OPUS_JAVA_HOME";
pub const RBW_JAVA_HOME_ENV: &str = "RBW_JAVA_HOME";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperatingSystem {
    Windows,
    MacOs,
    Linux,
}

impl OperatingSystem {
    pub fn minecraft_rule_name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::MacOs => "osx",
            Self::Linux => "linux",
        }
    }

    pub fn executable_name(self, base: &str) -> String {
        match self {
            Self::Windows => format!("{base}.exe"),
            Self::MacOs | Self::Linux => base.to_owned(),
        }
    }
}

impl Display for OperatingSystem {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Windows => "windows",
            Self::MacOs => "macos",
            Self::Linux => "linux",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Architecture {
    X86,
    X86_64,
    Aarch64,
}

impl Architecture {
    pub fn bits(self) -> &'static str {
        match self {
            Self::X86 => "32",
            Self::X86_64 | Self::Aarch64 => "64",
        }
    }
}

impl Display for Architecture {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::X86 => "x86",
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Platform {
    pub os: OperatingSystem,
    pub host_arch: Architecture,
    pub game_arch: Architecture,
}

impl Platform {
    pub fn detect() -> Result<Self, PlatformError> {
        let os = match env::consts::OS {
            "windows" => OperatingSystem::Windows,
            "macos" => OperatingSystem::MacOs,
            "linux" => OperatingSystem::Linux,
            other => return Err(PlatformError::UnsupportedOs(other.to_owned())),
        };

        let host_arch = match env::consts::ARCH {
            "x86" => Architecture::X86,
            "x86_64" => Architecture::X86_64,
            "aarch64" => Architecture::Aarch64,
            other => return Err(PlatformError::UnsupportedArchitecture(other.to_owned())),
        };

        // Mojang does not publish jre-legacy or LWJGL 2 natives for macOS ARM64.
        // The foundation therefore runs the complete game JVM as x86_64 under
        // Rosetta, keeping the Java and native architectures consistent.
        let game_arch = if os == OperatingSystem::MacOs && host_arch == Architecture::Aarch64 {
            Architecture::X86_64
        } else {
            host_arch
        };

        Ok(Self {
            os,
            host_arch,
            game_arch,
        })
    }

    pub fn minecraft_runtime_key(self) -> Result<&'static str, PlatformError> {
        match (self.os, self.game_arch) {
            (OperatingSystem::Windows, Architecture::X86) => Ok("windows-x86"),
            (OperatingSystem::Windows, Architecture::X86_64) => Ok("windows-x64"),
            (OperatingSystem::MacOs, Architecture::X86_64) => Ok("mac-os"),
            (OperatingSystem::MacOs, Architecture::Aarch64) => Ok("mac-os-arm64"),
            (OperatingSystem::Linux, Architecture::X86_64) => Ok("linux"),
            combination => Err(PlatformError::UnsupportedGamePlatform(format!(
                "{}-{}",
                combination.0, combination.1
            ))),
        }
    }

    pub fn requires_translation(self) -> bool {
        self.host_arch != self.game_arch
    }

    pub fn translation_available(self) -> Result<bool, PlatformError> {
        if !self.requires_translation() {
            return Ok(true);
        }
        if self.os == OperatingSystem::MacOs
            && self.host_arch == Architecture::Aarch64
            && self.game_arch == Architecture::X86_64
        {
            let output = Command::new("/usr/bin/arch")
                .args(["-x86_64", "/usr/bin/true"])
                .output()
                .map_err(PlatformError::TranslationProbeFailed)?;
            return Ok(output.status.success());
        }
        Ok(false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RbwPaths {
    pub root: PathBuf,
    pub cache: PathBuf,
    pub minecraft: PathBuf,
    pub runtime: PathBuf,
    pub game: PathBuf,
    pub logs: PathBuf,
    pub sessions: PathBuf,
}

impl RbwPaths {
    pub fn discover() -> Result<Self, PlatformError> {
        if let Some(configured) = env::var_os(OPUS_HOME_ENV).or_else(|| env::var_os(RBW_HOME_ENV)) {
            return Self::from_root(PathBuf::from(configured));
        }

        let platform = Platform::detect()?;
        let base = directories::BaseDirs::new().ok_or(PlatformError::HomeDirectoryUnavailable)?;
        let (root, legacy_root) = match platform.os {
            OperatingSystem::Windows => (
                base.data_local_dir().join("OpusLauncher"),
                base.data_local_dir().join("RBWClient"),
            ),
            OperatingSystem::MacOs | OperatingSystem::Linux => (
                base.home_dir().join(".opus-launcher"),
                base.home_dir().join(".rbw-client"),
            ),
        };
        Self::from_root(migrate_default_root(root, legacy_root))
    }

    /// Resolve the storage root for the offline QA edition.
    ///
    /// QA intentionally ignores `RBW_HOME` and has its own opt-in override.
    /// This prevents a demo build from reading or modifying the Premium
    /// launcher's managed runtime, settings, logs, or credentials by mistake.
    pub fn discover_qa() -> Result<Self, PlatformError> {
        let root = if let Some(configured) =
            env::var_os(OPUS_QA_HOME_ENV).or_else(|| env::var_os(RBW_QA_HOME_ENV))
        {
            let root = PathBuf::from(configured);
            if !root.is_absolute() {
                return Err(PlatformError::InvalidRoot(root));
            }
            root
        } else {
            let platform = Platform::detect()?;
            let base =
                directories::BaseDirs::new().ok_or(PlatformError::HomeDirectoryUnavailable)?;
            let (root, legacy_root) = match platform.os {
                OperatingSystem::Windows => (
                    base.data_local_dir().join("OpusLauncherQA"),
                    base.data_local_dir().join("RBWClientQA"),
                ),
                OperatingSystem::MacOs | OperatingSystem::Linux => (
                    base.home_dir().join(".opus-launcher-qa"),
                    base.home_dir().join(".rbw-client-qa"),
                ),
            };
            migrate_default_root(root, legacy_root)
        };

        // The override is useful for disposable demos, but it must not turn
        // the QA edition into a reader/writer of Premium state. Compare with
        // the Premium resolver, including its RBW_HOME override, before any
        // caller constructs a layout below this root.
        let premium_root = Self::discover()?.root;
        ensure_qa_root_isolated(&root, &premium_root)?;
        Self::from_root(root)
    }

    /// Resolve the storage root for the isolated in-game UI preview edition.
    ///
    /// This uses the same offline identity policy as QA, but it deliberately
    /// owns neither QA's game cache nor its settings. Keeping the migration
    /// preview separate lets us prove a normal Forge client mod without
    /// replacing the user's existing Demo installation.
    pub fn discover_ui_preview() -> Result<Self, PlatformError> {
        let root = if let Some(configured) =
            env::var_os(OPUS_UI_PREVIEW_HOME_ENV).or_else(|| env::var_os(RBW_UI_PREVIEW_HOME_ENV))
        {
            let root = PathBuf::from(configured);
            if !root.is_absolute() {
                return Err(PlatformError::InvalidRoot(root));
            }
            root
        } else {
            let platform = Platform::detect()?;
            let base =
                directories::BaseDirs::new().ok_or(PlatformError::HomeDirectoryUnavailable)?;
            let (root, legacy_root) = match platform.os {
                OperatingSystem::Windows => (
                    base.data_local_dir().join("OpusLauncherUiPreview"),
                    base.data_local_dir().join("RBWClientUiPreview"),
                ),
                OperatingSystem::MacOs | OperatingSystem::Linux => (
                    base.home_dir().join(".opus-launcher-ui-preview"),
                    base.home_dir().join(".rbw-client-ui-preview"),
                ),
            };
            migrate_default_root(root, legacy_root)
        };

        let premium_root = Self::discover()?.root;
        let qa_root = Self::discover_qa()?.root;
        ensure_ui_preview_root_isolated(&root, &premium_root, &qa_root)?;
        Self::from_root(root)
    }

    pub fn from_root(root: PathBuf) -> Result<Self, PlatformError> {
        if root.as_os_str().is_empty() {
            return Err(PlatformError::InvalidRoot(root));
        }
        Ok(Self {
            cache: root.join("cache"),
            minecraft: root.join("minecraft"),
            runtime: root.join("runtime"),
            game: root.join("game"),
            logs: root.join("logs"),
            sessions: root.join("sessions"),
            root,
        })
    }

    pub fn create_all(&self) -> Result<(), std::io::Error> {
        for directory in [
            &self.root,
            &self.cache,
            &self.minecraft,
            &self.runtime,
            &self.game,
            &self.logs,
            &self.sessions,
        ] {
            std::fs::create_dir_all(directory)?;
        }
        Ok(())
    }
}

fn migrate_default_root(root: PathBuf, legacy_root: PathBuf) -> PathBuf {
    if root.exists() || !legacy_root.is_dir() {
        return root;
    }
    match std::fs::rename(&legacy_root, &root) {
        Ok(()) => root,
        Err(_) => default_root_after_failed_rename(root, legacy_root),
    }
}

fn default_root_after_failed_rename(root: PathBuf, legacy_root: PathBuf) -> PathBuf {
    // Snapshot, settings, and utility commands can resolve paths concurrently
    // on first launch. If another caller completed the rename after our
    // preflight check, converge on the migrated root instead of recreating and
    // splitting state across the legacy directory.
    if root.is_dir() { root } else { legacy_root }
}

fn ensure_qa_root_isolated(qa_root: &Path, premium_root: &Path) -> Result<(), PlatformError> {
    if paths_overlap(qa_root, premium_root) {
        return Err(PlatformError::QaRootConflictsWithPremium(
            qa_root.to_path_buf(),
        ));
    }
    Ok(())
}

fn ensure_ui_preview_root_isolated(
    preview_root: &Path,
    premium_root: &Path,
    qa_root: &Path,
) -> Result<(), PlatformError> {
    if paths_overlap(preview_root, premium_root) || paths_overlap(preview_root, qa_root) {
        return Err(PlatformError::UiPreviewRootConflictsWithExistingEdition(
            preview_root.to_path_buf(),
        ));
    }
    Ok(())
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right
        || match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaProbe {
    pub executable: PathBuf,
    pub major_version: u32,
    pub architecture: Architecture,
    pub raw_version: String,
}

impl JavaProbe {
    pub fn probe(java_home: &Path, os: OperatingSystem) -> Result<Self, PlatformError> {
        let executable = java_home.join("bin").join(os.executable_name("java"));
        if !executable.is_file() {
            return Err(PlatformError::JavaExecutableMissing(executable));
        }

        let output = Command::new(&executable)
            .args(["-XshowSettings:properties", "-version"])
            .output()
            .map_err(|source| PlatformError::JavaProbeFailed {
                executable: executable.clone(),
                source,
            })?;

        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !output.status.success() {
            return Err(PlatformError::JavaProbeExited {
                executable,
                status: output.status.code(),
                output: combined,
            });
        }

        let raw_version = property(&combined, "java.version")
            .ok_or_else(|| PlatformError::JavaPropertyMissing("java.version".to_owned()))?;
        let major_version = parse_java_major(&raw_version)?;
        let raw_arch = property(&combined, "os.arch")
            .ok_or_else(|| PlatformError::JavaPropertyMissing("os.arch".to_owned()))?;
        let architecture = parse_java_architecture(&raw_arch)?;

        Ok(Self {
            executable,
            major_version,
            architecture,
            raw_version,
        })
    }
}

fn property(output: &str, name: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (key, value) = line.trim().split_once('=')?;
        (key.trim() == name).then(|| value.trim().to_owned())
    })
}

fn parse_java_major(version: &str) -> Result<u32, PlatformError> {
    let major = if let Some(rest) = version.strip_prefix("1.") {
        rest.split('.').next()
    } else {
        version.split(['.', '-', '_']).next()
    };
    major
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| PlatformError::InvalidJavaVersion(version.to_owned()))
}

fn parse_java_architecture(architecture: &str) -> Result<Architecture, PlatformError> {
    match architecture.to_ascii_lowercase().as_str() {
        "x86" | "i386" | "i486" | "i586" | "i686" => Ok(Architecture::X86),
        "amd64" | "x86_64" => Ok(Architecture::X86_64),
        "aarch64" | "arm64" => Ok(Architecture::Aarch64),
        other => Err(PlatformError::UnsupportedArchitecture(other.to_owned())),
    }
}

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("unsupported operating system: {0}")]
    UnsupportedOs(String),
    #[error("unsupported architecture: {0}")]
    UnsupportedArchitecture(String),
    #[error("unsupported game platform: {0}")]
    UnsupportedGamePlatform(String),
    #[error("home directory is unavailable")]
    HomeDirectoryUnavailable,
    #[error("invalid Opus root directory: {path}", path = .0.display())]
    InvalidRoot(PathBuf),
    #[error("Opus QA data root must not overlap the Premium data root: {0}")]
    QaRootConflictsWithPremium(PathBuf),
    #[error("Opus UI Preview data root must not overlap the Premium or QA data root: {0}")]
    UiPreviewRootConflictsWithExistingEdition(PathBuf),
    #[error("Java executable does not exist: {path}", path = .0.display())]
    JavaExecutableMissing(PathBuf),
    #[error("failed to execute Java at {executable}", executable = .executable.display())]
    JavaProbeFailed {
        executable: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Java probe at {executable} exited with {status:?}: {output}", executable = .executable.display())]
    JavaProbeExited {
        executable: PathBuf,
        status: Option<i32>,
        output: String,
    },
    #[error("Java probe did not report property {0}")]
    JavaPropertyMissing(String),
    #[error("invalid Java version: {0}")]
    InvalidJavaVersion(String),
    #[error("failed to probe the host translation layer")]
    TranslationProbeFailed(#[source] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_java_version() {
        assert_eq!(parse_java_major("1.8.0_74").unwrap(), 8);
    }

    #[test]
    fn parses_modern_java_version() {
        assert_eq!(parse_java_major("25.0.2").unwrap(), 25);
    }

    #[test]
    fn creates_isolated_layout() {
        let temp = tempfile::tempdir().unwrap();
        let paths = RbwPaths::from_root(temp.path().join("rbw")).unwrap();
        paths.create_all().unwrap();

        assert!(paths.minecraft.is_dir());
        assert!(paths.game.is_dir());
        assert_ne!(paths.game, paths.minecraft);
    }

    #[test]
    fn rejects_a_qa_root_that_overlaps_premium_storage() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("rbw");
        assert!(ensure_qa_root_isolated(&root, &root).is_err());
        assert!(ensure_qa_root_isolated(&temp.path().join("qa"), &root).is_ok());
    }

    #[test]
    fn rejects_a_ui_preview_root_that_overlaps_existing_storage() {
        let temp = tempfile::tempdir().unwrap();
        let preview = temp.path().join("preview");
        let premium = temp.path().join("premium");
        let qa = temp.path().join("qa");

        assert!(ensure_ui_preview_root_isolated(&premium, &premium, &qa).is_err());
        assert!(ensure_ui_preview_root_isolated(&qa, &premium, &qa).is_err());
        assert!(ensure_ui_preview_root_isolated(&preview, &premium, &qa).is_ok());
    }

    #[test]
    fn migrates_a_legacy_default_root_without_copying_data() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join(".rbw-client");
        let opus = temp.path().join(".opus-launcher");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("accounts-v1.json"), b"legacy").unwrap();

        let resolved = migrate_default_root(opus.clone(), legacy.clone());

        assert_eq!(resolved, opus);
        assert!(resolved.join("accounts-v1.json").is_file());
        assert!(!legacy.exists());
    }

    #[test]
    fn failed_concurrent_migration_converges_on_the_new_root() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join(".rbw-client");
        let opus = temp.path().join(".opus-launcher");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&opus).unwrap();

        let resolved = default_root_after_failed_rename(opus.clone(), legacy);

        assert_eq!(resolved, opus);
    }

    #[test]
    fn maps_java_architectures() {
        assert_eq!(
            parse_java_architecture("amd64").unwrap(),
            Architecture::X86_64
        );
        assert_eq!(
            parse_java_architecture("aarch64").unwrap(),
            Architecture::Aarch64
        );
    }
}
