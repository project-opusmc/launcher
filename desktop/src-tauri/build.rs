#[cfg(target_os = "macos")]
use std::env;
#[cfg(target_os = "macos")]
use std::fs;
#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=resources/macos-game-stub.c");
    println!("cargo:rerun-if-changed=resources/Opus Client.app/Contents/Info.plist");

    #[cfg(target_os = "macos")]
    build_macos_game_stub();

    tauri_build::build()
}

#[cfg(target_os = "macos")]
fn build_macos_game_stub() {
    let manifest_directory = PathBuf::from(
        env::var("CARGO_MANIFEST_DIR").expect("Cargo must provide CARGO_MANIFEST_DIR"),
    );
    let source = manifest_directory.join("resources/macos-game-stub.c");
    let executable =
        manifest_directory.join("resources/Opus Client.app/Contents/MacOS/Opus Client");

    if !macos_game_stub_needs_rebuild(&source, &executable) {
        return;
    }

    fs::create_dir_all(
        executable
            .parent()
            .expect("game stub executable must have a parent directory"),
    )
    .expect("could not create the macOS game app executable directory");

    // Tauri watches the bundled game app in development. Clang writes a
    // transient `.lipo` file next to a universal-binary output, so compiling
    // directly into that directory causes its own resource watcher to trigger
    // another Cargo build. Stage the binary under OUT_DIR instead, then replace
    // the watched resource only when the C source is actually newer.
    let staged_executable = PathBuf::from(
        env::var("OUT_DIR").expect("Cargo must provide OUT_DIR for the game app stub"),
    )
    .join("rbw-game-app-stub");
    let _ = fs::remove_file(&staged_executable);
    let _ = fs::remove_file(staged_executable.with_extension("lipo"));

    let status = Command::new("xcrun")
        .args([
            "--sdk",
            "macosx",
            "clang",
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-arch",
            "arm64",
            "-arch",
            "x86_64",
            "-mmacosx-version-min=11.0",
        ])
        .arg(&source)
        .arg("-o")
        .arg(&staged_executable)
        .status()
        .expect("could not invoke clang for the macOS game app stub");
    assert!(
        status.success(),
        "could not compile the macOS game app stub"
    );

    fs::rename(&staged_executable, &executable)
        .expect("could not install the macOS game app stub into resources");
}

#[cfg(target_os = "macos")]
fn macos_game_stub_needs_rebuild(source: &Path, executable: &Path) -> bool {
    let source_modified = fs::metadata(source)
        .expect("could not read the macOS game app stub source")
        .modified()
        .expect("could not read the macOS game app stub source modification time");

    match fs::metadata(executable) {
        Ok(metadata) => metadata
            .modified()
            .map(|executable_modified| source_modified > executable_modified)
            .unwrap_or(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => panic!("could not read the macOS game app stub executable: {error}"),
    }
}
