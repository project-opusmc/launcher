//! One-time native Minecraft/OptiFine defaults for the managed Opus game.
//! This runs before the JVM reads options, not from a CEF preview or per frame.

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

const PRESET_ID: &str = "opus-native-defaults-v1\n";
const BACKUP_DIRECTORY: &str = ".opus-native-defaults-v1";
const MUSIC_DISABLED: &[(&str, &str)] = &[("soundCategory_music", "0.0")];

// Minecraft 1.8.9 serializes FOV as (degrees - 70) / 40. Quake Pro is
// therefore 1.0 (110 degrees), NOT 100 degrees or a literal fov:110.
// 260 is the native Unlimited sentinel, not a 260-FPS cap; VSync must be off.
const MINECRAFT_DEFAULTS: &[(&str, &str)] = &[
    ("fov", "1.0"),
    ("maxFps", "240"),
    ("enableVsync", "false"),
    ("renderDistance", "8"),
    ("fancyGraphics", "false"),
    ("ao", "0"),
    ("bobView", "false"),
    ("guiScale", "3"),
    ("useVbo", "true"),
    ("gamma", "1.0"),
    ("allowBlockAlternatives", "true"),
    ("renderClouds", "false"),
    ("entityShadows", "false"),
    ("heldItemTooltips", "true"),
    ("mipmapLevels", "4"),
    ("particles", "0"), // All, even with the individual animations disabled.
    ("soundCategory_music", "0.0"),
];

// Values audited against the locally imported, pinned OptiFine 1.8.9 HD U M5:
// OFF is 3 for fog/clouds/rain/grass/lights, 2 for animated water/lava,
// and 1 (not 0) for disabled anisotropic filtering.
const OPTIFINE_DEFAULTS: &[(&str, &str)] = &[
    ("ofFastRender", "true"),
    ("ofAoLevel", "0.0"),
    ("ofDynamicLights", "3"),
    ("ofDynamicFov", "false"),
    ("ofClouds", "3"),
    ("ofCloudsHeight", "0.0"),
    ("ofTrees", "1"),
    ("ofRain", "3"),
    ("ofSky", "false"),
    ("ofStars", "true"),
    ("ofSunMoon", "false"),
    ("ofShowCapes", "true"),
    ("ofFogType", "3"),
    ("ofFogStart", "0.6"),
    ("ofTranslucentBlocks", "0"),
    ("ofDroppedItems", "0"),
    ("ofVignette", "0"),
    ("ofAlternateBlocks", "true"),
    ("ofSwampColors", "true"),
    ("ofSmoothBiomes", "false"),
    ("ofMipmapType", "0"),
    ("ofAfLevel", "1"),
    ("ofAaLevel", "0"),
    ("ofClearWater", "false"),
    ("ofRandomEntities", "false"),
    ("ofBetterGrass", "3"),
    ("ofBetterSnow", "false"),
    ("ofCustomFonts", "false"),
    ("ofCustomColors", "false"),
    ("ofConnectedTextures", "1"),
    ("ofNaturalTextures", "true"),
    ("ofCustomSky", "true"),
    ("ofCustomItems", "true"),
    ("ofCustomEntityModels", "false"),
    ("ofCustomGuis", "true"),
    ("ofEmissiveTextures", "true"),
    ("ofAnimatedWater", "2"),
    ("ofAnimatedLava", "2"),
    ("ofAnimatedFire", "false"),
    ("ofAnimatedPortal", "false"),
    ("ofAnimatedRedstone", "false"),
    ("ofAnimatedExplosion", "false"),
    ("ofAnimatedFlame", "false"),
    ("ofAnimatedSmoke", "false"),
    ("ofVoidParticles", "false"),
    ("ofWaterParticles", "false"),
    ("ofRainSplash", "false"),
    ("ofPortalParticles", "false"),
    ("ofPotionParticles", "false"),
    ("ofDrippingWaterLava", "false"),
    ("ofAnimatedTerrain", "false"),
    ("ofAnimatedTextures", "false"),
    ("ofFireworkParticles", "false"),
    ("ofSmoothFps", "true"),
    ("ofFastMath", "true"),
    ("ofSmartAnimations", "true"),
    ("ofRenderRegions", "true"),
    ("ofChunkUpdatesDynamic", "true"),
    ("ofLazyChunkLoading", "true"),
];

#[derive(Serialize, Deserialize)]
struct OriginalFiles {
    options_existed: bool,
    optifine_options_existed: bool,
}

/// A linked game directory may belong to another client. Check before the
/// launch engine stages mods as well as immediately before preset writes.
pub(crate) fn validate_game_directory(game: &Path) -> io::Result<()> {
    match fs::symlink_metadata(game) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(unsafe_path(game)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(crate) fn apply_native_defaults(game: &Path) -> io::Result<()> {
    validate_game_directory(game)?;
    fs::create_dir_all(game)?;
    validate_game_directory(game)?;
    let backup = game.join(BACKUP_DIRECTORY);
    match fs::symlink_metadata(&backup) {
        Ok(metadata) if !metadata.is_dir() => return Err(unsafe_path(&backup)),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(&backup)?,
        Err(error) => return Err(error),
    }
    let lock_path = backup.join("lock");
    check_regular_file_or_missing(&lock_path)?;
    let lock = private_file_options()
        .read(true)
        .create(true)
        .open(&lock_path)?;
    // Never race two launch preparations for the same managed game directory.
    FileExt::try_lock_exclusive(&lock)?;

    let options = game.join("options.txt");
    let optifine = game.join("optionsof.txt");
    let original_options = read_optional(&options)?;
    let original_optifine = read_optional(&optifine)?;
    // Validate both inputs before backing up or changing either settings file.
    let options_text = settings_text(original_options.as_deref())?;
    let optifine_text = settings_text(original_optifine.as_deref())?;
    let applied_path = backup.join("applied");
    let applied = match read_optional(&applied_path)? {
        Some(bytes) if bytes == PRESET_ID.as_bytes() => true,
        Some(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid Opus native-defaults receipt",
            ));
        }
        None => false,
    };

    if !applied {
        let originals_path = backup.join("originals.json");
        if let Some(bytes) = read_optional(&originals_path)? {
            let originals: OriginalFiles =
                serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            // A prior interrupted migration keeps its first originals. A
            // missing backup is an error, never permission to overwrite it.
            for (name, existed) in [
                ("options.txt", originals.options_existed),
                ("optionsof.txt", originals.optifine_options_existed),
            ] {
                if existed && read_optional(&backup.join(name))?.is_none() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Opus native-defaults original backup is missing",
                    ));
                }
            }
        } else {
            backup_original(&backup.join("options.txt"), original_options.as_deref())?;
            backup_original(&backup.join("optionsof.txt"), original_optifine.as_deref())?;
            let originals = OriginalFiles {
                options_existed: original_options.is_some(),
                optifine_options_existed: original_optifine.is_some(),
            };
            let metadata = serde_json::to_vec_pretty(&originals).map_err(io::Error::other)?;
            write_atomic(&originals_path, &metadata)?;
        }
    }

    let next_options = merge_settings(
        options_text,
        if applied && original_options.is_some() {
            MUSIC_DISABLED
        } else {
            MINECRAFT_DEFAULTS
        },
    );
    if original_options.as_deref() != Some(next_options.as_bytes()) {
        write_atomic(&options, next_options.as_bytes())?;
    }
    if !applied || original_optifine.is_none() {
        let next_optifine = merge_settings(optifine_text, OPTIFINE_DEFAULTS);
        if original_optifine.as_deref() != Some(next_optifine.as_bytes()) {
            write_atomic(&optifine, next_optifine.as_bytes())?;
        }
    }
    // Commit only after both files are durable. On interruption, retry the
    // migration while retaining the immutable, pre-migration originals.
    if !applied {
        write_atomic(&applied_path, PRESET_ID.as_bytes())?;
    }
    Ok(())
}

fn unsafe_path(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "native settings path is not a regular owned file/directory: {}",
            path.display()
        ),
    )
}

fn check_regular_file_or_missing(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(unsafe_path(path)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    check_regular_file_or_missing(path)?;
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn settings_text(bytes: Option<&[u8]>) -> io::Result<&str> {
    std::str::from_utf8(bytes.unwrap_or_default())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn backup_original(path: &Path, bytes: Option<&[u8]>) -> io::Result<()> {
    let Some(bytes) = bytes else {
        return Ok(());
    };
    if let Some(previous) = read_optional(path)? {
        if previous != bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "uncommitted Opus native-defaults backup differs from original; refusing to overwrite",
            ));
        }
        return Ok(());
    }
    write_atomic(path, bytes)
}

fn merge_settings(text: &str, changes: &[(&str, &str)]) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut seen = HashSet::new();
    let mut merged = String::with_capacity(text.len() + changes.len() * 32);
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let change = body
            .split_once(':')
            .and_then(|(key, _)| changes.iter().find(|(name, _)| *name == key));
        if let Some((key, value)) = change {
            if seen.insert(*key) {
                merged.push_str(&format!("{key}:{value}{newline}"));
            }
        } else {
            merged.push_str(line);
        }
    }
    for (key, value) in changes {
        if seen.insert(*key) {
            if !merged.is_empty() && !merged.ends_with('\n') {
                merged.push_str(newline);
            }
            merged.push_str(&format!("{key}:{value}{newline}"));
        }
    }
    merged
}

fn private_file_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    check_regular_file_or_missing(path)?;
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
    let id: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let temporary = path.with_file_name(format!(".opus-preset-{id}.tmp"));
    let result = (|| {
        let mut file = private_file_options().create_new(true).open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        fs::File::open(path.parent().expect("settings file has a parent"))?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
