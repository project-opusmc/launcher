use anyhow::Result;
use clap::{Parser, Subcommand};
use rbw_auth::{MICROSOFT_CLIENT_ID_ENV, MicrosoftAuthenticator, RefreshTokenStore};
use rbw_platform::{OperatingSystem, Platform, RbwPaths};
use rbw_runtime::{
    GameIdentity, Installer, LaunchMode, LaunchOptions, LaunchPlan, MinecraftLayout, launch_game,
};
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(name = "rbw", version, about = "Opus Client 1.8.9 launcher")]
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
    /// Manage the official Microsoft Minecraft account.
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
        #[arg(long, default_value = "game/build/bootstrap")]
        bootstrap_dir: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum AccountCommand {
    /// Sign in through Microsoft's browser/device flow and verify ownership.
    Login,
    /// Report whether a refresh credential exists in the OS keychain.
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
            "CLI game launch is disabled on macOS. Use the packaged Opus Client.app, which starts managed Java through its LaunchServices game stub. --dry-run remains available for diagnostics."
        );
    }
    let paths = RbwPaths::discover()?;
    let layout = MinecraftLayout::new(paths);
    let installer = Installer::new(layout.clone(), platform)?;

    println!("Opus Client");
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
    println!("mode: Opus Forge bootstrap + coremod + typed client mod");
    let mode = LaunchMode::ForgeBootstrap {
        bootstrap_jar: artifacts.bootstrap_jar,
        coremod_jar: artifacts.coremod_jar,
        client_mod_jar: artifacts.client_mod_jar,
    };
    let plan = LaunchPlan::build(
        &layout,
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
                println!("A Microsoft refresh credential is stored in the OS keychain");
            } else {
                println!("No Microsoft account is stored");
            }
            Ok(())
        }
        AccountCommand::Logout => {
            if store.delete()? {
                println!("Microsoft credential removed from the OS keychain");
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
    client_mod_jar: PathBuf,
}

fn forge_bootstrap_artifacts(directory: &Path) -> Result<ForgeBootstrapArtifacts> {
    if !directory.is_dir() {
        anyhow::bail!(
            "OPUS Runtime artifacts are missing at {}; stage a verified prepareRuntime output",
            directory.display()
        );
    }

    let mut bootstrap_jar = None;
    let mut coremod_jar = None;
    let mut client_mod_jar = None;
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
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
        } else if name.starts_with("opus-client-legacy-1.8.9-") && name.ends_with(".jar") {
            if client_mod_jar.replace(path).is_some() {
                anyhow::bail!("bootstrap directory contains multiple Forge client-mod JARs");
            }
        } else {
            anyhow::bail!("bootstrap directory contains unexpected JAR: {name}");
        }
    }
    Ok(ForgeBootstrapArtifacts {
        bootstrap_jar: bootstrap_jar
            .ok_or_else(|| anyhow::anyhow!("Forge bootstrap JAR is missing"))?,
        coremod_jar: coremod_jar.ok_or_else(|| anyhow::anyhow!("Forge coremod JAR is missing"))?,
        client_mod_jar: client_mod_jar
            .ok_or_else(|| anyhow::anyhow!("Forge client-mod JAR is missing"))?,
    })
}

fn install() -> Result<()> {
    let platform = Platform::detect()?;
    let paths = RbwPaths::discover()?;
    let root = paths.root.clone();
    let installer = Installer::new(MinecraftLayout::new(paths), platform)?;

    println!("Opus Client installer");
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
    let paths = RbwPaths::discover()?;
    let installer = Installer::new(MinecraftLayout::new(paths), platform)?;
    let destination = installer.import_optifine(path)?;
    println!("OptiFine verified and imported: {}", destination.display());
    Ok(())
}

fn doctor() -> Result<()> {
    let platform = Platform::detect()?;
    let paths = RbwPaths::discover()?;
    let translation_available = platform.translation_available()?;

    println!("Opus Client doctor");
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
    match forge_bootstrap_artifacts(Path::new("game/build/bootstrap")) {
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
