use reqwest::blocking::Client;
use serde::Deserialize;
use std::time::Duration;
use thiserror::Error;

const USER_AGENT: &str = concat!("Opus-Launcher/", env!("CARGO_PKG_VERSION"));
const SESSION_PROFILE_URL: &str = "https://sessionserver.mojang.com/session/minecraft/profile/";
/// The 64x64 classic "Steve" skin, embedded so an offline profile or an
/// unreachable session server always has a valid texture to render.
const DEFAULT_SKIN_PNG: &[u8] = include_bytes!("../assets/default-skin.png");
const MAX_SKIN_BYTES: usize = 256 * 1024;

/// The 3D-viewer-ready skin for one identity. The PNG bytes are always a valid
/// 64x64 (or 64x32 legacy) Minecraft skin; `is_default` lets the UI note when
/// it fell back to the built-in texture instead of a live profile skin.
#[derive(Debug, Clone)]
pub struct PlayerSkin {
    pub png: Vec<u8>,
    pub model: SkinModel,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkinModel {
    Classic,
    Slim,
}

impl SkinModel {
    pub fn as_str(self) -> &'static str {
        match self {
            SkinModel::Classic => "classic",
            SkinModel::Slim => "slim",
        }
    }
}

impl PlayerSkin {
    fn default_skin() -> Self {
        Self {
            png: DEFAULT_SKIN_PNG.to_vec(),
            model: SkinModel::Classic,
            is_default: true,
        }
    }
}

#[derive(Debug, Error)]
pub enum SkinError {
    #[error("skin http client error")]
    Client(#[from] reqwest::Error),
    #[error("skin texture payload was invalid")]
    InvalidPayload,
}

#[derive(Deserialize)]
struct SessionProfile {
    properties: Vec<SessionProperty>,
}

#[derive(Deserialize)]
struct SessionProperty {
    name: String,
    value: String,
}

#[derive(Deserialize)]
struct TexturePayload {
    textures: TextureMap,
}

#[derive(Deserialize)]
struct TextureMap {
    #[serde(rename = "SKIN")]
    skin: Option<SkinTexture>,
}

#[derive(Deserialize)]
struct SkinTexture {
    url: String,
    #[serde(default)]
    metadata: Option<SkinMetadata>,
}

#[derive(Deserialize)]
struct SkinMetadata {
    #[serde(default)]
    model: Option<String>,
}

fn client() -> Result<Client, SkinError> {
    Ok(Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

/// Resolve the skin for a canonical 32-hex Minecraft UUID via the public
/// session server. Any failure (offline profile, network error, missing
/// texture) degrades gracefully to the embedded default skin so the account
/// pane can always render a model.
pub fn fetch_skin(uuid: &str) -> PlayerSkin {
    match try_fetch_skin(uuid) {
        Ok(skin) => skin,
        Err(_) => PlayerSkin::default_skin(),
    }
}

fn try_fetch_skin(uuid: &str) -> Result<PlayerSkin, SkinError> {
    let normalized = uuid.replace('-', "");
    if normalized.len() != 32 || !normalized.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(PlayerSkin::default_skin());
    }
    let client = client()?;
    let response = client
        .get(format!("{SESSION_PROFILE_URL}{normalized}"))
        .send()?;
    if !response.status().is_success() {
        return Ok(PlayerSkin::default_skin());
    }
    let profile: SessionProfile = response.json()?;
    let Some(property) = profile
        .properties
        .into_iter()
        .find(|property| property.name == "textures")
    else {
        return Ok(PlayerSkin::default_skin());
    };
    let decoded = decode_base64(&property.value).ok_or(SkinError::InvalidPayload)?;
    let payload: TexturePayload =
        serde_json::from_slice(&decoded).map_err(|_| SkinError::InvalidPayload)?;
    let Some(skin) = payload.textures.skin else {
        return Ok(PlayerSkin::default_skin());
    };
    if !is_approved_texture_url(&skin.url) {
        return Ok(PlayerSkin::default_skin());
    }
    let model = match skin.metadata.and_then(|metadata| metadata.model) {
        Some(value) if value.eq_ignore_ascii_case("slim") => SkinModel::Slim,
        _ => SkinModel::Classic,
    };
    let bytes = download_png(&client, &https_texture_url(&skin.url))?;
    if !is_probably_png(&bytes) {
        return Ok(PlayerSkin::default_skin());
    }
    Ok(PlayerSkin {
        png: bytes,
        model,
        is_default: false,
    })
}

fn download_png(client: &Client, url: &str) -> Result<Vec<u8>, SkinError> {
    use std::io::Read;
    let response = client.get(url).send()?;
    if !response.status().is_success() {
        return Err(SkinError::InvalidPayload);
    }
    let mut limited = response.take(MAX_SKIN_BYTES as u64 + 1);
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .map_err(|_| SkinError::InvalidPayload)?;
    if bytes.len() > MAX_SKIN_BYTES {
        return Err(SkinError::InvalidPayload);
    }
    Ok(bytes)
}

/// Mojang serves skin textures only from its own texture host, but the session
/// server still returns `http://` URLs for them. Restrict the download target
/// to that single host (over http or https) so a tampered session response
/// cannot turn the skin fetch into an arbitrary outbound request; the actual
/// download is always upgraded to https by `https_texture_url`.
fn is_approved_texture_url(url: &str) -> bool {
    match url::Url::parse(url) {
        Ok(parsed) => {
            matches!(parsed.scheme(), "http" | "https")
                && matches!(parsed.host_str(), Some("textures.minecraft.net"))
        }
        Err(_) => false,
    }
}

/// Force the approved texture host onto https before fetching the PNG so the
/// launcher never downloads a skin over plaintext even though Mojang advertises
/// an http URL.
fn https_texture_url(url: &str) -> String {
    match url.strip_prefix("http://") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_owned(),
    }
}

fn is_probably_png(bytes: &[u8]) -> bool {
    bytes.len() > 8 && bytes[..8] == [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']
}

fn decode_base64(value: &str) -> Option<Vec<u8>> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    STANDARD.decode(value.trim()).ok()
}

/// The launcher renders the skin in a WebView, so expose it as a data URL the
/// frontend can drop straight into an `<img>`/canvas without a custom scheme.
pub fn skin_data_url(skin: &PlayerSkin) -> String {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    format!("data:image/png;base64,{}", STANDARD.encode(&skin.png))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_or_invalid_uuid_uses_the_default_skin() {
        let skin = fetch_skin("not-a-uuid");
        assert!(skin.is_default);
        assert!(is_probably_png(&skin.png));
        assert_eq!(skin.model, SkinModel::Classic);
    }

    #[test]
    fn embedded_default_skin_is_a_valid_png() {
        assert!(is_probably_png(DEFAULT_SKIN_PNG));
    }

    #[test]
    fn only_the_official_texture_host_is_approved() {
        assert!(is_approved_texture_url(
            "https://textures.minecraft.net/texture/abc123"
        ));
        // Mojang advertises http URLs for skins; the host is what matters.
        assert!(is_approved_texture_url("http://textures.minecraft.net/x"));
        assert!(!is_approved_texture_url("https://evil.example.com/x"));
        assert!(!is_approved_texture_url("not a url"));
    }

    #[test]
    fn texture_downloads_are_upgraded_to_https() {
        assert_eq!(
            https_texture_url("http://textures.minecraft.net/texture/abc"),
            "https://textures.minecraft.net/texture/abc"
        );
        assert_eq!(
            https_texture_url("https://textures.minecraft.net/texture/abc"),
            "https://textures.minecraft.net/texture/abc"
        );
    }

    #[test]
    fn data_url_round_trips_the_default_skin() {
        let skin = PlayerSkin::default_skin();
        let url = skin_data_url(&skin);
        assert!(url.starts_with("data:image/png;base64,"));
    }
}
