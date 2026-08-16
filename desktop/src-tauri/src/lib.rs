#[cfg(not(feature = "qa-edition"))]
use opus_auth::{BrowserLoginCancellation, MicrosoftAuthenticator, RefreshTokenStore};
use opus_engine::{
    GameIdentity, InstallProgress, Installer, LaunchMode, LaunchOptions, LaunchPlan,
    MinecraftLayout, launch_game as launch_direct_game, launch_game_via_macos_app,
};
use opus_platform::{OperatingSystem, OpusPaths, Platform};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(not(feature = "qa-edition"))]
use std::sync::MutexGuard;
use std::sync::{Arc, Mutex};
use tauri::Manager;
use tauri::{AppHandle, Emitter};

#[cfg(all(
    not(debug_assertions),
    any(feature = "developer-test-profile", feature = "qa-edition")
))]
compile_error!("developer-test-profile and qa-edition are only allowed in debug builds");

#[cfg(all(
    debug_assertions,
    feature = "developer-test-profile",
    not(feature = "qa-edition")
))]
mod developer_test;
#[cfg(all(
    debug_assertions,
    feature = "developer-test-profile",
    not(feature = "qa-edition")
))]
use developer_test::DeveloperTestCoordinator;
mod accounts;
use accounts::{AccountKind, AccountSummary, ResolvedAccount};

const SETTINGS_FILE: &str = "launcher-settings-v1.json";
/// Client-only HUD utility preferences deliberately live outside the launcher
/// settings file. The filename is versioned so a future incompatible schema
/// can migrate independently without risking launch settings.
const UTILITY_SETTINGS_FILE: &str = "utility-settings-v1.json";
const UTILITY_SETTINGS_SCHEMA_VERSION: u8 = 1;
#[cfg(feature = "qa-edition")]
const QA_PROFILE_FILE: &str = "offline-profile-v1.json";
/// Public identifier of Opus's Microsoft Entra application. Desktop clients are
/// public OAuth clients, so this identifier is safe to distribute; refresh
/// credentials remain in the operating-system keychain.
#[cfg(not(feature = "qa-edition"))]
const OPUS_MICROSOFT_CLIENT_ID: &str = "352b876e-6d3b-4cb8-9095-82957a752784";

#[derive(Default)]
struct AppState {
    game_launch: Arc<GameLaunchCoordinator>,
    #[cfg(not(feature = "qa-edition"))]
    login: Arc<LoginCoordinator>,
    #[cfg(all(
        debug_assertions,
        feature = "developer-test-profile",
        not(feature = "qa-edition")
    ))]
    developer_test: Arc<DeveloperTestCoordinator>,
}

#[derive(Default)]
struct GameLaunchCoordinator {
    active: Mutex<BTreeMap<String, usize>>,
    instances: Mutex<BTreeMap<String, RunningInstance>>,
}

/// A live game instance the launcher is tracking. It is intentionally free of
/// tokens, paths outside the launcher's own log directory, and any secret so it
/// can be serialized straight to the UI's instance manager.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunningInstance {
    session_id: String,
    account_id: String,
    username: String,
    badge: String,
    title: String,
    log_directory: String,
}

struct GameLaunchAttempt {
    coordinator: Arc<GameLaunchCoordinator>,
    account_id: String,
}

impl GameLaunchCoordinator {
    fn begin(self: &Arc<Self>, account_id: &str) -> Result<GameLaunchAttempt, String> {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active.get(account_id).copied().unwrap_or_default() > 0 {
            return Err(
                "This account already has a Minecraft instance starting or running.".to_owned(),
            );
        }
        *active.entry(account_id.to_owned()).or_default() += 1;
        Ok(GameLaunchAttempt {
            coordinator: Arc::clone(self),
            account_id: account_id.to_owned(),
        })
    }

    fn active_count(&self) -> usize {
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .sum()
    }

    fn active_account_ids(&self) -> Vec<String> {
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    fn register_instance(&self, instance: RunningInstance) {
        self.instances
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(instance.session_id.clone(), instance);
    }

    fn remove_instance(&self, session_id: &str) {
        self.instances
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(session_id);
    }

    fn list_instances(&self) -> Vec<RunningInstance> {
        self.instances
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect()
    }

    fn instance(&self, session_id: &str) -> Option<RunningInstance> {
        self.instances
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(session_id)
            .cloned()
    }
}

impl GameLaunchAttempt {
    fn rekey(&mut self, account_id: &str) -> Result<(), String> {
        if self.account_id == account_id {
            return Ok(());
        }
        let mut active = self
            .coordinator
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active.get(account_id).copied().unwrap_or_default() > 0 {
            return Err(
                "This account already has a Minecraft instance starting or running.".to_owned(),
            );
        }
        if let Some(count) = active.get_mut(&self.account_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                active.remove(&self.account_id);
            }
        }
        *active.entry(account_id.to_owned()).or_default() += 1;
        self.account_id = account_id.to_owned();
        Ok(())
    }
}

impl Drop for GameLaunchAttempt {
    fn drop(&mut self) {
        let mut active = self
            .coordinator
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(count) = active.get_mut(&self.account_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                active.remove(&self.account_id);
            }
        }
    }
}

#[cfg(not(feature = "qa-edition"))]
#[derive(Default)]
struct LoginCoordinator {
    state: Mutex<LoginCoordinatorState>,
}

#[cfg(not(feature = "qa-edition"))]
#[derive(Default)]
struct LoginCoordinatorState {
    next_id: u64,
    active: Option<ActiveLogin>,
}

#[cfg(not(feature = "qa-edition"))]
struct ActiveLogin {
    id: u64,
    cancellation: BrowserLoginCancellation,
}

#[cfg(not(feature = "qa-edition"))]
struct LoginAttempt {
    coordinator: Arc<LoginCoordinator>,
    id: u64,
    cancellation: BrowserLoginCancellation,
}

#[cfg(not(feature = "qa-edition"))]
impl LoginCoordinator {
    fn lock_state(&self) -> MutexGuard<'_, LoginCoordinatorState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(all(
        debug_assertions,
        feature = "developer-test-profile",
        not(feature = "qa-edition")
    ))]
    fn is_active(&self) -> bool {
        self.lock_state().active.is_some()
    }
}

#[cfg(not(feature = "qa-edition"))]
impl LoginAttempt {
    fn cancellation(&self) -> &BrowserLoginCancellation {
        &self.cancellation
    }
}

#[cfg(not(feature = "qa-edition"))]
impl Drop for LoginAttempt {
    fn drop(&mut self) {
        let mut state = self.coordinator.lock_state();
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.id == self.id)
        {
            state.active = None;
        }
    }
}

impl AppState {
    fn begin_game_launch(&self, account_id: &str) -> Result<GameLaunchAttempt, String> {
        self.game_launch.begin(account_id)
    }

    fn active_launch_count(&self) -> usize {
        self.game_launch.active_count()
    }

    fn active_launch_account_ids(&self) -> Vec<String> {
        self.game_launch.active_account_ids()
    }

    fn developer_test_profile(&self) -> DeveloperTestProfile {
        #[cfg(all(
            debug_assertions,
            feature = "developer-test-profile",
            not(feature = "qa-edition")
        ))]
        {
            self.developer_test.profile()
        }
        #[cfg(not(all(
            debug_assertions,
            feature = "developer-test-profile",
            not(feature = "qa-edition")
        )))]
        {
            DeveloperTestProfile::unavailable()
        }
    }

    /// A developer simulation must never start an installer or game process.
    /// The QA offline edition is a real runtime path and therefore bypasses
    /// only this *developer simulation* guard, never an authentication guard.
    fn require_runtime_mode(&self) -> Result<(), String> {
        #[cfg(all(
            debug_assertions,
            feature = "developer-test-profile",
            not(feature = "qa-edition")
        ))]
        {
            if self.developer_test.profile().active {
                Err("This action is unavailable while Developer Test Profile is active".to_owned())
            } else {
                Ok(())
            }
        }
        #[cfg(not(all(
            debug_assertions,
            feature = "developer-test-profile",
            not(feature = "qa-edition")
        )))]
        {
            Ok(())
        }
    }

    #[cfg(not(feature = "qa-edition"))]
    fn begin_login(&self) -> Result<LoginAttempt, String> {
        let mut state = self.login.lock_state();
        if state.active.is_some() {
            return Err("A Microsoft sign-in is already in progress".to_owned());
        }
        let id = state.next_id;
        state.next_id = state.next_id.wrapping_add(1);
        let cancellation = BrowserLoginCancellation::new();
        state.active = Some(ActiveLogin {
            id,
            cancellation: cancellation.clone(),
        });
        Ok(LoginAttempt {
            coordinator: Arc::clone(&self.login),
            id,
            cancellation,
        })
    }

    #[cfg(not(feature = "qa-edition"))]
    fn cancel_login(&self) -> bool {
        let state = self.login.lock_state();
        let Some(active) = state.active.as_ref() else {
            return false;
        };
        active.cancellation.cancel();
        true
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LauncherSnapshot {
    build_edition: BuildEdition,
    platform: String,
    data_directory: String,
    minecraft_ready: bool,
    minecraft_status: String,
    optifine_ready: bool,
    optifine_status: String,
    java_status: String,
    account_stored: bool,
    offline_profile: Option<OfflineProfileSnapshot>,
    accounts: Vec<AccountSummary>,
    selected_account_id: Option<String>,
    active_launches: usize,
    active_account_ids: Vec<String>,
    game_launch_ready: bool,
    developer_test_profile: DeveloperTestProfile,
}

/// Safe status only. The real launcher always reports this as unavailable;
/// debug builds with the explicit feature report their in-memory simulation
/// state. It never represents a real account or a real game session.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct DeveloperTestProfile {
    available: bool,
    active: bool,
    simulation_active: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum BuildEdition {
    #[cfg(not(feature = "qa-edition"))]
    Premium,
    #[cfg(feature = "qa-edition")]
    QaOffline,
}

fn build_edition() -> BuildEdition {
    #[cfg(feature = "qa-edition")]
    {
        BuildEdition::QaOffline
    }
    #[cfg(not(feature = "qa-edition"))]
    {
        BuildEdition::Premium
    }
}

impl DeveloperTestProfile {
    #[cfg(not(all(
        debug_assertions,
        feature = "developer-test-profile",
        not(feature = "qa-edition")
    )))]
    fn unavailable() -> Self {
        Self {
            available: false,
            active: false,
            simulation_active: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LauncherSettings {
    max_memory_mib: u32,
    close_launcher_on_game_start: bool,
}

impl Default for LauncherSettings {
    fn default() -> Self {
        Self {
            max_memory_mib: 2048,
            close_launcher_on_game_start: false,
        }
    }
}

/// Per-utility presentation preferences. These controls affect only the
/// client-side HUD and are intentionally kept separate from game launch,
/// account, and runtime settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct UtilityPreference {
    enabled: bool,
    anchor: String,
    offset: String,
    scale: u8,
    opacity: u8,
}

/// A map keeps the IPC shape stable for the frontend while allowing additional
/// low-risk client utilities to be introduced without changing this command's
/// top-level JSON contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct UtilitySettings {
    #[serde(default)]
    schema_version: u8,
    utilities: BTreeMap<String, UtilityPreference>,
}

impl Default for UtilitySettings {
    fn default() -> Self {
        let mut utilities = BTreeMap::new();
        utilities.insert(
            "fps".to_owned(),
            UtilityPreference {
                // A HUD element must be an explicit player choice. In
                // particular, do not populate a fresh client with a frame
                // counter simply because the game can provide one.
                enabled: false,
                anchor: "top-left".to_owned(),
                offset: "12 · 12".to_owned(),
                scale: 100,
                opacity: 100,
            },
        );
        utilities.insert(
            "cps".to_owned(),
            UtilityPreference {
                enabled: false,
                anchor: "top-left".to_owned(),
                offset: "12 · 26".to_owned(),
                scale: 100,
                opacity: 100,
            },
        );
        utilities.insert(
            "memory".to_owned(),
            UtilityPreference {
                enabled: false,
                anchor: "top-right".to_owned(),
                offset: "12 · 12".to_owned(),
                scale: 100,
                opacity: 100,
            },
        );
        utilities.insert(
            "coordinates".to_owned(),
            UtilityPreference {
                enabled: false,
                anchor: "bottom-left".to_owned(),
                offset: "12 · 28".to_owned(),
                scale: 100,
                opacity: 100,
            },
        );
        utilities.insert(
            "clock".to_owned(),
            UtilityPreference {
                enabled: false,
                anchor: "top-right".to_owned(),
                offset: "12 · 26".to_owned(),
                scale: 100,
                opacity: 100,
            },
        );
        utilities.insert(
            "keystrokes".to_owned(),
            UtilityPreference {
                enabled: false,
                anchor: "bottom-right".to_owned(),
                offset: "12 · 28".to_owned(),
                scale: 100,
                opacity: 100,
            },
        );
        Self {
            schema_version: UTILITY_SETTINGS_SCHEMA_VERSION,
            utilities,
        }
    }
}

/// The only identity material persisted by the QA launcher. It has no token,
/// UUID, entitlement, or Microsoft account data; the runtime derives the
/// standard deterministic Minecraft offline UUID at launch time.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct QaProfile {
    username: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct OfflineProfileSnapshot {
    username: String,
    valid: bool,
}

impl From<QaProfile> for OfflineProfileSnapshot {
    fn from(profile: QaProfile) -> Self {
        let valid = GameIdentity::offline(&profile.username).is_ok();
        Self {
            username: profile.username,
            valid,
        }
    }
}

#[cfg(not(feature = "qa-edition"))]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountResult {
    profile: String,
    account: AccountSummary,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GameLaunchStarted {
    session_id: String,
    log_directory: Option<String>,
    account_id: String,
    simulated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GameLaunchFinished {
    session_id: String,
    log_directory: Option<String>,
    account_id: String,
    outcome: &'static str,
    message: String,
    simulated: bool,
}

/// Safe, aggregate install information emitted to the launcher UI. It never
/// contains artifact URLs, filesystem paths, or account data.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallProgressEvent {
    phase: &'static str,
    completed_files: usize,
    total_files: usize,
    downloaded_files: usize,
    cached_files: usize,
}

impl From<InstallProgress> for InstallProgressEvent {
    fn from(progress: InstallProgress) -> Self {
        Self {
            phase: progress.phase.label(),
            completed_files: progress.completed_files,
            total_files: progress.total_files,
            downloaded_files: progress.downloaded_files,
            cached_files: progress.cached_files,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallResult {
    minecraft_version: String,
    java_version: String,
    optifine_ready: bool,
    downloaded_files: usize,
    cached_files: usize,
}

/// Result of importing a user-owned OptiFine JAR into the isolated Opus game
/// directory. The runtime verifies the immutable checksum before any copy is
/// made; the original source file is never modified or removed.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OptiFineImportResult {
    file_name: String,
}

struct PreparedGameLaunch {
    plan: LaunchPlan,
    macos_game_app: Option<PathBuf>,
    account_id: String,
    username: String,
    badge: String,
    window_title: String,
}

struct ForgeBootstrapArtifacts {
    bootstrap_jar: PathBuf,
    coremod_jar: PathBuf,
    client_mod_jar: PathBuf,
}

#[tauri::command]
async fn launcher_snapshot(
    state: tauri::State<'_, AppState>,
    app: AppHandle,
) -> Result<LauncherSnapshot, String> {
    let developer_test_profile = state.developer_test_profile();
    let active_launches = state.active_launch_count();
    let active_account_ids = state.active_launch_account_ids();
    let app_for_snapshot = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        snapshot_for_profile(developer_test_profile, |profile| {
            snapshot_blocking(
                &app_for_snapshot,
                profile,
                active_launches,
                active_account_ids,
            )
        })
    })
    .await
    .map_err(|error| format!("launcher status task failed: {error}"))?
}

/// Selects an entirely synthetic snapshot before touching any real launcher
/// service. Keeping this boundary small makes it auditable that a developer
/// simulation cannot read the keychain, inspect game files, or probe a runner.
fn developer_test_snapshot(developer_test_profile: DeveloperTestProfile) -> LauncherSnapshot {
    LauncherSnapshot {
        build_edition: build_edition(),
        platform: "Developer Test Profile".to_owned(),
        data_directory: "Not used by simulation".to_owned(),
        minecraft_ready: false,
        minecraft_status: "Simulation only (no game files used)".to_owned(),
        optifine_ready: false,
        optifine_status: "Simulation only (no OptiFine file used)".to_owned(),
        java_status: "Simulation only (no Java runtime used)".to_owned(),
        account_stored: false,
        offline_profile: None,
        accounts: Vec::new(),
        selected_account_id: None,
        active_launches: 0,
        active_account_ids: Vec::new(),
        game_launch_ready: false,
        developer_test_profile,
    }
}

fn snapshot_for_profile<F>(
    developer_test_profile: DeveloperTestProfile,
    real_snapshot: F,
) -> Result<LauncherSnapshot, String>
where
    F: FnOnce(DeveloperTestProfile) -> Result<LauncherSnapshot, String>,
{
    if developer_test_profile.active {
        return Ok(developer_test_snapshot(developer_test_profile));
    }
    real_snapshot(developer_test_profile)
}

fn snapshot_blocking(
    app: &AppHandle,
    developer_test_profile: DeveloperTestProfile,
    active_launches: usize,
    active_account_ids: Vec<String>,
) -> Result<LauncherSnapshot, String> {
    let platform = Platform::detect().map_err(display_error)?;
    let paths = launcher_paths()?;
    let data_directory = paths.root.display().to_string();
    let account_view = accounts::load(&paths)?;
    let account_summaries = account_view.summaries();
    let selected_account_id = accounts::selected_id(&account_view);
    let installer =
        Installer::new(MinecraftLayout::new(paths.clone()), platform).map_err(display_error)?;
    let (minecraft_ready, minecraft_status, optifine_ready, optifine_status, java_status) =
        match installer.load_cached() {
            Ok(report) => {
                let optifine_ready = report.minecraft.optifine_jar.is_some();
                (
                    true,
                    format!("Forge {} verified", report.minecraft.version.id),
                    optifine_ready,
                    if optifine_ready {
                        "OptiFine 1.8.9 HD U M5 verified".to_owned()
                    } else {
                        "Import your local OptiFine 1.8.9 HD U M5 JAR to launch".to_owned()
                    },
                    format!("Java {} ready", report.java.version_name),
                )
            }
            Err(_) => (
                false,
                "Forge 1.8.9 is not installed or needs repair".to_owned(),
                false,
                "Forge runtime must be installed first".to_owned(),
                "Will install with the Forge runtime".to_owned(),
            ),
        };
    let account_stored = account_summaries
        .iter()
        .any(|account| matches!(account.kind, AccountKind::Microsoft) && account.ready);
    let offline_profile = account_summaries
        .iter()
        .find(|account| matches!(account.kind, AccountKind::Offline))
        .map(|account| OfflineProfileSnapshot {
            username: account.username.clone(),
            valid: account.ready,
        });
    let game_launch_ready = match platform.os {
        OperatingSystem::MacOs => macos_game_app_path(app).is_ok(),
        OperatingSystem::Windows | OperatingSystem::Linux => true,
    };

    Ok(LauncherSnapshot {
        build_edition: build_edition(),
        platform: format!("{} {}", platform.os, platform.game_arch),
        data_directory,
        minecraft_ready,
        minecraft_status,
        optifine_ready,
        optifine_status,
        java_status,
        account_stored,
        offline_profile,
        accounts: account_summaries,
        selected_account_id,
        active_launches,
        active_account_ids,
        game_launch_ready,
        developer_test_profile,
    })
}

#[tauri::command]
async fn install_minecraft(
    state: tauri::State<'_, AppState>,
    app: AppHandle,
) -> Result<InstallResult, String> {
    state.require_runtime_mode()?;
    let app_for_progress = app.clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        let platform = Platform::detect().map_err(display_error)?;
        let paths = launcher_paths()?;
        let installer =
            Installer::new(MinecraftLayout::new(paths), platform).map_err(display_error)?;
        installer
            .install_with_progress(move |progress| {
                // The window may be closed while an install is finishing; that
                // does not invalidate the verified installation itself.
                let _ = app_for_progress.emit(
                    "opus://install-progress",
                    InstallProgressEvent::from(progress),
                );
            })
            .map_err(display_error)
    })
    .await
    .map_err(|error| format!("install task failed: {error}"))??;

    Ok(InstallResult {
        minecraft_version: report.minecraft.version.id,
        java_version: report.java.version_name,
        optifine_ready: report.minecraft.optifine_jar.is_some(),
        downloaded_files: report.downloaded_files,
        cached_files: report.cached_files,
    })
}

/// Imports the exact OptiFine build selected by the locked Forge profile.
/// This deliberately accepts a local path only: Opus does not download, bundle,
/// or redistribute OptiFine.
#[tauri::command]
async fn import_optifine(
    state: tauri::State<'_, AppState>,
    source_path: String,
) -> Result<OptiFineImportResult, String> {
    state.require_runtime_mode()?;
    if source_path.trim().is_empty() {
        return Err("Choose the local OptiFine 1.8.9 HD U M5 JAR first".to_owned());
    }
    let source = PathBuf::from(source_path);
    let imported = tauri::async_runtime::spawn_blocking(move || {
        let platform = Platform::detect().map_err(display_error)?;
        let paths = launcher_paths()?;
        let installer =
            Installer::new(MinecraftLayout::new(paths), platform).map_err(display_error)?;
        installer.import_optifine(&source).map_err(display_error)
    })
    .await
    .map_err(|error| format!("OptiFine import task failed: {error}"))??;

    let file_name = imported
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Imported OptiFine file has an invalid name".to_owned())?
        .to_owned();
    Ok(OptiFineImportResult { file_name })
}

#[tauri::command]
fn get_settings() -> Result<LauncherSettings, String> {
    load_settings()
}

fn load_settings() -> Result<LauncherSettings, String> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(LauncherSettings::default());
    }
    let bytes = fs::read(&path).map_err(display_error)?;
    serde_json::from_slice(&bytes).map_err(|_| "Opus settings file is invalid".to_owned())
}

#[tauri::command]
fn save_settings(settings: LauncherSettings) -> Result<(), String> {
    validate_settings(&settings)?;
    write_settings_atomic(&settings_path()?, &settings)
}

/// Returns the persisted client-only HUD utility preferences, or the complete
/// v1 defaults when this installation has not configured them yet.
#[tauri::command]
fn get_utility_settings() -> Result<UtilitySettings, String> {
    load_utility_settings()
}

/// Persists client-only HUD utility preferences independently from launcher
/// settings, account data, and runtime state.
#[tauri::command]
fn save_utility_settings(settings: UtilitySettings) -> Result<(), String> {
    validate_utility_settings(&settings)?;
    write_utility_settings_atomic(&utility_settings_path()?, &settings)
}

#[tauri::command]
fn list_accounts() -> Result<Vec<AccountSummary>, String> {
    let paths = launcher_paths()?;
    Ok(accounts::load(&paths)?.summaries())
}

#[tauri::command]
fn select_account(account_id: String) -> Result<AccountSummary, String> {
    let paths = launcher_paths()?;
    accounts::select(&paths, account_id.trim())
}

#[tauri::command]
fn save_offline_profile(username: String) -> Result<AccountSummary, String> {
    let paths = launcher_paths()?;
    accounts::upsert_offline(&paths, username.trim())
}

#[tauri::command]
fn remove_account(account_id: String) -> Result<bool, String> {
    let paths = launcher_paths()?;
    accounts::remove(&paths, account_id.trim())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountSkin {
    data_url: String,
    model: String,
    is_default: bool,
}

/// Resolve the 3D-viewer skin for an account UUID. Offline profiles and any
/// network failure fall back to the embedded default skin, so the account pane
/// always has a texture to render. Runs on a blocking thread because it may hit
/// the Mojang session server.
#[tauri::command]
async fn account_skin(uuid: String) -> Result<AccountSkin, String> {
    let skin = tauri::async_runtime::spawn_blocking(move || opus_engine::fetch_skin(uuid.trim()))
        .await
        .map_err(|error| format!("skin task failed: {error}"))?;
    Ok(AccountSkin {
        data_url: opus_engine::skin_data_url(&skin),
        model: skin.model.as_str().to_owned(),
        is_default: skin.is_default,
    })
}

/// Read the QA-only offline profile. Premium builds do not register this
/// command, so the browser frontend can never create an unauthenticated
/// Premium launch identity.
#[cfg(feature = "qa-edition")]
#[tauri::command]
fn get_qa_profile() -> Result<QaProfile, String> {
    Ok(load_qa_profile()?.unwrap_or_default())
}

#[cfg(feature = "qa-edition")]
#[tauri::command]
fn save_qa_profile(profile: QaProfile) -> Result<(), String> {
    validate_qa_profile(&profile)?;
    write_json_atomic(&qa_profile_path()?, &profile)
}

#[cfg(not(feature = "qa-edition"))]
#[tauri::command]
async fn login_with_microsoft(
    state: tauri::State<'_, AppState>,
    app: AppHandle,
) -> Result<AccountResult, String> {
    state.require_runtime_mode()?;
    let attempt = state.begin_login()?;
    // The attempt is intentionally moved into the blocking task. If the IPC
    // future is dropped (for example when a webview reloads), that task keeps
    // the login slot occupied until it has actually stopped.
    let result = tauri::async_runtime::spawn_blocking(move || {
        let result = complete_microsoft_login(attempt.cancellation());
        drop(attempt);
        result
    })
    .await
    .map_err(|error| format!("login task failed: {error}"))?;

    // Microsoft finishes in the system browser. Bring the launcher back so
    // the user immediately sees the verified account or a retryable error.
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
    result
}

#[cfg(not(feature = "qa-edition"))]
fn complete_microsoft_login(
    cancellation: &BrowserLoginCancellation,
) -> Result<AccountResult, String> {
    cancellation.ensure_not_cancelled().map_err(display_error)?;
    let authenticator = microsoft_authenticator()?;
    // The loopback listener is bound before the browser is opened, so no
    // callback can race the local listener into existence.
    let authorization = authenticator
        .begin_browser_authorization()
        .map_err(display_error)?;
    cancellation.ensure_not_cancelled().map_err(display_error)?;
    open::that(authorization.authorization_url()).map_err(|_| {
        "Opus Launcher could not open Microsoft sign-in in your default browser".to_owned()
    })?;
    let account = authenticator
        .complete_browser_authorization(authorization, cancellation)
        .map_err(display_error)?;
    // A cancelled flow must never persist a newly obtained credential.
    cancellation.ensure_not_cancelled().map_err(display_error)?;
    account
        .save_refresh_token_for_profile()
        .map_err(display_error)?;
    let paths = launcher_paths()?;
    let summary =
        accounts::upsert_microsoft(&paths, &account.session.username, &account.session.uuid)?;
    Ok(AccountResult {
        profile: account.session.redacted_summary(),
        account: summary,
    })
}

#[cfg(not(feature = "qa-edition"))]
#[tauri::command]
fn cancel_microsoft_login(state: tauri::State<'_, AppState>) -> bool {
    state.cancel_login()
}

#[tauri::command]
async fn launch_game(
    state: tauri::State<'_, AppState>,
    settings: LauncherSettings,
    account_id: String,
    app: AppHandle,
) -> Result<GameLaunchStarted, String> {
    state.require_runtime_mode()?;
    let hide_launcher = settings.close_launcher_on_game_start;
    let app_for_preparation = app.clone();
    let requested_account_id = account_id.trim().to_owned();
    if requested_account_id.is_empty() {
        return Err("Choose an account or offline profile before launching".to_owned());
    }
    // Acquire identity ownership before authentication and artifact staging.
    // This prevents two concurrent requests for the same profile from racing
    // on its token, mods directory, options and logs.
    let mut launch_attempt = state.begin_game_launch(&requested_account_id)?;
    let prepared = tauri::async_runtime::spawn_blocking(move || {
        prepare_game_launch(&app_for_preparation, &settings, &requested_account_id)
    })
    .await
    .map_err(|error| format!("game launch preparation task failed: {error}"))??;
    // Preparation resolves the canonical catalog identity before the running
    // session is exposed to the frontend.
    launch_attempt.rekey(&prepared.account_id)?;

    let session_id = prepared.plan.session_id.clone();
    let log_directory = Some(prepared.plan.log_directory.display().to_string());
    let account_id = prepared.account_id.clone();
    let username = prepared.username.clone();
    let event_session_id = session_id.clone();
    let event_log_directory = log_directory.clone();
    let event_account_id = account_id.clone();
    // Publish the live instance so the UI's instance manager can show a labeled
    // row and offer an explicit kill. Removed again when the game lifecycle ends.
    state.game_launch.register_instance(RunningInstance {
        session_id: session_id.clone(),
        account_id: account_id.clone(),
        username: username.clone(),
        badge: prepared.badge.clone(),
        title: prepared.window_title.clone(),
        log_directory: prepared.plan.log_directory.display().to_string(),
    });
    let coordinator_for_wait = Arc::clone(&state.game_launch);
    let wait_session_id = session_id.clone();
    let app_for_wait = app.clone();
    std::thread::spawn(move || {
        let _launch_attempt = launch_attempt;
        let result = match prepared.macos_game_app {
            Some(game_app) => launch_game_via_macos_app(prepared.plan, &game_app),
            None => launch_direct_game(prepared.plan),
        };
        coordinator_for_wait.remove_instance(&wait_session_id);
        let event = match result {
            Ok(result) => GameLaunchFinished {
                session_id: result.session_id,
                log_directory: Some(result.log_directory.display().to_string()),
                account_id: event_account_id.clone(),
                outcome: "exited",
                message: format!("Minecraft for {username} exited with {}", result.status),
                simulated: false,
            },
            Err(error) => GameLaunchFinished {
                session_id: event_session_id,
                log_directory: event_log_directory,
                account_id: event_account_id,
                outcome: "failed",
                message: format!("Minecraft for {username} failed: {error}"),
                simulated: false,
            },
        };
        let _ = app_for_wait.emit("opus://game-finished", event);
        if hide_launcher && let Some(window) = app_for_wait.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    });

    if hide_launcher && let Some(window) = app.get_webview_window("main") {
        window.hide().map_err(display_error)?;
    }

    Ok(GameLaunchStarted {
        session_id,
        log_directory,
        account_id,
        simulated: false,
    })
}

/// List the game instances the launcher is currently tracking so the UI can
/// render an instance manager. Payload-free by construction.
#[tauri::command]
fn list_instances(state: tauri::State<'_, AppState>) -> Vec<RunningInstance> {
    state.game_launch.list_instances()
}

/// Terminate a tracked game instance by session id. The launcher started the
/// JVM through the macOS game stub, so the game JVM records its own pid beside
/// the session status file; the launcher signals that pid directly. The
/// coordinator entry is removed by the waiting launch thread when the process
/// exits, which also emits the normal `opus://game-finished` event.
#[tauri::command]
fn kill_instance(state: tauri::State<'_, AppState>, session_id: String) -> Result<(), String> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err("Missing instance id".to_owned());
    }
    let Some(instance) = state.game_launch.instance(session_id) else {
        return Err("That Minecraft instance is no longer running.".to_owned());
    };
    terminate_instance_process(Path::new(&instance.log_directory))
}

/// Read the game JVM pid recorded by the Opus bootstrap and send it SIGTERM.
/// The pid file lives inside the launcher-owned session log directory. Refuse
/// anything that is not a positive integer so nothing else can be signaled.
fn terminate_instance_process(log_directory: &Path) -> Result<(), String> {
    let pid_path = log_directory.join("game.pid");
    let raw = fs::read_to_string(&pid_path).map_err(|_| {
        "Could not read the instance process id yet. Try again in a moment.".to_owned()
    })?;
    let pid: u32 = raw
        .trim()
        .parse()
        .map_err(|_| "The instance process id is invalid.".to_owned())?;
    if pid <= 1 {
        return Err("The instance process id is invalid.".to_owned());
    }
    // Signal the recorded game pid with the system `kill`. Using a validated
    // positive integer and an absolute binary path keeps this free of shell
    // interpolation. A non-zero exit almost always means the process already
    // exited, which the launch wait thread settles on its own; treat it as
    // success so the instance manager does not surface a spurious error.
    let status = std::process::Command::new("/bin/kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|error| format!("Could not stop the instance: {error}"))?;
    let _ = status;
    Ok(())
}

/// Opt into an in-memory developer profile for local UI work. This command is
/// not compiled or registered outside a debug build with the explicit feature.
#[cfg(all(
    debug_assertions,
    feature = "developer-test-profile",
    not(feature = "qa-edition")
))]
#[tauri::command]
fn set_developer_test_profile(
    state: tauri::State<'_, AppState>,
    active: bool,
) -> Result<DeveloperTestProfile, String> {
    if active && state.login.is_active() {
        return Err(
            "Finish or cancel Microsoft sign-in before enabling Developer Test Profile".to_owned(),
        );
    }
    state.developer_test.set_active(active)
}

/// Simulates the normal start/finish event lifecycle without calling the
/// installer, authentication services, keychain, Java, or the native runner.
#[cfg(all(
    debug_assertions,
    feature = "developer-test-profile",
    not(feature = "qa-edition")
))]
#[tauri::command]
fn simulate_developer_game(
    state: tauri::State<'_, AppState>,
    app: AppHandle,
) -> Result<GameLaunchStarted, String> {
    let session = state.developer_test.start_simulation()?;
    let session_id = session.session_id.clone();
    let coordinator = Arc::clone(&state.developer_test);
    let app_for_finish = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let Some(finished) = coordinator.finish_simulation(&session_id) else {
            return;
        };
        let _ = app_for_finish.emit(
            "opus://game-finished",
            GameLaunchFinished {
                session_id: finished.session_id,
                log_directory: None,
                account_id: "developer-test".to_owned(),
                outcome: "exited",
                message: "Developer test session finished. Minecraft was not started.".to_owned(),
                simulated: true,
            },
        );
    });

    Ok(GameLaunchStarted {
        session_id: session.session_id,
        log_directory: None,
        account_id: "developer-test".to_owned(),
        simulated: true,
    })
}

fn settings_path() -> Result<PathBuf, String> {
    let paths = launcher_paths()?;
    Ok(paths.root.join(SETTINGS_FILE))
}

fn utility_settings_path() -> Result<PathBuf, String> {
    let paths = launcher_paths()?;
    Ok(paths.root.join(UTILITY_SETTINGS_FILE))
}

/// Resolve storage for the active launcher flavor. The QA flavor never uses
/// `OpusPaths::discover`, which is the Premium root and honors `OPUS_HOME`.
fn launcher_paths() -> Result<OpusPaths, String> {
    #[cfg(feature = "ui-preview")]
    {
        OpusPaths::discover_ui_preview().map_err(display_error)
    }
    #[cfg(all(feature = "qa-edition", not(feature = "ui-preview")))]
    {
        OpusPaths::discover_qa().map_err(display_error)
    }
    #[cfg(not(feature = "qa-edition"))]
    {
        OpusPaths::discover().map_err(display_error)
    }
}

fn validate_settings(settings: &LauncherSettings) -> Result<(), String> {
    if !(512..=16 * 1024).contains(&settings.max_memory_mib) {
        return Err("Maximum memory must be between 512 and 16384 MiB".to_owned());
    }
    Ok(())
}

fn load_utility_settings() -> Result<UtilitySettings, String> {
    let path = utility_settings_path()?;
    if !path.exists() {
        return Ok(UtilitySettings::default());
    }
    let bytes = fs::read(&path).map_err(display_error)?;
    let settings: UtilitySettings = serde_json::from_slice(&bytes)
        .map_err(|_| "Opus utility settings file is invalid".to_owned())?;
    let (settings, needs_migration) = normalize_utility_settings(settings)?;
    if needs_migration {
        write_utility_settings_atomic(&path, &settings)?;
    }
    Ok(settings)
}

fn normalize_utility_settings(
    mut settings: UtilitySettings,
) -> Result<(UtilitySettings, bool), String> {
    let mut needs_migration = false;
    if settings.schema_version == 0 {
        // Previous launcher previews had no in-game renderer, yet one of them
        // defaulted FPS to enabled. That must never turn into an unsolicited
        // HUD item now that a real renderer exists.
        for preference in settings.utilities.values_mut() {
            preference.enabled = false;
        }
        settings.schema_version = UTILITY_SETTINGS_SCHEMA_VERSION;
        needs_migration = true;
    }
    if !settings.utilities.contains_key("fps")
        && let Some(previous) = settings.utilities.remove("performance-overlay")
    {
        settings.utilities.insert("fps".to_owned(), previous);
    }
    let defaults = UtilitySettings::default();
    settings
        .utilities
        .retain(|utility_id, _| defaults.utilities.contains_key(utility_id));
    for (utility_id, preference) in defaults.utilities {
        settings.utilities.entry(utility_id).or_insert(preference);
    }
    validate_utility_settings(&settings)
        .map_err(|_| "Opus utility settings file is invalid".to_owned())?;
    Ok((settings, needs_migration))
}

fn validate_utility_settings(settings: &UtilitySettings) -> Result<(), String> {
    if settings.schema_version != UTILITY_SETTINGS_SCHEMA_VERSION {
        return Err("Opus utility settings schema is unsupported".to_owned());
    }
    for (utility_id, preference) in &settings.utilities {
        if utility_id.trim().is_empty() {
            return Err("Utility identifier must not be empty".to_owned());
        }
        if utility_id.chars().count() > 96 {
            return Err("Utility identifier must be at most 96 characters".to_owned());
        }
        if !matches!(
            preference.anchor.as_str(),
            "top-left" | "top-right" | "bottom-left" | "bottom-right"
        ) {
            return Err(format!(
                "Utility {utility_id} must use a supported screen anchor"
            ));
        }
        if preference.offset.trim().is_empty() {
            return Err(format!("Utility {utility_id} must provide an offset"));
        }
        if preference.offset.chars().count() > 64 {
            return Err(format!(
                "Utility {utility_id} offset must be at most 64 characters"
            ));
        }
        if !(50..=150).contains(&preference.scale) {
            return Err(format!(
                "Utility {utility_id} scale must be between 50 and 150"
            ));
        }
        if !(25..=100).contains(&preference.opacity) {
            return Err(format!(
                "Utility {utility_id} opacity must be between 25 and 100"
            ));
        }
    }
    Ok(())
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Opus settings path has no parent directory".to_owned())?;
    fs::create_dir_all(parent).map_err(display_error)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Opus settings path has no file name".to_owned())?;
    let part = path.with_file_name(format!(".{name}-{}.part", std::process::id()));
    if part.exists() {
        fs::remove_file(&part).map_err(display_error)?;
    }
    let result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&part)
            .map_err(display_error)?;
        serde_json::to_writer_pretty(&mut file, value).map_err(display_error)?;
        file.write_all(b"\n").map_err(display_error)?;
        file.sync_all().map_err(display_error)?;
        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(path).map_err(display_error)?;
        }
        fs::rename(&part, path).map_err(display_error)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

fn write_settings_atomic(path: &Path, settings: &LauncherSettings) -> Result<(), String> {
    write_json_atomic(path, settings)
}

fn write_utility_settings_atomic(path: &Path, settings: &UtilitySettings) -> Result<(), String> {
    write_json_atomic(path, settings)
}

#[cfg(feature = "qa-edition")]
fn qa_profile_path() -> Result<PathBuf, String> {
    Ok(launcher_paths()?.root.join(QA_PROFILE_FILE))
}

#[cfg(feature = "qa-edition")]
fn load_qa_profile() -> Result<Option<QaProfile>, String> {
    let path = qa_profile_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(display_error)?;
    let profile = serde_json::from_slice(&bytes)
        .map_err(|_| "Opus QA offline profile is invalid. Choose a username again.".to_owned())?;
    Ok(Some(profile))
}

#[cfg(feature = "qa-edition")]
fn validate_qa_profile(profile: &QaProfile) -> Result<(), String> {
    GameIdentity::offline(&profile.username)
        .map(|_| ())
        .map_err(|error| {
            format!("Offline username must be 3–16 letters, digits, or underscores. ({error})")
        })
}

/// Compose the per-instance game window title so concurrently running
/// instances are distinguishable, e.g. "Opus Client - [OFFICIAL] zvwgvx".
/// A blank badge degrades gracefully to "Opus Client - <username>".
fn instance_window_title(badge: &str, username: &str) -> String {
    let badge = badge.trim();
    if badge.is_empty() {
        format!("Opus Client - {username}")
    } else {
        format!("Opus Client - [{}] {username}", badge.to_uppercase())
    }
}

fn prepare_game_launch(
    app: &AppHandle,
    settings: &LauncherSettings,
    requested_account_id: &str,
) -> Result<PreparedGameLaunch, String> {
    validate_settings(settings)?;
    let platform = Platform::detect().map_err(display_error)?;
    if !platform.translation_available().map_err(display_error)? {
        return Err(
            "Rosetta is required for Minecraft 1.8.9 on this Mac, but it is unavailable".to_owned(),
        );
    }

    let paths = launcher_paths()?;
    let installer =
        Installer::new(MinecraftLayout::new(paths.clone()), platform).map_err(display_error)?;
    let report = installer.load_cached().map_err(|error| {
        format!("Minecraft 1.8.9 is not ready. Use Install / Repair first. ({error})")
    })?;
    let (identity, account_id, username, badge) =
        resolve_launch_identity(&paths, requested_account_id)?;
    let window_title = instance_window_title(&badge, &username);
    let mut instance_paths = paths.clone();
    instance_paths.game = paths
        .root
        .join("instances")
        .join(&identity.uuid)
        .join("game");
    let layout = MinecraftLayout::new(instance_paths);

    let bootstrap_directory = bootstrap_resource_directory(app)?;
    let forge_artifacts = forge_bootstrap_artifacts(&bootstrap_directory)?;
    let macos_game_app = match platform.os {
        OperatingSystem::MacOs => Some(macos_game_app_path(app)?),
        OperatingSystem::Windows | OperatingSystem::Linux => None,
    };
    let plan = LaunchPlan::build(
        &layout,
        platform,
        &report.minecraft,
        &report.java,
        &identity,
        &LaunchOptions {
            max_memory_mib: settings.max_memory_mib,
            utility_settings_file: Some(utility_settings_path()?),
            brand_wordmark_file: Some(brand_wordmark_path(app)?),
            window_title: Some(window_title.clone()),
            ..LaunchOptions::default()
        },
        &LaunchMode::ForgeBootstrap {
            bootstrap_jar: forge_artifacts.bootstrap_jar,
            coremod_jar: forge_artifacts.coremod_jar,
            client_mod_jar: forge_artifacts.client_mod_jar,
        },
    )
    .map_err(display_error)?;

    Ok(PreparedGameLaunch {
        plan,
        macos_game_app,
        account_id,
        username,
        badge,
        window_title,
    })
}

fn resolve_launch_identity(
    paths: &OpusPaths,
    requested_account_id: &str,
) -> Result<(GameIdentity, String, String, String), String> {
    let view = accounts::load(paths)?;
    let account_id = if requested_account_id.trim().is_empty() {
        accounts::selected_id(&view).ok_or_else(|| {
            "Add a Microsoft account or unofficial profile before launching".to_owned()
        })?
    } else {
        requested_account_id.trim().to_owned()
    };
    match accounts::resolve(&view, &account_id)? {
        ResolvedAccount::Offline(record) => {
            let identity = GameIdentity::offline(&record.username).map_err(display_error)?;
            let username = identity.username.clone();
            let badge = record.badge.clone();
            Ok((identity, record.id, username, badge))
        }
        ResolvedAccount::Microsoft(record) => {
            #[cfg(feature = "qa-edition")]
            {
                let _ = record;
                Err("Microsoft accounts are unavailable in the QA build".to_owned())
            }
            #[cfg(not(feature = "qa-edition"))]
            {
                let authenticator = microsoft_authenticator()?;
                let store = RefreshTokenStore::for_profile(&record.uuid).map_err(display_error)?;
                let account = authenticator
                    .refresh_session(&store)
                    .map_err(display_error)?;
                account
                    .save_refresh_token_for_profile()
                    .map_err(display_error)?;
                let summary = accounts::upsert_microsoft(
                    paths,
                    &account.session.username,
                    &account.session.uuid,
                )?;
                let identity = GameIdentity::authenticated(
                    &account.session.username,
                    &account.session.uuid,
                    account.session.access_token.clone(),
                    &account.session.user_type,
                )
                .map_err(display_error)?;
                let username = identity.username.clone();
                let badge = summary.badge.clone();
                Ok((identity, summary.id, username, badge))
            }
        }
    }
}

#[cfg(not(feature = "qa-edition"))]
fn microsoft_authenticator() -> Result<MicrosoftAuthenticator, String> {
    MicrosoftAuthenticator::new(OPUS_MICROSOFT_CLIENT_ID.to_owned()).map_err(display_error)
}

fn bootstrap_resource_directory(app: &AppHandle) -> Result<PathBuf, String> {
    launcher_resource_path(
        app,
        Path::new("bootstrap"),
        Path::new("resources/bootstrap"),
    )
}

fn brand_wordmark_path(app: &AppHandle) -> Result<PathBuf, String> {
    launcher_resource_path(
        app,
        Path::new("brand/opus-wordmark-transparent.png"),
        Path::new("resources/brand/opus-wordmark-transparent.png"),
    )
}

fn macos_game_app_path(app: &AppHandle) -> Result<PathBuf, String> {
    let path = launcher_resource_path(
        app,
        Path::new("Opus Client.app"),
        Path::new("resources/Opus Client.app"),
    )?;
    let executable = path.join("Contents/MacOS/Opus Client");
    if !path.is_dir() || !executable.is_file() {
        return Err(format!("macOS game app is incomplete: {}", path.display()));
    }
    Ok(path)
}

fn launcher_resource_path(
    app: &AppHandle,
    bundled_relative: &Path,
    development_relative: &Path,
) -> Result<PathBuf, String> {
    if let Ok(resource_directory) = app.path().resource_dir() {
        let bundled = resource_directory.join(bundled_relative);
        if bundled.exists() {
            return Ok(bundled);
        }
    }

    let development = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(development_relative);
    if development.exists() {
        return Ok(development);
    }
    Err(format!(
        "required launcher resource is missing: {}",
        bundled_relative.display()
    ))
}

fn forge_bootstrap_artifacts(directory: &Path) -> Result<ForgeBootstrapArtifacts, String> {
    if !directory.is_dir() {
        return Err(format!(
            "Opus Forge bootstrap artifacts are missing at {}",
            directory.display()
        ));
    }

    let mut bootstrap_jar = None;
    let mut coremod_jar = None;
    let mut client_mod_jar = None;
    for entry in fs::read_dir(directory).map_err(display_error)? {
        let path = entry.map_err(display_error)?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("jar") {
            continue;
        }
        if !path.is_file() {
            return Err(format!(
                "Opus Forge artifact is not a regular file: {}",
                path.display()
            ));
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                format!(
                    "Opus Forge artifact has an invalid name: {}",
                    path.display()
                )
            })?;
        if name.starts_with("opus-bootstrap-") && name.ends_with(".jar") {
            if bootstrap_jar.replace(path).is_some() {
                return Err("Opus bootstrap directory contains multiple bootstrap JARs".to_owned());
            }
        } else if name.starts_with("opus-runtime-legacy-1.8.9-") && name.ends_with(".jar") {
            if coremod_jar.replace(path).is_some() {
                return Err(
                    "Opus bootstrap directory contains multiple Forge coremod JARs".to_owned(),
                );
            }
        } else if name.starts_with("opus-client-legacy-1.8.9-") && name.ends_with(".jar") {
            if client_mod_jar.replace(path).is_some() {
                return Err(
                    "Opus bootstrap directory contains multiple Forge client-mod JARs".to_owned(),
                );
            }
        } else {
            return Err(format!(
                "Opus bootstrap directory contains an unexpected JAR: {name}. Reinstall Opus Launcher."
            ));
        }
    }
    Ok(ForgeBootstrapArtifacts {
        bootstrap_jar: bootstrap_jar.ok_or_else(|| {
            "Opus Forge bootstrap JAR is missing. Reinstall Opus Launcher.".to_owned()
        })?,
        coremod_jar: coremod_jar.ok_or_else(|| {
            "Opus Forge coremod JAR is missing. Reinstall Opus Launcher.".to_owned()
        })?,
        client_mod_jar: client_mod_jar.ok_or_else(|| {
            "Opus Forge client-mod JAR is missing. Reinstall Opus Launcher.".to_owned()
        })?,
    })
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub fn run() {
    #[cfg(feature = "qa-edition")]
    let builder = tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            launcher_snapshot,
            install_minecraft,
            import_optifine,
            get_settings,
            save_settings,
            get_utility_settings,
            save_utility_settings,
            list_accounts,
            select_account,
            save_offline_profile,
            remove_account,
            account_skin,
            get_qa_profile,
            save_qa_profile,
            launch_game,
            list_instances,
            kill_instance,
        ]);

    #[cfg(all(
        debug_assertions,
        feature = "developer-test-profile",
        not(feature = "qa-edition")
    ))]
    let builder = tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            launcher_snapshot,
            install_minecraft,
            import_optifine,
            get_settings,
            save_settings,
            get_utility_settings,
            save_utility_settings,
            list_accounts,
            select_account,
            save_offline_profile,
            remove_account,
            account_skin,
            login_with_microsoft,
            cancel_microsoft_login,
            launch_game,
            list_instances,
            kill_instance,
            set_developer_test_profile,
            simulate_developer_game,
        ]);

    #[cfg(all(
        not(feature = "qa-edition"),
        not(all(debug_assertions, feature = "developer-test-profile"))
    ))]
    let builder = tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            launcher_snapshot,
            install_minecraft,
            import_optifine,
            get_settings,
            save_settings,
            get_utility_settings,
            save_utility_settings,
            list_accounts,
            select_account,
            save_offline_profile,
            remove_account,
            account_skin,
            login_with_microsoft,
            cancel_microsoft_login,
            launch_game,
            list_instances,
            kill_instance,
        ]);

    builder
        .run(tauri::generate_context!())
        .expect("error while running Opus Launcher");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_reject_memory_below_the_game_minimum() {
        assert!(
            validate_settings(&LauncherSettings {
                max_memory_mib: 511,
                close_launcher_on_game_start: false,
            })
            .is_err()
        );
    }

    #[test]
    fn settings_accept_supported_memory_range() {
        assert!(
            validate_settings(&LauncherSettings {
                max_memory_mib: 4096,
                close_launcher_on_game_start: true,
            })
            .is_ok()
        );
    }

    #[test]
    fn settings_ignore_unknown_fields() {
        let settings: LauncherSettings = serde_json::from_str(
            r#"{"maxMemoryMib":2048,"closeLauncherOnGameStart":false,"unknownField":"ignored"}"#,
        )
        .unwrap();
        assert_eq!(settings.max_memory_mib, 2048);
        assert!(!settings.close_launcher_on_game_start);
    }

    #[test]
    fn settings_do_not_serialize_a_microsoft_client_id() {
        let value = serde_json::to_value(LauncherSettings::default()).unwrap();
        assert!(value.get("microsoftClientId").is_none());
    }

    #[test]
    fn game_launch_attempt_allows_different_accounts_but_blocks_duplicates() {
        let state = AppState::default();
        let attempt = state.begin_game_launch("account-a").unwrap();
        assert!(state.begin_game_launch("account-a").is_err());
        let other = state.begin_game_launch("account-b").unwrap();
        assert_eq!(state.active_launch_count(), 2);
        drop(other);
        drop(attempt);
        assert!(state.begin_game_launch("account-a").is_ok());
    }

    #[test]
    fn game_launch_attempt_rekeys_provisional_identity_without_opening_a_duplicate_slot() {
        let state = AppState::default();
        let mut attempt = state.begin_game_launch("offline:pending").unwrap();
        attempt.rekey("microsoft:1234").unwrap();

        let provisional_again = state.begin_game_launch("offline:pending").unwrap();
        assert!(state.begin_game_launch("microsoft:1234").is_err());
        assert_eq!(
            state.active_launch_account_ids(),
            vec!["microsoft:1234".to_owned(), "offline:pending".to_owned()]
        );
        drop(provisional_again);
    }

    #[test]
    fn utility_settings_defaults_cover_the_client_utility_catalog() {
        let settings = UtilitySettings::default();
        assert_eq!(settings.utilities.len(), 6);
        for id in ["fps", "cps", "memory", "coordinates", "clock", "keystrokes"] {
            assert!(settings.utilities.contains_key(id), "missing utility {id}");
            assert!(
                !settings.utilities[id].enabled,
                "a fresh utility catalog must not enable {id} without player intent"
            );
        }
        assert!(validate_utility_settings(&settings).is_ok());
    }

    #[test]
    fn pre_renderer_utility_settings_migrate_to_an_empty_hud() {
        let mut legacy = UtilitySettings {
            schema_version: 0,
            ..UtilitySettings::default()
        };
        legacy.utilities.get_mut("fps").unwrap().enabled = true;
        legacy.utilities.get_mut("cps").unwrap().enabled = true;

        let (migrated, needs_persistence) = normalize_utility_settings(legacy).unwrap();

        assert!(needs_persistence);
        assert_eq!(migrated.schema_version, UTILITY_SETTINGS_SCHEMA_VERSION);
        assert!(migrated.utilities.values().all(|utility| !utility.enabled));
    }

    #[test]
    fn utility_settings_use_the_frontend_record_json_contract() {
        let value = serde_json::to_value(UtilitySettings::default()).unwrap();
        assert_eq!(
            value.get("schemaVersion"),
            Some(&serde_json::json!(UTILITY_SETTINGS_SCHEMA_VERSION))
        );
        let utilities = value
            .get("utilities")
            .and_then(serde_json::Value::as_object)
            .unwrap();
        let preference = utilities
            .get("fps")
            .and_then(serde_json::Value::as_object)
            .unwrap();
        assert_eq!(
            preference.get("enabled"),
            Some(&serde_json::Value::Bool(false))
        );
        assert_eq!(
            preference.get("anchor"),
            Some(&serde_json::json!("top-left"))
        );
        assert_eq!(
            preference.get("offset"),
            Some(&serde_json::json!("12 · 12"))
        );
        assert_eq!(preference.get("scale"), Some(&serde_json::json!(100)));
        assert_eq!(preference.get("opacity"), Some(&serde_json::json!(100)));
    }

    #[test]
    fn utility_settings_reject_values_outside_the_supported_ranges() {
        let mut settings = UtilitySettings::default();

        settings.utilities.get_mut("keystrokes").unwrap().scale = 49;
        assert!(validate_utility_settings(&settings).is_err());
        settings.utilities.get_mut("keystrokes").unwrap().scale = 151;
        assert!(validate_utility_settings(&settings).is_err());
        settings.utilities.get_mut("keystrokes").unwrap().scale = 50;

        settings.utilities.get_mut("keystrokes").unwrap().opacity = 24;
        assert!(validate_utility_settings(&settings).is_err());
        settings.utilities.get_mut("keystrokes").unwrap().opacity = 101;
        assert!(validate_utility_settings(&settings).is_err());
        settings.utilities.get_mut("keystrokes").unwrap().opacity = 25;

        assert!(validate_utility_settings(&settings).is_ok());
    }

    #[cfg(not(feature = "qa-edition"))]
    #[test]
    fn embedded_microsoft_client_id_is_valid() {
        assert!(microsoft_authenticator().is_ok());
    }

    #[cfg(not(feature = "qa-edition"))]
    #[test]
    fn task_owned_login_attempt_prevents_parallel_login_until_dropped() {
        let state = AppState::default();
        let attempt = state.begin_login().unwrap();
        assert!(state.begin_login().is_err());
        drop(attempt);
        assert!(state.begin_login().is_ok());
    }

    #[cfg(not(feature = "qa-edition"))]
    #[test]
    fn background_task_ownership_survives_the_callers_scope() {
        let state = AppState::default();
        let attempt = state.begin_login().unwrap();
        let (started_sender, started_receiver) = std::sync::mpsc::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let task = std::thread::spawn(move || {
            started_sender.send(()).unwrap();
            release_receiver.recv().unwrap();
            drop(attempt);
        });

        started_receiver.recv().unwrap();
        assert!(state.begin_login().is_err());
        release_sender.send(()).unwrap();
        task.join().unwrap();
        assert!(state.begin_login().is_ok());
    }

    #[cfg(not(feature = "qa-edition"))]
    #[test]
    fn cancellation_keeps_the_login_slot_until_task_cleanup_then_allows_retry() {
        let state = AppState::default();
        let attempt = state.begin_login().unwrap();
        assert!(state.cancel_login());
        assert!(attempt.cancellation().is_cancelled());
        assert!(state.begin_login().is_err());
        drop(attempt);
        assert!(!state.cancel_login());
        assert!(state.begin_login().is_ok());
    }

    #[cfg(not(feature = "qa-edition"))]
    #[test]
    fn install_progress_event_only_serializes_safe_aggregate_fields() {
        let event = InstallProgressEvent::from(InstallProgress {
            phase: opus_engine::InstallPhase::MinecraftAssets,
            completed_files: 12,
            total_files: 42,
            downloaded_files: 7,
            cached_files: 5,
        });
        let value = serde_json::to_value(event).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 5);
        assert_eq!(
            object.get("phase").and_then(serde_json::Value::as_str),
            Some("Downloading Minecraft assets")
        );
        assert!(object.get("url").is_none());
        assert!(object.get("path").is_none());
        assert!(object.get("token").is_none());
    }

    #[cfg(feature = "qa-edition")]
    #[test]
    fn qa_edition_exposes_an_offline_runtime_build() {
        assert_eq!(build_edition(), BuildEdition::QaOffline);
        let profile = AppState::default().developer_test_profile();
        assert!(!profile.available);
        assert!(!profile.active);
    }

    #[cfg(feature = "qa-edition")]
    #[test]
    fn qa_profile_validation_accepts_only_minecraft_offline_usernames() {
        assert!(
            validate_qa_profile(&QaProfile {
                username: "Qa_User9".to_owned(),
            })
            .is_ok()
        );
        assert!(
            validate_qa_profile(&QaProfile {
                username: "no spaces".to_owned(),
            })
            .is_err()
        );
        assert!(
            validate_qa_profile(&QaProfile {
                username: "ab".to_owned(),
            })
            .is_err()
        );
    }

    #[cfg(feature = "qa-edition")]
    #[test]
    fn qa_profile_snapshot_has_no_token_or_uuid() {
        let snapshot = OfflineProfileSnapshot::from(QaProfile {
            username: "Qa_User9".to_owned(),
        });
        let value = serde_json::to_value(snapshot).unwrap();
        assert_eq!(value.get("username"), Some(&serde_json::json!("Qa_User9")));
        assert_eq!(value.get("valid"), Some(&serde_json::Value::Bool(true)));
        assert!(value.get("accessToken").is_none());
        assert!(value.get("uuid").is_none());
    }

    #[cfg(all(
        debug_assertions,
        feature = "developer-test-profile",
        not(feature = "qa-edition")
    ))]
    #[test]
    fn active_developer_profile_skips_real_snapshot_work() {
        let state = AppState::default();
        state.developer_test.set_active(true).unwrap();
        let real_snapshot_called = std::cell::Cell::new(false);

        let snapshot = snapshot_for_profile(state.developer_test_profile(), |_| {
            real_snapshot_called.set(true);
            Err("the real snapshot must not run".to_owned())
        })
        .unwrap();

        assert!(!real_snapshot_called.get());
        assert!(snapshot.developer_test_profile.available);
        assert!(snapshot.developer_test_profile.active);
        assert!(!snapshot.developer_test_profile.simulation_active);
        assert!(!snapshot.minecraft_ready);
        assert!(!snapshot.account_stored);
        assert!(!snapshot.game_launch_ready);
        assert_eq!(snapshot.data_directory, "Not used by simulation");
    }

    #[cfg(all(
        debug_assertions,
        feature = "developer-test-profile",
        not(feature = "qa-edition")
    ))]
    #[test]
    fn developer_profile_blocks_all_real_mode_entrypoints() {
        let state = AppState::default();
        state.developer_test.set_active(true).unwrap();

        assert!(state.require_runtime_mode().is_err());
    }

    #[cfg(all(
        debug_assertions,
        feature = "developer-test-profile",
        not(feature = "qa-edition")
    ))]
    #[test]
    fn developer_simulation_has_a_deterministic_in_memory_lifecycle() {
        let state = AppState::default();
        state.developer_test.set_active(true).unwrap();

        let session = state.developer_test.start_simulation().unwrap();
        assert_eq!(session.session_id, "developer-test-0000");
        assert!(state.developer_test.profile().simulation_active);
        assert!(state.developer_test.start_simulation().is_err());
        assert!(
            state
                .developer_test
                .finish_simulation("developer-test-stale")
                .is_none()
        );
        assert!(state.developer_test.profile().simulation_active);
        assert!(
            state
                .developer_test
                .finish_simulation(&session.session_id)
                .is_some()
        );
        assert!(!state.developer_test.profile().simulation_active);
        assert!(state.developer_test.set_active(false).unwrap().available);
    }

    #[cfg(all(
        debug_assertions,
        feature = "developer-test-profile",
        not(feature = "qa-edition")
    ))]
    #[test]
    fn developer_profile_cannot_be_disabled_while_a_simulation_is_running() {
        let state = AppState::default();
        state.developer_test.set_active(true).unwrap();
        let session = state.developer_test.start_simulation().unwrap();

        assert!(state.developer_test.set_active(false).is_err());
        state
            .developer_test
            .finish_simulation(&session.session_id)
            .unwrap();
        assert!(!state.developer_test.set_active(false).unwrap().active);
    }

    #[cfg(all(
        debug_assertions,
        feature = "developer-test-profile",
        not(feature = "qa-edition")
    ))]
    #[test]
    fn simulated_game_finished_event_has_no_log_directory_or_real_claim() {
        let event = GameLaunchFinished {
            session_id: "developer-test-0000".to_owned(),
            log_directory: None,
            account_id: "developer-test".to_owned(),
            outcome: "exited",
            message: "Developer test session finished. Minecraft was not started.".to_owned(),
            simulated: true,
        };

        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value.get("logDirectory"), Some(&serde_json::Value::Null));
        assert_eq!(value.get("simulated"), Some(&serde_json::Value::Bool(true)));
        assert_eq!(
            value.get("message").and_then(serde_json::Value::as_str),
            Some("Developer test session finished. Minecraft was not started.")
        );
    }
}
