use rbw_platform::{Architecture, OperatingSystem, Platform};
use rbw_runtime::{
    DownloadSpec, Downloader, JAVA_RUNTIME_INDEX_SHA1, JAVA_RUNTIME_INDEX_SIZE,
    JAVA_RUNTIME_INDEX_URL, JavaRuntimeIndex, MINECRAFT_VERSION, MINECRAFT_VERSION_JSON_SHA1,
    MINECRAFT_VERSION_JSON_URL, MinecraftVersion, RuleContext, VERSION_MANIFEST_URL,
    VersionManifest, library_is_allowed, native_classifier,
};
use std::fs::File;
use std::io::BufReader;

#[test]
#[ignore = "requires official Mojang network endpoints"]
fn official_1_8_9_contract_matches_foundation_assumptions() {
    let downloader = Downloader::new().unwrap();
    let manifest: VersionManifest = downloader.get_json(VERSION_MANIFEST_URL).unwrap();
    let summary = manifest
        .versions
        .iter()
        .find(|version| version.id == MINECRAFT_VERSION)
        .expect("official manifest must still expose 1.8.9");
    assert_eq!(summary.url, MINECRAFT_VERSION_JSON_URL);
    assert_eq!(summary.sha1, MINECRAFT_VERSION_JSON_SHA1);

    let temp = tempfile::tempdir().unwrap();
    let version_path = temp.path().join("1.8.9.json");
    downloader
        .ensure(
            &DownloadSpec {
                url: summary.url.clone(),
                sha1: summary.sha1.clone(),
                size: None,
            },
            &version_path,
        )
        .unwrap();
    let version: MinecraftVersion =
        serde_json::from_reader(BufReader::new(File::open(version_path).unwrap())).unwrap();

    assert_eq!(version.id, "1.8.9");
    assert_eq!(version.main_class, "net.minecraft.client.main.Main");
    assert_eq!(version.java_version.as_ref().unwrap().major_version, 8);
    assert_eq!(
        version.java_version.as_ref().unwrap().component,
        "jre-legacy"
    );
    assert!(version.minecraft_arguments.contains("${auth_access_token}"));

    assert_platform_libraries(
        &version,
        Platform {
            os: OperatingSystem::MacOs,
            host_arch: Architecture::Aarch64,
            game_arch: Architecture::X86_64,
        },
        "2.9.2-nightly-20140822",
        "2.9.4-nightly-20150209",
    );
    assert_platform_libraries(
        &version,
        Platform {
            os: OperatingSystem::Windows,
            host_arch: Architecture::X86_64,
            game_arch: Architecture::X86_64,
        },
        "2.9.4-nightly-20150209",
        "2.9.2-nightly-20140822",
    );

    let logging = &version.logging.as_ref().unwrap().client.file;
    let logging_path = temp.path().join("client-log4j.xml");
    downloader
        .ensure(
            &DownloadSpec {
                url: logging.url.clone(),
                sha1: logging.sha1.clone(),
                size: Some(logging.size),
            },
            &logging_path,
        )
        .unwrap();
    let logging_text = std::fs::read_to_string(logging_path).unwrap();
    assert!(logging_text.contains("RegexFilter"));
    assert!(logging_text.contains("\\$\\{"));

    let runtime_index_path = temp.path().join("java-runtime-index.json");
    downloader
        .ensure(
            &DownloadSpec {
                url: JAVA_RUNTIME_INDEX_URL.to_owned(),
                sha1: JAVA_RUNTIME_INDEX_SHA1.to_owned(),
                size: Some(JAVA_RUNTIME_INDEX_SIZE),
            },
            &runtime_index_path,
        )
        .unwrap();
    let runtime_index: JavaRuntimeIndex =
        serde_json::from_reader(BufReader::new(File::open(runtime_index_path).unwrap())).unwrap();
    assert_runtime(&runtime_index, "mac-os", "8u74-cacert462b08");
    assert_runtime(&runtime_index, "windows-x64", "8u51-cacert462b08");
}

fn assert_runtime(index: &JavaRuntimeIndex, platform: &str, expected_version: &str) {
    let entry = index
        .get(platform)
        .and_then(|components| components.get("jre-legacy"))
        .and_then(|entries| entries.last())
        .expect("pinned runtime index must contain jre-legacy");
    assert_eq!(entry.version.name, expected_version);
    assert_eq!(entry.manifest.url.split('/').next(), Some("https:"));
    assert_eq!(entry.manifest.sha1.len(), 40);
    assert!(entry.manifest.size > 0);
}

fn assert_platform_libraries(
    version: &MinecraftVersion,
    platform: Platform,
    required_lwjgl: &str,
    forbidden_lwjgl: &str,
) {
    let context = RuleContext {
        platform,
        os_version: None,
    };
    let selected = version
        .libraries
        .iter()
        .filter(|library| library_is_allowed(library, &context))
        .collect::<Vec<_>>();
    assert!(
        selected.len() >= 25,
        "selected classpath is unexpectedly small"
    );
    assert!(
        selected
            .iter()
            .any(|library| library.name.contains(required_lwjgl))
    );
    assert!(
        !selected
            .iter()
            .any(|library| library.name.contains(forbidden_lwjgl))
    );

    let mut native_count = 0;
    for library in selected {
        if let Some(artifact) = &library.downloads.artifact {
            assert!(
                artifact.path.is_some(),
                "{} has no artifact path",
                library.name
            );
        }
        if let Some(classifier) = native_classifier(library, platform) {
            assert!(
                library.downloads.classifiers.contains_key(&classifier),
                "{} is missing classifier {}",
                library.name,
                classifier
            );
            native_count += 1;
        }
    }
    assert!(native_count >= 2, "native selection is unexpectedly small");
}
