use crate::{
    FORGE_COREMOD_SHA1, FORGE_COREMOD_SIZE, FORGE_ELEMENTA_SHA1, FORGE_ELEMENTA_SIZE,
    FORGE_LWJGL_MACOS_COMPAT_SHA1, FORGE_LWJGL_MACOS_COMPAT_SIZE, FORGE_LWJGL_MACOS_NATIVE_SHA1,
    FORGE_LWJGL_MACOS_NATIVE_SIZE, FORGE_LWJGL_RESIZE_GUARD_FILE_NAME,
    FORGE_LWJGL_RESIZE_GUARD_SHA1, FORGE_LWJGL_RESIZE_GUARD_SIZE, FORGE_NATIVE_UI_SHA1,
    FORGE_NATIVE_UI_SIZE, FORGE_OPUSCONFIG_SHA1, FORGE_OPUSCONFIG_SIZE, FORGE_RUNTIME_ID,
    FORGE_UNIVERSALCRAFT_SHA1, FORGE_UNIVERSALCRAFT_SIZE, ForgeRuntimeLock, InstalledMinecraft,
    ManagedJava, MinecraftLayout, extract_natives, replace_native_from_archive, verify_file,
};
use md5::{Digest, Md5};
use opus_platform::{OperatingSystem, Platform};
use serde::Serialize;
use sha1::Sha1;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;

const SESSION_MANIFEST_SCHEMA_VERSION: u32 = 1;
const DIAGNOSTICS_SCHEMA_VERSION: u32 = 1;
const LAUNCHER_SUMMARY_SCHEMA_VERSION: u32 = 1;
const MACOS_GAME_WINDOW_STARTUP_TIMEOUT: Duration = Duration::from_secs(90);
const LWJGL_MACOS_JAVA_ARTIFACT_FILE_NAME: &str = "lwjgl-2.9.2-nightly-20140822.jar";

#[derive(Clone)]
pub struct GameIdentity {
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    pub user_type: String,
    pub user_properties: String,
}

impl GameIdentity {
    pub fn offline(username: &str) -> Result<Self, LaunchError> {
        validate_username(username)?;
        Ok(Self {
            username: username.to_owned(),
            uuid: offline_uuid(username),
            access_token: "0".to_owned(),
            user_type: "legacy".to_owned(),
            user_properties: "{}".to_owned(),
        })
    }

    pub fn authenticated(
        username: &str,
        uuid: &str,
        access_token: String,
        user_type: &str,
    ) -> Result<Self, LaunchError> {
        validate_username(username)?;
        let normalized_uuid = uuid.replace('-', "");
        if normalized_uuid.len() != 32
            || !normalized_uuid.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(LaunchError::InvalidAuthenticatedUuid(uuid.to_owned()));
        }
        if access_token.len() < 8
            || access_token.len() > 4096
            || access_token.chars().any(char::is_whitespace)
        {
            return Err(LaunchError::InvalidAccessToken);
        }
        if user_type != "msa" {
            return Err(LaunchError::InvalidUserType(user_type.to_owned()));
        }
        Ok(Self {
            username: username.to_owned(),
            uuid: normalized_uuid,
            access_token,
            user_type: user_type.to_owned(),
            user_properties: "{}".to_owned(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub min_memory_mib: u32,
    pub max_memory_mib: u32,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub utility_settings_file: Option<PathBuf>,
    pub brand_wordmark_file: Option<PathBuf>,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            min_memory_mib: 512,
            max_memory_mib: 2048,
            width: None,
            height: None,
            utility_settings_file: None,
            brand_wordmark_file: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ForgeUiArtifacts {
    pub native_ui_jar: PathBuf,
    pub opusconfig_jar: PathBuf,
    pub elementa_jar: PathBuf,
    pub universalcraft_jar: PathBuf,
    pub lwjgl_macos_compat_jar: PathBuf,
}

#[derive(Debug, Clone)]
pub enum LaunchMode {
    /// Forge owns Minecraft's class loader. The small Opus bootstrap keeps
    /// identity arguments off the process command line, while the Core Mod and
    /// locked Elementa/OpusConfig UI set are staged into the isolated Forge
    /// `mods` directory.
    ForgeBootstrap {
        bootstrap_jar: PathBuf,
        coremod_jar: PathBuf,
        ui: ForgeUiArtifacts,
    },
}

pub struct LaunchPlan {
    pub java_executable: PathBuf,
    pub jvm_arguments: Vec<OsString>,
    pub main_class: String,
    pub game_arguments: Vec<OsString>,
    pub working_directory: PathBuf,
    pub log_directory: PathBuf,
    pub session_id: String,
    redactions: Vec<String>,
    stdin_payload: Option<Vec<u8>>,
    log_capture_not_before: SystemTime,
}

/// The immutable artifacts a Forge launch is allowed to load outside the
/// normal Minecraft classpath. Keeping this internal makes the production
/// path use the checked-in lock while allowing hermetic launch-plan tests to
/// exercise the same staging and verification code with temporary fixtures.
struct ForgeLaunchContract {
    optifine: crate::DownloadSpec,
    coremod: crate::DownloadSpec,
    native_ui: crate::DownloadSpec,
    opusconfig: crate::DownloadSpec,
    elementa: crate::DownloadSpec,
    universalcraft: crate::DownloadSpec,
    lwjgl_macos_compat: crate::DownloadSpec,
}

struct LaunchBuildRequest<'a> {
    layout: &'a MinecraftLayout,
    platform: Platform,
    minecraft: &'a InstalledMinecraft,
    java: &'a ManagedJava,
    identity: &'a GameIdentity,
    options: &'a LaunchOptions,
    mode: &'a LaunchMode,
}

impl ForgeLaunchContract {
    fn pinned() -> Result<Self, LaunchError> {
        let lock = ForgeRuntimeLock::load().map_err(LaunchError::ForgeLock)?;
        Ok(Self {
            optifine: crate::DownloadSpec {
                // This URL is metadata only for local integrity verification.
                // OptiFine is never fetched through the downloader.
                url: "https://optifine.net/downloads".to_owned(),
                sha1: lock.optifine.sha1,
                size: Some(lock.optifine.size),
            },
            coremod: forge_coremod_spec(),
            native_ui: forge_native_ui_spec(),
            opusconfig: forge_opusconfig_spec(),
            elementa: forge_elementa_spec(),
            universalcraft: forge_universalcraft_spec(),
            lwjgl_macos_compat: forge_lwjgl_macos_compat_spec(),
        })
    }
}

/// Deliberately payload-free launch metadata. It is safe to use when comparing
/// performance sessions because it never contains an identity, a server, a
/// path, a command line, or an authentication value.
#[derive(Serialize)]
struct SessionManifest<'a> {
    schema_version: u32,
    session_id: &'a str,
    minecraft_version: &'a str,
    runtime_id: &'a str,
    platform: SessionPlatform,
    java_version: &'a str,
    memory_mib: SessionMemory,
    launch_mode: &'static str,
    telemetry: TelemetryPolicy,
}

struct SessionManifestRequest<'a> {
    session_id: &'a str,
    platform: Platform,
    java: &'a ManagedJava,
    minecraft_version: &'a str,
    runtime_id: &'a str,
    options: &'a LaunchOptions,
    mode: &'a LaunchMode,
}

#[derive(Serialize)]
struct SessionPlatform {
    os: String,
    host_architecture: String,
    game_architecture: String,
}

#[derive(Serialize)]
struct SessionMemory {
    minimum: u32,
    maximum: u32,
}

#[derive(Serialize)]
struct TelemetryPolicy {
    schema_version: u32,
    storage: &'static str,
    packet_payloads: &'static str,
    player_chat: &'static str,
    authentication: &'static str,
}

#[derive(Serialize)]
struct LauncherSessionSummary<'a> {
    schema_version: u32,
    session_id: &'a str,
    outcome: &'static str,
    exit_code: Option<i32>,
    succeeded: bool,
    minecraft_latest_log_captured: bool,
}

impl LaunchPlan {
    pub fn build(
        layout: &MinecraftLayout,
        platform: Platform,
        minecraft: &InstalledMinecraft,
        java: &ManagedJava,
        identity: &GameIdentity,
        options: &LaunchOptions,
        mode: &LaunchMode,
    ) -> Result<Self, LaunchError> {
        Self::build_with_forge_contract(
            LaunchBuildRequest {
                layout,
                platform,
                minecraft,
                java,
                identity,
                options,
                mode,
            },
            None,
        )
    }

    fn build_with_forge_contract(
        request: LaunchBuildRequest<'_>,
        forge_contract: Option<&ForgeLaunchContract>,
    ) -> Result<Self, LaunchError> {
        let LaunchBuildRequest {
            layout,
            platform,
            minecraft,
            java,
            identity,
            options,
            mode,
        } = request;
        validate_memory(options)?;
        let session_id = new_session_id()?;
        let session_directory = layout.paths.sessions.join(&session_id);
        let natives_directory = session_directory.join("natives");
        let log_directory = layout.paths.logs.join(&session_id);
        create_private_directory(&session_directory)?;
        create_private_directory(&natives_directory)?;
        create_private_directory(&log_directory)?;

        let diagnostics_path = log_directory.join("diagnostics.jsonl");
        let gc_log_path = log_directory.join("gc.log");
        let manifest_path = log_directory.join("session-manifest.json");
        write_session_manifest(
            &manifest_path,
            SessionManifestRequest {
                session_id: &session_id,
                platform,
                java,
                minecraft_version: &minecraft.profile_id,
                runtime_id: &minecraft.runtime_id,
                options,
                mode,
            },
        )?;
        // Pre-create files the JVM/core will write so their permissions are
        // private even when the caller's umask is permissive.
        create_private_file(&diagnostics_path, &[])?;
        create_private_file(&gc_log_path, &[])?;

        let extracted = extract_natives(&minecraft.native_archives, &natives_directory)?;
        validate_native_architectures(platform, &extracted)?;

        let mut jvm_arguments = platform_jvm_arguments(platform.os);
        jvm_arguments.extend([
            OsString::from(format!("-Xms{}M", options.min_memory_mib)),
            OsString::from(format!("-Xmx{}M", options.max_memory_mib)),
            prefixed_path_argument("-Djava.library.path=", &natives_directory),
            prefixed_path_argument("-Dorg.lwjgl.librarypath=", &natives_directory),
            prefixed_path_argument("-Dnet.java.games.input.librarypath=", &natives_directory),
            prefixed_path_argument("-Dopus.diagnostics.file=", &diagnostics_path),
            OsString::from(format!("-Dopus.native.ui.session={session_id}")),
            OsString::from("-Dopus.native.ui.enabled=true"),
            OsString::from("-Dopus.native.ui.requireOptifine=true"),
            prefixed_path_argument("-Xloggc:", &gc_log_path),
            OsString::from("-XX:+PrintGCDetails"),
            OsString::from("-XX:+PrintGCDateStamps"),
            prefixed_path_argument("-XX:ErrorFile=", &log_directory.join("jvm_crash_%p.log")),
        ]);
        if let Some(utility_settings_file) = &options.utility_settings_file {
            jvm_arguments.push(prefixed_path_argument(
                "-Dopus.utility.settings.file=",
                utility_settings_file,
            ));
        }
        if let Some(brand_wordmark_file) = &options.brand_wordmark_file {
            jvm_arguments.push(prefixed_path_argument(
                "-Dopus.brand.wordmark.file=",
                brand_wordmark_file,
            ));
        }
        if let (Some(logging), Some(path)) = (&minecraft.version.logging, &minecraft.logging_config)
        {
            let filtered_path = session_directory.join("log4j-opus.xml");
            prepare_logging_config(path, &filtered_path)?;
            jvm_arguments.push(OsString::from(
                logging
                    .client
                    .argument
                    .replace("${path}", &filtered_path.to_string_lossy()),
            ));
        }
        let mut vanilla_arguments =
            render_game_arguments(&minecraft.minecraft_arguments, layout, minecraft, identity)?;
        if let Some(width) = options.width {
            vanilla_arguments
                .extend([OsString::from("--width"), OsString::from(width.to_string())]);
        }
        if let Some(height) = options.height {
            vanilla_arguments.extend([
                OsString::from("--height"),
                OsString::from(height.to_string()),
            ]);
        }

        let LaunchMode::ForgeBootstrap {
            bootstrap_jar,
            coremod_jar,
            ui,
        } = mode;
        let pinned_contract = if forge_contract.is_none() {
            Some(ForgeLaunchContract::pinned()?)
        } else {
            None
        };
        let contract = forge_contract
            .or(pinned_contract.as_ref())
            .expect("Forge launch must have an integrity contract");
        if minecraft.runtime_id != FORGE_RUNTIME_ID {
            return Err(LaunchError::ForgeRuntimeRequired(
                minecraft.runtime_id.clone(),
            ));
        }
        let optifine_jar = minecraft
            .optifine_jar
            .as_deref()
            .ok_or(LaunchError::OptiFineRequired)?;
        let staged_optifine = stage_forge_optifine(layout, optifine_jar, &contract.optifine)?;
        let staged_coremod = stage_forge_coremod(layout, coremod_jar, &contract.coremod)?;
        let staged_native_ui = stage_forge_ui_artifact(
            layout,
            &ui.native_ui_jar,
            &contract.native_ui,
            "opus-native-ui-1.8.9-0.1.0.jar",
        )?;
        let staged_opusconfig = stage_forge_ui_artifact(
            layout,
            &ui.opusconfig_jar,
            &contract.opusconfig,
            "opus-oneconfig-1.8.9-forge-0.2.2-alpha228-full.jar",
        )?;
        let staged_elementa = stage_forge_ui_artifact(
            layout,
            &ui.elementa_jar,
            &contract.elementa,
            "opus-elementa-762.jar",
        )?;
        let staged_universalcraft = stage_forge_ui_artifact(
            layout,
            &ui.universalcraft_jar,
            &contract.universalcraft,
            "opus-universalcraft-1.8.9-forge-516.jar",
        )?;
        let lwjgl_macos_compat = if platform.os == OperatingSystem::MacOs {
            validate_forge_lwjgl_compat_artifact(
                &ui.lwjgl_macos_compat_jar,
                &contract.lwjgl_macos_compat,
            )?;
            replace_native_from_archive(
                &ui.lwjgl_macos_compat_jar,
                "liblwjgl.dylib",
                &natives_directory.join("liblwjgl.dylib"),
                FORGE_LWJGL_MACOS_NATIVE_SHA1,
                FORGE_LWJGL_MACOS_NATIVE_SIZE,
            )?;
            replace_native_from_archive(
                &ui.lwjgl_macos_compat_jar,
                FORGE_LWJGL_RESIZE_GUARD_FILE_NAME,
                &natives_directory.join(FORGE_LWJGL_RESIZE_GUARD_FILE_NAME),
                FORGE_LWJGL_RESIZE_GUARD_SHA1,
                FORGE_LWJGL_RESIZE_GUARD_SIZE,
            )?;
            Some(
                canonicalize_classpath(std::slice::from_ref(&ui.lwjgl_macos_compat_jar))?[0]
                    .clone(),
            )
        } else {
            None
        };
        remove_retired_forge_client_mod(layout)?;
        validate_managed_forge_mods(
            layout,
            &[
                staged_optifine.as_path(),
                staged_coremod.as_path(),
                staged_native_ui.as_path(),
                staged_opusconfig.as_path(),
                staged_elementa.as_path(),
                staged_universalcraft.as_path(),
            ],
        )?;

        let bootstrap = canonicalize_single_bootstrap_jar(bootstrap_jar)?;
        let mut game_classpath = canonicalize_classpath(&minecraft.classpath)?;
        if platform.os == OperatingSystem::MacOs {
            // The verified compatibility JAR is the complete LWJGL 2 Java
            // replacement on macOS. Keeping Mojang's original Java JAR in
            // LaunchWrapper's URL list would let its sealed packages win
            // before OneConfig reopens the org.lwjgl loader exception.
            game_classpath.retain(|entry| !is_pinned_lwjgl_macos_java_artifact(entry));
        }
        let mut system_classpath = Vec::with_capacity(game_classpath.len() + 2);
        system_classpath.push(bootstrap);
        if let Some(compatibility_jar) = lwjgl_macos_compat {
            jvm_arguments.push(OsString::from("-Dopus.lwjgl.macos.compat=true"));
            system_classpath.push(compatibility_jar);
        }
        system_classpath.extend(game_classpath);
        let game_arguments = vec![OsString::from("--opus-game-arguments-stdin")];
        let stdin_payload = Some(encode_game_arguments(&vanilla_arguments)?);
        let main_class = "org.polydevs.opusmc.bootstrap.ForgeBootstrapMain".to_owned();
        let classpath = std::env::join_paths(&system_classpath)
            .map_err(|source| LaunchError::InvalidClasspath(source.to_string()))?;
        jvm_arguments.push(OsString::from("-cp"));
        jvm_arguments.push(classpath);

        Ok(Self {
            java_executable: java.executable.clone(),
            jvm_arguments,
            main_class,
            game_arguments,
            working_directory: layout.paths.game.clone(),
            log_directory,
            session_id,
            redactions: if identity.access_token.len() >= 8 {
                vec![identity.access_token.clone()]
            } else {
                Vec::new()
            },
            stdin_payload,
            log_capture_not_before: SystemTime::now(),
        })
    }

    pub fn redacted_summary(&self) -> String {
        format!(
            "{} <{} JVM args> {} <{} game args>",
            self.java_executable.display(),
            self.jvm_arguments.len(),
            self.main_class,
            self.game_arguments.len()
        )
    }
}

fn write_session_manifest(
    path: &Path,
    request: SessionManifestRequest<'_>,
) -> Result<(), LaunchError> {
    let manifest = SessionManifest {
        schema_version: SESSION_MANIFEST_SCHEMA_VERSION,
        session_id: request.session_id,
        minecraft_version: request.minecraft_version,
        runtime_id: request.runtime_id,
        platform: SessionPlatform {
            os: request.platform.os.to_string(),
            host_architecture: request.platform.host_arch.to_string(),
            game_architecture: request.platform.game_arch.to_string(),
        },
        java_version: &request.java.version_name,
        memory_mib: SessionMemory {
            minimum: request.options.min_memory_mib,
            maximum: request.options.max_memory_mib,
        },
        launch_mode: launch_mode_name(request.mode),
        telemetry: TelemetryPolicy {
            schema_version: DIAGNOSTICS_SCHEMA_VERSION,
            storage: "local-only",
            packet_payloads: "not-recorded",
            player_chat: "not-recorded",
            authentication: "not-recorded",
        },
    };
    write_json_private(path, &manifest)
}

fn write_launcher_summary(
    plan: &LaunchPlan,
    outcome: &'static str,
    status: Option<&ExitStatus>,
    succeeded: bool,
    minecraft_latest_log_captured: bool,
) -> Result<(), LaunchError> {
    let summary = LauncherSessionSummary {
        schema_version: LAUNCHER_SUMMARY_SCHEMA_VERSION,
        session_id: &plan.session_id,
        outcome,
        exit_code: status.and_then(ExitStatus::code),
        succeeded,
        minecraft_latest_log_captured,
    };
    write_json_private(&plan.log_directory.join("launcher-summary.json"), &summary)
}

fn write_json_private<T: Serialize>(path: &Path, value: &T) -> Result<(), LaunchError> {
    let mut content = serde_json::to_vec_pretty(value).map_err(LaunchError::DiagnosticsJson)?;
    content.push(b'\n');
    create_private_file(path, &content)
}

fn launch_mode_name(mode: &LaunchMode) -> &'static str {
    match mode {
        LaunchMode::ForgeBootstrap { .. } => "forge-bootstrap",
    }
}

fn platform_jvm_arguments(os: OperatingSystem) -> Vec<OsString> {
    match os {
        // Java 8u74's AWT/AppKit bridge crashes on this macOS host when
        // `-XstartOnFirstThread` is supplied. The compatibility LWJGL
        // implementation keeps display work on its normal game/render path;
        // native Cocoa resize notifications no longer update the GL context
        // asynchronously, so the flag is not needed here.
        OperatingSystem::MacOs => vec![OsString::from("-Xdock:name=Minecraft")],
        OperatingSystem::Windows | OperatingSystem::Linux => Vec::new(),
    }
}

fn canonicalize_classpath(classpath: &[PathBuf]) -> Result<Vec<PathBuf>, LaunchError> {
    classpath
        .iter()
        .map(|entry| {
            fs::canonicalize(entry).map_err(|source| LaunchError::CanonicalizeClasspath {
                path: entry.clone(),
                source,
            })
        })
        .collect()
}

fn is_pinned_lwjgl_macos_java_artifact(path: &Path) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| name == LWJGL_MACOS_JAVA_ARTIFACT_FILE_NAME)
}

fn canonicalize_single_bootstrap_jar(path: &Path) -> Result<PathBuf, LaunchError> {
    if !path.is_file() {
        return Err(LaunchError::MissingBootstrapEntry(path.to_path_buf()));
    }
    let name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
    if !name.starts_with("opus-bootstrap-") || !name.ends_with(".jar") {
        return Err(LaunchError::MissingBootstrapJar);
    }
    fs::canonicalize(path).map_err(|source| LaunchError::CanonicalizeClasspath {
        path: path.to_path_buf(),
        source,
    })
}

fn validate_forge_optifine(path: &Path, spec: &crate::DownloadSpec) -> Result<(), LaunchError> {
    let valid =
        verify_file(path, spec).map_err(|_| LaunchError::InvalidOptiFine(path.to_path_buf()))?;
    if !path.is_file() || !valid {
        return Err(LaunchError::InvalidOptiFine(path.to_path_buf()));
    }
    Ok(())
}

fn stage_forge_optifine(
    layout: &MinecraftLayout,
    source: &Path,
    spec: &crate::DownloadSpec,
) -> Result<PathBuf, LaunchError> {
    validate_forge_optifine(source, spec)?;
    let mods_directory = layout.mods_dir();
    fs::create_dir_all(&mods_directory)?;
    let destination = mods_directory.join(crate::FORGE_OPTIFINE_FILE_NAME);
    if destination.is_file() && fs::canonicalize(source).ok() == fs::canonicalize(&destination).ok()
    {
        return Ok(destination);
    }
    let temporary = mods_directory.join(format!(".opus-optifine.part-{}", std::process::id()));
    if temporary.exists() {
        fs::remove_file(&temporary)?;
    }
    let result = (|| -> Result<(), LaunchError> {
        fs::copy(source, &temporary)?;
        validate_forge_optifine(&temporary, spec)?;
        #[cfg(windows)]
        if destination.exists() {
            fs::remove_file(&destination)?;
        }
        fs::rename(&temporary, &destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(destination)
}

fn stage_forge_coremod(
    layout: &MinecraftLayout,
    source: &Path,
    spec: &crate::DownloadSpec,
) -> Result<PathBuf, LaunchError> {
    validate_forge_coremod(source, spec)?;
    // Forge 1.8.9 discovers `FMLCorePlugin` manifests from `gameDir/mods`
    // (and `mods/<mc-version>`), not the newer `coremods` convention.
    let mods_directory = layout.mods_dir();
    fs::create_dir_all(&mods_directory)?;
    let destination = mods_directory.join("opus-runtime-legacy-1.8.9.jar");
    let temporary = mods_directory.join(format!(".opus-runtime.part-{}", std::process::id()));
    if temporary.exists() {
        fs::remove_file(&temporary)?;
    }
    let result = (|| -> Result<(), LaunchError> {
        fs::copy(source, &temporary)?;
        validate_forge_coremod(&temporary, spec)?;
        #[cfg(windows)]
        if destination.exists() {
            fs::remove_file(&destination)?;
        }
        fs::rename(&temporary, &destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(destination)
}

fn forge_coremod_spec() -> crate::DownloadSpec {
    crate::DownloadSpec {
        // This is not a download endpoint: the checked-in release artifact is
        // verified locally before staging beside the managed OptiFine mod.
        url: "https://opus.invalid/release/opus-runtime-legacy-1.8.9.jar".to_owned(),
        sha1: FORGE_COREMOD_SHA1.to_owned(),
        size: Some(FORGE_COREMOD_SIZE),
    }
}

fn forge_native_ui_spec() -> crate::DownloadSpec {
    crate::DownloadSpec {
        url: "https://opus.invalid/release/opus-native-ui-1.8.9-0.1.0.jar".to_owned(),
        sha1: FORGE_NATIVE_UI_SHA1.to_owned(),
        size: Some(FORGE_NATIVE_UI_SIZE),
    }
}

fn forge_opusconfig_spec() -> crate::DownloadSpec {
    crate::DownloadSpec {
        url: "https://opus.invalid/release/opus-oneconfig-1.8.9-forge-0.2.2-alpha228-full.jar"
            .to_owned(),
        sha1: FORGE_OPUSCONFIG_SHA1.to_owned(),
        size: Some(FORGE_OPUSCONFIG_SIZE),
    }
}

fn forge_elementa_spec() -> crate::DownloadSpec {
    crate::DownloadSpec {
        url: "https://opus.invalid/release/opus-elementa-762.jar".to_owned(),
        sha1: FORGE_ELEMENTA_SHA1.to_owned(),
        size: Some(FORGE_ELEMENTA_SIZE),
    }
}

fn forge_universalcraft_spec() -> crate::DownloadSpec {
    crate::DownloadSpec {
        url: "https://opus.invalid/release/opus-universalcraft-1.8.9-forge-516.jar".to_owned(),
        sha1: FORGE_UNIVERSALCRAFT_SHA1.to_owned(),
        size: Some(FORGE_UNIVERSALCRAFT_SIZE),
    }
}

fn forge_lwjgl_macos_compat_spec() -> crate::DownloadSpec {
    crate::DownloadSpec {
        url: "https://opus.invalid/release/opus-lwjgl-macos-compat-2.9.2-nightly-20140822.jar"
            .to_owned(),
        sha1: FORGE_LWJGL_MACOS_COMPAT_SHA1.to_owned(),
        size: Some(FORGE_LWJGL_MACOS_COMPAT_SIZE),
    }
}

fn stage_forge_ui_artifact(
    layout: &MinecraftLayout,
    source: &Path,
    spec: &crate::DownloadSpec,
    file_name: &str,
) -> Result<PathBuf, LaunchError> {
    validate_forge_ui_artifact(source, spec)?;
    let mods_directory = layout.mods_dir();
    fs::create_dir_all(&mods_directory)?;
    let destination = mods_directory.join(file_name);
    let temporary = mods_directory.join(format!(".{file_name}.part-{}", std::process::id()));
    if temporary.exists() {
        fs::remove_file(&temporary)?;
    }
    let result = (|| -> Result<(), LaunchError> {
        fs::copy(source, &temporary)?;
        validate_forge_ui_artifact(&temporary, spec)?;
        #[cfg(windows)]
        if destination.exists() {
            fs::remove_file(&destination)?;
        }
        fs::rename(&temporary, &destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(destination)
}

fn validate_forge_ui_artifact(path: &Path, spec: &crate::DownloadSpec) -> Result<(), LaunchError> {
    if !path.is_file() {
        return Err(LaunchError::MissingForgeUiArtifact(path.to_path_buf()));
    }
    if !verify_file(path, spec)
        .map_err(|_| LaunchError::InvalidForgeUiArtifact(path.to_path_buf()))?
    {
        return Err(LaunchError::InvalidForgeUiArtifact(path.to_path_buf()));
    }
    let file =
        File::open(path).map_err(|_| LaunchError::InvalidForgeUiArtifact(path.to_path_buf()))?;
    zip::ZipArchive::new(file)
        .map(|_| ())
        .map_err(|_| LaunchError::InvalidForgeUiArtifact(path.to_path_buf()))
}

fn validate_forge_lwjgl_compat_artifact(
    path: &Path,
    spec: &crate::DownloadSpec,
) -> Result<(), LaunchError> {
    if !path.is_file() {
        return Err(LaunchError::MissingLwjglMacosCompat(path.to_path_buf()));
    }
    if !verify_file(path, spec)
        .map_err(|_| LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()))?
    {
        return Err(LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()));
    }
    let file =
        File::open(path).map_err(|_| LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()))?;
    let mut manifest_bytes = Vec::new();
    {
        let mut manifest = archive
            .by_name("META-INF/MANIFEST.MF")
            .map_err(|_| LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()))?;
        manifest
            .read_to_end(&mut manifest_bytes)
            .map_err(|_| LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()))?;
    }
    let manifest_text = String::from_utf8(manifest_bytes)
        .map_err(|_| LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()))?;
    if !manifest_text
        .lines()
        .any(|line| line.trim() == "Sealed: true")
    {
        return Err(LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()));
    }
    for required in [
        "org/lwjgl/MacOSXSysImplementation.class",
        "org/lwjgl/J2SESysImplementation.class",
        "org/lwjgl/DefaultSysImplementation.class",
        "org/lwjgl/SysImplementation.class",
        "org/lwjgl/opengl/MacOSXDisplay.class",
        "org/lwjgl/opengl/MacOSXCanvasPeerInfo.class",
        "org/lwjgl/opengl/MacOSXCanvasPeerInfo$1.class",
        "org/lwjgl/opengl/MacOSXDisplayPeerInfo.class",
    ] {
        if archive.by_name(required).is_err() {
            return Err(LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()));
        }
    }
    let native_bytes = {
        let mut native = archive
            .by_name("liblwjgl.dylib")
            .map_err(|_| LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()))?;
        let mut bytes = Vec::new();
        native
            .read_to_end(&mut bytes)
            .map_err(|_| LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()))?;
        bytes
    };
    let native_sha1 = format!("{:x}", Sha1::digest(&native_bytes));
    let mach_o = native_bytes.starts_with(&[0xca, 0xfe, 0xba, 0xbe])
        || native_bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe]);
    if native_bytes.len() as u64 != FORGE_LWJGL_MACOS_NATIVE_SIZE
        || !native_sha1.eq_ignore_ascii_case(FORGE_LWJGL_MACOS_NATIVE_SHA1)
        || !mach_o
    {
        return Err(LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()));
    }
    let replacement_bytes = {
        let mut replacement = archive
            .by_name("org/lwjgl/MacOSXSysImplementation.class")
            .map_err(|_| LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()))?;
        let mut bytes = Vec::new();
        replacement
            .read_to_end(&mut bytes)
            .map_err(|_| LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()))?;
        bytes
    };
    let symbols = String::from_utf8_lossy(&replacement_bytes);
    if symbols.contains("java/awt/Toolkit") || !symbols.contains("com.apple.eio.FileManager") {
        return Err(LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()));
    }
    let display_patch_bytes = {
        let mut display_patch = archive
            .by_name("org/lwjgl/opengl/MacOSXDisplay.class")
            .map_err(|_| LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()))?;
        let mut bytes = Vec::new();
        display_patch
            .read_to_end(&mut bytes)
            .map_err(|_| LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()))?;
        bytes
    };
    if !String::from_utf8_lossy(&display_patch_bytes)
        .contains("OPUS_MODERN_MACOS_DISPLAY_COMPAT_V1")
        || !String::from_utf8_lossy(&display_patch_bytes)
        .contains("OPUS_COCOA_LIVE_RESIZE_HANDSHAKE_V10_APPKIT_DRAWABLE_UPDATE")
    {
        return Err(LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()));
    }
    let peer_info_patch_bytes = {
        let mut peer_info_patch = archive
            .by_name("org/lwjgl/opengl/MacOSXCanvasPeerInfo.class")
            .map_err(|_| LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()))?;
        let mut bytes = Vec::new();
        peer_info_patch
            .read_to_end(&mut bytes)
            .map_err(|_| LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()))?;
        bytes
    };
    if !String::from_utf8_lossy(&peer_info_patch_bytes)
        .contains("OPUS_MODERN_MACOS_PIXEL_FORMAT_COMPAT_V2")
    {
        return Err(LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()));
    }
    let guard_bytes = {
        let mut guard = archive
            .by_name(FORGE_LWJGL_RESIZE_GUARD_FILE_NAME)
            .map_err(|_| LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()))?;
        let mut bytes = Vec::new();
        guard
            .read_to_end(&mut bytes)
            .map_err(|_| LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()))?;
        bytes
    };
    let guard_sha1 = format!("{:x}", Sha1::digest(&guard_bytes));
    let guard_is_x86_64_mach_o = guard_bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe]);
    if guard_bytes.len() as u64 != FORGE_LWJGL_RESIZE_GUARD_SIZE
        || !guard_sha1.eq_ignore_ascii_case(FORGE_LWJGL_RESIZE_GUARD_SHA1)
        || !guard_is_x86_64_mach_o
        || !String::from_utf8_lossy(&guard_bytes)
            .contains("OPUS_COCOA_LIVE_RESIZE_GUARD_V9_APPKIT_DRAWABLE_UPDATE")
    {
        return Err(LaunchError::InvalidLwjglMacosCompat(path.to_path_buf()));
    }
    Ok(())
}

fn validate_forge_coremod(path: &Path, spec: &crate::DownloadSpec) -> Result<(), LaunchError> {
    if !path.is_file() {
        return Err(LaunchError::MissingForgeCoremod(path.to_path_buf()));
    }
    if !verify_file(path, spec).map_err(|_| LaunchError::InvalidForgeCoremod(path.to_path_buf()))? {
        return Err(LaunchError::InvalidForgeCoremod(path.to_path_buf()));
    }

    let file =
        File::open(path).map_err(|_| LaunchError::InvalidForgeCoremod(path.to_path_buf()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| LaunchError::InvalidForgeCoremod(path.to_path_buf()))?;
    let mut manifest = String::new();
    archive
        .by_name("META-INF/MANIFEST.MF")
        .map_err(|_| LaunchError::InvalidForgeCoremod(path.to_path_buf()))?
        .read_to_string(&mut manifest)
        .map_err(|_| LaunchError::InvalidForgeCoremod(path.to_path_buf()))?;
    if !manifest_has_attribute(
        &manifest,
        "FMLCorePlugin",
        "org.polydevs.opusmc.forge.OpusLoadingPlugin",
    ) || !manifest_has_attribute(&manifest, "FMLCorePluginContainsFMLMod", "false")
    {
        return Err(LaunchError::InvalidForgeCoremod(path.to_path_buf()));
    }
    Ok(())
}

fn manifest_has_attribute(manifest: &str, key: &str, expected: &str) -> bool {
    let target = format!("{key}: {expected}");
    manifest
        .lines()
        .any(|line| line.trim_end_matches('\r') == target)
}

fn validate_managed_forge_mods(
    layout: &MinecraftLayout,
    managed_jars: &[&Path],
) -> Result<(), LaunchError> {
    let expected = managed_jars
        .iter()
        .map(|path| {
            fs::canonicalize(path).map_err(|source| LaunchError::CanonicalizeClasspath {
                path: (*path).to_path_buf(),
                source,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    for entry in fs::read_dir(layout.mods_dir())? {
        let entry = entry?;
        let candidate = entry.path();
        let file_name = entry.file_name();
        if file_name
            .to_str()
            .map(|name| name.starts_with('.'))
            .unwrap_or(false)
        {
            continue;
        }
        if !candidate.is_file() {
            return Err(LaunchError::UnmanagedForgeMod(candidate));
        }
        let canonical =
            fs::canonicalize(&candidate).map_err(|source| LaunchError::CanonicalizeClasspath {
                path: candidate.clone(),
                source,
            })?;
        if !expected.iter().any(|managed| managed == &canonical) {
            return Err(LaunchError::UnmanagedForgeMod(candidate));
        }
    }
    Ok(())
}

/// The old typed client-mod name is a bounded migration target. Remove only
/// that exact regular file before validating the managed Forge set so an
/// older launcher session cannot resurrect any in-game UI path.
fn remove_retired_forge_client_mod(layout: &MinecraftLayout) -> Result<(), LaunchError> {
    let path = layout.mods_dir().join("opus-client-legacy-1.8.9.jar");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            fs::remove_file(path)?;
            Ok(())
        }
        Ok(_) => Err(LaunchError::UnmanagedForgeMod(path)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(LaunchError::Io(error)),
    }
}

#[derive(Debug)]
pub struct LaunchResult {
    pub status: ExitStatus,
    pub session_id: String,
    pub log_directory: PathBuf,
}

pub fn launch_game(plan: LaunchPlan) -> Result<LaunchResult, LaunchError> {
    let java_executable = plan.java_executable.clone();
    launch_game_inner(plan, java_executable, Vec::new())
}

/// Starts the managed JVM through a minimal LaunchServices app bundle on macOS.
/// The bundle executable immediately `exec`s Java; it performs no AWT, Carbon,
/// JNI, or window manipulation of its own.
pub fn launch_game_via_macos_app(
    mut plan: LaunchPlan,
    game_app: &Path,
) -> Result<LaunchResult, LaunchError> {
    validate_macos_game_app(game_app)?;
    fs::create_dir_all(&plan.working_directory)?;
    create_private_directory(&plan.log_directory)?;

    let stdin_path = plan.log_directory.join("game.stdin.bin");
    let raw_stdout_path = plan.log_directory.join("game.stdout.raw.log");
    let raw_stderr_path = plan.log_directory.join("game.stderr.raw.log");
    let stdout_path = plan.log_directory.join("game.stdout.log");
    let stderr_path = plan.log_directory.join("game.stderr.log");
    let status_path = plan.log_directory.join("game.status");
    let window_ready_path = plan.log_directory.join("game.window.ready");
    let window_failure_path = plan.log_directory.join("game.window.failure");
    insert_jvm_argument_before_classpath(
        &mut plan.jvm_arguments,
        OsString::from(format!("-Dopus.game.statusFile={}", status_path.display())),
    );
    create_private_file(&stdout_path, &[])?;
    create_private_file(&stderr_path, &[])?;
    write_launch_log(&plan, game_app)?;

    let launch_status = (|| {
        create_private_file(&raw_stdout_path, &[])?;
        create_private_file(&raw_stderr_path, &[])?;
        create_private_file(&status_path, b"starting\n")?;
        let stdin_for_open = if let Some(payload) = plan.stdin_payload.as_deref() {
            create_private_file(&stdin_path, payload)?;
            stdin_path.as_path()
        } else {
            Path::new("/dev/null")
        };

        let open_executable = PathBuf::from("/usr/bin/open");
        let status = Command::new(&open_executable)
            .args(macos_open_arguments(
                stdin_for_open,
                &raw_stdout_path,
                &raw_stderr_path,
                &plan.working_directory,
                game_app,
                &plan.java_executable,
                &plan.jvm_arguments,
                &plan.main_class,
                &plan.game_arguments,
                &window_ready_path,
                &window_failure_path,
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|source| LaunchError::Spawn {
                executable: open_executable,
                source,
            })?;
        if !status.success() {
            return Err(LaunchError::MacosGameAppLaunchFailed(status));
        }
        Ok(status)
    })();

    let status = match launch_status {
        Ok(status) => status,
        Err(error) => {
            let _ = finalize_macos_game_logs(
                &plan,
                &stdin_path,
                &raw_stdout_path,
                &raw_stderr_path,
                &stdout_path,
                &stderr_path,
            );
            finalize_session(&plan, "launcher-launch-failed", None, false, false);
            return Err(error);
        }
    };
    let game_status = wait_for_macos_game_lifecycle(&status_path);
    let redaction_result = finalize_macos_game_logs(
        &plan,
        &stdin_path,
        &raw_stdout_path,
        &raw_stderr_path,
        &stdout_path,
        &stderr_path,
    );
    if let Err(error) = redaction_result {
        finalize_session(&plan, "log-redaction-failed", Some(&status), false, true);
        return Err(error);
    }
    let game_status = match game_status {
        Ok(game_status) => game_status,
        Err(error) => {
            finalize_session(&plan, "game-lifecycle-failed", Some(&status), false, true);
            return Err(error);
        }
    };
    match game_status.as_str() {
        "exited" | "terminated" => {
            finalize_session(&plan, "game-lifecycle-complete", Some(&status), true, true);
        }
        "failed" => {
            finalize_session(&plan, "game-lifecycle-failed", Some(&status), false, true);
            return Err(LaunchError::MacosGameFailed);
        }
        "starting" | "running" | "" => {
            finalize_session(
                &plan,
                "game-lifecycle-incomplete",
                Some(&status),
                false,
                true,
            );
            return Err(LaunchError::MacosGameDidNotExit);
        }
        other => {
            finalize_session(&plan, "game-lifecycle-invalid", Some(&status), false, true);
            return Err(LaunchError::MacosGameStatusInvalid(other.to_owned()));
        }
    }
    Ok(LaunchResult {
        status,
        session_id: plan.session_id,
        log_directory: plan.log_directory,
    })
}

/// LaunchServices cannot reliably wait for an app whose executable immediately
/// `exec`s Java: `open -W` reports a false failure when the stub process is
/// replaced. Start the foreground app normally and use the bootstrap's status
/// file as the sole lifecycle authority instead.
#[allow(clippy::too_many_arguments)]
fn macos_open_arguments(
    stdin_path: &Path,
    raw_stdout_path: &Path,
    raw_stderr_path: &Path,
    working_directory: &Path,
    game_app: &Path,
    java_executable: &Path,
    jvm_arguments: &[OsString],
    main_class: &str,
    game_arguments: &[OsString],
    window_ready_path: &Path,
    window_failure_path: &Path,
) -> Vec<OsString> {
    let mut working_directory_environment = OsString::from("OPUS_GAME_WORKDIR=");
    working_directory_environment.push(working_directory);
    let mut arguments = vec![
        OsString::from("-n"),
        OsString::from("--stdin"),
        stdin_path.as_os_str().to_owned(),
        OsString::from("--stdout"),
        raw_stdout_path.as_os_str().to_owned(),
        OsString::from("--stderr"),
        raw_stderr_path.as_os_str().to_owned(),
        OsString::from("--env"),
        working_directory_environment,
        OsString::from("--env"),
        OsString::from(format!(
            "OPUS_GAME_WINDOW_READY_FILE={}",
            window_ready_path.display()
        )),
        OsString::from("--env"),
        OsString::from(format!(
            "OPUS_GAME_WINDOW_FAILURE_FILE={}",
            window_failure_path.display()
        )),
        game_app.as_os_str().to_owned(),
        OsString::from("--args"),
        java_executable.as_os_str().to_owned(),
    ];
    arguments.extend_from_slice(jvm_arguments);
    arguments.push(OsString::from(main_class));
    arguments.extend_from_slice(game_arguments);
    arguments
}

fn wait_for_macos_game_lifecycle(status_path: &Path) -> Result<String, LaunchError> {
    let log_directory = status_path.parent().unwrap_or_else(|| Path::new("."));
    let startup_deadline = Instant::now() + MACOS_GAME_WINDOW_STARTUP_TIMEOUT;
    loop {
        let game_status = fs::read_to_string(status_path)?.trim().to_owned();
        match game_status.as_str() {
            "running" => {
                wait_for_macos_game_window_ready(status_path, log_directory, startup_deadline)?;
                check_macos_game_health(status_path, log_directory)?;
                return wait_for_macos_game_exit(status_path, log_directory);
            }
            "exited" | "terminated" => {
                ensure_macos_window_did_not_fail(log_directory)?;
                if !macos_game_window_ready(log_directory).is_file() {
                    return Err(LaunchError::MacosGameWindowNotReady);
                }
                return Ok(game_status);
            }
            "failed" => return Err(LaunchError::MacosGameFailed),
            "starting" | "" => {
                // The macOS stub can fail activation before Java publishes a
                // terminal lifecycle state. Check both markers here so a
                // locked console or missing window is reported immediately.
                ensure_macos_window_did_not_fail(log_directory)?;
                // A native JVM crash can happen before the bootstrap gets a
                // chance to publish `failed`. Detect its fatal-error marker
                // during startup so LaunchServices cannot leave us waiting
                // for the startup timeout unnecessarily.
                check_macos_jvm_crash_marker(log_directory)?;
                if Instant::now() >= startup_deadline {
                    return Err(LaunchError::MacosGameStartupTimedOut);
                }
                thread::sleep(Duration::from_millis(100));
            }
            other => return Err(LaunchError::MacosGameStatusInvalid(other.to_owned())),
        }
    }
}

fn wait_for_macos_game_window_ready(
    status_path: &Path,
    log_directory: &Path,
    deadline: Instant,
) -> Result<(), LaunchError> {
    let ready_path = macos_game_window_ready(log_directory);
    loop {
        if ready_path.is_file() {
            return Ok(());
        }
        ensure_macos_window_did_not_fail(log_directory)?;
        check_macos_jvm_crash_marker(log_directory)?;

        let status = fs::read_to_string(status_path)?.trim().to_owned();
        match status.as_str() {
            "failed" => return Err(LaunchError::MacosGameFailed),
            "exited" | "terminated" => return Err(LaunchError::MacosGameWindowNotReady),
            "running" | "starting" | "" => {}
            other => return Err(LaunchError::MacosGameStatusInvalid(other.to_owned())),
        }

        check_macos_game_health(status_path, log_directory)?;
        if Instant::now() >= deadline {
            return Err(LaunchError::MacosGameWindowStartupTimedOut);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn macos_game_window_ready(log_directory: &Path) -> PathBuf {
    log_directory.join("game.window.ready")
}

fn macos_game_window_failure(log_directory: &Path) -> PathBuf {
    log_directory.join("game.window.failure")
}

fn ensure_macos_window_did_not_fail(log_directory: &Path) -> Result<(), LaunchError> {
    let failure = macos_game_window_failure(log_directory);
    if failure.is_file() {
        return Err(LaunchError::MacosGameWindowActivationFailed(failure));
    }
    Ok(())
}

fn wait_for_macos_game_exit(
    status_path: &Path,
    log_directory: &Path,
) -> Result<String, LaunchError> {
    loop {
        let game_status = fs::read_to_string(status_path)?.trim().to_owned();
        match game_status.as_str() {
            "exited" | "terminated" => {
                ensure_macos_window_did_not_fail(log_directory)?;
                return Ok(game_status);
            }
            "failed" => return Err(LaunchError::MacosGameFailed),
            "running" => {
                check_macos_game_health(status_path, log_directory)?;
                thread::sleep(Duration::from_millis(100));
            }
            "starting" | "" => {
                ensure_macos_window_did_not_fail(log_directory)?;
                check_macos_jvm_crash_marker(log_directory)?;
                thread::sleep(Duration::from_millis(100));
            }
            other => return Err(LaunchError::MacosGameStatusInvalid(other.to_owned())),
        }
    }
}

/// The bootstrap writes the JVM pid immediately before publishing `running`.
/// If that process disappears without updating the status file (for example a
/// native AppKit crash), do not leave the launcher waiting forever.
fn check_macos_game_health(status_path: &Path, log_directory: &Path) -> Result<(), LaunchError> {
    check_macos_jvm_crash_marker(log_directory)?;

    let pid_path = status_path.with_file_name("game.pid");
    let raw_pid = match fs::read_to_string(&pid_path) {
        Ok(raw_pid) => raw_pid,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(LaunchError::MacosGamePidMissing(pid_path));
        }
        Err(error) => return Err(error.into()),
    };
    let pid = raw_pid
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|pid| *pid > 1)
        .ok_or_else(|| LaunchError::MacosGamePidInvalid(pid_path.clone()))?;
    if !macos_game_process_is_alive(pid)? {
        return Err(LaunchError::MacosGameProcessExited(pid));
    }
    Ok(())
}

fn check_macos_jvm_crash_marker(log_directory: &Path) -> Result<(), LaunchError> {
    if let Some(path) = find_macos_jvm_crash_marker(log_directory)? {
        return Err(LaunchError::MacosGameNativeCrash(path));
    }
    Ok(())
}

fn find_macos_jvm_crash_marker(log_directory: &Path) -> Result<Option<PathBuf>, LaunchError> {
    let entries = match fs::read_dir(log_directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let is_marker = name
            .to_str()
            .map(|name| name.starts_with("jvm_crash_") && name.ends_with(".log"))
            .unwrap_or(false);
        if is_marker && entry.path().is_file() {
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

#[cfg(unix)]
fn macos_game_process_is_alive(pid: u32) -> Result<bool, LaunchError> {
    let status = Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(status.success())
}

#[cfg(not(unix))]
fn macos_game_process_is_alive(_pid: u32) -> Result<bool, LaunchError> {
    // The macOS app lifecycle is only used on Unix. Keep non-Unix builds
    // portable without pulling in a platform-specific process API.
    Ok(true)
}

fn finalize_macos_game_logs(
    plan: &LaunchPlan,
    stdin_path: &Path,
    raw_stdout_path: &Path,
    raw_stderr_path: &Path,
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<(), LaunchError> {
    let redact_stdout = if raw_stdout_path.is_file() {
        redact_log_file(raw_stdout_path, stdout_path, &plan.redactions)
    } else {
        Ok(())
    };
    let redact_stderr = if raw_stderr_path.is_file() {
        redact_log_file(raw_stderr_path, stderr_path, &plan.redactions)
    } else {
        Ok(())
    };
    let result = redact_stdout.and(redact_stderr);
    let _ = fs::remove_file(stdin_path);
    if result.is_ok() {
        let _ = fs::remove_file(raw_stdout_path);
        let _ = fs::remove_file(raw_stderr_path);
    }
    result
}

fn insert_jvm_argument_before_classpath(arguments: &mut Vec<OsString>, argument: OsString) {
    let classpath_index = arguments
        .iter()
        .position(|existing| existing == "-cp")
        .unwrap_or(arguments.len());
    arguments.insert(classpath_index, argument);
}

fn launch_game_inner(
    plan: LaunchPlan,
    executable: PathBuf,
    entrypoint_arguments: Vec<OsString>,
) -> Result<LaunchResult, LaunchError> {
    fs::create_dir_all(&plan.working_directory)?;
    create_private_directory(&plan.log_directory)?;
    let stdout_path = plan.log_directory.join("game.stdout.log");
    let stderr_path = plan.log_directory.join("game.stderr.log");
    create_private_file(&stdout_path, &[])?;
    create_private_file(&stderr_path, &[])?;
    write_launch_log(&plan, &executable)?;

    let uses_stdin_protocol = plan.stdin_payload.is_some();
    let mut command = Command::new(&executable);
    command
        .args(&entrypoint_arguments)
        .args(&plan.jvm_arguments)
        .arg(&plan.main_class)
        .args(&plan.game_arguments)
        .current_dir(&plan.working_directory)
        .stdin(if uses_stdin_protocol {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(source) => {
            finalize_session(&plan, "launcher-launch-failed", None, false, false);
            return Err(LaunchError::Spawn { executable, source });
        }
    };

    if let Some(payload) = plan.stdin_payload.as_deref() {
        let Some(mut stdin) = child.stdin.take() else {
            finalize_session(&plan, "stdin-unavailable", None, false, false);
            return Err(LaunchError::MissingChildPipe("stdin"));
        };
        if let Err(error) = stdin.write_all(payload).and_then(|_| stdin.flush()) {
            finalize_session(&plan, "stdin-write-failed", None, false, false);
            return Err(error.into());
        }
        drop(stdin);
    }

    let Some(stdout) = child.stdout.take() else {
        finalize_session(&plan, "stdout-unavailable", None, false, false);
        return Err(LaunchError::MissingChildPipe("stdout"));
    };
    let Some(stderr) = child.stderr.take() else {
        finalize_session(&plan, "stderr-unavailable", None, false, false);
        return Err(LaunchError::MissingChildPipe("stderr"));
    };
    let stdout_redactions = plan.redactions.clone();
    let stderr_redactions = plan.redactions.clone();
    let stdout_thread =
        thread::spawn(move || tee_lines(stdout, &stdout_path, false, &stdout_redactions));
    let stderr_thread =
        thread::spawn(move || tee_lines(stderr, &stderr_path, true, &stderr_redactions));
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            finalize_session(&plan, "process-wait-failed", None, false, false);
            return Err(error.into());
        }
    };
    if let Err(error) = join_tee(stdout_thread) {
        finalize_session(&plan, "stdout-log-failed", Some(&status), false, true);
        return Err(error);
    }
    if let Err(error) = join_tee(stderr_thread) {
        finalize_session(&plan, "stderr-log-failed", Some(&status), false, true);
        return Err(error);
    }

    finalize_session(
        &plan,
        "process-exited",
        Some(&status),
        status.success(),
        true,
    );

    Ok(LaunchResult {
        status,
        session_id: plan.session_id,
        log_directory: plan.log_directory,
    })
}

fn write_launch_log(plan: &LaunchPlan, entrypoint: &Path) -> Result<(), LaunchError> {
    let launcher_log_path = plan.log_directory.join("launcher.log");
    let content = format!(
        "session={}\nentrypoint={}\ncommand={}\n",
        plan.session_id,
        entrypoint.display(),
        plan.redacted_summary()
    );
    create_private_file(&launcher_log_path, content.as_bytes())
}

fn finalize_session(
    plan: &LaunchPlan,
    outcome: &'static str,
    status: Option<&ExitStatus>,
    succeeded: bool,
    capture_minecraft_latest_log: bool,
) {
    // Diagnostics must never make a completed game look like a failed launch.
    // The per-session directory remains private even if a best-effort artifact
    // could not be written.
    let minecraft_latest_log_captured = if capture_minecraft_latest_log {
        snapshot_minecraft_latest_log(plan).unwrap_or(false)
    } else {
        false
    };
    let _ = write_launcher_summary(
        plan,
        outcome,
        status,
        succeeded,
        minecraft_latest_log_captured,
    );
}

fn snapshot_minecraft_latest_log(plan: &LaunchPlan) -> Result<bool, LaunchError> {
    let source_path = plan.working_directory.join("logs").join("latest.log");
    let metadata = match fs::metadata(&source_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let modified = metadata.modified()?;
    if modified
        .duration_since(plan.log_capture_not_before)
        .is_err()
    {
        return Ok(false);
    }
    let mut source = File::open(&source_path)?;
    let destination_path = plan.log_directory.join("minecraft.latest.log");
    let mut destination = open_private_file_for_overwrite(&destination_path)?;
    io::copy(&mut source, &mut destination)?;
    destination.sync_all()?;
    Ok(true)
}

fn create_private_directory(path: &Path) -> Result<(), LaunchError> {
    fs::create_dir_all(path)?;
    restrict_private_directory_permissions(path)?;
    Ok(())
}

fn create_private_file(path: &Path, contents: &[u8]) -> Result<(), LaunchError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    restrict_private_file_permissions(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}

fn open_private_file_for_overwrite(path: &Path) -> Result<File, LaunchError> {
    if !path.exists() {
        create_private_file(path, &[])?;
    }
    restrict_private_file_permissions(path)?;
    Ok(OpenOptions::new().write(true).truncate(true).open(path)?)
}

fn redact_log_file(
    source: &Path,
    destination: &Path,
    redactions: &[String],
) -> Result<(), LaunchError> {
    let source = File::open(source)?;
    let mut destination = open_private_file_for_overwrite(destination)?;
    for line in BufReader::new(source).lines() {
        let mut line = line?;
        for secret in redactions {
            line = line.replace(secret, "<redacted>");
        }
        writeln!(destination, "{line}")?;
    }
    destination.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn restrict_private_file_permissions(path: &Path) -> Result<(), io::Error> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(unix)]
fn restrict_private_directory_permissions(path: &Path) -> Result<(), io::Error> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict_private_directory_permissions(_: &Path) -> Result<(), io::Error> {
    Ok(())
}

#[cfg(not(unix))]
fn restrict_private_file_permissions(_: &Path) -> Result<(), io::Error> {
    Ok(())
}

fn validate_macos_game_app(game_app: &Path) -> Result<(), LaunchError> {
    let executable = game_app.join("Contents/MacOS/Minecraft");
    if !game_app.is_dir() || !executable.is_file() {
        return Err(LaunchError::MacosGameAppMissing(game_app.to_path_buf()));
    }
    Ok(())
}

fn encode_game_arguments(arguments: &[OsString]) -> Result<Vec<u8>, LaunchError> {
    const MAX_ARGUMENTS: usize = 256;
    const MAX_ARGUMENT_LENGTH: usize = 1024 * 1024;
    if arguments.len() > MAX_ARGUMENTS {
        return Err(LaunchError::TooManyGameArguments(arguments.len()));
    }

    let mut payload = Vec::new();
    payload.extend_from_slice(&(arguments.len() as u32).to_be_bytes());
    for argument in arguments {
        let text = argument
            .to_str()
            .ok_or_else(|| LaunchError::NonUtf8Argument(argument.clone()))?;
        let bytes = text.as_bytes();
        if bytes.len() > MAX_ARGUMENT_LENGTH {
            return Err(LaunchError::GameArgumentTooLong(bytes.len()));
        }
        payload.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        payload.extend_from_slice(bytes);
    }
    Ok(payload)
}

fn prepare_logging_config(source: &Path, destination: &Path) -> Result<(), LaunchError> {
    const FILTERS_MARKER: &str = "<filters>";
    const LOOKUP_FILTER_ANCHOR: &str =
        r#"<RegexFilter regex="(?s).*\$\{[^}]*\}.*" onMatch="DENY" onMismatch="NEUTRAL"/>"#;
    const TOKEN_FILTER: &str = "<RegexFilter regex=\"(?s).*Session ID is token:.*\" onMatch=\"DENY\" onMismatch=\"NEUTRAL\"/>";

    let original = fs::read_to_string(source)?;
    if original.matches(FILTERS_MARKER).count() != 1
        || original.matches(LOOKUP_FILTER_ANCHOR).count() != 1
    {
        return Err(LaunchError::UnexpectedLoggingConfiguration);
    }
    let filtered = original.replacen(
        FILTERS_MARKER,
        &format!("{FILTERS_MARKER}\n                {TOKEN_FILTER}"),
        1,
    );
    create_private_file(destination, filtered.as_bytes())
}

fn render_game_arguments(
    template: &str,
    layout: &MinecraftLayout,
    minecraft: &InstalledMinecraft,
    identity: &GameIdentity,
) -> Result<Vec<OsString>, LaunchError> {
    let tokens = shlex::split(template).ok_or(LaunchError::InvalidArgumentTemplate)?;
    let assets_directory = layout.assets_dir();
    let replacements = [
        ("${auth_player_name}", identity.username.as_str()),
        ("${version_name}", minecraft.profile_id.as_str()),
        ("${game_directory}", path_text(&layout.paths.game)?),
        ("${assets_root}", path_text(&assets_directory)?),
        ("${assets_index_name}", minecraft.version.assets.as_str()),
        ("${auth_uuid}", identity.uuid.as_str()),
        ("${auth_access_token}", identity.access_token.as_str()),
        ("${user_properties}", identity.user_properties.as_str()),
        ("${user_type}", identity.user_type.as_str()),
    ];

    tokens
        .into_iter()
        .map(|mut token| {
            for (placeholder, value) in replacements {
                token = token.replace(placeholder, value);
            }
            if token.contains("${") {
                return Err(LaunchError::UnresolvedPlaceholder(token));
            }
            Ok(OsString::from(token))
        })
        .collect()
}

fn prefixed_path_argument(prefix: &str, path: &Path) -> OsString {
    let mut argument = OsString::from(prefix);
    argument.push(path.as_os_str());
    argument
}

fn path_text(path: &Path) -> Result<&str, LaunchError> {
    path.to_str()
        .ok_or_else(|| LaunchError::NonUtf8Path(path.to_path_buf()))
}

fn validate_memory(options: &LaunchOptions) -> Result<(), LaunchError> {
    if options.min_memory_mib < 256
        || options.max_memory_mib < options.min_memory_mib
        || options.max_memory_mib > 16 * 1024
    {
        return Err(LaunchError::InvalidMemory {
            minimum: options.min_memory_mib,
            maximum: options.max_memory_mib,
        });
    }
    Ok(())
}

fn validate_username(username: &str) -> Result<(), LaunchError> {
    if !(3..=16).contains(&username.len())
        || !username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(LaunchError::InvalidOfflineUsername(username.to_owned()));
    }
    Ok(())
}

fn offline_uuid(username: &str) -> String {
    let mut digest = Md5::digest(format!("OfflinePlayer:{username}").as_bytes());
    digest[6] = (digest[6] & 0x0f) | 0x30;
    digest[8] = (digest[8] & 0x3f) | 0x80;
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn new_session_id() -> Result<String, LaunchError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| LaunchError::SystemClockBeforeEpoch)?
        .as_millis();
    Ok(format!("{millis}-{}", std::process::id()))
}

fn validate_native_architectures(
    platform: Platform,
    extracted: &[PathBuf],
) -> Result<(), LaunchError> {
    // Architecture is ultimately validated by the JVM when loading each
    // library. This preflight at least guarantees that the expected LWJGL and
    // JInput native families were selected, catching rule/classifier mistakes
    // before the game reaches its main loop.
    let names: Vec<_> = extracted
        .iter()
        .filter_map(|path| path.file_name().and_then(OsStr::to_str))
        .collect();
    let has_lwjgl = names
        .iter()
        .any(|name| name.to_ascii_lowercase().contains("lwjgl"));
    let has_input = names
        .iter()
        .any(|name| name.to_ascii_lowercase().contains("jinput"));
    if !has_lwjgl || !has_input {
        return Err(LaunchError::MissingNativeFamilies {
            platform: format!("{}-{}", platform.os, platform.game_arch),
            has_lwjgl,
            has_input,
        });
    }
    Ok(())
}

fn tee_lines<R: io::Read>(
    reader: R,
    path: &Path,
    stderr: bool,
    redactions: &[String],
) -> Result<(), io::Error> {
    let mut file = open_private_file_for_overwrite(path).map_err(|error| match error {
        LaunchError::Io(error) => error,
        other => io::Error::other(other.to_string()),
    })?;
    for line in BufReader::new(reader).lines() {
        let mut line = line?;
        for secret in redactions {
            line = line.replace(secret, "<redacted>");
        }
        writeln!(file, "{line}")?;
        if stderr {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
    file.sync_all()
}

fn join_tee(handle: thread::JoinHandle<Result<(), io::Error>>) -> Result<(), LaunchError> {
    handle
        .join()
        .map_err(|_| LaunchError::LogThreadPanicked)??;
    Ok(())
}

#[derive(Debug, Error)]
pub enum LaunchError {
    #[error("native preparation failed")]
    Native(#[from] crate::natives::NativeError),
    #[error("Forge runtime lock is invalid")]
    ForgeLock(#[source] crate::ForgeLockError),
    #[error("filesystem or process operation failed")]
    Io(#[from] io::Error),
    #[error("could not serialize session diagnostics")]
    DiagnosticsJson(#[source] serde_json::Error),
    #[error("invalid classpath: {0}")]
    InvalidClasspath(String),
    #[error("bootstrap classpath does not contain opus-bootstrap-*.jar")]
    MissingBootstrapJar,
    #[error("bootstrap classpath entry does not exist: {path}", path = .0.display())]
    MissingBootstrapEntry(PathBuf),
    #[error("Forge launch requires the pinned Forge + OptiFine runtime, got {0}")]
    ForgeRuntimeRequired(String),
    #[error("Forge launch requires an imported OptiFine 1.8.9 HD U M5 JAR")]
    OptiFineRequired,
    #[error("managed OptiFine failed integrity verification: {}", .0.display())]
    InvalidOptiFine(PathBuf),
    #[error("Opus Forge coremod is missing: {}", .0.display())]
    MissingForgeCoremod(PathBuf),
    #[error("Opus Forge coremod is invalid: {}", .0.display())]
    InvalidForgeCoremod(PathBuf),
    #[error("Opus Forge UI artifact is missing: {}", .0.display())]
    MissingForgeUiArtifact(PathBuf),
    #[error("Opus Forge UI artifact is invalid: {}", .0.display())]
    InvalidForgeUiArtifact(PathBuf),
    #[error("Opus macOS LWJGL compatibility artifact is missing: {}", .0.display())]
    MissingLwjglMacosCompat(PathBuf),
    #[error("Opus macOS LWJGL compatibility artifact is invalid: {}", .0.display())]
    InvalidLwjglMacosCompat(PathBuf),
    #[error("unmanaged JAR or directory in isolated Forge mods directory: {}", .0.display())]
    UnmanagedForgeMod(PathBuf),
    #[error("failed to canonicalize classpath entry {path}", path = .path.display())]
    CanonicalizeClasspath {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Minecraft argument template is invalid")]
    InvalidArgumentTemplate,
    #[error("official logging configuration has an unexpected structure")]
    UnexpectedLoggingConfiguration,
    #[error("unresolved launch placeholder: {0}")]
    UnresolvedPlaceholder(String),
    #[error("path is not valid UTF-8: {path}", path = .0.display())]
    NonUtf8Path(PathBuf),
    #[error("invalid memory range: Xms={minimum} MiB, Xmx={maximum} MiB")]
    InvalidMemory { minimum: u32, maximum: u32 },
    #[error("invalid offline development username: {0}")]
    InvalidOfflineUsername(String),
    #[error("authenticated Minecraft UUID is invalid: {0}")]
    InvalidAuthenticatedUuid(String),
    #[error("authenticated access token has an invalid shape")]
    InvalidAccessToken,
    #[error("unsupported authenticated user type: {0}")]
    InvalidUserType(String),
    #[error("game argument is not valid UTF-8")]
    NonUtf8Argument(OsString),
    #[error("too many game arguments: {0}")]
    TooManyGameArguments(usize),
    #[error("game argument is too long: {0} bytes")]
    GameArgumentTooLong(usize),
    #[error("system clock is before the Unix epoch")]
    SystemClockBeforeEpoch,
    #[error(
        "native selection for {platform} is incomplete (LWJGL={has_lwjgl}, JInput={has_input})"
    )]
    MissingNativeFamilies {
        platform: String,
        has_lwjgl: bool,
        has_input: bool,
    },
    #[error("failed to start game process at {executable}", executable = .executable.display())]
    Spawn {
        executable: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("macOS game app is incomplete or missing: {path}", path = .0.display())]
    MacosGameAppMissing(PathBuf),
    #[error("macOS LaunchServices could not start the game app ({0})")]
    MacosGameAppLaunchFailed(ExitStatus),
    #[error("Minecraft reported a startup or runtime failure")]
    MacosGameFailed,
    #[error("Minecraft native JVM crash marker was written: {0}")]
    MacosGameNativeCrash(PathBuf),
    #[error("Minecraft status is running but its process id file is missing: {0}")]
    MacosGamePidMissing(PathBuf),
    #[error("Minecraft status is running but its process id file is invalid: {0}")]
    MacosGamePidInvalid(PathBuf),
    #[error("Minecraft process {0} exited without updating its lifecycle status")]
    MacosGameProcessExited(u32),
    #[error("Minecraft window was not visible when the game lifecycle ended")]
    MacosGameWindowNotReady,
    #[error("Minecraft window did not become visible before the startup deadline")]
    MacosGameWindowStartupTimedOut,
    #[error("Minecraft window activation failed; see {0}")]
    MacosGameWindowActivationFailed(PathBuf),
    #[error("Minecraft did not report a completed game lifecycle")]
    MacosGameDidNotExit,
    #[error("Minecraft did not report that startup completed within 90 seconds")]
    MacosGameStartupTimedOut,
    #[error("Minecraft reported an invalid game lifecycle state: {0}")]
    MacosGameStatusInvalid(String),
    #[error("child process did not expose {0}")]
    MissingChildPipe(&'static str),
    #[error("game log thread panicked")]
    LogThreadPanicked,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha1::{Digest as Sha1Digest, Sha1};
    use std::io::Write;
    use std::path::Path;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    struct ForgeFixture {
        _temporary: tempfile::TempDir,
        layout: MinecraftLayout,
        platform: Platform,
        minecraft: InstalledMinecraft,
        java: ManagedJava,
        options: LaunchOptions,
        bootstrap_jar: PathBuf,
        coremod_jar: PathBuf,
        native_ui_jar: PathBuf,
        opusconfig_jar: PathBuf,
        elementa_jar: PathBuf,
        universalcraft_jar: PathBuf,
        lwjgl_macos_compat_jar: PathBuf,
        optifine_jar: PathBuf,
        contract: ForgeLaunchContract,
    }

    impl ForgeFixture {
        fn new() -> Self {
            let temporary = tempfile::tempdir().unwrap();
            let layout = MinecraftLayout::new(
                opus_platform::OpusPaths::from_root(temporary.path().join("opus")).unwrap(),
            );
            let bootstrap_jar = temporary.path().join("opus-bootstrap-0.0.1.jar");
            let forge_classpath = temporary.path().join("forge-runtime.jar");
            let vanilla_classpath = temporary.path().join("vanilla-client.jar");
            let coremod_jar = temporary.path().join("opus-runtime-legacy-1.8.9-0.0.1.jar");
            let native_ui_jar = temporary.path().join("opus-native-ui-1.8.9-0.1.0.jar");
            let opusconfig_jar = temporary
                .path()
                .join("opus-oneconfig-1.8.9-forge-0.2.2-alpha228-full.jar");
            let elementa_jar = temporary.path().join("opus-elementa-762.jar");
            let universalcraft_jar = temporary
                .path()
                .join("opus-universalcraft-1.8.9-forge-516.jar");
            let lwjgl_macos_compat_jar = temporary
                .path()
                .join("opus-lwjgl-macos-compat-2.9.2-nightly-20140822.jar");
            let native_archive = temporary.path().join("natives.jar");
            fs::write(&bootstrap_jar, b"bootstrap fixture").unwrap();
            fs::write(&forge_classpath, b"forge fixture").unwrap();
            fs::write(&vanilla_classpath, b"vanilla fixture").unwrap();
            write_zip(
                &native_archive,
                &[
                    ("liblwjgl.so", b"lwjgl fixture"),
                    ("libjinput-linux64.so", b"jinput fixture"),
                ],
            );
            write_zip(
                &coremod_jar,
                &[
                    (
                        "META-INF/MANIFEST.MF",
                        b"Manifest-Version: 1.0\r\nFMLCorePlugin: org.polydevs.opusmc.forge.OpusLoadingPlugin\r\nFMLCorePluginContainsFMLMod: false\r\n\r\n",
                    ),
                    ("org/polydevs/opusmc/forge/OpusLoadingPlugin.class", b"coremod fixture"),
                ],
            );
            for (path, marker) in [
                (&native_ui_jar, b"native UI fixture".as_slice()),
                (&opusconfig_jar, b"OpusConfig fixture".as_slice()),
                (&elementa_jar, b"Elementa fixture".as_slice()),
                (&universalcraft_jar, b"UniversalCraft fixture".as_slice()),
            ] {
                write_zip(path, &[("fixture.txt", marker)]);
            }
            write_zip(
                &lwjgl_macos_compat_jar,
                &[
                    (
                        "META-INF/MANIFEST.MF",
                        b"Manifest-Version: 1.0\r\nSealed: true\r\n\r\n",
                    ),
                    (
                        "org/lwjgl/MacOSXSysImplementation.class",
                        b"com.apple.eio.FileManager",
                    ),
                    ("org/lwjgl/J2SESysImplementation.class", b"parent"),
                    ("org/lwjgl/DefaultSysImplementation.class", b"parent"),
                    ("org/lwjgl/SysImplementation.class", b"parent"),
                ],
            );
            let optifine_jar = layout.optifine_mod();
            fs::create_dir_all(optifine_jar.parent().unwrap()).unwrap();
            fs::write(&optifine_jar, b"optifine fixture").unwrap();
            let contract = ForgeLaunchContract {
                optifine: fixture_spec(&optifine_jar),
                coremod: fixture_spec(&coremod_jar),
                native_ui: fixture_spec(&native_ui_jar),
                opusconfig: fixture_spec(&opusconfig_jar),
                elementa: fixture_spec(&elementa_jar),
                universalcraft: fixture_spec(&universalcraft_jar),
                lwjgl_macos_compat: fixture_spec(&lwjgl_macos_compat_jar),
            };
            let minecraft = InstalledMinecraft {
                version: fixture_minecraft_version(),
                client_jar: vanilla_classpath.clone(),
                classpath: vec![forge_classpath, vanilla_classpath],
                native_archives: vec![crate::NativeArchive {
                    path: native_archive,
                    excludes: Vec::new(),
                }],
                logging_config: None,
                runtime_id: FORGE_RUNTIME_ID.to_owned(),
                profile_id: "1.8.9-forge-fixture".to_owned(),
                main_class: "net.minecraft.launchwrapper.Launch".to_owned(),
                minecraft_arguments: "--username ${auth_player_name} --version ${version_name} --gameDir ${game_directory} --assetsDir ${assets_root} --assetIndex ${assets_index_name} --uuid ${auth_uuid} --accessToken ${auth_access_token} --userProperties ${user_properties} --userType ${user_type} --tweakClass net.minecraftforge.fml.common.launcher.FMLTweaker".to_owned(),
                optifine_jar: Some(optifine_jar.clone()),
            };
            Self {
                _temporary: temporary,
                layout,
                platform: Platform {
                    os: OperatingSystem::Linux,
                    host_arch: opus_platform::Architecture::X86_64,
                    game_arch: opus_platform::Architecture::X86_64,
                },
                minecraft,
                java: ManagedJava {
                    version_name: "1.8.0-fixture".to_owned(),
                    java_home: PathBuf::from("/fixture/java"),
                    executable: PathBuf::from("/fixture/java/bin/java"),
                },
                options: LaunchOptions::default(),
                bootstrap_jar,
                coremod_jar,
                native_ui_jar,
                opusconfig_jar,
                elementa_jar,
                universalcraft_jar,
                lwjgl_macos_compat_jar,
                optifine_jar,
                contract,
            }
        }

        fn mode(&self) -> LaunchMode {
            LaunchMode::ForgeBootstrap {
                bootstrap_jar: self.bootstrap_jar.clone(),
                coremod_jar: self.coremod_jar.clone(),
                ui: ForgeUiArtifacts {
                    native_ui_jar: self.native_ui_jar.clone(),
                    opusconfig_jar: self.opusconfig_jar.clone(),
                    elementa_jar: self.elementa_jar.clone(),
                    universalcraft_jar: self.universalcraft_jar.clone(),
                    lwjgl_macos_compat_jar: self.lwjgl_macos_compat_jar.clone(),
                },
            }
        }

        fn request<'a>(
            &'a self,
            identity: &'a GameIdentity,
            mode: &'a LaunchMode,
        ) -> LaunchBuildRequest<'a> {
            LaunchBuildRequest {
                layout: &self.layout,
                platform: self.platform,
                minecraft: &self.minecraft,
                java: &self.java,
                identity,
                options: &self.options,
                mode,
            }
        }
    }

    fn fixture_minecraft_version() -> crate::MinecraftVersion {
        serde_json::from_value(serde_json::json!({
            "id": "1.8.9",
            "mainClass": "net.minecraft.client.main.Main",
            "minecraftArguments": "",
            "assets": "legacy",
            "assetIndex": {
                "id": "legacy",
                "sha1": "0000000000000000000000000000000000000000",
                "size": 0,
                "url": "https://fixture.invalid/assets.json"
            },
            "downloads": {
                "client": {
                    "path": null,
                    "sha1": "0000000000000000000000000000000000000000",
                    "size": 0,
                    "url": "https://fixture.invalid/client.jar"
                }
            },
            "libraries": [],
            "logging": null,
            "javaVersion": null
        }))
        .unwrap()
    }

    fn fixture_spec(path: &Path) -> crate::DownloadSpec {
        let contents = fs::read(path).unwrap();
        let mut hasher = Sha1::new();
        hasher.update(&contents);
        crate::DownloadSpec {
            url: "https://fixture.invalid/artifact.jar".to_owned(),
            sha1: format!("{:x}", hasher.finalize()),
            size: Some(contents.len() as u64),
        }
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

    fn system_classpath(plan: &LaunchPlan) -> Vec<PathBuf> {
        let index = plan
            .jvm_arguments
            .iter()
            .position(|argument| argument == OsStr::new("-cp"))
            .unwrap();
        std::env::split_paths(&plan.jvm_arguments[index + 1]).collect()
    }

    #[test]
    fn offline_uuid_matches_minecraft_algorithm() {
        assert_eq!(offline_uuid("Notch"), "b50ad385829d3141a2167e7d7539ba7f");
    }

    #[test]
    fn rejects_unsafe_offline_username() {
        assert!(GameIdentity::offline("ab").is_err());
        assert!(GameIdentity::offline("name with spaces").is_err());
        assert!(GameIdentity::offline("valid_Name9").is_ok());
    }

    #[test]
    fn macos_jvm_arguments_keep_the_verified_lwjgl_launch_contract() {
        assert_eq!(
            platform_jvm_arguments(OperatingSystem::MacOs),
            vec![OsString::from("-Xdock:name=Minecraft")]
        );
        assert!(platform_jvm_arguments(OperatingSystem::Windows).is_empty());
        assert!(platform_jvm_arguments(OperatingSystem::Linux).is_empty());
    }

    #[test]
    fn macos_launchservices_arguments_do_not_wait_for_an_exec_replaced_stub() {
        let arguments = macos_open_arguments(
            Path::new("/tmp/stdin"),
            Path::new("/tmp/stdout"),
            Path::new("/tmp/stderr"),
            Path::new("/tmp/game"),
            Path::new("/tmp/Minecraft.app"),
            Path::new("/tmp/java"),
            &[OsString::from("-Xmx2G")],
            "org.polydevs.opusmc.bootstrap.ForgeBootstrapMain",
            &[OsString::from("--opus-game-arguments-stdin")],
            Path::new("/tmp/game.window.ready"),
            Path::new("/tmp/game.window.failure"),
        );

        assert!(arguments.contains(&OsString::from("-n")));
        assert!(!arguments.contains(&OsString::from("-W")));
        assert!(arguments.contains(&OsString::from("--stdin")));
        assert!(arguments.contains(&OsString::from("--args")));
    }

    #[test]
    fn macos_lifecycle_accepts_a_terminal_status_without_waiting_for_open() {
        let temporary = tempfile::tempdir().unwrap();
        let status_path = temporary.path().join("game.status");
        fs::write(&status_path, "exited\n").unwrap();
        fs::write(temporary.path().join("game.window.ready"), "ready\n").unwrap();

        assert_eq!(
            wait_for_macos_game_lifecycle(&status_path).unwrap(),
            "exited"
        );
    }

    #[test]
    fn macos_lifecycle_reports_bootstrap_failure() {
        let temporary = tempfile::tempdir().unwrap();
        let status_path = temporary.path().join("game.status");
        fs::write(&status_path, "failed\n").unwrap();

        assert!(matches!(
            wait_for_macos_game_lifecycle(&status_path),
            Err(LaunchError::MacosGameFailed)
        ));
    }

    #[test]
    fn macos_lifecycle_reports_window_activation_failure_during_startup() {
        let temporary = tempfile::tempdir().unwrap();
        let status_path = temporary.path().join("game.status");
        let failure_path = temporary.path().join("game.window.failure");
        fs::write(&status_path, "starting\n").unwrap();
        fs::write(&failure_path, "macOS console is locked\n").unwrap();

        assert!(matches!(
            wait_for_macos_game_lifecycle(&status_path),
            Err(LaunchError::MacosGameWindowActivationFailed(path)) if path == failure_path
        ));
    }

    #[test]
    fn macos_lifecycle_detects_a_native_jvm_crash_marker_while_running() {
        let temporary = tempfile::tempdir().unwrap();
        let status_path = temporary.path().join("game.status");
        let crash_path = temporary.path().join("jvm_crash_1234.log");
        fs::write(&status_path, "running\n").unwrap();
        fs::write(&crash_path, "fatal native error\n").unwrap();

        assert!(matches!(
            wait_for_macos_game_lifecycle(&status_path),
            Err(LaunchError::MacosGameNativeCrash(path)) if path == crash_path
        ));
    }

    #[cfg(unix)]
    #[test]
    fn macos_lifecycle_detects_a_dead_game_pid_while_running() {
        let temporary = tempfile::tempdir().unwrap();
        let status_path = temporary.path().join("game.status");
        let pid_path = temporary.path().join("game.pid");
        let mut process = Command::new("/bin/sh")
            .arg("-c")
            .arg("exit 0")
            .spawn()
            .unwrap();
        let pid = process.id();
        process.wait().unwrap();
        fs::write(&status_path, "running\n").unwrap();
        fs::write(&pid_path, format!("{pid}\n")).unwrap();

        assert!(matches!(
            wait_for_macos_game_lifecycle(&status_path),
            Err(LaunchError::MacosGameProcessExited(actual)) if actual == pid
        ));
    }

    #[test]
    fn macos_lifecycle_rejects_running_status_without_a_pid() {
        let temporary = tempfile::tempdir().unwrap();
        let status_path = temporary.path().join("game.status");
        fs::write(&status_path, "running\n").unwrap();

        assert!(matches!(
            wait_for_macos_game_lifecycle(&status_path),
            Err(LaunchError::MacosGamePidMissing(path))
                if path == temporary.path().join("game.pid")
        ));
    }

    #[test]
    fn lifecycle_property_is_inserted_before_the_classpath() {
        let mut arguments = vec![
            OsString::from("-Xmx2048M"),
            OsString::from("-cp"),
            OsString::from("game.jar"),
        ];
        insert_jvm_argument_before_classpath(
            &mut arguments,
            OsString::from("-Dopus.game.statusFile=/tmp/game.status"),
        );
        assert_eq!(
            arguments,
            vec![
                OsString::from("-Xmx2048M"),
                OsString::from("-Dopus.game.statusFile=/tmp/game.status"),
                OsString::from("-cp"),
                OsString::from("game.jar"),
            ]
        );
    }

    #[test]
    fn session_manifest_is_payload_free_and_describes_telemetry_policy() {
        let temp = tempfile::tempdir().unwrap();
        let manifest_path = temp.path().join("session-manifest.json");
        let java = ManagedJava {
            version_name: "1.8.0_452".to_owned(),
            java_home: PathBuf::from("/private/runtime"),
            executable: PathBuf::from("/private/runtime/bin/java"),
        };
        let platform = Platform {
            os: OperatingSystem::MacOs,
            host_arch: opus_platform::Architecture::Aarch64,
            game_arch: opus_platform::Architecture::X86_64,
        };
        let mode = LaunchMode::ForgeBootstrap {
            bootstrap_jar: PathBuf::from("/private/opus-bootstrap-0.0.1.jar"),
            coremod_jar: PathBuf::from("/private/opus-runtime-legacy-1.8.9-0.0.1.jar"),
            ui: ForgeUiArtifacts {
                native_ui_jar: PathBuf::from("/private/opus-native-ui-1.8.9-0.1.0.jar"),
                opusconfig_jar: PathBuf::from(
                    "/private/opus-oneconfig-1.8.9-forge-0.2.2-alpha228-full.jar",
                ),
                elementa_jar: PathBuf::from("/private/opus-elementa-762.jar"),
                universalcraft_jar: PathBuf::from(
                    "/private/opus-universalcraft-1.8.9-forge-516.jar",
                ),
                lwjgl_macos_compat_jar: PathBuf::from(
                    "/private/opus-lwjgl-macos-compat-2.9.2-nightly-20140822.jar",
                ),
            },
        };

        write_session_manifest(
            &manifest_path,
            SessionManifestRequest {
                session_id: "session-123",
                platform,
                java: &java,
                minecraft_version: "1.8.9-forge-test",
                runtime_id: "forge-optifine-test",
                options: &LaunchOptions::default(),
                mode: &mode,
            },
        )
        .unwrap();

        let manifest = fs::read_to_string(manifest_path).unwrap();
        assert!(manifest.contains("\"session_id\": \"session-123\""));
        assert!(manifest.contains("\"storage\": \"local-only\""));
        assert!(manifest.contains("\"packet_payloads\": \"not-recorded\""));
        assert!(manifest.contains("\"game_architecture\": \"x86_64\""));
        assert!(!manifest.contains("/private"));
        assert!(!manifest.contains("opus-bootstrap-0.0.1.jar"));
    }

    #[cfg(unix)]
    #[test]
    fn private_session_artifacts_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("session");
        let file = directory.join("diagnostics.jsonl");
        create_private_directory(&directory).unwrap();
        create_private_file(&file, b"{}\n").unwrap();

        assert_eq!(
            fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn memory_validation_rejects_inverted_range() {
        assert!(
            validate_memory(&LaunchOptions {
                min_memory_mib: 2048,
                max_memory_mib: 1024,
                ..LaunchOptions::default()
            })
            .is_err()
        );
    }

    #[test]
    fn canonicalizes_relative_child_process_classpath() {
        let temp = tempfile::tempdir_in(".").unwrap();
        let relative = temp.path().join("opus-bootstrap-0.0.1.jar");
        File::create(&relative).unwrap().write_all(b"jar").unwrap();
        let canonical = canonicalize_classpath(std::slice::from_ref(&relative)).unwrap();
        assert!(canonical[0].is_absolute());
        assert!(canonical[0].is_file());
    }

    #[test]
    fn identifies_only_the_pinned_lwjgl_java_artifact_for_macos_removal() {
        assert!(is_pinned_lwjgl_macos_java_artifact(Path::new(
            "/private/libraries/org/lwjgl/lwjgl/2.9.2-nightly-20140822/lwjgl-2.9.2-nightly-20140822.jar",
        )));
        assert!(!is_pinned_lwjgl_macos_java_artifact(Path::new(
            "/private/libraries/org/lwjgl/lwjgl_util/2.9.2-nightly-20140822/lwjgl_util-2.9.2-nightly-20140822.jar",
        )));
        assert!(!is_pinned_lwjgl_macos_java_artifact(Path::new(
            "/private/mods/opus-lwjgl-macos-compat-2.9.2-nightly-20140822.jar",
        )));
    }

    #[test]
    fn derived_logging_config_blocks_session_token_messages() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("official.xml");
        let destination = temp.path().join("opus.xml");
        File::create(&source)
            .unwrap()
            .write_all(b"<Configuration><filters><RegexFilter regex=\"(?s).*\\$\\{[^}]*\\}.*\" onMatch=\"DENY\" onMismatch=\"NEUTRAL\"/></filters></Configuration>")
            .unwrap();

        prepare_logging_config(&source, &destination).unwrap();
        let derived = fs::read_to_string(destination).unwrap();
        assert!(derived.contains("Session ID is token:"));
        assert!(derived.contains("onMatch=\"DENY\""));
    }

    #[test]
    fn logging_config_without_mojang_lookup_mitigation_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("unsafe.xml");
        let destination = temp.path().join("opus.xml");
        fs::write(
            &source,
            "<Configuration><filters></filters></Configuration>",
        )
        .unwrap();

        assert!(matches!(
            prepare_logging_config(&source, &destination),
            Err(LaunchError::UnexpectedLoggingConfiguration)
        ));
        assert!(!destination.exists());
    }

    #[test]
    fn authenticated_identity_validates_uuid_token_and_type() {
        assert!(
            GameIdentity::authenticated(
                "Player",
                "01234567-89ab-cdef-0123-456789abcdef",
                "long-enough-token".to_owned(),
                "msa"
            )
            .is_ok()
        );
        assert!(
            GameIdentity::authenticated(
                "Player",
                "bad-uuid",
                "long-enough-token".to_owned(),
                "msa"
            )
            .is_err()
        );
    }

    #[test]
    fn game_argument_protocol_is_big_endian_and_contains_no_shell_encoding() {
        let payload = encode_game_arguments(&[
            OsString::from("--username"),
            OsString::from("name with spaces"),
        ])
        .unwrap();
        assert_eq!(&payload[..4], &[0, 0, 0, 2]);
        assert!(payload.windows(16).any(|part| part == b"name with spaces"));
    }

    #[test]
    fn forge_bootstrap_plan_stages_the_ui_set_and_removes_the_retired_client_mod() {
        let fixture = ForgeFixture::new();
        // A prior launcher may have left same-named, stale UI artifacts in an
        // identity directory. The managed artifact contract must remove it
        // before Forge is allowed to discover the managed mod set.
        fs::write(
            fixture
                .layout
                .mods_dir()
                .join("opus-runtime-legacy-1.8.9.jar"),
            b"stale coremod artifact",
        )
        .unwrap();
        fs::write(
            fixture
                .layout
                .mods_dir()
                .join("opus-client-legacy-1.8.9.jar"),
            b"stale client artifact",
        )
        .unwrap();
        let mode = fixture.mode();
        let identity = GameIdentity::offline("ForgeFixture").unwrap();
        let plan = LaunchPlan::build_with_forge_contract(
            fixture.request(&identity, &mode),
            Some(&fixture.contract),
        )
        .unwrap();

        assert_eq!(
            plan.main_class,
            "org.polydevs.opusmc.bootstrap.ForgeBootstrapMain"
        );
        assert_eq!(
            plan.game_arguments,
            vec![OsString::from("--opus-game-arguments-stdin")]
        );
        assert!(plan.stdin_payload.is_some());
        assert!(plan.jvm_arguments.contains(&OsString::from(format!(
            "-Dopus.native.ui.session={}",
            plan.session_id
        ))));
        assert!(
            plan.jvm_arguments
                .contains(&OsString::from("-Dopus.native.ui.enabled=true"))
        );
        assert!(
            plan.jvm_arguments
                .contains(&OsString::from("-Dopus.native.ui.requireOptifine=true"))
        );

        let classpath = system_classpath(&plan);
        assert_eq!(
            classpath,
            vec![
                fs::canonicalize(&fixture.bootstrap_jar).unwrap(),
                fs::canonicalize(&fixture.minecraft.classpath[0]).unwrap(),
                fs::canonicalize(&fixture.minecraft.classpath[1]).unwrap(),
            ]
        );
        assert!(!classpath.contains(&fs::canonicalize(&fixture.optifine_jar).unwrap()));
        assert!(!classpath.contains(&fs::canonicalize(&fixture.coremod_jar).unwrap()));

        let staged_coremod = fixture
            .layout
            .mods_dir()
            .join("opus-runtime-legacy-1.8.9.jar");
        assert_eq!(
            fs::read(&staged_coremod).unwrap(),
            fs::read(&fixture.coremod_jar).unwrap()
        );
        validate_forge_coremod(&staged_coremod, &fixture.contract.coremod).unwrap();

        for (name, source, spec) in [
            (
                "opus-native-ui-1.8.9-0.1.0.jar",
                &fixture.native_ui_jar,
                &fixture.contract.native_ui,
            ),
            (
                "opus-oneconfig-1.8.9-forge-0.2.2-alpha228-full.jar",
                &fixture.opusconfig_jar,
                &fixture.contract.opusconfig,
            ),
            (
                "opus-elementa-762.jar",
                &fixture.elementa_jar,
                &fixture.contract.elementa,
            ),
            (
                "opus-universalcraft-1.8.9-forge-516.jar",
                &fixture.universalcraft_jar,
                &fixture.contract.universalcraft,
            ),
        ] {
            let staged = fixture.layout.mods_dir().join(name);
            assert_eq!(fs::read(&staged).unwrap(), fs::read(source).unwrap());
            validate_forge_ui_artifact(&staged, spec).unwrap();
        }

        assert!(
            !fixture
                .layout
                .mods_dir()
                .join("opus-client-legacy-1.8.9.jar")
                .exists()
        );
        validate_managed_forge_mods(
            &fixture.layout,
            &[
                fixture.optifine_jar.as_path(),
                staged_coremod.as_path(),
                fixture
                    .layout
                    .mods_dir()
                    .join("opus-native-ui-1.8.9-0.1.0.jar")
                    .as_path(),
                fixture
                    .layout
                    .mods_dir()
                    .join("opus-oneconfig-1.8.9-forge-0.2.2-alpha228-full.jar")
                    .as_path(),
                fixture
                    .layout
                    .mods_dir()
                    .join("opus-elementa-762.jar")
                    .as_path(),
                fixture
                    .layout
                    .mods_dir()
                    .join("opus-universalcraft-1.8.9-forge-516.jar")
                    .as_path(),
            ],
        )
        .unwrap();
    }

    #[test]
    fn forge_bootstrap_plan_rejects_an_unmanaged_mod() {
        let fixture = ForgeFixture::new();
        let unmanaged = fixture.layout.mods_dir().join("unmanaged.jar");
        fs::write(&unmanaged, b"not in the Forge contract").unwrap();
        let mode = fixture.mode();
        let identity = GameIdentity::offline("ForgeFixture").unwrap();

        match LaunchPlan::build_with_forge_contract(
            fixture.request(&identity, &mode),
            Some(&fixture.contract),
        ) {
            Err(LaunchError::UnmanagedForgeMod(path)) => assert_eq!(path, unmanaged),
            Err(error) => panic!("expected unmanaged Forge mod error, got {error:?}"),
            Ok(_) => panic!("expected unmanaged Forge mod error"),
        }
    }

    #[test]
    fn forge_bootstrap_plan_rejects_an_unmanaged_mod_directory() {
        let fixture = ForgeFixture::new();
        let unmanaged = fixture.layout.mods_dir().join("1.8.9");
        fs::create_dir_all(&unmanaged).unwrap();
        let mode = fixture.mode();
        let identity = GameIdentity::offline("ForgeFixture").unwrap();

        match LaunchPlan::build_with_forge_contract(
            fixture.request(&identity, &mode),
            Some(&fixture.contract),
        ) {
            Err(LaunchError::UnmanagedForgeMod(path)) => assert_eq!(path, unmanaged),
            Err(error) => panic!("expected unmanaged Forge mod error, got {error:?}"),
            Ok(_) => panic!("expected unmanaged Forge mod error"),
        }
    }
}
