use crate::download::DownloadError;
use crate::layout::UnsafeRelativePath;
use crate::{
    Artifact, AssetIndex, DownloadOutcome, DownloadSpec, Downloader, FORGE_PROFILE_ID,
    FORGE_RUNTIME_ID, ForgeRuntimeLock, JAVA_RUNTIME_INDEX_SHA1, JAVA_RUNTIME_INDEX_SIZE,
    JAVA_RUNTIME_INDEX_URL, JavaRuntimeIndex, MINECRAFT_VERSION, MINECRAFT_VERSION_JSON_SHA1,
    MINECRAFT_VERSION_JSON_URL, MinecraftLayout, MinecraftVersion, RuntimeFileKind,
    RuntimeManifest, VERSION_MANIFEST_URL, VersionManifest, library_is_allowed, native_classifier,
    safe_join, verify_file,
};
use fs2::FileExt;
use opus_platform::{Architecture, JavaProbe, Platform};
use rayon::prelude::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use thiserror::Error;
use url::Url;

const ASSET_OBJECT_BASE_URL: &str = "https://resources.download.minecraft.net";
const INSTALL_STATE_SCHEMA: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstallState {
    schema_version: u32,
    minecraft_version: String,
    runtime_id: String,
    profile_id: String,
    runtime_platform: String,
    java_component: String,
    java_version: String,
    runtime_manifest: DownloadSpec,
}

struct RuntimeInstallState {
    platform: String,
    version: String,
    manifest: DownloadSpec,
}

#[derive(Debug, Clone)]
pub struct NativeArchive {
    pub path: PathBuf,
    pub excludes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct InstalledMinecraft {
    pub version: MinecraftVersion,
    pub client_jar: PathBuf,
    pub classpath: Vec<PathBuf>,
    pub native_archives: Vec<NativeArchive>,
    pub logging_config: Option<PathBuf>,
    /// The effective launcher profile. The Mojang `version` remains the
    /// inherited 1.8.9 asset/Java contract while these fields select Forge.
    pub runtime_id: String,
    pub profile_id: String,
    pub main_class: String,
    pub minecraft_arguments: String,
    /// OptiFine is optional only while the installer is waiting for the user
    /// to import their locally obtained, verified JAR. Launching requires it.
    pub optifine_jar: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ManagedJava {
    pub version_name: String,
    pub java_home: PathBuf,
    pub executable: PathBuf,
}

#[derive(Debug, Clone)]
pub struct InstallReport {
    pub minecraft: InstalledMinecraft,
    pub java: ManagedJava,
    pub downloaded_files: usize,
    pub cached_files: usize,
}

/// A user-facing installation stage. These names intentionally describe work,
/// rather than implementation details or URLs, so launchers can present safe
/// progress feedback without exposing paths or credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallPhase {
    WaitingForLock,
    MinecraftMetadata,
    MinecraftLibraries,
    ForgeRuntime,
    MinecraftAssets,
    JavaMetadata,
    JavaRuntime,
    Finalizing,
    Complete,
}

impl InstallPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::WaitingForLock => "Waiting for the installer",
            Self::MinecraftMetadata => "Checking Minecraft 1.8.9 metadata",
            Self::MinecraftLibraries => "Verifying Minecraft libraries",
            Self::ForgeRuntime => "Installing Forge 1.8.9 runtime",
            Self::MinecraftAssets => "Downloading Minecraft assets",
            Self::JavaMetadata => "Checking Mojang Java 8 metadata",
            Self::JavaRuntime => "Installing Mojang Java 8",
            Self::Finalizing => "Finalizing verified installation",
            Self::Complete => "Forge 1.8.9 runtime is ready",
        }
    }
}

/// Verified-artifact progress for the current installation stage.
/// `completed_files` includes both cached and newly downloaded files; it is
/// not a byte count. The cached and downloaded counts are cumulative for the
/// whole installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallProgress {
    pub phase: InstallPhase,
    pub completed_files: usize,
    pub total_files: usize,
    pub downloaded_files: usize,
    pub cached_files: usize,
}

#[derive(Debug, Clone)]
pub struct Installer {
    layout: MinecraftLayout,
    platform: Platform,
    downloader: Downloader,
}

impl Installer {
    pub fn new(layout: MinecraftLayout, platform: Platform) -> Result<Self, InstallError> {
        Ok(Self {
            layout,
            platform,
            downloader: Downloader::new()?,
        })
    }

    pub fn install(&self) -> Result<InstallReport, InstallError> {
        self.install_with_progress(|_| {})
    }

    /// Install or repair Minecraft while reporting verified-artifact progress.
    /// The callback may be called from Rayon's worker threads, so consumers
    /// must be thread-safe and should avoid expensive work for every event.
    pub fn install_with_progress<F>(&self, progress: F) -> Result<InstallReport, InstallError>
    where
        F: Fn(InstallProgress) + Send + Sync,
    {
        let callback: &ProgressCallback<'_> = &progress;
        let reporter = ProgressReporter::new(callback);
        reporter.begin_phase(InstallPhase::WaitingForLock, 0);
        self.with_install_lock(|| self.install_locked(&reporter))
    }

    /// Verify and use a fully installed runtime without making a network request.
    pub fn load_cached(&self) -> Result<InstallReport, InstallError> {
        self.with_install_lock(|| self.load_cached_locked())
    }

    /// Prefer a verified offline cache, repairing or installing from Mojang only
    /// when the cache is absent or fails integrity validation.
    pub fn prepare(&self) -> Result<InstallReport, InstallError> {
        let progress = |_| {};
        let callback: &ProgressCallback<'_> = &progress;
        let reporter = ProgressReporter::new(callback);
        self.with_install_lock(|| match self.load_cached_locked() {
            Ok(report) => Ok(report),
            Err(cached) => {
                self.install_locked(&reporter)
                    .map_err(|install| InstallError::PrepareFailed {
                        cached: Box::new(cached),
                        install: Box::new(install),
                    })
            }
        })
    }

    fn with_install_lock<T>(
        &self,
        operation: impl FnOnce() -> Result<T, InstallError>,
    ) -> Result<T, InstallError> {
        self.layout.paths.create_all()?;
        let lock_path = self.layout.paths.root.join(".install.lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        FileExt::lock_exclusive(&lock).map_err(|source| InstallError::InstallLock {
            path: lock_path,
            source,
        })?;

        let result = operation();
        FileExt::unlock(&lock)?;
        result
    }

    fn install_locked(
        &self,
        reporter: &ProgressReporter<'_>,
    ) -> Result<InstallReport, InstallError> {
        let (minecraft, minecraft_counts) = self.install_minecraft(reporter)?;
        let java_component = minecraft
            .version
            .java_version
            .as_ref()
            .map(|java| java.component.as_str())
            .ok_or(InstallError::MissingJavaVersion)?;
        let (java, java_counts, runtime_state) = self.install_java(java_component, reporter)?;

        reporter.begin_phase(InstallPhase::Finalizing, 0);
        write_json_atomic(
            &self.layout.install_state(),
            &InstallState {
                schema_version: INSTALL_STATE_SCHEMA,
                minecraft_version: MINECRAFT_VERSION.to_owned(),
                runtime_id: minecraft.runtime_id.clone(),
                profile_id: minecraft.profile_id.clone(),
                runtime_platform: runtime_state.platform,
                java_component: java_component.to_owned(),
                java_version: runtime_state.version,
                runtime_manifest: runtime_state.manifest,
            },
        )?;

        let report = InstallReport {
            minecraft,
            java,
            downloaded_files: minecraft_counts.downloaded + java_counts.downloaded,
            cached_files: minecraft_counts.cached + java_counts.cached,
        };
        reporter.begin_phase(InstallPhase::Complete, 0);
        Ok(report)
    }

    fn load_cached_locked(&self) -> Result<InstallReport, InstallError> {
        let state_path = self.layout.install_state();
        if !state_path.is_file() {
            return Err(InstallError::CachedStateMissing(state_path));
        }
        let state: InstallState = read_json(&state_path)?;
        self.validate_install_state(&state)?;

        let (minecraft, minecraft_counts) = self.load_cached_minecraft()?;
        let java_component = minecraft
            .version
            .java_version
            .as_ref()
            .map(|java| java.component.as_str())
            .ok_or(InstallError::MissingJavaVersion)?;
        if java_component != state.java_component {
            return Err(InstallError::CachedStateInvalid(format!(
                "Java component is {}, metadata requires {java_component}",
                state.java_component
            )));
        }
        let (java, java_counts) = self.load_cached_java(java_component, &state)?;

        Ok(InstallReport {
            minecraft,
            java,
            downloaded_files: 0,
            cached_files: minecraft_counts.cached + java_counts.cached,
        })
    }

    fn install_minecraft(
        &self,
        reporter: &ProgressReporter<'_>,
    ) -> Result<(InstalledMinecraft, InstallCounts), InstallError> {
        let (base, counts) = self.install_base_minecraft(reporter)?;
        self.install_forge_profile(base, counts, reporter)
    }

    fn install_base_minecraft(
        &self,
        reporter: &ProgressReporter<'_>,
    ) -> Result<(InstalledMinecraft, InstallCounts), InstallError> {
        reporter.begin_phase(InstallPhase::MinecraftMetadata, 0);
        let manifest: VersionManifest = self.downloader.get_json(VERSION_MANIFEST_URL)?;
        let summary = manifest
            .versions
            .iter()
            .find(|candidate| candidate.id == MINECRAFT_VERSION)
            .ok_or_else(|| InstallError::VersionNotFound(MINECRAFT_VERSION.to_owned()))?;
        if summary.url != MINECRAFT_VERSION_JSON_URL
            || !summary
                .sha1
                .eq_ignore_ascii_case(MINECRAFT_VERSION_JSON_SHA1)
        {
            return Err(InstallError::VersionContractChanged {
                url: summary.url.clone(),
                sha1: summary.sha1.clone(),
            });
        }

        let mut counts = InstallCounts::default();
        reporter.add_total(1);
        let version_metadata = self.downloader.ensure(
            &DownloadSpec {
                url: MINECRAFT_VERSION_JSON_URL.to_owned(),
                sha1: MINECRAFT_VERSION_JSON_SHA1.to_owned(),
                size: None,
            },
            &self.layout.version_json(),
        )?;
        counts.record(version_metadata);
        reporter.record(InstallPhase::MinecraftMetadata, version_metadata);

        let version: MinecraftVersion = read_json(&self.layout.version_json())?;
        validate_version(&version)?;

        reporter.begin_phase(
            InstallPhase::MinecraftLibraries,
            planned_minecraft_non_asset_files(&version, self.platform),
        );

        let client_outcome = self.downloader.ensure(
            &artifact_spec(&version.downloads.client),
            &self.layout.client_jar(),
        )?;
        counts.record(client_outcome);
        reporter.record(InstallPhase::MinecraftLibraries, client_outcome);

        let context = crate::RuleContext {
            platform: self.platform,
            os_version: None,
        };
        let mut classpath = Vec::new();
        let mut native_archives = Vec::new();

        for library in &version.libraries {
            if !library_is_allowed(library, &context) {
                continue;
            }

            if let Some(artifact) = &library.downloads.artifact {
                let relative_path = artifact
                    .path
                    .as_deref()
                    .ok_or_else(|| InstallError::MissingArtifactPath(library.name.clone()))?;
                let destination = self.layout.library_file(relative_path)?;
                let outcome = self
                    .downloader
                    .ensure(&artifact_spec(artifact), &destination)?;
                counts.record(outcome);
                reporter.record(InstallPhase::MinecraftLibraries, outcome);
                classpath.push(destination);
            }

            if let Some(classifier) = native_classifier(library, self.platform) {
                let artifact = library
                    .downloads
                    .classifiers
                    .get(&classifier)
                    .ok_or_else(|| InstallError::MissingNativeClassifier {
                        library: library.name.clone(),
                        classifier: classifier.clone(),
                    })?;
                let relative_path = artifact.path.as_deref().ok_or_else(|| {
                    InstallError::MissingArtifactPath(format!("{}:{classifier}", library.name))
                })?;
                let destination = self.layout.library_file(relative_path)?;
                let outcome = self
                    .downloader
                    .ensure(&artifact_spec(artifact), &destination)?;
                counts.record(outcome);
                reporter.record(InstallPhase::MinecraftLibraries, outcome);
                native_archives.push(NativeArchive {
                    path: destination,
                    excludes: library
                        .extract
                        .as_ref()
                        .map(|extract| extract.exclude.clone())
                        .unwrap_or_default(),
                });
            }
        }

        let logging_config = if let Some(logging) = &version.logging {
            let path = self.layout.logging_file(&logging.client.file.id)?;
            let outcome = self.downloader.ensure(
                &DownloadSpec {
                    url: logging.client.file.url.clone(),
                    sha1: logging.client.file.sha1.clone(),
                    size: Some(logging.client.file.size),
                },
                &path,
            )?;
            counts.record(outcome);
            reporter.record(InstallPhase::MinecraftLibraries, outcome);
            Some(path)
        } else {
            None
        };

        let index_path = self.layout.asset_index(&version.asset_index.id)?;
        let asset_index_outcome = self.downloader.ensure(
            &DownloadSpec {
                url: version.asset_index.url.clone(),
                sha1: version.asset_index.sha1.clone(),
                size: Some(version.asset_index.size),
            },
            &index_path,
        )?;
        counts.record(asset_index_outcome);
        reporter.record(InstallPhase::MinecraftLibraries, asset_index_outcome);
        let asset_index: AssetIndex = read_json(&index_path)?;

        reporter.begin_phase(InstallPhase::MinecraftAssets, asset_index.objects.len());

        let asset_results: Result<Vec<_>, InstallError> = asset_index
            .objects
            .par_iter()
            .map(|(_, object)| {
                let destination = self.layout.asset_object(&object.hash)?;
                let url = format!(
                    "{ASSET_OBJECT_BASE_URL}/{}/{}",
                    &object.hash[..2],
                    object.hash
                );
                let outcome = self.downloader.ensure(
                    &DownloadSpec {
                        url,
                        sha1: object.hash.clone(),
                        size: Some(object.size),
                    },
                    &destination,
                )?;
                reporter.record(InstallPhase::MinecraftAssets, outcome);
                Ok(outcome)
            })
            .collect();
        for outcome in asset_results? {
            counts.record(outcome);
        }

        classpath.push(self.layout.client_jar());
        Ok((
            InstalledMinecraft {
                runtime_id: "vanilla-1.8.9".to_owned(),
                profile_id: MINECRAFT_VERSION.to_owned(),
                main_class: version.main_class.clone(),
                minecraft_arguments: version.minecraft_arguments.clone(),
                version,
                client_jar: self.layout.client_jar(),
                classpath,
                native_archives,
                logging_config,
                optifine_jar: None,
            },
            counts,
        ))
    }

    fn load_cached_minecraft(&self) -> Result<(InstalledMinecraft, InstallCounts), InstallError> {
        let (base, counts) = self.load_cached_base_minecraft()?;
        self.load_cached_forge_profile(base, counts)
    }

    fn load_cached_base_minecraft(
        &self,
    ) -> Result<(InstalledMinecraft, InstallCounts), InstallError> {
        let mut counts = InstallCounts::default();
        counts.record(require_cached(
            &DownloadSpec {
                url: MINECRAFT_VERSION_JSON_URL.to_owned(),
                sha1: MINECRAFT_VERSION_JSON_SHA1.to_owned(),
                size: None,
            },
            &self.layout.version_json(),
        )?);

        let version: MinecraftVersion = read_json(&self.layout.version_json())?;
        validate_version(&version)?;
        counts.record(require_cached(
            &artifact_spec(&version.downloads.client),
            &self.layout.client_jar(),
        )?);

        let context = crate::RuleContext {
            platform: self.platform,
            os_version: None,
        };
        let mut classpath = Vec::new();
        let mut native_archives = Vec::new();

        for library in &version.libraries {
            if !library_is_allowed(library, &context) {
                continue;
            }

            if let Some(artifact) = &library.downloads.artifact {
                let relative_path = artifact
                    .path
                    .as_deref()
                    .ok_or_else(|| InstallError::MissingArtifactPath(library.name.clone()))?;
                let destination = self.layout.library_file(relative_path)?;
                counts.record(require_cached(&artifact_spec(artifact), &destination)?);
                classpath.push(destination);
            }

            if let Some(classifier) = native_classifier(library, self.platform) {
                let artifact = library
                    .downloads
                    .classifiers
                    .get(&classifier)
                    .ok_or_else(|| InstallError::MissingNativeClassifier {
                        library: library.name.clone(),
                        classifier: classifier.clone(),
                    })?;
                let relative_path = artifact.path.as_deref().ok_or_else(|| {
                    InstallError::MissingArtifactPath(format!("{}:{classifier}", library.name))
                })?;
                let destination = self.layout.library_file(relative_path)?;
                counts.record(require_cached(&artifact_spec(artifact), &destination)?);
                native_archives.push(NativeArchive {
                    path: destination,
                    excludes: library
                        .extract
                        .as_ref()
                        .map(|extract| extract.exclude.clone())
                        .unwrap_or_default(),
                });
            }
        }

        let logging_config = if let Some(logging) = &version.logging {
            let path = self.layout.logging_file(&logging.client.file.id)?;
            counts.record(require_cached(
                &DownloadSpec {
                    url: logging.client.file.url.clone(),
                    sha1: logging.client.file.sha1.clone(),
                    size: Some(logging.client.file.size),
                },
                &path,
            )?);
            Some(path)
        } else {
            None
        };

        let index_path = self.layout.asset_index(&version.asset_index.id)?;
        counts.record(require_cached(
            &DownloadSpec {
                url: version.asset_index.url.clone(),
                sha1: version.asset_index.sha1.clone(),
                size: Some(version.asset_index.size),
            },
            &index_path,
        )?);
        let asset_index: AssetIndex = read_json(&index_path)?;
        let asset_results: Result<Vec<_>, InstallError> = asset_index
            .objects
            .par_iter()
            .map(|(_, object)| {
                let destination = self.layout.asset_object(&object.hash)?;
                require_cached(
                    &DownloadSpec {
                        url: format!(
                            "{ASSET_OBJECT_BASE_URL}/{}/{}",
                            &object.hash[..2],
                            object.hash
                        ),
                        sha1: object.hash.clone(),
                        size: Some(object.size),
                    },
                    &destination,
                )
            })
            .collect();
        for outcome in asset_results? {
            counts.record(outcome);
        }

        classpath.push(self.layout.client_jar());
        Ok((
            InstalledMinecraft {
                runtime_id: "vanilla-1.8.9".to_owned(),
                profile_id: MINECRAFT_VERSION.to_owned(),
                main_class: version.main_class.clone(),
                minecraft_arguments: version.minecraft_arguments.clone(),
                version,
                client_jar: self.layout.client_jar(),
                classpath,
                native_archives,
                logging_config,
                optifine_jar: None,
            },
            counts,
        ))
    }

    fn install_forge_profile(
        &self,
        base: InstalledMinecraft,
        mut counts: InstallCounts,
        reporter: &ProgressReporter<'_>,
    ) -> Result<(InstalledMinecraft, InstallCounts), InstallError> {
        let lock = ForgeRuntimeLock::load()?;
        reporter.begin_phase(InstallPhase::ForgeRuntime, lock.libraries.len());

        let mut forge_classpath = Vec::with_capacity(lock.libraries.len());
        for library in &lock.libraries {
            let destination = self.layout.library_file(&library.relative_path)?;
            let outcome = self
                .downloader
                .ensure(&forge_library_spec(library), &destination)?;
            counts.record(outcome);
            reporter.record(InstallPhase::ForgeRuntime, outcome);
            push_unique(&mut forge_classpath, destination);
        }

        let optifine_jar = self.valid_optifine_jar(&lock)?;
        Ok((
            apply_forge_profile(base, &lock, forge_classpath, optifine_jar),
            counts,
        ))
    }

    fn load_cached_forge_profile(
        &self,
        base: InstalledMinecraft,
        mut counts: InstallCounts,
    ) -> Result<(InstalledMinecraft, InstallCounts), InstallError> {
        let lock = ForgeRuntimeLock::load()?;
        let mut forge_classpath = Vec::with_capacity(lock.libraries.len());
        for library in &lock.libraries {
            let destination = self.layout.library_file(&library.relative_path)?;
            counts.record(require_cached(&forge_library_spec(library), &destination)?);
            push_unique(&mut forge_classpath, destination);
        }

        let optifine_jar = self.valid_optifine_jar(&lock)?;
        Ok((
            apply_forge_profile(base, &lock, forge_classpath, optifine_jar),
            counts,
        ))
    }

    fn valid_optifine_jar(&self, lock: &ForgeRuntimeLock) -> Result<Option<PathBuf>, InstallError> {
        let path = self.layout.optifine_mod();
        if !path.exists() {
            return Ok(None);
        }
        if !path.is_file() || !verify_file(&path, &optifine_spec(lock))? {
            return Err(InstallError::InvalidOptiFine(path));
        }
        Ok(Some(path))
    }

    /// Import a locally user-provided OptiFine JAR after verifying it against
    /// the checked-in contract. Opus never downloads, bundles, or removes the
    /// original file.
    pub fn import_optifine(&self, source: &Path) -> Result<PathBuf, InstallError> {
        self.with_install_lock(|| self.import_optifine_locked(source))
    }

    fn import_optifine_locked(&self, source: &Path) -> Result<PathBuf, InstallError> {
        let lock = ForgeRuntimeLock::load()?;
        if !source.is_file() || !verify_file(source, &optifine_spec(&lock))? {
            return Err(InstallError::InvalidOptiFineSource(source.to_path_buf()));
        }

        let destination = self.layout.optifine_mod();
        if destination.is_file() && verify_file(&destination, &optifine_spec(&lock))? {
            return Ok(destination);
        }
        let parent = destination
            .parent()
            .ok_or_else(|| InstallError::MissingParent(destination.clone()))?;
        fs::create_dir_all(parent)?;
        let temporary = destination.with_file_name(format!(
            ".{}.part-{}",
            lock.optifine.file_name,
            std::process::id()
        ));
        if temporary.exists() {
            fs::remove_file(&temporary)?;
        }
        fs::copy(source, &temporary)?;
        if !verify_file(&temporary, &optifine_spec(&lock))? {
            let _ = fs::remove_file(&temporary);
            return Err(InstallError::InvalidOptiFineSource(source.to_path_buf()));
        }
        fs::rename(&temporary, &destination)?;
        Ok(destination)
    }

    fn validate_install_state(&self, state: &InstallState) -> Result<(), InstallError> {
        if state.schema_version != INSTALL_STATE_SCHEMA {
            return Err(InstallError::CachedStateInvalid(format!(
                "unsupported schema {}",
                state.schema_version
            )));
        }
        if state.minecraft_version != MINECRAFT_VERSION {
            return Err(InstallError::CachedStateInvalid(format!(
                "unexpected Minecraft version {}",
                state.minecraft_version
            )));
        }
        if state.runtime_id != FORGE_RUNTIME_ID || state.profile_id != FORGE_PROFILE_ID {
            return Err(InstallError::CachedStateInvalid(
                "runtime is not the pinned Forge + OptiFine profile".to_owned(),
            ));
        }
        let expected_platform = self.platform.minecraft_runtime_key()?;
        if state.runtime_platform != expected_platform {
            return Err(InstallError::CachedStateInvalid(format!(
                "runtime platform is {}, expected {expected_platform}",
                state.runtime_platform
            )));
        }
        if state.java_component != "jre-legacy" {
            return Err(InstallError::CachedStateInvalid(format!(
                "unexpected Java component {}",
                state.java_component
            )));
        }
        if state.java_version.is_empty() || state.java_version.len() > 128 {
            return Err(InstallError::CachedStateInvalid(
                "Java version name is invalid".to_owned(),
            ));
        }
        safe_join(Path::new("runtime-version"), &state.java_version)
            .map_err(|error| InstallError::CachedStateInvalid(error.to_string()))?;

        let url = Url::parse(&state.runtime_manifest.url).map_err(|_| {
            InstallError::CachedStateInvalid("runtime manifest URL is invalid".into())
        })?;
        if url.scheme() != "https" || url.host_str() != Some("piston-meta.mojang.com") {
            return Err(InstallError::CachedStateInvalid(
                "runtime manifest URL is not an approved Mojang HTTPS URL".to_owned(),
            ));
        }
        if state.runtime_manifest.sha1.len() != 40
            || !state
                .runtime_manifest
                .sha1
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || !matches!(state.runtime_manifest.size, Some(1..=16_777_216))
        {
            return Err(InstallError::CachedStateInvalid(
                "runtime manifest integrity metadata is invalid".to_owned(),
            ));
        }
        Ok(())
    }

    fn load_cached_java(
        &self,
        component: &str,
        state: &InstallState,
    ) -> Result<(ManagedJava, InstallCounts), InstallError> {
        let mut counts = InstallCounts::default();
        counts.record(require_cached(
            &java_runtime_index_spec(),
            &self.layout.java_runtime_index(),
        )?);
        let index: JavaRuntimeIndex = read_json(&self.layout.java_runtime_index())?;
        let expected_entry = index
            .get(&state.runtime_platform)
            .and_then(|components| components.get(component))
            .and_then(|entries| entries.last())
            .ok_or_else(|| InstallError::JavaRuntimeNotFound {
                platform: state.runtime_platform.clone(),
                component: component.to_owned(),
            })?;
        let state_matches_index = expected_entry.version.name == state.java_version
            && expected_entry.manifest.url == state.runtime_manifest.url
            && expected_entry
                .manifest
                .sha1
                .eq_ignore_ascii_case(&state.runtime_manifest.sha1)
            && state.runtime_manifest.size == Some(expected_entry.manifest.size);
        if !state_matches_index {
            return Err(InstallError::CachedStateInvalid(
                "Java runtime selection does not match the pinned Mojang index".to_owned(),
            ));
        }

        let manifest_path = safe_join(
            &self.layout.paths.runtime.join("manifests"),
            &format!(
                "{}/{component}/{}.json",
                state.runtime_platform, state.java_version
            ),
        )?;
        counts.record(require_cached(&state.runtime_manifest, &manifest_path)?);
        let manifest: RuntimeManifest = read_json(&manifest_path)?;

        let runtime_root = safe_join(
            &self.layout.paths.runtime.join("java"),
            &format!(
                "{}/{component}/{}",
                state.runtime_platform, state.java_version
            ),
        )?;
        for (relative, entry) in &manifest.files {
            let destination = safe_join(&runtime_root, relative)?;
            match entry.kind {
                RuntimeFileKind::File => {
                    let artifact = entry
                        .downloads
                        .get("raw")
                        .ok_or_else(|| InstallError::MissingRawRuntimeDownload(relative.clone()))?;
                    counts.record(require_cached(&artifact_spec(artifact), &destination)?);
                    if entry.executable {
                        make_executable(&destination)?;
                    }
                }
                RuntimeFileKind::Directory => {
                    if !destination.is_dir() {
                        return Err(InstallError::CachedArtifactInvalid(destination));
                    }
                }
                RuntimeFileKind::Link => {
                    return Err(InstallError::UnsupportedRuntimeLink {
                        path: relative.clone(),
                        target: entry.target.clone().unwrap_or_default(),
                    });
                }
            }
        }

        let java_home = match self.platform.os {
            opus_platform::OperatingSystem::MacOs => runtime_root.join("jre.bundle/Contents/Home"),
            opus_platform::OperatingSystem::Windows | opus_platform::OperatingSystem::Linux => {
                runtime_root
            }
        };
        let probe = JavaProbe::probe(&java_home, self.platform.os)?;
        if probe.major_version != 8 || probe.architecture != self.platform.game_arch {
            return Err(InstallError::IncompatibleManagedJava {
                version: probe.raw_version,
                architecture: probe.architecture,
                expected_architecture: self.platform.game_arch,
            });
        }

        Ok((
            ManagedJava {
                version_name: state.java_version.clone(),
                java_home,
                executable: probe.executable,
            },
            counts,
        ))
    }

    fn install_java(
        &self,
        component: &str,
        reporter: &ProgressReporter<'_>,
    ) -> Result<(ManagedJava, InstallCounts, RuntimeInstallState), InstallError> {
        let mut counts = InstallCounts::default();
        reporter.begin_phase(InstallPhase::JavaMetadata, 2);
        let runtime_index_outcome = self.downloader.ensure(
            &java_runtime_index_spec(),
            &self.layout.java_runtime_index(),
        )?;
        counts.record(runtime_index_outcome);
        reporter.record(InstallPhase::JavaMetadata, runtime_index_outcome);
        let index: JavaRuntimeIndex = read_json(&self.layout.java_runtime_index())?;
        let platform_key = self.platform.minecraft_runtime_key()?;
        let entry = index
            .get(platform_key)
            .and_then(|components| components.get(component))
            .and_then(|entries| entries.last())
            .ok_or_else(|| InstallError::JavaRuntimeNotFound {
                platform: platform_key.to_owned(),
                component: component.to_owned(),
            })?;

        let manifest_path = safe_join(
            &self.layout.paths.runtime.join("manifests"),
            &format!("{platform_key}/{component}/{}.json", entry.version.name),
        )?;
        let runtime_manifest_outcome = self.downloader.ensure(
            &DownloadSpec {
                url: entry.manifest.url.clone(),
                sha1: entry.manifest.sha1.clone(),
                size: Some(entry.manifest.size),
            },
            &manifest_path,
        )?;
        counts.record(runtime_manifest_outcome);
        reporter.record(InstallPhase::JavaMetadata, runtime_manifest_outcome);
        let manifest: RuntimeManifest = read_json(&manifest_path)?;

        let runtime_root = safe_join(
            &self.layout.paths.runtime.join("java"),
            &format!("{platform_key}/{component}/{}", entry.version.name),
        )?;
        fs::create_dir_all(&runtime_root)?;

        let file_entries: Vec<_> = manifest
            .files
            .iter()
            .filter(|(_, entry)| entry.kind == RuntimeFileKind::File)
            .collect();
        reporter.begin_phase(InstallPhase::JavaRuntime, file_entries.len());
        let results: Result<Vec<_>, InstallError> = file_entries
            .par_iter()
            .map(|(relative, entry)| {
                let destination = safe_join(&runtime_root, relative)?;
                let artifact = entry
                    .downloads
                    .get("raw")
                    .ok_or_else(|| InstallError::MissingRawRuntimeDownload((*relative).clone()))?;
                let outcome = self
                    .downloader
                    .ensure(&artifact_spec(artifact), &destination)?;
                if entry.executable {
                    make_executable(&destination)?;
                }
                reporter.record(InstallPhase::JavaRuntime, outcome);
                Ok(outcome)
            })
            .collect();
        for outcome in results? {
            counts.record(outcome);
        }

        for (relative, entry) in &manifest.files {
            match entry.kind {
                RuntimeFileKind::Directory => {
                    fs::create_dir_all(safe_join(&runtime_root, relative)?)?
                }
                RuntimeFileKind::File => {}
                RuntimeFileKind::Link => {
                    return Err(InstallError::UnsupportedRuntimeLink {
                        path: relative.clone(),
                        target: entry.target.clone().unwrap_or_default(),
                    });
                }
            }
        }

        let java_home = match self.platform.os {
            opus_platform::OperatingSystem::MacOs => runtime_root.join("jre.bundle/Contents/Home"),
            opus_platform::OperatingSystem::Windows | opus_platform::OperatingSystem::Linux => {
                runtime_root.clone()
            }
        };
        let probe = JavaProbe::probe(&java_home, self.platform.os)?;
        if probe.major_version != 8 || probe.architecture != self.platform.game_arch {
            return Err(InstallError::IncompatibleManagedJava {
                version: probe.raw_version,
                architecture: probe.architecture,
                expected_architecture: self.platform.game_arch,
            });
        }

        Ok((
            ManagedJava {
                version_name: entry.version.name.clone(),
                java_home,
                executable: probe.executable,
            },
            counts,
            RuntimeInstallState {
                platform: platform_key.to_owned(),
                version: entry.version.name.clone(),
                manifest: DownloadSpec {
                    url: entry.manifest.url.clone(),
                    sha1: entry.manifest.sha1.clone(),
                    size: Some(entry.manifest.size),
                },
            },
        ))
    }
}

type ProgressCallback<'a> = dyn Fn(InstallProgress) + Send + Sync + 'a;

#[derive(Debug)]
struct ProgressState {
    phase: InstallPhase,
    completed_files: usize,
    total_files: usize,
    downloaded_files: usize,
    cached_files: usize,
}

impl ProgressState {
    fn snapshot(&self) -> InstallProgress {
        InstallProgress {
            phase: self.phase,
            completed_files: self.completed_files,
            total_files: self.total_files,
            downloaded_files: self.downloaded_files,
            cached_files: self.cached_files,
        }
    }
}

/// Serializes progress callbacks originating from the parallel artifact
/// workers. This keeps frontend updates monotonic inside a stage without
/// exposing downloader URLs, paths, or credentials.
struct ProgressReporter<'a> {
    callback: &'a ProgressCallback<'a>,
    state: Mutex<ProgressState>,
}

impl<'a> ProgressReporter<'a> {
    fn new(callback: &'a ProgressCallback<'a>) -> Self {
        Self {
            callback,
            state: Mutex::new(ProgressState {
                phase: InstallPhase::WaitingForLock,
                completed_files: 0,
                total_files: 0,
                downloaded_files: 0,
                cached_files: 0,
            }),
        }
    }

    fn begin_phase(&self, phase: InstallPhase, total_files: usize) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.phase = phase;
        state.completed_files = 0;
        state.total_files = total_files;
        (self.callback)(state.snapshot());
    }

    fn add_total(&self, files: usize) {
        if files == 0 {
            return;
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.total_files += files;
        (self.callback)(state.snapshot());
    }

    fn record(&self, phase: InstallPhase, outcome: DownloadOutcome) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        debug_assert_eq!(state.phase, phase);
        if state.phase != phase {
            return;
        }
        state.completed_files += 1;
        match outcome {
            DownloadOutcome::AlreadyPresent => state.cached_files += 1,
            DownloadOutcome::Downloaded => state.downloaded_files += 1,
        }

        // The first artifact, every fourth one, and the final artifact make
        // the dialog feel live without flooding its event queue for assets.
        if state.completed_files == 1
            || state.completed_files == state.total_files
            || state.completed_files.is_multiple_of(4)
        {
            (self.callback)(state.snapshot());
        }
    }
}

fn planned_minecraft_non_asset_files(version: &MinecraftVersion, platform: Platform) -> usize {
    let context = crate::RuleContext {
        platform,
        os_version: None,
    };
    // Client JAR and asset index are always present. Version metadata is its
    // own stage because the public version manifest must be fetched first.
    let mut total = 2;
    for library in &version.libraries {
        if !library_is_allowed(library, &context) {
            continue;
        }
        total += usize::from(library.downloads.artifact.is_some());
        total += usize::from(native_classifier(library, platform).is_some());
    }
    total + usize::from(version.logging.is_some())
}

fn artifact_spec(artifact: &Artifact) -> DownloadSpec {
    DownloadSpec {
        url: artifact.url.clone(),
        sha1: artifact.sha1.clone(),
        size: Some(artifact.size),
    }
}

fn forge_library_spec(library: &crate::ForgeLibrary) -> DownloadSpec {
    DownloadSpec {
        url: library.url.clone(),
        sha1: library.sha1.clone(),
        size: Some(library.size),
    }
}

fn optifine_spec(lock: &ForgeRuntimeLock) -> DownloadSpec {
    // This URL is metadata only for local integrity verification. OptiFine is
    // never fetched through the downloader.
    DownloadSpec {
        url: "https://optifine.net/downloads".to_owned(),
        sha1: lock.optifine.sha1.clone(),
        size: Some(lock.optifine.size),
    }
}

fn apply_forge_profile(
    mut base: InstalledMinecraft,
    lock: &ForgeRuntimeLock,
    forge_classpath: Vec<PathBuf>,
    optifine_jar: Option<PathBuf>,
) -> InstalledMinecraft {
    let mut classpath = Vec::with_capacity(forge_classpath.len() + base.classpath.len());
    for path in forge_classpath {
        push_unique(&mut classpath, path);
    }
    for path in std::mem::take(&mut base.classpath) {
        push_unique(&mut classpath, path);
    }
    base.classpath = classpath;
    base.runtime_id = lock.runtime_id.clone();
    base.profile_id = lock.profile_id.clone();
    base.main_class = lock.main_class.clone();
    base.minecraft_arguments = lock.minecraft_arguments.clone();
    base.optifine_jar = optifine_jar;
    base
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn java_runtime_index_spec() -> DownloadSpec {
    DownloadSpec {
        url: JAVA_RUNTIME_INDEX_URL.to_owned(),
        sha1: JAVA_RUNTIME_INDEX_SHA1.to_owned(),
        size: Some(JAVA_RUNTIME_INDEX_SIZE),
    }
}

fn validate_version(version: &MinecraftVersion) -> Result<(), InstallError> {
    if version.id != MINECRAFT_VERSION {
        return Err(InstallError::UnexpectedVersion(version.id.clone()));
    }
    if version.main_class != "net.minecraft.client.main.Main" {
        return Err(InstallError::UnexpectedMainClass(
            version.main_class.clone(),
        ));
    }
    match &version.java_version {
        Some(java) if java.major_version == 8 && java.component == "jre-legacy" => {}
        Some(java) => {
            return Err(InstallError::UnexpectedJavaContract {
                component: java.component.clone(),
                major: java.major_version,
            });
        }
        None => return Err(InstallError::MissingJavaVersion),
    }
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, InstallError> {
    let file = File::open(path)?;
    Ok(serde_json::from_reader(BufReader::new(file))?)
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), InstallError> {
    let parent = path
        .parent()
        .ok_or_else(|| InstallError::CachedStateInvalid("state path has no parent".to_owned()))?;
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("install-state-v1.json");
    let part_path = path.with_file_name(format!(".{file_name}.part-{}", std::process::id()));
    if part_path.exists() {
        fs::remove_file(&part_path)?;
    }

    let result = (|| -> Result<(), InstallError> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&part_path)?;
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.flush()?;
        file.sync_all()?;

        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(path)?;
        }
        fs::rename(&part_path, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&part_path);
    }
    result
}

fn require_cached(spec: &DownloadSpec, path: &Path) -> Result<DownloadOutcome, InstallError> {
    if verify_file(path, spec)? {
        Ok(DownloadOutcome::AlreadyPresent)
    } else {
        Err(InstallError::CachedArtifactInvalid(path.to_path_buf()))
    }
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

#[derive(Debug, Default, Clone, Copy)]
struct InstallCounts {
    downloaded: usize,
    cached: usize,
}

impl InstallCounts {
    fn record(&mut self, outcome: DownloadOutcome) {
        match outcome {
            DownloadOutcome::AlreadyPresent => self.cached += 1,
            DownloadOutcome::Downloaded => self.downloaded += 1,
        }
    }
}

#[derive(Debug, Error)]
pub enum InstallError {
    #[error(transparent)]
    Download(#[from] DownloadError),
    #[error(transparent)]
    ForgeLock(#[from] crate::ForgeLockError),
    #[error(transparent)]
    Platform(#[from] opus_platform::PlatformError),
    #[error(transparent)]
    UnsafePath(#[from] UnsafeRelativePath),
    #[error("filesystem operation failed")]
    Io(#[from] io::Error),
    #[error("JSON metadata is invalid")]
    Json(#[from] serde_json::Error),
    #[error("failed to acquire install lock {path}", path = .path.display())]
    InstallLock {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Minecraft version {0} was not found in the official manifest")]
    VersionNotFound(String),
    #[error("official Minecraft 1.8.9 metadata contract changed (URL {url}, SHA-1 {sha1})")]
    VersionContractChanged { url: String, sha1: String },
    #[error("unexpected Minecraft version in verified metadata: {0}")]
    UnexpectedVersion(String),
    #[error("unexpected Minecraft main class: {0}")]
    UnexpectedMainClass(String),
    #[error("Minecraft metadata does not declare a Java runtime")]
    MissingJavaVersion,
    #[error("unexpected Java contract: component {component}, major {major}")]
    UnexpectedJavaContract { component: String, major: u32 },
    #[error("library {0} has no artifact path")]
    MissingArtifactPath(String),
    #[error("filesystem destination has no parent: {}", .0.display())]
    MissingParent(PathBuf),
    #[error("the managed OptiFine JAR is missing or does not match the approved local import: {}", .0.display())]
    InvalidOptiFine(PathBuf),
    #[error("the selected OptiFine JAR does not match the approved local import: {}", .0.display())]
    InvalidOptiFineSource(PathBuf),
    #[error("library {library} does not publish required classifier {classifier}")]
    MissingNativeClassifier { library: String, classifier: String },
    #[error("no Mojang Java runtime for {platform}/{component}")]
    JavaRuntimeNotFound { platform: String, component: String },
    #[error("runtime entry {0} has no raw download")]
    MissingRawRuntimeDownload(String),
    #[error("runtime symbolic link is not supported yet: {path} -> {target}")]
    UnsupportedRuntimeLink { path: String, target: String },
    #[error(
        "managed Java is incompatible: got Java {version} {architecture}, expected Java 8 {expected_architecture}"
    )]
    IncompatibleManagedJava {
        version: String,
        architecture: Architecture,
        expected_architecture: Architecture,
    },
    #[error("cached installation state is missing at {}", .0.display())]
    CachedStateMissing(PathBuf),
    #[error("cached installation state is invalid: {0}")]
    CachedStateInvalid(String),
    #[error("cached artifact is missing or failed integrity verification: {}", .0.display())]
    CachedArtifactInvalid(PathBuf),
    #[error("cached installation was unusable ({cached}); online repair also failed ({install})")]
    PrepareFailed {
        cached: Box<InstallError>,
        install: Box<InstallError>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use opus_platform::{Architecture, OperatingSystem, OpusPaths};
    use std::sync::Mutex;

    #[test]
    fn progress_is_stage_local_and_keeps_cumulative_source_counts() {
        let events = Mutex::new(Vec::new());
        let callback = |progress| events.lock().unwrap().push(progress);
        let reporter = ProgressReporter::new(&callback);

        reporter.begin_phase(InstallPhase::MinecraftAssets, 5);
        reporter.record(
            InstallPhase::MinecraftAssets,
            DownloadOutcome::AlreadyPresent,
        );
        reporter.record(InstallPhase::MinecraftAssets, DownloadOutcome::Downloaded);
        reporter.record(
            InstallPhase::MinecraftAssets,
            DownloadOutcome::AlreadyPresent,
        );
        reporter.record(InstallPhase::MinecraftAssets, DownloadOutcome::Downloaded);
        reporter.record(
            InstallPhase::MinecraftAssets,
            DownloadOutcome::AlreadyPresent,
        );
        reporter.begin_phase(InstallPhase::JavaRuntime, 2);
        reporter.record(InstallPhase::JavaRuntime, DownloadOutcome::Downloaded);
        reporter.record(InstallPhase::JavaRuntime, DownloadOutcome::AlreadyPresent);

        let events = events.into_inner().unwrap();
        let asset_progress = events
            .iter()
            .filter(|event| event.phase == InstallPhase::MinecraftAssets)
            .map(|event| event.completed_files)
            .collect::<Vec<_>>();
        let java_progress = events
            .iter()
            .filter(|event| event.phase == InstallPhase::JavaRuntime)
            .map(|event| event.completed_files)
            .collect::<Vec<_>>();

        assert_eq!(asset_progress, vec![0, 1, 4, 5]);
        assert_eq!(java_progress, vec![0, 1, 2]);
        let completed = events.last().unwrap();
        assert_eq!(completed.cached_files, 4);
        assert_eq!(completed.downloaded_files, 3);
    }

    #[test]
    fn cached_artifact_verification_rejects_corruption() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("artifact");
        fs::write(&path, b"corrupt").unwrap();
        let spec = DownloadSpec {
            url: "https://example.invalid/artifact".to_owned(),
            sha1: "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d".to_owned(),
            size: Some(5),
        };
        assert!(matches!(
            require_cached(&spec, &path),
            Err(InstallError::CachedArtifactInvalid(invalid)) if invalid == path
        ));
    }

    #[test]
    fn install_state_is_written_as_complete_json() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("install-state-v1.json");
        let state = valid_test_state();
        write_json_atomic(&path, &state).unwrap();

        let loaded: InstallState = read_json(&path).unwrap();
        assert_eq!(loaded.schema_version, INSTALL_STATE_SCHEMA);
        assert_eq!(loaded.java_version, state.java_version);
        assert!(
            !temp
                .path()
                .join(format!(
                    ".install-state-v1.json.part-{}",
                    std::process::id()
                ))
                .exists()
        );
    }

    #[test]
    fn install_state_rejects_wrong_platform_and_unapproved_url() {
        let temp = tempfile::tempdir().unwrap();
        let platform = Platform {
            os: OperatingSystem::MacOs,
            host_arch: Architecture::Aarch64,
            game_arch: Architecture::X86_64,
        };
        let paths = OpusPaths::from_root(temp.path().join("opus")).unwrap();
        let installer = Installer::new(MinecraftLayout::new(paths), platform).unwrap();

        let mut state = valid_test_state();
        assert!(installer.validate_install_state(&state).is_ok());
        state.runtime_platform = "windows-x64".to_owned();
        assert!(installer.validate_install_state(&state).is_err());

        state = valid_test_state();
        state.runtime_manifest.url = "https://microsoft.com/not-mojang.json".to_owned();
        assert!(installer.validate_install_state(&state).is_err());
    }

    fn valid_test_state() -> InstallState {
        InstallState {
            schema_version: INSTALL_STATE_SCHEMA,
            minecraft_version: MINECRAFT_VERSION.to_owned(),
            runtime_id: FORGE_RUNTIME_ID.to_owned(),
            profile_id: FORGE_PROFILE_ID.to_owned(),
            runtime_platform: "mac-os".to_owned(),
            java_component: "jre-legacy".to_owned(),
            java_version: "8u74-cacert462b08".to_owned(),
            runtime_manifest: DownloadSpec {
                url: "https://piston-meta.mojang.com/v1/packages/78e31aa15b3f437f37263351436f63e0a96e0067/manifest.json".to_owned(),
                sha1: "78e31aa15b3f437f37263351436f63e0a96e0067".to_owned(),
                size: Some(77_882),
            },
        }
    }
}
