use crate::{FORGE_OPTIFINE_FILE_NAME, FORGE_PROFILE_ID, MINECRAFT_VERSION};
use rbw_platform::RbwPaths;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct MinecraftLayout {
    pub paths: RbwPaths,
}

impl MinecraftLayout {
    pub fn new(paths: RbwPaths) -> Self {
        Self { paths }
    }

    pub fn versions_dir(&self) -> PathBuf {
        self.paths.minecraft.join("versions")
    }

    pub fn version_dir(&self) -> PathBuf {
        self.version_dir_for(MINECRAFT_VERSION)
    }

    pub fn version_dir_for(&self, version_id: &str) -> PathBuf {
        self.versions_dir().join(version_id)
    }

    pub fn version_json(&self) -> PathBuf {
        self.version_dir().join(format!("{MINECRAFT_VERSION}.json"))
    }

    pub fn client_jar(&self) -> PathBuf {
        self.version_dir().join(format!("{MINECRAFT_VERSION}.jar"))
    }

    pub fn forge_version_dir(&self) -> PathBuf {
        self.version_dir_for(FORGE_PROFILE_ID)
    }

    pub fn forge_profile_marker(&self) -> PathBuf {
        self.forge_version_dir().join("rbw-forge-profile.json")
    }

    pub fn install_state(&self) -> PathBuf {
        self.paths.root.join("install-state-v1.json")
    }

    pub fn java_runtime_index(&self) -> PathBuf {
        self.paths.runtime.join("indexes/java-runtime-all-v1.json")
    }

    pub fn libraries_dir(&self) -> PathBuf {
        self.paths.minecraft.join("libraries")
    }

    pub fn mods_dir(&self) -> PathBuf {
        self.paths.game.join("mods")
    }

    pub fn optifine_mod(&self) -> PathBuf {
        self.mods_dir().join(FORGE_OPTIFINE_FILE_NAME)
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.paths.minecraft.join("assets")
    }

    pub fn asset_index(&self, id: &str) -> Result<PathBuf, UnsafeRelativePath> {
        safe_join(&self.assets_dir().join("indexes"), &format!("{id}.json"))
    }

    pub fn asset_object(&self, hash: &str) -> Result<PathBuf, UnsafeRelativePath> {
        if hash.len() != 40 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(UnsafeRelativePath(hash.to_owned()));
        }
        safe_join(
            &self.assets_dir().join("objects"),
            &format!("{}/{hash}", &hash[..2]),
        )
    }

    pub fn logging_file(&self, id: &str) -> Result<PathBuf, UnsafeRelativePath> {
        safe_join(&self.assets_dir().join("log_configs"), id)
    }

    pub fn library_file(&self, path: &str) -> Result<PathBuf, UnsafeRelativePath> {
        safe_join(&self.libraries_dir(), path)
    }
}

pub fn safe_join(base: &Path, relative: &str) -> Result<PathBuf, UnsafeRelativePath> {
    let candidate = Path::new(relative);
    let has_unsafe_segment = relative
        .split(['/', '\\'])
        .any(|segment| segment.is_empty() || segment == "." || segment == "..");
    let looks_like_windows_absolute = relative.as_bytes().get(1) == Some(&b':');
    if candidate.as_os_str().is_empty()
        || candidate.is_absolute()
        || relative.starts_with('/')
        || relative.starts_with('\\')
        || looks_like_windows_absolute
        || has_unsafe_segment
        || candidate
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(UnsafeRelativePath(relative.to_owned()));
    }
    Ok(base.join(candidate))
}

#[derive(Debug, Error)]
#[error("unsafe relative path from remote metadata: {0}")]
pub struct UnsafeRelativePath(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_join_accepts_maven_paths() {
        let joined = safe_join(Path::new("/tmp/base"), "a/b/library.jar").unwrap();
        assert_eq!(joined, Path::new("/tmp/base/a/b/library.jar"));
    }

    #[test]
    fn safe_join_rejects_traversal_and_absolute_paths() {
        assert!(safe_join(Path::new("/tmp/base"), "../escape").is_err());
        assert!(safe_join(Path::new("/tmp/base"), "/escape").is_err());
        assert!(safe_join(Path::new("/tmp/base"), "a/./b").is_err());
        assert!(safe_join(Path::new("/tmp/base"), "..\\escape").is_err());
        assert!(safe_join(Path::new("/tmp/base"), "C:\\escape").is_err());
    }
}
