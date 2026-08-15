use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::blocking::{Client, Response};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;
use url::Url;

pub const MICROSOFT_CLIENT_ID_ENV: &str = "OPUS_MICROSOFT_CLIENT_ID";
const OAUTH_SCOPE: &str = "XboxLive.signin offline_access";
const AUTHORIZATION_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize";
const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBOX_USER_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MINECRAFT_LOGIN_URL: &str =
    "https://api.minecraftservices.com/authentication/login_with_xbox";
const MINECRAFT_INVALID_APP_REGISTRATION_MESSAGE: &str =
    "Invalid app registration, see https://aka.ms/AppRegInfo for more information";
const MINECRAFT_ENTITLEMENTS_URL: &str = "https://api.minecraftservices.com/entitlements/mcstore";
const MINECRAFT_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
const KEYRING_SERVICE: &str = "org.polydevs.opusmc.launcher.microsoft.refresh-token";
const KEYRING_ACCOUNT: &str = "default";
const KEYRING_PROFILE_PREFIX: &str = "profile-";
const LOOPBACK_CALLBACK_HOST: &str = "localhost";
const LOOPBACK_CALLBACK_PATH: &str = "/";
const BROWSER_AUTH_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const LOOPBACK_POLL_INTERVAL: Duration = Duration::from_millis(50);
const CALLBACK_READ_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CALLBACK_REQUEST_BYTES: usize = 8 * 1024;

#[derive(Clone)]
pub struct MinecraftSession {
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    pub user_type: String,
}

impl MinecraftSession {
    pub fn redacted_summary(&self) -> String {
        format!("{} ({})", self.username, self.uuid)
    }
}

pub struct AuthenticatedAccount {
    pub session: MinecraftSession,
    refresh_token: String,
}

impl AuthenticatedAccount {
    pub fn save_refresh_token(&self, store: &RefreshTokenStore) -> Result<(), AuthError> {
        store.save(&self.refresh_token)
    }

    pub fn save_refresh_token_for_profile(&self) -> Result<(), AuthError> {
        let store = RefreshTokenStore::for_profile(&self.session.uuid)?;
        self.save_refresh_token(&store)
    }
}

pub struct DeviceAuthorization {
    device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub message: String,
    pub expires_in: Duration,
    pub interval: Duration,
}

/// Cooperative cancellation for a desktop browser login. The signal contains
/// no credential material and can be shared safely between the Tauri command
/// and its blocking OAuth task.
#[derive(Clone)]
pub struct BrowserLoginCancellation {
    cancelled: Arc<AtomicBool>,
}

impl BrowserLoginCancellation {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn ensure_not_cancelled(&self) -> Result<(), AuthError> {
        if self.is_cancelled() {
            return Err(AuthError::BrowserAuthorizationCancelled);
        }
        Ok(())
    }
}

impl Default for BrowserLoginCancellation {
    fn default() -> Self {
        Self::new()
    }
}

/// An in-memory OAuth authorization request for a native desktop client. It
/// owns the loopback listener, state and PKCE verifier until the browser
/// redirects back to Opus Launcher.
pub struct BrowserAuthorization {
    listeners: Vec<TcpListener>,
    authorization_url: String,
    redirect_uri: String,
    callback_host: String,
    state: String,
    code_verifier: String,
    deadline: Instant,
}

impl BrowserAuthorization {
    pub fn authorization_url(&self) -> &str {
        &self.authorization_url
    }

    fn wait_for_callback(
        &mut self,
        cancellation: &BrowserLoginCancellation,
    ) -> Result<String, AuthError> {
        self.wait_for_callback_until(self.deadline, cancellation)
    }

    fn wait_for_callback_until(
        &mut self,
        deadline: Instant,
        cancellation: &BrowserLoginCancellation,
    ) -> Result<String, AuthError> {
        loop {
            cancellation.ensure_not_cancelled()?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AuthError::BrowserAuthorizationTimedOut);
            }

            for listener in &self.listeners {
                cancellation.ensure_not_cancelled()?;
                let (mut stream, peer) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(AuthError::LoopbackCallback(error)),
                };

                if !peer.ip().is_loopback() {
                    write_callback_page(&mut stream, CallbackPage::Invalid);
                    continue;
                }

                let read_timeout = remaining.min(CALLBACK_READ_TIMEOUT);
                let outcome = read_callback_target(&mut stream, read_timeout, &self.callback_host)
                    .map(|target| parse_browser_callback(&target, &self.state))
                    .unwrap_or(CallbackOutcome::Ignore);
                cancellation.ensure_not_cancelled()?;
                match outcome {
                    CallbackOutcome::Code(code) => {
                        write_callback_page(&mut stream, CallbackPage::Complete);
                        return Ok(code);
                    }
                    CallbackOutcome::Declined => {
                        write_callback_page(&mut stream, CallbackPage::Declined);
                        return Err(AuthError::AuthorizationDeclined);
                    }
                    CallbackOutcome::Ignore => {
                        write_callback_page(&mut stream, CallbackPage::Invalid)
                    }
                }
            }

            thread::sleep(remaining.min(LOOPBACK_POLL_INTERVAL));
        }
    }
}

enum CallbackOutcome {
    Code(String),
    Declined,
    Ignore,
}

enum CallbackPage {
    Complete,
    Declined,
    Invalid,
}

pub struct MicrosoftAuthenticator {
    client_id: String,
    client: Client,
}

impl MicrosoftAuthenticator {
    pub fn from_environment() -> Result<Self, AuthError> {
        let client_id = std::env::var(MICROSOFT_CLIENT_ID_ENV)
            .map_err(|_| AuthError::MissingClientId(MICROSOFT_CLIENT_ID_ENV.to_owned()))?;
        Self::new(client_id)
    }

    pub fn new(client_id: String) -> Result<Self, AuthError> {
        validate_client_id(&client_id)?;
        let client = Client::builder()
            .user_agent(concat!("Opus-Launcher/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(45))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { client_id, client })
    }

    pub fn begin_device_authorization(&self) -> Result<DeviceAuthorization, AuthError> {
        let response = self
            .client
            .post(DEVICE_CODE_URL)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("scope", OAUTH_SCOPE),
            ])
            .send()?;
        let payload: DeviceCodeResponse =
            decode_success(response, "Microsoft device authorization")?;
        if payload.device_code.is_empty()
            || payload.user_code.is_empty()
            || payload.verification_uri.is_empty()
        {
            return Err(AuthError::InvalidServiceResponse(
                "Microsoft device authorization returned empty fields".to_owned(),
            ));
        }
        validate_verification_uri(&payload.verification_uri)?;
        Ok(DeviceAuthorization {
            device_code: payload.device_code,
            user_code: payload.user_code,
            verification_uri: payload.verification_uri,
            message: payload.message,
            expires_in: Duration::from_secs(payload.expires_in),
            interval: Duration::from_secs(payload.interval.max(1)),
        })
    }

    /// Starts a native-browser OAuth Authorization Code flow. The caller must
    /// open `BrowserAuthorization::authorization_url` in the system browser,
    /// then pass the same authorization back to
    /// `complete_browser_authorization`.
    pub fn begin_browser_authorization(&self) -> Result<BrowserAuthorization, AuthError> {
        let (listeners, port) = bind_loopback_listeners()?;
        let redirect_uri =
            format!("http://{LOOPBACK_CALLBACK_HOST}:{port}{LOOPBACK_CALLBACK_PATH}");
        let callback_host = format!("{LOOPBACK_CALLBACK_HOST}:{port}");
        let state = random_urlsafe_value(32)?;
        let code_verifier = random_urlsafe_value(64)?;
        let code_challenge = pkce_challenge(&code_verifier);
        let mut authorization_url = Url::parse(AUTHORIZATION_URL).map_err(|_| {
            AuthError::InvalidServiceResponse(
                "Microsoft authorization endpoint is invalid".to_owned(),
            )
        })?;
        authorization_url
            .query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("response_type", "code")
            .append_pair("response_mode", "query")
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("scope", OAUTH_SCOPE)
            .append_pair("state", &state)
            .append_pair("code_challenge", &code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("prompt", "select_account");

        Ok(BrowserAuthorization {
            listeners,
            authorization_url: authorization_url.into(),
            redirect_uri,
            callback_host,
            state,
            code_verifier,
            deadline: Instant::now() + BROWSER_AUTH_TIMEOUT,
        })
    }

    pub fn complete_browser_authorization(
        &self,
        mut authorization: BrowserAuthorization,
        cancellation: &BrowserLoginCancellation,
    ) -> Result<AuthenticatedAccount, AuthError> {
        let authorization_code = authorization.wait_for_callback(cancellation)?;
        cancellation.ensure_not_cancelled()?;
        let response = self
            .client
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", self.client_id.as_str()),
                ("code", authorization_code.as_str()),
                ("redirect_uri", authorization.redirect_uri.as_str()),
                ("code_verifier", authorization.code_verifier.as_str()),
            ])
            .send()?;
        let tokens: OAuthTokenResponse =
            decode_success(response, "Microsoft OAuth authorization code")?;
        let refresh_token = tokens.refresh_token.ok_or_else(|| {
            AuthError::InvalidServiceResponse(
                "Microsoft token response omitted refresh_token".to_owned(),
            )
        })?;
        cancellation.ensure_not_cancelled()?;
        self.finish_login(&tokens.access_token, refresh_token)
    }

    pub fn complete_device_authorization(
        &self,
        authorization: &DeviceAuthorization,
    ) -> Result<AuthenticatedAccount, AuthError> {
        let started = Instant::now();
        let mut interval = authorization.interval;
        loop {
            if started.elapsed() >= authorization.expires_in {
                return Err(AuthError::DeviceCodeExpired);
            }
            thread::sleep(interval);

            let response = self
                .client
                .post(TOKEN_URL)
                .form(&[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("client_id", self.client_id.as_str()),
                    ("device_code", authorization.device_code.as_str()),
                ])
                .send()?;
            let status = response.status();
            let bytes = response.bytes()?;
            if status.is_success() {
                let tokens: OAuthTokenResponse = serde_json::from_slice(&bytes)?;
                let refresh_token = tokens.refresh_token.ok_or_else(|| {
                    AuthError::InvalidServiceResponse(
                        "Microsoft token response omitted refresh_token".to_owned(),
                    )
                })?;
                return self.finish_login(&tokens.access_token, refresh_token);
            }

            let pending: OAuthErrorResponse =
                serde_json::from_slice(&bytes).map_err(|_| AuthError::ServiceRejected {
                    service: "Microsoft OAuth".to_owned(),
                    status: status.as_u16(),
                    code: "unparseable_error".to_owned(),
                })?;
            match pending.error.as_str() {
                "authorization_pending" => {}
                "slow_down" => interval += Duration::from_secs(5),
                "authorization_declined" => return Err(AuthError::AuthorizationDeclined),
                "expired_token" | "bad_verification_code" => {
                    return Err(AuthError::DeviceCodeExpired);
                }
                code => {
                    return Err(AuthError::ServiceRejected {
                        service: "Microsoft OAuth".to_owned(),
                        status: status.as_u16(),
                        code: code.to_owned(),
                    });
                }
            }
        }
    }

    pub fn refresh_session(
        &self,
        store: &RefreshTokenStore,
    ) -> Result<AuthenticatedAccount, AuthError> {
        let refresh_token = store.load()?.ok_or(AuthError::NoStoredAccount)?;
        let response = self
            .client
            .post(TOKEN_URL)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("scope", OAUTH_SCOPE),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.as_str()),
            ])
            .send()?;
        let tokens: OAuthTokenResponse = decode_success(response, "Microsoft OAuth refresh")?;
        let rotated_refresh_token = tokens.refresh_token.unwrap_or(refresh_token);
        self.finish_login(&tokens.access_token, rotated_refresh_token)
    }

    fn finish_login(
        &self,
        microsoft_access_token: &str,
        refresh_token: String,
    ) -> Result<AuthenticatedAccount, AuthError> {
        let xbox = self.xbox_user_token(microsoft_access_token)?;
        let xsts = self.xsts_token(&xbox.token)?;
        if xbox.user_hash != xsts.user_hash {
            return Err(AuthError::InvalidServiceResponse(
                "Xbox user hash changed during XSTS exchange".to_owned(),
            ));
        }
        let minecraft_access_token = self.minecraft_token(&xsts.user_hash, &xsts.token)?;
        self.verify_entitlement(&minecraft_access_token)?;
        let profile = self.minecraft_profile(&minecraft_access_token)?;

        Ok(AuthenticatedAccount {
            session: MinecraftSession {
                username: profile.name,
                uuid: profile.id,
                access_token: minecraft_access_token,
                user_type: "msa".to_owned(),
            },
            refresh_token,
        })
    }

    fn xbox_user_token(&self, microsoft_access_token: &str) -> Result<XboxToken, AuthError> {
        let response = self
            .client
            .post(XBOX_USER_AUTH_URL)
            .header("x-xbl-contract-version", "1")
            .json(&json!({
                "Properties": {
                    "AuthMethod": "RPS",
                    "SiteName": "user.auth.xboxlive.com",
                    "RpsTicket": format!("d={microsoft_access_token}")
                },
                "RelyingParty": "http://auth.xboxlive.com",
                "TokenType": "JWT"
            }))
            .send()?;
        decode_xbox_token(response, "Xbox Live authentication")
    }

    fn xsts_token(&self, xbox_user_token: &str) -> Result<XboxToken, AuthError> {
        let response = self
            .client
            .post(XSTS_AUTH_URL)
            .header("x-xbl-contract-version", "1")
            .json(&json!({
                "Properties": {
                    "SandboxId": "RETAIL",
                    "UserTokens": [xbox_user_token]
                },
                "RelyingParty": "rp://api.minecraftservices.com/",
                "TokenType": "JWT"
            }))
            .send()?;
        decode_xbox_token(response, "Xbox XSTS authorization")
    }

    fn minecraft_token(&self, user_hash: &str, xsts_token: &str) -> Result<String, AuthError> {
        let response = self
            .client
            .post(MINECRAFT_LOGIN_URL)
            .json(&json!({
                "identityToken": format!("XBL3.0 x={user_hash};{xsts_token}")
            }))
            .send()?;
        let token = decode_minecraft_login(response)?;
        if token.access_token.is_empty() {
            return Err(AuthError::InvalidServiceResponse(
                "Minecraft login returned an empty access token".to_owned(),
            ));
        }
        Ok(token.access_token)
    }

    fn verify_entitlement(&self, minecraft_token: &str) -> Result<(), AuthError> {
        let response = self
            .client
            .get(MINECRAFT_ENTITLEMENTS_URL)
            .bearer_auth(minecraft_token)
            .send()?;
        let entitlements: EntitlementsResponse =
            decode_success(response, "Minecraft entitlements")?;
        if entitlements.items.is_empty() {
            return Err(AuthError::MinecraftOwnershipRequired);
        }
        Ok(())
    }

    fn minecraft_profile(&self, minecraft_token: &str) -> Result<MinecraftProfile, AuthError> {
        let response = self
            .client
            .get(MINECRAFT_PROFILE_URL)
            .bearer_auth(minecraft_token)
            .send()?;
        let profile: MinecraftProfile = decode_success(response, "Minecraft profile")?;
        if profile.id.len() != 32 || profile.name.is_empty() {
            return Err(AuthError::InvalidServiceResponse(
                "Minecraft profile has an invalid id or name".to_owned(),
            ));
        }
        Ok(profile)
    }
}

pub struct RefreshTokenStore {
    entry: keyring::Entry,
}

impl RefreshTokenStore {
    pub fn new() -> Result<Self, AuthError> {
        Self::for_keyring_account(KEYRING_ACCOUNT)
    }

    pub fn for_profile(profile_uuid: &str) -> Result<Self, AuthError> {
        let normalized = profile_uuid.replace('-', "");
        if normalized.len() != 32 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(AuthError::InvalidServiceResponse(
                "Microsoft profile identifier is invalid".to_owned(),
            ));
        }
        Self::for_keyring_account(&format!("{KEYRING_PROFILE_PREFIX}{normalized}"))
    }

    fn for_keyring_account(account: &str) -> Result<Self, AuthError> {
        Ok(Self {
            entry: keyring::Entry::new(KEYRING_SERVICE, account)?,
        })
    }

    pub fn save(&self, refresh_token: &str) -> Result<(), AuthError> {
        if refresh_token.is_empty() {
            return Err(AuthError::EmptyRefreshToken);
        }
        self.entry.set_password(refresh_token)?;
        Ok(())
    }

    pub fn load(&self) -> Result<Option<String>, AuthError> {
        match self.entry.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn delete(&self) -> Result<bool, AuthError> {
        let deleted = match self.entry.delete_credential() {
            Ok(()) => true,
            Err(keyring::Error::NoEntry) => false,
            Err(error) => return Err(error.into()),
        };
        Ok(deleted)
    }
}

fn decode_success<T: for<'de> Deserialize<'de>>(
    response: Response,
    service: &str,
) -> Result<T, AuthError> {
    let status = response.status();
    let bytes = response.bytes()?;
    if !status.is_success() {
        return Err(service_rejected(service, status.as_u16(), &bytes));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

/// Decode only the documented, non-sensitive Minecraft Services error needed
/// to distinguish an application-registration rejection from account
/// ownership. All other response bodies stay private and become the generic
/// service rejection used elsewhere in this module.
fn decode_minecraft_login(response: Response) -> Result<MinecraftTokenResponse, AuthError> {
    let status = response.status();
    let bytes = response.bytes()?;
    if !status.is_success() {
        if let Some(error) = minecraft_login_error(status.as_u16(), &bytes) {
            return Err(error);
        }
        return Err(service_rejected("Minecraft login", status.as_u16(), &bytes));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

fn minecraft_login_error(status: u16, bytes: &[u8]) -> Option<AuthError> {
    if status != 403 {
        return None;
    }
    let error: MinecraftServicesErrorResponse = serde_json::from_slice(bytes).ok()?;
    error
        .error_message
        .as_deref()
        .map(str::trim)
        .filter(|message| message.eq_ignore_ascii_case(MINECRAFT_INVALID_APP_REGISTRATION_MESSAGE))
        .map(|_| AuthError::MinecraftAppRegistrationRejected)
}

fn service_rejected(service: &str, status: u16, bytes: &[u8]) -> AuthError {
    let code = serde_json::from_slice::<GenericServiceError>(bytes)
        .ok()
        .and_then(|error| error.code.or(error.error))
        .unwrap_or_else(|| "request_rejected".to_owned());
    AuthError::ServiceRejected {
        service: service.to_owned(),
        status,
        code,
    }
}

fn decode_xbox_token(response: Response, service: &str) -> Result<XboxToken, AuthError> {
    let payload: XboxTokenResponse = decode_success(response, service)?;
    let user_hash = payload
        .display_claims
        .xui
        .first()
        .map(|claim| claim.user_hash.clone())
        .filter(|hash| !hash.is_empty())
        .ok_or_else(|| {
            AuthError::InvalidServiceResponse(format!("{service} omitted Xbox user hash"))
        })?;
    if payload.token.is_empty() {
        return Err(AuthError::InvalidServiceResponse(format!(
            "{service} returned an empty token"
        )));
    }
    Ok(XboxToken {
        token: payload.token,
        user_hash,
    })
}

fn validate_client_id(client_id: &str) -> Result<(), AuthError> {
    if client_id.is_empty()
        || client_id.len() > 128
        || !client_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(AuthError::InvalidClientId);
    }
    Ok(())
}

fn validate_verification_uri(raw: &str) -> Result<(), AuthError> {
    let url = Url::parse(raw).map_err(|_| AuthError::InvalidVerificationUri)?;
    let host = url.host_str().ok_or(AuthError::InvalidVerificationUri)?;
    let is_microsoft_host = host == "microsoft.com" || host.ends_with(".microsoft.com");
    if url.scheme() != "https"
        || !is_microsoft_host
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(AuthError::InvalidVerificationUri);
    }
    Ok(())
}

fn bind_loopback_listeners() -> Result<(Vec<TcpListener>, u16), AuthError> {
    let ipv4 = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(AuthError::LoopbackBind)?;
    let port = ipv4.local_addr().map_err(AuthError::LoopbackBind)?.port();
    ipv4.set_nonblocking(true)
        .map_err(AuthError::LoopbackBind)?;

    let mut listeners = vec![ipv4];
    // `localhost` may resolve to IPv6 first. A second, IPv6-only loopback
    // listener is best-effort because some systems do not provide IPv6 or
    // treat the IPv4 listener as already covering that port.
    if let Ok(ipv6) = TcpListener::bind((Ipv6Addr::LOCALHOST, port)) {
        ipv6.set_nonblocking(true)
            .map_err(AuthError::LoopbackBind)?;
        listeners.push(ipv6);
    }

    Ok((listeners, port))
}

fn random_urlsafe_value(byte_count: usize) -> Result<String, AuthError> {
    let mut bytes = vec![0_u8; byte_count];
    getrandom::fill(&mut bytes).map_err(|_| AuthError::SecureRandom)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn pkce_challenge(code_verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()))
}

fn read_callback_target(
    stream: &mut TcpStream,
    timeout: Duration,
    expected_host: &str,
) -> Option<String> {
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let mut request = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 1024];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => return None,
            Ok(count) => {
                if request.len() + count > MAX_CALLBACK_REQUEST_BYTES {
                    return None;
                }
                request.extend_from_slice(&buffer[..count]);
                let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
                else {
                    continue;
                };
                let header = std::str::from_utf8(&request[..header_end]).ok()?;
                let request_line = header.split("\r\n").next()?;
                let mut fields = request_line.split_ascii_whitespace();
                if fields.next()? != "GET" {
                    return None;
                }
                let target = fields.next()?;
                let version = fields.next()?;
                if !(version == "HTTP/1.1" || version == "HTTP/1.0")
                    || fields.next().is_some()
                    || !target.starts_with('/')
                    || target.starts_with("//")
                {
                    return None;
                }
                let mut host = None;
                for header_line in header.split("\r\n").skip(1) {
                    let (name, value) = header_line.split_once(':')?;
                    if name.eq_ignore_ascii_case("host") && host.replace(value.trim()).is_some() {
                        return None;
                    }
                }
                if !host.is_some_and(|host| host.eq_ignore_ascii_case(expected_host)) {
                    return None;
                }
                return Some(target.to_owned());
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
}

fn parse_browser_callback(request_target: &str, expected_state: &str) -> CallbackOutcome {
    let callback_url = match Url::parse(&format!("http://{LOOPBACK_CALLBACK_HOST}{request_target}"))
    {
        Ok(url) if url.path() == LOOPBACK_CALLBACK_PATH => url,
        _ => return CallbackOutcome::Ignore,
    };

    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (name, value) in callback_url.query_pairs() {
        let value = value.into_owned();
        let destination = match name.as_ref() {
            "code" => &mut code,
            "state" => &mut state,
            "error" => &mut error,
            _ => continue,
        };
        if destination.replace(value).is_some() {
            return CallbackOutcome::Ignore;
        }
    }

    if state.as_deref() != Some(expected_state) {
        return CallbackOutcome::Ignore;
    }
    if error.is_some() {
        return if code.is_none() {
            CallbackOutcome::Declined
        } else {
            CallbackOutcome::Ignore
        };
    }
    match code.filter(|code| !code.is_empty() && code.len() <= 4096) {
        Some(code) => CallbackOutcome::Code(code),
        None => CallbackOutcome::Ignore,
    }
}

fn write_callback_page(stream: &mut TcpStream, page: CallbackPage) {
    let (status, body) = match page {
        CallbackPage::Complete => (
            "200 OK",
            "<!doctype html><meta charset=\"utf-8\"><title>Opus Launcher</title><p>Microsoft sign-in is complete. You can return to Opus Launcher.</p>",
        ),
        CallbackPage::Declined => (
            "200 OK",
            "<!doctype html><meta charset=\"utf-8\"><title>Opus Launcher</title><p>Microsoft sign-in was cancelled or declined. You can return to Opus Launcher.</p>",
        ),
        CallbackPage::Invalid => (
            "400 Bad Request",
            "<!doctype html><meta charset=\"utf-8\"><title>Opus Launcher</title><p>This sign-in response is not valid. Return to Opus Launcher and try again.</p>",
        ),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

struct XboxToken {
    token: String,
    user_hash: String,
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    message: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct OAuthErrorResponse {
    error: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XboxTokenResponse {
    token: String,
    display_claims: XboxDisplayClaims,
}

#[derive(Deserialize)]
struct XboxDisplayClaims {
    xui: Vec<XboxUserClaim>,
}

#[derive(Deserialize)]
struct XboxUserClaim {
    #[serde(rename = "uhs")]
    user_hash: String,
}

#[derive(Deserialize)]
struct MinecraftTokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct MinecraftServicesErrorResponse {
    #[serde(rename = "errorMessage")]
    error_message: Option<String>,
}

#[derive(Deserialize)]
struct EntitlementsResponse {
    items: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct MinecraftProfile {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct GenericServiceError {
    code: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("{0} is required; register an Opus public desktop application before login")]
    MissingClientId(String),
    #[error("Microsoft OAuth client id is invalid")]
    InvalidClientId,
    #[error("Microsoft returned an unsafe device verification URL")]
    InvalidVerificationUri,
    #[error("Opus Launcher could not create a local Microsoft sign-in callback")]
    LoopbackBind(#[source] std::io::Error),
    #[error("Opus Launcher could not receive the Microsoft sign-in callback")]
    LoopbackCallback(#[source] std::io::Error),
    #[error("secure random generation failed")]
    SecureRandom,
    #[error("authentication HTTP request failed")]
    Http(#[from] reqwest::Error),
    #[error("authentication service returned invalid JSON")]
    Json(#[from] serde_json::Error),
    #[error("secure credential storage failed")]
    Keyring(#[from] keyring::Error),
    #[error("authentication service response is invalid: {0}")]
    InvalidServiceResponse(String),
    #[error("{service} rejected the request (HTTP {status}, code {code})")]
    ServiceRejected {
        service: String,
        status: u16,
        code: String,
    },
    #[error("Microsoft device code expired")]
    DeviceCodeExpired,
    #[error("Microsoft browser sign-in timed out")]
    BrowserAuthorizationTimedOut,
    #[error("Microsoft browser sign-in was cancelled")]
    BrowserAuthorizationCancelled,
    #[error("Microsoft authorization was declined")]
    AuthorizationDeclined,
    #[error("Minecraft Java ownership is required")]
    MinecraftOwnershipRequired,
    #[error(
        "Minecraft Services rejected Opus's application registration before Java Edition ownership could be checked. Update Opus Launcher or contact the Opus administrator."
    )]
    MinecraftAppRegistrationRejected,
    #[error("no Microsoft account is stored")]
    NoStoredAccount,
    #[error("refusing to store an empty refresh token")]
    EmptyRefreshToken,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_id_validation_rejects_whitespace_and_empty_values() {
        assert!(validate_client_id("").is_err());
        assert!(validate_client_id("client id").is_err());
        assert!(validate_client_id("12345678-abcd-1234-abcd-1234567890ab").is_ok());
    }

    #[test]
    fn verification_uri_accepts_only_microsoft_https_hosts() {
        assert!(validate_verification_uri("https://microsoft.com/devicelogin").is_ok());
        assert!(validate_verification_uri("https://www.microsoft.com/link").is_ok());
        assert!(validate_verification_uri("http://microsoft.com/link").is_err());
        assert!(validate_verification_uri("https://microsoft.com.evil.test/link").is_err());
        assert!(validate_verification_uri("https://user@www.microsoft.com/link").is_err());
    }

    #[test]
    fn pkce_challenge_matches_the_rfc_s256_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn browser_authorization_uses_s256_and_the_registered_localhost_root() {
        let authenticator =
            MicrosoftAuthenticator::new("352b876e-6d3b-4cb8-9095-82957a752784".to_owned()).unwrap();
        let authorization = authenticator.begin_browser_authorization().unwrap();
        let url = Url::parse(authorization.authorization_url()).unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("login.microsoftonline.com"));
        assert_eq!(url.path(), "/consumers/oauth2/v2.0/authorize");

        let parameter = |name| {
            url.query_pairs()
                .find_map(|(key, value)| (key == name).then(|| value.into_owned()))
                .unwrap()
        };
        assert_eq!(parameter("client_id"), authenticator.client_id);
        assert_eq!(parameter("response_type"), "code");
        assert_eq!(parameter("response_mode"), "query");
        assert_eq!(parameter("scope"), OAUTH_SCOPE);
        assert_eq!(parameter("prompt"), "select_account");
        assert_eq!(parameter("code_challenge_method"), "S256");
        assert_eq!(
            parameter("code_challenge"),
            pkce_challenge(&authorization.code_verifier)
        );
        assert!(!url.query_pairs().any(|(key, _)| key == "code_verifier"));

        let redirect = Url::parse(&parameter("redirect_uri")).unwrap();
        assert_eq!(redirect.scheme(), "http");
        assert_eq!(redirect.host_str(), Some(LOOPBACK_CALLBACK_HOST));
        assert_eq!(redirect.path(), LOOPBACK_CALLBACK_PATH);
        assert_eq!(
            redirect.port(),
            Some(authorization.listeners[0].local_addr().unwrap().port())
        );
    }

    #[test]
    fn browser_callback_requires_one_matching_state_at_the_root_path() {
        assert!(matches!(
            parse_browser_callback("/?code=authorization-code&state=expected", "expected"),
            CallbackOutcome::Code(code) if code == "authorization-code"
        ));
        assert!(matches!(
            parse_browser_callback("/?code=authorization-code&state=wrong", "expected"),
            CallbackOutcome::Ignore
        ));
        assert!(matches!(
            parse_browser_callback(
                "/?code=authorization-code&state=expected&state=expected",
                "expected"
            ),
            CallbackOutcome::Ignore
        ));
        assert!(matches!(
            parse_browser_callback(
                "/oauth/callback?code=authorization-code&state=expected",
                "expected"
            ),
            CallbackOutcome::Ignore
        ));
        assert!(matches!(
            parse_browser_callback("/?error=access_denied&state=expected", "expected"),
            CallbackOutcome::Declined
        ));
    }

    #[test]
    fn loopback_callback_returns_no_authorization_code_to_the_browser() {
        let authenticator =
            MicrosoftAuthenticator::new("352b876e-6d3b-4cb8-9095-82957a752784".to_owned()).unwrap();
        let mut authorization = authenticator.begin_browser_authorization().unwrap();
        let port = authorization.listeners[0].local_addr().unwrap().port();
        let state = authorization.state.clone();
        let host = authorization.callback_host.clone();
        let browser = thread::spawn(move || {
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
            let request = format!(
                "GET /?code=authorization-code&state={state} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(request.as_bytes()).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        });

        let code = authorization
            .wait_for_callback_until(
                Instant::now() + Duration::from_secs(2),
                &BrowserLoginCancellation::new(),
            )
            .unwrap();
        assert_eq!(code, "authorization-code");
        let response = browser.join().unwrap();
        assert!(response.contains("Microsoft sign-in is complete"));
        assert!(!response.contains("authorization-code"));
    }

    #[test]
    fn browser_callback_wait_is_bounded() {
        let authenticator =
            MicrosoftAuthenticator::new("352b876e-6d3b-4cb8-9095-82957a752784".to_owned()).unwrap();
        let mut authorization = authenticator.begin_browser_authorization().unwrap();
        assert!(matches!(
            authorization
                .wait_for_callback_until(Instant::now(), &BrowserLoginCancellation::new(),),
            Err(AuthError::BrowserAuthorizationTimedOut)
        ));
    }

    #[test]
    fn browser_callback_wait_stops_when_cancelled() {
        let authenticator =
            MicrosoftAuthenticator::new("352b876e-6d3b-4cb8-9095-82957a752784".to_owned()).unwrap();
        let mut authorization = authenticator.begin_browser_authorization().unwrap();
        let cancellation = BrowserLoginCancellation::new();
        cancellation.cancel();
        assert!(matches!(
            authorization
                .wait_for_callback_until(Instant::now() + Duration::from_secs(2), &cancellation,),
            Err(AuthError::BrowserAuthorizationCancelled)
        ));
    }

    #[test]
    fn session_summary_never_contains_access_token() {
        let session = MinecraftSession {
            username: "player".to_owned(),
            uuid: "0123456789abcdef0123456789abcdef".to_owned(),
            access_token: "very-secret-token".to_owned(),
            user_type: "msa".to_owned(),
        };
        let summary = session.redacted_summary();
        assert!(summary.contains("player"));
        assert!(!summary.contains("very-secret-token"));
    }

    #[test]
    fn minecraft_invalid_app_registration_is_typed_without_echoing_response() {
        let body = br#"{
            "path": "/authentication/login_with_xbox",
            "errorMessage": "Invalid app registration, see https://aka.ms/AppRegInfo for more information",
            "requestId": "private-request-id"
        }"#;

        let error = minecraft_login_error(403, body)
            .expect("the documented invalid-registration response should be classified");
        assert!(matches!(
            &error,
            AuthError::MinecraftAppRegistrationRejected
        ));

        let message = error.to_string();
        assert!(message.contains("application registration"));
        assert!(!message.contains("AppRegInfo"));
        assert!(!message.contains("private-request-id"));
    }

    #[test]
    fn minecraft_login_classifies_only_the_known_invalid_registration_response() {
        let known_message = br#"{
            "errorMessage": "Invalid app registration, see https://aka.ms/AppRegInfo for more information"
        }"#;
        assert!(minecraft_login_error(400, known_message).is_none());
        assert!(minecraft_login_error(403, br#"{"errorMessage":"token=very-secret"}"#).is_none());

        let fallback = service_rejected(
            "Minecraft login",
            403,
            br#"{"errorMessage":"token=very-secret"}"#,
        );
        assert!(matches!(
            &fallback,
            AuthError::ServiceRejected {
                service,
                status,
                code,
            } if service == "Minecraft login" && *status == 403 && code == "request_rejected"
        ));
        assert!(!fallback.to_string().contains("very-secret"));
    }

    #[test]
    fn xbox_payload_parses_user_hash_without_exposing_extra_claims() {
        let payload: XboxTokenResponse = serde_json::from_str(
            r#"{"Token":"xbox-token","DisplayClaims":{"xui":[{"uhs":"12345"}]}}"#,
        )
        .unwrap();
        assert_eq!(payload.token, "xbox-token");
        assert_eq!(payload.display_claims.xui[0].user_hash, "12345");
    }
}
