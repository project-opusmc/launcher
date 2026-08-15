use serde::Deserialize;
use thiserror::Error;

pub const FORGE_RUNTIME_ID: &str = "forge-optifine-1.8.9";
pub const FORGE_PROFILE_ID: &str = "1.8.9-forge1.8.9-11.15.1.2318-1.8.9";
pub const FORGE_MAIN_CLASS: &str = "net.minecraft.launchwrapper.Launch";
pub const FORGE_OPTIFINE_FILE_NAME: &str = "OptiFine_1.8.9_HD_U_M5.jar";
/// The reproducible RBW coremod artifact staged beside the desktop launcher.
/// All launcher flavors use the same reviewed bytes so Demo/Release artifact
/// drift cannot produce a build that installs successfully but cannot launch.
pub const FORGE_COREMOD_SHA1: &str = "5d4e44450083b28559d067ccf7f53ab2b73b9984";
pub const FORGE_COREMOD_SIZE: u64 = 101_444;
/// The reviewed typed Forge client mod staged by `prepareBootstrap`.
pub const FORGE_CLIENT_MOD_SHA1: &str = "07b137286bf10e4459405a775b36374730e597c9";
pub const FORGE_CLIENT_MOD_SIZE: u64 = 335_621;

const FORGE_LOCK_JSON: &str = include_str!("../runtime-lock/forge-1.8.9-11.15.1.2318.lock.json");

/// A checked-in, audited loader manifest. It deliberately contains fully
/// resolved URLs, paths, hashes, and sizes so a user installation never needs
/// to execute the Forge installer or trust mutable Forge profile metadata.
#[derive(Debug, Clone, Deserialize)]
pub struct ForgeRuntimeLock {
    pub schema_version: u32,
    pub runtime_id: String,
    pub minecraft_version: String,
    pub profile_id: String,
    pub main_class: String,
    pub minecraft_arguments: String,
    pub libraries: Vec<ForgeLibrary>,
    pub optifine: OptiFineLock,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ForgeLibrary {
    pub coordinate: String,
    pub relative_path: String,
    pub url: String,
    pub sha1: String,
    pub size: u64,
}

/// OptiFine is never downloaded or bundled by RBW. The caller may only import
/// a locally user-provided JAR matching this immutable contract into RBW's
/// isolated runtime.
#[derive(Debug, Clone, Deserialize)]
pub struct OptiFineLock {
    pub mode: String,
    pub coordinate: String,
    pub file_name: String,
    pub sha1: String,
    pub size: u64,
}

impl ForgeRuntimeLock {
    pub fn load() -> Result<Self, ForgeLockError> {
        let lock: Self = serde_json::from_str(FORGE_LOCK_JSON)?;
        lock.validate()?;
        Ok(lock)
    }

    fn validate(&self) -> Result<(), ForgeLockError> {
        if self.schema_version != 1 {
            return Err(ForgeLockError::UnsupportedSchema(self.schema_version));
        }
        if self.runtime_id != FORGE_RUNTIME_ID
            || self.minecraft_version != crate::MINECRAFT_VERSION
            || self.profile_id != FORGE_PROFILE_ID
            || self.main_class != FORGE_MAIN_CLASS
        {
            return Err(ForgeLockError::UnexpectedIdentity);
        }
        if !self
            .minecraft_arguments
            .contains("--tweakClass net.minecraftforge.fml.common.launcher.FMLTweaker")
            || !self.minecraft_arguments.contains("${version_name}")
        {
            return Err(ForgeLockError::UnexpectedLaunchArguments);
        }
        if self.libraries.is_empty() {
            return Err(ForgeLockError::NoLibraries);
        }
        for library in &self.libraries {
            validate_library(library)?;
        }
        if self.optifine.mode != "user-provided"
            || self.optifine.file_name != FORGE_OPTIFINE_FILE_NAME
            || !is_sha1(&self.optifine.sha1)
            || self.optifine.size == 0
            || self.optifine.coordinate.trim().is_empty()
        {
            return Err(ForgeLockError::UnexpectedOptiFineContract);
        }
        Ok(())
    }
}

fn validate_library(library: &ForgeLibrary) -> Result<(), ForgeLockError> {
    if library.coordinate.trim().is_empty()
        || library.relative_path.trim().is_empty()
        || !library.relative_path.ends_with(".jar")
        || library.size == 0
        || !is_sha1(&library.sha1)
    {
        return Err(ForgeLockError::InvalidLibrary(library.coordinate.clone()));
    }
    let parsed = url::Url::parse(&library.url)
        .map_err(|_| ForgeLockError::InvalidLibrary(library.coordinate.clone()))?;
    let approved_host = matches!(
        parsed.host_str(),
        Some("maven.minecraftforge.net") | Some("libraries.minecraft.net")
    );
    if parsed.scheme() != "https" || !approved_host {
        return Err(ForgeLockError::UnapprovedLibraryUrl {
            coordinate: library.coordinate.clone(),
            url: library.url.clone(),
        });
    }
    Ok(())
}

fn is_sha1(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, Error)]
pub enum ForgeLockError {
    #[error("Forge runtime lock is invalid JSON")]
    Json(#[from] serde_json::Error),
    #[error("unsupported Forge runtime lock schema {0}")]
    UnsupportedSchema(u32),
    #[error("Forge runtime lock has an unexpected identity")]
    UnexpectedIdentity,
    #[error("Forge runtime lock has unexpected launch arguments")]
    UnexpectedLaunchArguments,
    #[error("Forge runtime lock contains no libraries")]
    NoLibraries,
    #[error("Forge runtime lock library is invalid: {0}")]
    InvalidLibrary(String),
    #[error("Forge runtime lock library URL is not approved for {coordinate}: {url}")]
    UnapprovedLibraryUrl { coordinate: String, url: String },
    #[error("Forge runtime lock has an unexpected OptiFine import contract")]
    UnexpectedOptiFineContract,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_lock_is_a_pinned_forge_optifine_contract() {
        let lock = ForgeRuntimeLock::load().unwrap();
        assert_eq!(lock.profile_id, FORGE_PROFILE_ID);
        assert_eq!(lock.main_class, FORGE_MAIN_CLASS);
        assert_eq!(
            lock.libraries[0].coordinate,
            "net.minecraftforge:forge:1.8.9-11.15.1.2318-1.8.9:universal"
        );
        assert_eq!(lock.optifine.file_name, "OptiFine_1.8.9_HD_U_M5.jar");
        assert_eq!(
            lock.optifine.sha1,
            "d362d58a28f5373b141b9e426e8e160638bfafcd"
        );
        assert_eq!(lock.optifine.size, 2_585_014);
        assert_eq!(
            FORGE_COREMOD_SHA1,
            "5d4e44450083b28559d067ccf7f53ab2b73b9984"
        );
        assert_eq!(FORGE_COREMOD_SIZE, 101_444);
        assert_eq!(
            FORGE_CLIENT_MOD_SHA1,
            "07b137286bf10e4459405a775b36374730e597c9"
        );
        assert_eq!(FORGE_CLIENT_MOD_SIZE, 335_621);
    }
}
