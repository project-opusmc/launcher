mod download;
mod forge;
mod install;
mod launch;
mod layout;
mod model;
mod natives;
mod rules;
mod skin;

pub use download::{DownloadOutcome, DownloadSpec, Downloader, verify_file};
pub use forge::{
    FORGE_COREMOD_SHA1, FORGE_COREMOD_SIZE, FORGE_ELEMENTA_SHA1, FORGE_ELEMENTA_SIZE,
    FORGE_LWJGL_MACOS_COMPAT_SHA1, FORGE_LWJGL_MACOS_COMPAT_SIZE, FORGE_LWJGL_MACOS_NATIVE_SHA1,
    FORGE_LWJGL_MACOS_NATIVE_SIZE, FORGE_LWJGL_RESIZE_GUARD_FILE_NAME,
    FORGE_LWJGL_RESIZE_GUARD_SHA1, FORGE_LWJGL_RESIZE_GUARD_SIZE, FORGE_MAIN_CLASS,
    FORGE_NATIVE_UI_SHA1, FORGE_NATIVE_UI_SIZE, FORGE_OPTIFINE_FILE_NAME, FORGE_OPUSCONFIG_SHA1,
    FORGE_OPUSCONFIG_SIZE, FORGE_PROFILE_ID, FORGE_RUNTIME_ID, FORGE_UNIVERSALCRAFT_SHA1,
    FORGE_UNIVERSALCRAFT_SIZE, ForgeLibrary, ForgeLockError, ForgeRuntimeLock, OptiFineLock,
};
pub use install::{
    InstallPhase, InstallProgress, InstallReport, InstalledMinecraft, Installer, ManagedJava,
    NativeArchive,
};
pub use launch::{
    ForgeUiArtifacts, GameIdentity, LaunchMode, LaunchOptions, LaunchPlan, LaunchResult,
    launch_game, launch_game_via_macos_app,
};
pub use layout::{MinecraftLayout, safe_join};
pub use model::*;
pub use natives::{extract_natives, replace_native_from_archive};
pub use rules::{RuleContext, library_is_allowed, native_classifier};
pub use skin::{PlayerSkin, SkinModel, fetch_skin, skin_data_url};

pub const MINECRAFT_VERSION: &str = "1.8.9";
pub const MINECRAFT_VERSION_JSON_URL: &str = "https://piston-meta.mojang.com/v1/packages/d546f1707a3f2b7d034eece5ea2e311eda875787/1.8.9.json";
pub const MINECRAFT_VERSION_JSON_SHA1: &str = "d546f1707a3f2b7d034eece5ea2e311eda875787";
pub const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const JAVA_RUNTIME_INDEX_URL: &str = "https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";
pub const JAVA_RUNTIME_INDEX_SHA1: &str = "0f47bc501bbc5009f34bdedb5a232d2ecce640fa";
pub const JAVA_RUNTIME_INDEX_SIZE: u64 = 13_385;
