use anyhow::Result;
use clap::{Parser, Subcommand};
use opus_auth::{MICROSOFT_CLIENT_ID_ENV, MicrosoftAuthenticator, RefreshTokenStore};
use opus_engine::{
    ForgeUiArtifacts, GameIdentity, Installer, LaunchMode, LaunchOptions, LaunchPlan,
    MinecraftLayout, launch_game,
};
#[cfg(all(feature = "opus-owned-m3-preview", debug_assertions))]
use opus_engine::{
    OpusOwnedM3Preview, cleanup_opus_owned_m3_preview_control_files,
    discard_opus_owned_m3_preview_control_files, launch_game_via_macos_app,
    validate_macos_game_app,
};
#[cfg(all(feature = "opus-owned-m3-preview", debug_assertions))]
use opus_platform::macos_console_is_locked;
use opus_platform::{OperatingSystem, OpusPaths, Platform};
use std::path::{Path, PathBuf};

const DEFAULT_BOOTSTRAP_DIR: &str = "desktop/src-tauri/resources/bootstrap";
#[cfg(all(feature = "opus-owned-m3-preview", debug_assertions))]
const DEFAULT_M3_MACOS_GAME_APP: &str = "desktop/src-tauri/resources/Minecraft.app";

#[derive(Debug, Parser)]
#[command(
    name = "opus",
    version,
    about = "Opus Launcher diagnostics for Minecraft 1.8.9"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate the host before downloading or launching anything.
    Doctor,
    /// Install the immutable Forge 1.8.9 runtime into the Opus directory.
    Install,
    /// Verify and import a user-provided OptiFine 1.8.9 HD U M5 JAR.
    ImportOptifine {
        /// Path to the local JAR obtained from OptiFine.
        path: PathBuf,
    },
    /// Manage the CLI Microsoft Minecraft credential.
    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },
    /// Launch Forge + OptiFine 1.8.9.
    Launch {
        /// Username used only with --offline.
        #[arg(long, default_value = "OpusDev")]
        username: String,
        /// Use a development identity; online-mode servers will reject it.
        #[arg(long)]
        offline: bool,
        #[arg(long, default_value_t = 2048)]
        max_memory_mib: u32,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, default_value = DEFAULT_BOOTSTRAP_DIR)]
        bootstrap_dir: PathBuf,
    },
    /// Developer-only OPUS-owned M3 preview. This command exists only in a
    /// debug build compiled with `opus-owned-m3-preview`; it is not a normal
    /// launcher or third-party-client transport.
    #[cfg(all(feature = "opus-owned-m3-preview", debug_assertions))]
    OwnedM3Preview {
        /// Required acknowledgement that this is an OPUS-owned preview target,
        /// not a certified client integration.
        #[arg(long)]
        acknowledge_preview_boundary: bool,
        /// Username used only with --offline.
        #[arg(long, default_value = "OpusM3Dev")]
        username: String,
        /// Use a development identity; online-mode servers will reject it.
        #[arg(long)]
        offline: bool,
        #[arg(long, default_value_t = 2048)]
        max_memory_mib: u32,
        /// Build a private session plan but do not start Minecraft.
        #[arg(long)]
        dry_run: bool,
        #[arg(long, default_value = DEFAULT_BOOTSTRAP_DIR)]
        bootstrap_dir: PathBuf,
        /// The source-controlled `opus-owned-m3-bootstrap-*.jar`.
        #[arg(long)]
        preview_bootstrap: PathBuf,
        /// A single same-architecture `libopus-runtime.dylib`.
        #[arg(long)]
        native_runtime: PathBuf,
        /// Version recorded with the launcher-bound OPUS-owned preview build.
        #[arg(long, default_value = "0.1.0")]
        target_version: String,
        #[arg(long, default_value = "0.1.0")]
        injector_version: String,
        #[arg(long, default_value = "0.1.0")]
        native_runtime_version: String,
        #[arg(long, default_value = "not-built")]
        java_runtime_version: String,
        /// The OPUS-owned macOS host app used to start the managed game JVM.
        #[arg(long, default_value = DEFAULT_M3_MACOS_GAME_APP)]
        macos_game_app: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum AccountCommand {
    /// Sign in through Microsoft's browser/device flow and verify ownership.
    Login,
    /// Report whether a refresh credential exists in Opus's local vault.
    Status,
    /// Remove the stored Microsoft refresh credential.
    Logout,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Doctor => doctor(),
        Command::Install => install(),
        Command::ImportOptifine { path } => import_optifine(&path),
        Command::Account { command } => account(command),
        Command::Launch {
            username,
            offline,
            max_memory_mib,
            dry_run,
            bootstrap_dir,
        } => launch(&username, offline, max_memory_mib, dry_run, &bootstrap_dir),
        #[cfg(all(feature = "opus-owned-m3-preview", debug_assertions))]
        Command::OwnedM3Preview {
            acknowledge_preview_boundary,
            username,
            offline,
            max_memory_mib,
            dry_run,
            bootstrap_dir,
            preview_bootstrap,
            native_runtime,
            target_version,
            injector_version,
            native_runtime_version,
            java_runtime_version,
            macos_game_app,
        } => launch_owned_m3_preview(
            acknowledge_preview_boundary,
            &username,
            offline,
            max_memory_mib,
            dry_run,
            &bootstrap_dir,
            preview_bootstrap,
            native_runtime,
            target_version,
            injector_version,
            native_runtime_version,
            java_runtime_version,
            macos_game_app,
        ),
    }
}

fn launch(
    username: &str,
    offline: bool,
    max_memory_mib: u32,
    dry_run: bool,
    bootstrap_dir: &Path,
) -> Result<()> {
    let platform = Platform::detect()?;
    if platform.os == OperatingSystem::MacOs && !dry_run {
        anyhow::bail!(
            "CLI game launch is disabled on macOS. Use the packaged Opus Launcher.app, which starts managed Java through the bundled Minecraft host. --dry-run remains available for diagnostics."
        );
    }
    let paths = OpusPaths::discover()?;
    let layout = MinecraftLayout::new(paths);
    let installer = Installer::new(layout.clone(), platform)?;

    println!("Opus Launcher");
    println!("verifying Forge + OptiFine 1.8.9 installation");
    let report = installer.prepare()?;
    println!(
        "installation ready (downloaded {}, cached {})",
        report.downloaded_files, report.cached_files
    );

    let identity = if offline {
        println!("identity: offline development profile {username}");
        GameIdentity::offline(username)?
    } else {
        let authenticator = MicrosoftAuthenticator::from_environment()?;
        let store = RefreshTokenStore::new()?;
        println!("refreshing Microsoft Minecraft session");
        let account = authenticator.refresh_session(&store)?;
        account.save_refresh_token(&store)?;
        println!("identity: {}", account.session.redacted_summary());
        GameIdentity::authenticated(
            &account.session.username,
            &account.session.uuid,
            account.session.access_token.clone(),
            &account.session.user_type,
        )?
    };

    let artifacts = forge_bootstrap_artifacts(bootstrap_dir)?;
    println!("mode: Forge + OptiFine + Web Surface V2");
    let mode = LaunchMode::ForgeBootstrap {
        bootstrap_jar: artifacts.bootstrap_jar,
        coremod_jar: artifacts.coremod_jar,
        ui: artifacts.ui,
    };
    // Keep CLI launches on the same identity-isolated game directory as the
    // desktop launcher. A shared `game/mods` directory can contain artifacts
    // from an older build and must never be part of the Forge discovery path.
    let mut instance_paths = layout.paths.clone();
    instance_paths.game = instance_paths
        .root
        .join("instances")
        .join(&identity.uuid)
        .join("game");
    let instance_layout = MinecraftLayout::new(instance_paths);
    let plan = LaunchPlan::build(
        &instance_layout,
        platform,
        &report.minecraft,
        &report.java,
        &identity,
        &LaunchOptions {
            max_memory_mib,
            ..LaunchOptions::default()
        },
        &mode,
    )?;
    println!("session: {}", plan.session_id);
    println!("launch plan: {}", plan.redacted_summary());
    if dry_run {
        println!("dry run complete; game process was not started");
        return Ok(());
    }

    println!("starting Minecraft");
    let result = launch_game(plan)?;
    println!("Minecraft exited with {}", result.status);
    println!("logs: {}", result.log_directory.display());
    if !result.status.success() {
        anyhow::bail!("Minecraft exited unsuccessfully");
    }
    Ok(())
}

#[cfg(all(feature = "opus-owned-m3-preview", debug_assertions))]
#[allow(clippy::too_many_arguments)]
fn launch_owned_m3_preview(
    acknowledge_preview_boundary: bool,
    username: &str,
    offline: bool,
    max_memory_mib: u32,
    dry_run: bool,
    bootstrap_dir: &Path,
    preview_bootstrap: PathBuf,
    native_runtime: PathBuf,
    target_version: String,
    injector_version: String,
    native_runtime_version: String,
    java_runtime_version: String,
    macos_game_app: PathBuf,
) -> Result<()> {
    if !acknowledge_preview_boundary {
        anyhow::bail!(
            "Pass --acknowledge-preview-boundary to confirm this starts only an OPUS-owned M3 preview target. It does not certify or modify third-party clients."
        );
    }

    let platform = Platform::detect()?;
    if platform.os != OperatingSystem::MacOs {
        anyhow::bail!(
            "The OPUS-owned M3 preview launcher entrypoint is currently defined only for macOS."
        );
    }
    // Reject an incomplete host bundle before discovering or preparing the
    // managed game installation. `prepare` may otherwise create or update the
    // managed rollback host before a preview can report this input error.
    validate_macos_game_app(&macos_game_app)?;
    if !dry_run && macos_console_is_locked()? {
        anyhow::bail!(
            "macOS console is locked; unlock the Mac before launching the OPUS-owned M3 preview."
        );
    }
    if !platform.translation_available()? {
        anyhow::bail!("Rosetta is required for the managed Minecraft 1.8.9 runtime on this Mac.");
    }

    let paths = OpusPaths::discover()?;
    let layout = MinecraftLayout::new(paths);
    let installer = Installer::new(layout.clone(), platform)?;
    println!("Opus Launcher developer preview");
    println!("verifying the managed Forge 1.8.9 rollback host");
    let report = installer.prepare()?;

    let identity = if offline {
        println!("identity: offline development profile {username}");
        GameIdentity::offline(username)?
    } else {
        let authenticator = MicrosoftAuthenticator::from_environment()?;
        let store = RefreshTokenStore::new()?;
        println!("refreshing Microsoft Minecraft session");
        let account = authenticator.refresh_session(&store)?;
        account.save_refresh_token(&store)?;
        println!("identity: {}", account.session.redacted_summary());
        GameIdentity::authenticated(
            &account.session.username,
            &account.session.uuid,
            account.session.access_token.clone(),
            &account.session.user_type,
        )?
    };

    let artifacts = forge_bootstrap_artifacts(bootstrap_dir)?;
    let preview = OpusOwnedM3Preview::new(
        preview_bootstrap,
        native_runtime,
        target_version,
        injector_version,
        native_runtime_version,
        java_runtime_version,
    )?;
    let mode = LaunchMode::OpusOwnedM3Preview {
        bootstrap_jar: artifacts.bootstrap_jar,
        lwjgl_macos_compat_jar: artifacts.ui.lwjgl_macos_compat_jar,
        preview,
    };

    let mut instance_paths = layout.paths.clone();
    instance_paths.game = instance_paths
        .root
        .join("instances")
        .join(&identity.uuid)
        .join("game");
    let instance_layout = MinecraftLayout::new(instance_paths);
    let plan = LaunchPlan::build(
        &instance_layout,
        platform,
        &report.minecraft,
        &report.java,
        &identity,
        &LaunchOptions {
            max_memory_mib,
            ..LaunchOptions::default()
        },
        &mode,
    )?;
    let session_directory = instance_layout.paths.sessions.join(&plan.session_id);
    let descriptor = session_directory.join("opus-owned-m3-target.properties");
    let log_directory = plan.log_directory.clone();
    println!("mode: developer-only OPUS-owned M3 preview");
    println!("session: {}", plan.session_id);
    println!("private control descriptor: {}", descriptor.display());
    println!("launch plan: {}", plan.redacted_summary());
    if dry_run {
        discard_opus_owned_m3_preview_control_files(&session_directory)?;
        println!("dry run complete; game process was not started");
        return Ok(());
    }

    println!("starting OPUS-owned preview target");
    let launch_result = launch_game_via_macos_app(plan, &macos_game_app);
    let cleanup_result =
        cleanup_opus_owned_m3_preview_control_files(&session_directory, &log_directory);
    match (launch_result, cleanup_result) {
        (Ok(result), Ok(true)) => {
            println!("Minecraft exited with {}", result.status);
            println!("logs: {}", result.log_directory.display());
            if !result.status.success() {
                anyhow::bail!("Minecraft exited unsuccessfully");
            }
        }
        (Ok(_), Ok(false)) => {
            anyhow::bail!(
                "The preview launcher did not confirm a terminal target state; private control files were retained."
            );
        }
        (Ok(_), Err(cleanup_error)) => {
            return Err(anyhow::Error::new(cleanup_error)
                .context("The game ended, but private M3 control-file cleanup failed"));
        }
        (Err(launch_error), Ok(true)) => {
            return Err(anyhow::Error::new(launch_error)
                .context("The terminal preview session's private control files were removed"));
        }
        (Err(launch_error), Ok(false)) => {
            return Err(anyhow::Error::new(launch_error).context(
                "Private M3 control files were retained because the target may still be alive",
            ));
        }
        (Err(launch_error), Err(cleanup_error)) => {
            return Err(anyhow::Error::new(launch_error).context(format!(
                "The preview launcher also failed to clean private M3 control files: {cleanup_error}"
            )));
        }
    }
    Ok(())
}

fn account(command: AccountCommand) -> Result<()> {
    let store = RefreshTokenStore::new()?;
    match command {
        AccountCommand::Login => {
            let authenticator = MicrosoftAuthenticator::from_environment()?;
            let authorization = authenticator.begin_device_authorization()?;
            println!("Microsoft sign-in");
            println!("URL: {}", authorization.verification_uri);
            println!("Code: {}", authorization.user_code);
            println!("{}", authorization.message);
            if let Err(error) = open::that(&authorization.verification_uri) {
                eprintln!("Could not open the browser automatically: {error}");
            }
            let account = authenticator.complete_device_authorization(&authorization)?;
            account.save_refresh_token(&store)?;
            println!("Signed in: {}", account.session.redacted_summary());
            println!("Minecraft Java ownership verified");
            Ok(())
        }
        AccountCommand::Status => {
            if store.load()?.is_some() {
                println!("A Microsoft refresh credential is stored in Opus's local vault");
            } else {
                println!("No Microsoft account is stored");
            }
            Ok(())
        }
        AccountCommand::Logout => {
            if store.delete()? {
                println!("Microsoft credential removed from Opus's local vault");
            } else {
                println!("No Microsoft credential was stored");
            }
            Ok(())
        }
    }
}

struct ForgeBootstrapArtifacts {
    bootstrap_jar: PathBuf,
    coremod_jar: PathBuf,
    ui: ForgeUiArtifacts,
}

fn forge_bootstrap_artifacts(directory: &Path) -> Result<ForgeBootstrapArtifacts> {
    if !directory.is_dir() {
        anyhow::bail!(
            "OPUS Runtime artifacts are missing at {}; stage a verified Runtime output with scripts/prepare-desktop-assets.sh",
            directory.display()
        );
    }

    let mut bootstrap_jar = None;
    let mut coremod_jar = None;
    let mut native_ui_jar = None;
    let mut webview_helper_app = None;
    let mut lwjgl_macos_compat_jar = None;
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.file_name().and_then(|value| value.to_str()) == Some("opus-webview-helper.app") {
            if !path.is_dir() || webview_helper_app.replace(path).is_some() {
                anyhow::bail!(
                    "bootstrap directory contains an invalid or duplicate Web Surface helper"
                );
            }
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("jar") {
            continue;
        }
        if !path.is_file() {
            anyhow::bail!(
                "Forge bootstrap artifact is not a regular file: {}",
                path.display()
            );
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| anyhow::anyhow!("Forge bootstrap artifact has an invalid name"))?;
        if name.starts_with("opus-bootstrap-") && name.ends_with(".jar") {
            if bootstrap_jar.replace(path).is_some() {
                anyhow::bail!("bootstrap directory contains multiple bootstrap JARs");
            }
        } else if name.starts_with("opus-runtime-legacy-1.8.9-") && name.ends_with(".jar") {
            if coremod_jar.replace(path).is_some() {
                anyhow::bail!("bootstrap directory contains multiple Forge coremod JARs");
            }
        } else if name == "opus-native-ui-1.8.9-0.1.0.jar" {
            if native_ui_jar.replace(path).is_some() {
                anyhow::bail!("bootstrap directory contains multiple Web Surface client JARs");
            }
        } else if name == "opus-lwjgl-macos-compat-2.9.2-nightly-20140822.jar" {
            if lwjgl_macos_compat_jar.replace(path).is_some() {
                anyhow::bail!("bootstrap directory contains multiple LWJGL compatibility JARs");
            }
        } else {
            anyhow::bail!("bootstrap directory contains unexpected JAR: {name}");
        }
    }
    Ok(ForgeBootstrapArtifacts {
        bootstrap_jar: bootstrap_jar
            .ok_or_else(|| anyhow::anyhow!("Forge bootstrap JAR is missing"))?,
        coremod_jar: coremod_jar.ok_or_else(|| anyhow::anyhow!("Forge coremod JAR is missing"))?,
        ui: ForgeUiArtifacts {
            native_ui_jar: native_ui_jar
                .ok_or_else(|| anyhow::anyhow!("Web Surface client JAR is missing"))?,
            webview_helper_app: webview_helper_app
                .ok_or_else(|| anyhow::anyhow!("Web Surface helper is missing"))?,
            lwjgl_macos_compat_jar: lwjgl_macos_compat_jar
                .ok_or_else(|| anyhow::anyhow!("LWJGL compatibility JAR is missing"))?,
        },
    })
}

fn install() -> Result<()> {
    let platform = Platform::detect()?;
    let paths = OpusPaths::discover()?;
    let root = paths.root.clone();
    let installer = Installer::new(MinecraftLayout::new(paths), platform)?;

    println!("Opus Launcher installer");
    println!("version: Forge + OptiFine 1.8.9 (locked)");
    println!("data directory: {}", root.display());
    let report = installer.install()?;
    println!("Forge Minecraft: {}", report.minecraft.version.id);
    println!("Java: {}", report.java.version_name);
    println!("Java executable: {}", report.java.executable.display());
    println!("downloaded files: {}", report.downloaded_files);
    println!("verified cached files: {}", report.cached_files);
    println!("installation verified");
    if report.minecraft.optifine_jar.is_none() {
        println!("OptiFine: import your local 1.8.9 HD U M5 JAR before launching");
    }
    Ok(())
}

fn import_optifine(path: &Path) -> Result<()> {
    let platform = Platform::detect()?;
    let paths = OpusPaths::discover()?;
    let installer = Installer::new(MinecraftLayout::new(paths), platform)?;
    let destination = installer.import_optifine(path)?;
    println!("OptiFine verified and imported: {}", destination.display());
    Ok(())
}

fn doctor() -> Result<()> {
    let platform = Platform::detect()?;
    let paths = OpusPaths::discover()?;
    let translation_available = platform.translation_available()?;

    println!("Opus Launcher doctor");
    println!("host: {} {}", platform.os, platform.host_arch);
    println!("game runtime: {} {}", platform.os, platform.game_arch);
    println!("translation required: {}", platform.requires_translation());
    println!("translation available: {translation_available}");
    println!(
        "runtime metadata key: {}",
        platform.minecraft_runtime_key()?
    );
    println!("data directory: {}", paths.root.display());
    let installer = Installer::new(MinecraftLayout::new(paths), platform)?;
    match installer.load_cached() {
        Ok(report) => println!(
            "managed runtime: ready (Forge Minecraft {}, Java {}, {} verified artifacts)",
            report.minecraft.version.id, report.java.version_name, report.cached_files
        ),
        Err(error) => println!("managed runtime: not ready ({error})"),
    }
    match forge_bootstrap_artifacts(Path::new(DEFAULT_BOOTSTRAP_DIR)) {
        Ok(_) => println!("Opus Forge bootstrap: ready"),
        Err(error) => println!("Opus Forge bootstrap: not ready ({error})"),
    }
    println!(
        "Microsoft client id: {}",
        if std::env::var_os(MICROSOFT_CLIENT_ID_ENV).is_some() {
            "configured"
        } else {
            "not configured (offline mode remains available)"
        }
    );
    if platform.requires_translation() && !translation_available {
        anyhow::bail!("the x86_64 translation layer required by Minecraft 1.8.9 is unavailable");
    }
    Ok(())
}
