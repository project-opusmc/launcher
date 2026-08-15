use crate::{
    FORGE_CLIENT_MOD_SHA1, FORGE_CLIENT_MOD_SIZE, FORGE_COREMOD_SHA1, FORGE_COREMOD_SIZE,
    FORGE_RUNTIME_ID, ForgeRuntimeLock, InstalledMinecraft, ManagedJava, MinecraftLayout,
    extract_natives, verify_file,
};
use md5::{Digest, Md5};
use rbw_platform::{OperatingSystem, Platform};
use serde::Serialize;
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

    fn is_authenticated(&self) -> bool {
        self.access_token.len() >= 8
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
pub enum LaunchMode {
    Direct,
    Bootstrap {
        classpath: Vec<PathBuf>,
    },
    /// Forge owns Minecraft's class loader. The small Opus bootstrap keeps
    /// identity arguments off the process command line, while the Opus coremod
    /// and typed client mod are staged into the isolated Forge `mods` directory.
    ForgeBootstrap {
        bootstrap_jar: PathBuf,
        coremod_jar: PathBuf,
        client_mod_jar: PathBuf,
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
    client_mod: crate::DownloadSpec,
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
            client_mod: forge_client_mod_spec(),
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
            prefixed_path_argument("-Drbw.diagnostics.file=", &diagnostics_path),
            prefixed_path_argument("-Xloggc:", &gc_log_path),
            OsString::from("-XX:+PrintGCDetails"),
            OsString::from("-XX:+PrintGCDateStamps"),
            prefixed_path_argument("-XX:ErrorFile=", &log_directory.join("jvm_crash_%p.log")),
        ]);
        if let Some(utility_settings_file) = &options.utility_settings_file {
            jvm_arguments.push(prefixed_path_argument(
                "-Drbw.utility.settings.file=",
                utility_settings_file,
            ));
        }
        if let Some(brand_wordmark_file) = &options.brand_wordmark_file {
            jvm_arguments.push(prefixed_path_argument(
                "-Drbw.brand.wordmark.file=",
                brand_wordmark_file,
            ));
        }
        if let (Some(logging), Some(path)) = (&minecraft.version.logging, &minecraft.logging_config)
        {
            let filtered_path = session_directory.join("log4j-rbw.xml");
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

        let (system_classpath, main_class, game_arguments, stdin_payload) = match mode {
            LaunchMode::Direct => {
                if identity.is_authenticated() {
                    return Err(LaunchError::AuthenticatedDirectLaunchForbidden);
                }
                (
                    minecraft.classpath.clone(),
                    minecraft.main_class.clone(),
                    vanilla_arguments,
                    None,
                )
            }
            LaunchMode::Bootstrap { classpath } => {
                if minecraft.runtime_id == FORGE_RUNTIME_ID {
                    return Err(LaunchError::ForgeBootstrapRequired);
                }
                validate_bootstrap_classpath(classpath)?;
                let absolute_bootstrap_classpath = canonicalize_classpath(classpath)?;
                let game_classpath_file = session_directory.join("game-classpath.txt");
                write_classpath_file(&game_classpath_file, &minecraft.classpath)?;
                let bootstrap_arguments = vec![
                    OsString::from("--rbw-game-main"),
                    OsString::from(&minecraft.main_class),
                    OsString::from("--rbw-game-classpath-file"),
                    game_classpath_file.into_os_string(),
                    OsString::from("--rbw-game-arguments-stdin"),
                ];
                let payload = encode_game_arguments(&vanilla_arguments)?;
                (
                    absolute_bootstrap_classpath,
                    "dev.rbw.bootstrap.BootstrapMain".to_owned(),
                    bootstrap_arguments,
                    Some(payload),
                )
            }
            LaunchMode::ForgeBootstrap {
                bootstrap_jar,
                coremod_jar,
                client_mod_jar,
            } => {
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
                let staged_optifine =
                    stage_forge_optifine(layout, optifine_jar, &contract.optifine)?;
                let staged_coremod = stage_forge_coremod(layout, coremod_jar, &contract.coremod)?;
                let staged_client_mod =
                    stage_forge_client_mod(layout, client_mod_jar, &contract.client_mod)?;
                validate_managed_forge_mods(
                    layout,
                    &staged_optifine,
                    &staged_coremod,
                    &staged_client_mod,
                )?;

                let bootstrap = canonicalize_single_bootstrap_jar(bootstrap_jar)?;
                let game_classpath = canonicalize_classpath(&minecraft.classpath)?;
                let mut system_classpath = Vec::with_capacity(game_classpath.len() + 1);
                system_classpath.push(bootstrap);
                system_classpath.extend(game_classpath);
                let bootstrap_arguments = vec![OsString::from("--rbw-game-arguments-stdin")];
                let payload = encode_game_arguments(&vanilla_arguments)?;
                (
                    system_classpath,
                    "dev.rbw.bootstrap.ForgeBootstrapMain".to_owned(),
                    bootstrap_arguments,
                    Some(payload),
                )
            }
        };
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
        LaunchMode::Direct => "direct",
        LaunchMode::Bootstrap { .. } => "bootstrap",
        LaunchMode::ForgeBootstrap { .. } => "forge-bootstrap",
    }
}

fn platform_jvm_arguments(os: OperatingSystem) -> Vec<OsString> {
    match os {
        OperatingSystem::MacOs => vec![OsString::from("-Xdock:name=Opus Client")],
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

fn validate_bootstrap_classpath(classpath: &[PathBuf]) -> Result<(), LaunchError> {
    if classpath.is_empty() {
        return Err(LaunchError::EmptyBootstrapClasspath);
    }
    let has_bootstrap = classpath.iter().any(|path| {
        path.file_name()
            .and_then(OsStr::to_str)
            .map(|name| name.starts_with("opus-bootstrap-") && name.ends_with(".jar"))
            .unwrap_or(false)
    });
    if !has_bootstrap {
        return Err(LaunchError::MissingBootstrapJar);
    }
    for entry in classpath {
        if !entry.is_file() {
            return Err(LaunchError::MissingBootstrapEntry(entry.clone()));
        }
    }
    Ok(())
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
    let temporary = mods_directory.join(format!(".rbw-optifine.part-{}", std::process::id()));
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

fn forge_client_mod_spec() -> crate::DownloadSpec {
    crate::DownloadSpec {
        // This is not a download endpoint: the checked-in release artifact is
        // verified locally before staging beside the managed Forge coremod.
        url: "https://opus.invalid/release/opus-client-legacy-1.8.9.jar".to_owned(),
        sha1: FORGE_CLIENT_MOD_SHA1.to_owned(),
        size: Some(FORGE_CLIENT_MOD_SIZE),
    }
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
    if !manifest_has_attribute(&manifest, "FMLCorePlugin", "dev.rbw.forge.RbwLoadingPlugin")
        || !manifest_has_attribute(&manifest, "FMLCorePluginContainsFMLMod", "false")
    {
        return Err(LaunchError::InvalidForgeCoremod(path.to_path_buf()));
    }
    Ok(())
}

fn stage_forge_client_mod(
    layout: &MinecraftLayout,
    source: &Path,
    spec: &crate::DownloadSpec,
) -> Result<PathBuf, LaunchError> {
    validate_forge_client_mod(source, spec)?;
    let mods_directory = layout.mods_dir();
    fs::create_dir_all(&mods_directory)?;
    let destination = mods_directory.join("opus-client-legacy-1.8.9.jar");
    let temporary = mods_directory.join(format!(".opus-client.part-{}", std::process::id()));
    if temporary.exists() {
        fs::remove_file(&temporary)?;
    }
    let result = (|| -> Result<(), LaunchError> {
        fs::copy(source, &temporary)?;
        validate_forge_client_mod(&temporary, spec)?;
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

fn validate_forge_client_mod(path: &Path, spec: &crate::DownloadSpec) -> Result<(), LaunchError> {
    if !path.is_file() {
        return Err(LaunchError::MissingForgeClientMod(path.to_path_buf()));
    }
    if !verify_file(path, spec)
        .map_err(|_| LaunchError::InvalidForgeClientMod(path.to_path_buf()))?
    {
        return Err(LaunchError::InvalidForgeClientMod(path.to_path_buf()));
    }

    let file =
        File::open(path).map_err(|_| LaunchError::InvalidForgeClientMod(path.to_path_buf()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| LaunchError::InvalidForgeClientMod(path.to_path_buf()))?;
    let mut manifest = String::new();
    archive
        .by_name("META-INF/MANIFEST.MF")
        .map_err(|_| LaunchError::InvalidForgeClientMod(path.to_path_buf()))?
        .read_to_string(&mut manifest)
        .map_err(|_| LaunchError::InvalidForgeClientMod(path.to_path_buf()))?;
    if manifest_has_key(&manifest, "FMLCorePlugin")
        || manifest_has_key(&manifest, "FMLCorePluginContainsFMLMod")
        || archive
            .by_name("dev/rbw/client/RbwClientMod.class")
            .is_err()
    {
        return Err(LaunchError::InvalidForgeClientMod(path.to_path_buf()));
    }
    let mut metadata = String::new();
    archive
        .by_name("mcmod.info")
        .map_err(|_| LaunchError::InvalidForgeClientMod(path.to_path_buf()))?
        .read_to_string(&mut metadata)
        .map_err(|_| LaunchError::InvalidForgeClientMod(path.to_path_buf()))?;
    if !metadata.contains("\"modid\": \"rbwclient\"") {
        return Err(LaunchError::InvalidForgeClientMod(path.to_path_buf()));
    }
    Ok(())
}

fn manifest_has_attribute(manifest: &str, key: &str, expected: &str) -> bool {
    let target = format!("{key}: {expected}");
    manifest
        .lines()
        .any(|line| line.trim_end_matches('\r') == target)
}

fn manifest_has_key(manifest: &str, key: &str) -> bool {
    let prefix = format!("{key}:");
    manifest
        .lines()
        .any(|line| line.trim_end_matches('\r').starts_with(&prefix))
}

fn validate_managed_forge_mods(
    layout: &MinecraftLayout,
    optifine_jar: &Path,
    coremod_jar: &Path,
    client_mod_jar: &Path,
) -> Result<(), LaunchError> {
    let expected_optifine =
        fs::canonicalize(optifine_jar).map_err(|source| LaunchError::CanonicalizeClasspath {
            path: optifine_jar.to_path_buf(),
            source,
        })?;
    let expected_coremod =
        fs::canonicalize(coremod_jar).map_err(|source| LaunchError::CanonicalizeClasspath {
            path: coremod_jar.to_path_buf(),
            source,
        })?;
    let expected_client_mod =
        fs::canonicalize(client_mod_jar).map_err(|source| LaunchError::CanonicalizeClasspath {
            path: client_mod_jar.to_path_buf(),
            source,
        })?;
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
        if canonical != expected_optifine
            && canonical != expected_coremod
            && canonical != expected_client_mod
        {
            return Err(LaunchError::UnmanagedForgeMod(candidate));
        }
    }
    Ok(())
}

fn write_classpath_file(path: &Path, classpath: &[PathBuf]) -> Result<(), LaunchError> {
    if classpath.is_empty() {
        return Err(LaunchError::InvalidClasspath(
            "game classpath is empty".to_owned(),
        ));
    }
    let mut content = String::new();
    for entry in classpath {
        if !entry.is_file() {
            return Err(LaunchError::MissingGameClasspathEntry(entry.clone()));
        }
        let text = path_text(entry)?;
        if text.contains('\n') || text.contains('\r') {
            return Err(LaunchError::InvalidClasspath(
                "classpath entry contains a newline".to_owned(),
            ));
        }
        content.push_str(text);
        content.push('\n');
    }
    create_private_file(path, content.as_bytes())
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
    insert_jvm_argument_before_classpath(
        &mut plan.jvm_arguments,
        OsString::from(format!("-Drbw.game.statusFile={}", status_path.display())),
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
) -> Vec<OsString> {
    let mut working_directory_environment = OsString::from("RBW_GAME_WORKDIR=");
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
    let startup_deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let game_status = fs::read_to_string(status_path)?.trim().to_owned();
        match game_status.as_str() {
            "running" => return wait_for_macos_game_exit(status_path),
            "exited" | "terminated" => return Ok(game_status),
            "failed" => return Err(LaunchError::MacosGameFailed),
            "starting" | "" => {
                if Instant::now() >= startup_deadline {
                    return Err(LaunchError::MacosGameStartupTimedOut);
                }
                thread::sleep(Duration::from_millis(100));
            }
            other => return Err(LaunchError::MacosGameStatusInvalid(other.to_owned())),
        }
    }
}

fn wait_for_macos_game_exit(status_path: &Path) -> Result<String, LaunchError> {
    loop {
        let game_status = fs::read_to_string(status_path)?.trim().to_owned();
        match game_status.as_str() {
            "exited" | "terminated" => return Ok(game_status),
            "failed" => return Err(LaunchError::MacosGameFailed),
            "running" | "starting" | "" => thread::sleep(Duration::from_millis(100)),
            other => return Err(LaunchError::MacosGameStatusInvalid(other.to_owned())),
        }
    }
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
    let executable = game_app.join("Contents/MacOS/Opus Client");
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
    #[error("bootstrap classpath is empty")]
    EmptyBootstrapClasspath,
    #[error("bootstrap classpath does not contain opus-bootstrap-*.jar")]
    MissingBootstrapJar,
    #[error("bootstrap classpath entry does not exist: {path}", path = .0.display())]
    MissingBootstrapEntry(PathBuf),
    #[error("Forge launch requires the pinned Forge + OptiFine runtime, got {0}")]
    ForgeRuntimeRequired(String),
    #[error("the Forge runtime must use the Forge bootstrap, not the legacy Opus bootstrap")]
    ForgeBootstrapRequired,
    #[error("Forge launch requires an imported OptiFine 1.8.9 HD U M5 JAR")]
    OptiFineRequired,
    #[error("managed OptiFine failed integrity verification: {}", .0.display())]
    InvalidOptiFine(PathBuf),
    #[error("Opus Forge coremod is missing: {}", .0.display())]
    MissingForgeCoremod(PathBuf),
    #[error("Opus Forge coremod is invalid: {}", .0.display())]
    InvalidForgeCoremod(PathBuf),
    #[error("Opus Forge client mod is missing: {}", .0.display())]
    MissingForgeClientMod(PathBuf),
    #[error("Opus Forge client mod is invalid: {}", .0.display())]
    InvalidForgeClientMod(PathBuf),
    #[error("unmanaged JAR or directory in isolated Forge mods directory: {}", .0.display())]
    UnmanagedForgeMod(PathBuf),
    #[error("game classpath entry does not exist: {path}", path = .0.display())]
    MissingGameClasspathEntry(PathBuf),
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
    #[error("authenticated sessions must launch through Opus bootstrap")]
    AuthenticatedDirectLaunchForbidden,
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
    #[error("Minecraft did not report a completed game lifecycle")]
    MacosGameDidNotExit,
    #[error("Minecraft did not report that startup completed within 30 seconds")]
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
        client_mod_jar: PathBuf,
        optifine_jar: PathBuf,
        contract: ForgeLaunchContract,
    }

    impl ForgeFixture {
        fn new() -> Self {
            let temporary = tempfile::tempdir().unwrap();
            let layout = MinecraftLayout::new(
                rbw_platform::RbwPaths::from_root(temporary.path().join("rbw")).unwrap(),
            );
            let bootstrap_jar = temporary.path().join("opus-bootstrap-0.0.1.jar");
            let forge_classpath = temporary.path().join("forge-runtime.jar");
            let vanilla_classpath = temporary.path().join("vanilla-client.jar");
            let coremod_jar = temporary.path().join("opus-runtime-legacy-1.8.9-0.0.1.jar");
            let client_mod_jar = temporary.path().join("opus-client-legacy-1.8.9-0.0.1.jar");
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
                        b"Manifest-Version: 1.0\r\nFMLCorePlugin: dev.rbw.forge.RbwLoadingPlugin\r\nFMLCorePluginContainsFMLMod: false\r\n\r\n",
                    ),
                    ("dev/rbw/forge/RbwLoadingPlugin.class", b"coremod fixture"),
                ],
            );
            write_zip(
                &client_mod_jar,
                &[
                    ("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\r\n\r\n"),
                    ("mcmod.info", b"[{\"modid\": \"rbwclient\"}]"),
                    ("dev/rbw/client/RbwClientMod.class", b"client mod fixture"),
                ],
            );

            let optifine_jar = layout.optifine_mod();
            fs::create_dir_all(optifine_jar.parent().unwrap()).unwrap();
            fs::write(&optifine_jar, b"optifine fixture").unwrap();
            let contract = ForgeLaunchContract {
                optifine: fixture_spec(&optifine_jar),
                coremod: fixture_spec(&coremod_jar),
                client_mod: fixture_spec(&client_mod_jar),
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
                    host_arch: rbw_platform::Architecture::X86_64,
                    game_arch: rbw_platform::Architecture::X86_64,
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
                client_mod_jar,
                optifine_jar,
                contract,
            }
        }

        fn mode(&self) -> LaunchMode {
            LaunchMode::ForgeBootstrap {
                bootstrap_jar: self.bootstrap_jar.clone(),
                coremod_jar: self.coremod_jar.clone(),
                client_mod_jar: self.client_mod_jar.clone(),
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
    fn macos_jvm_arguments_set_the_dock_name() {
        assert_eq!(
            platform_jvm_arguments(OperatingSystem::MacOs),
            vec![OsString::from("-Xdock:name=Opus Client",)]
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
            Path::new("/tmp/Opus Client.app"),
            Path::new("/tmp/java"),
            &[OsString::from("-Xmx2G")],
            "dev.rbw.bootstrap.ForgeBootstrapMain",
            &[OsString::from("--rbw-game-arguments-stdin")],
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
    fn lifecycle_property_is_inserted_before_the_classpath() {
        let mut arguments = vec![
            OsString::from("-Xmx2048M"),
            OsString::from("-cp"),
            OsString::from("game.jar"),
        ];
        insert_jvm_argument_before_classpath(
            &mut arguments,
            OsString::from("-Drbw.game.statusFile=/tmp/game.status"),
        );
        assert_eq!(
            arguments,
            vec![
                OsString::from("-Xmx2048M"),
                OsString::from("-Drbw.game.statusFile=/tmp/game.status"),
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
            host_arch: rbw_platform::Architecture::Aarch64,
            game_arch: rbw_platform::Architecture::X86_64,
        };
        let mode = LaunchMode::Bootstrap {
            classpath: vec![PathBuf::from("/private/bootstrap.jar")],
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
        assert!(!manifest.contains("bootstrap.jar"));
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
    fn derived_logging_config_blocks_session_token_messages() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("official.xml");
        let destination = temp.path().join("rbw.xml");
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
        let destination = temp.path().join("rbw.xml");
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
    fn forge_bootstrap_plan_stages_both_rbw_mods_and_keeps_them_out_of_system_classpath() {
        let fixture = ForgeFixture::new();
        let mode = fixture.mode();
        let identity = GameIdentity::offline("ForgeFixture").unwrap();
        let plan = LaunchPlan::build_with_forge_contract(
            fixture.request(&identity, &mode),
            Some(&fixture.contract),
        )
        .unwrap();

        assert_eq!(plan.main_class, "dev.rbw.bootstrap.ForgeBootstrapMain");
        assert_eq!(
            plan.game_arguments,
            vec![OsString::from("--rbw-game-arguments-stdin")]
        );
        assert!(plan.stdin_payload.is_some());

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
        assert!(!classpath.contains(&fs::canonicalize(&fixture.client_mod_jar).unwrap()));

        let staged_coremod = fixture
            .layout
            .mods_dir()
            .join("opus-runtime-legacy-1.8.9.jar");
        assert_eq!(
            fs::read(&staged_coremod).unwrap(),
            fs::read(&fixture.coremod_jar).unwrap()
        );
        validate_forge_coremod(&staged_coremod, &fixture.contract.coremod).unwrap();

        let staged_client_mod = fixture
            .layout
            .mods_dir()
            .join("opus-client-legacy-1.8.9.jar");
        assert_eq!(
            fs::read(&staged_client_mod).unwrap(),
            fs::read(&fixture.client_mod_jar).unwrap()
        );
        validate_forge_client_mod(&staged_client_mod, &fixture.contract.client_mod).unwrap();
        validate_managed_forge_mods(
            &fixture.layout,
            &fixture.optifine_jar,
            &staged_coremod,
            &staged_client_mod,
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
