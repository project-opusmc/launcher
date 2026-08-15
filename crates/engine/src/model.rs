use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
pub struct VersionManifest {
    pub versions: Vec<VersionSummary>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionSummary {
    pub id: String,
    pub url: String,
    pub sha1: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinecraftVersion {
    pub id: String,
    pub main_class: String,
    pub minecraft_arguments: String,
    pub assets: String,
    pub asset_index: AssetIndexSummary,
    pub downloads: VersionDownloads,
    pub libraries: Vec<Library>,
    pub logging: Option<LoggingConfiguration>,
    pub java_version: Option<JavaVersion>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JavaVersion {
    pub component: String,
    #[serde(rename = "majorVersion")]
    pub major_version: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionDownloads {
    pub client: Artifact,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIndexSummary {
    pub id: String,
    pub sha1: String,
    pub size: u64,
    pub total_size: Option<u64>,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetIndex {
    pub objects: BTreeMap<String, AssetObject>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Artifact {
    pub path: Option<String>,
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Library {
    pub name: String,
    pub downloads: LibraryDownloads,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub natives: BTreeMap<String, String>,
    pub extract: Option<LibraryExtract>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LibraryDownloads {
    pub artifact: Option<Artifact>,
    #[serde(default)]
    pub classifiers: BTreeMap<String, Artifact>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LibraryExtract {
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Allow,
    Disallow,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub action: RuleAction,
    pub os: Option<RuleOs>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuleOs {
    pub name: Option<String>,
    pub arch: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfiguration {
    pub client: ClientLoggingConfiguration,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClientLoggingConfiguration {
    pub argument: String,
    pub file: LoggingFile,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingFile {
    pub id: String,
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

pub type JavaRuntimeIndex = BTreeMap<String, BTreeMap<String, Vec<JavaRuntimeEntry>>>;

#[derive(Debug, Clone, Deserialize)]
pub struct JavaRuntimeEntry {
    pub manifest: RuntimeManifestSummary,
    pub version: RuntimeVersion,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeManifestSummary {
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeVersion {
    pub name: String,
    pub released: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeManifest {
    pub files: BTreeMap<String, RuntimeFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeFile {
    #[serde(rename = "type")]
    pub kind: RuntimeFileKind,
    #[serde(default)]
    pub downloads: BTreeMap<String, Artifact>,
    #[serde(default)]
    pub executable: bool,
    pub target: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeFileKind {
    File,
    Directory,
    Link,
}
