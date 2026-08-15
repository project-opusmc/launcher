use opus_auth::RefreshTokenStore;
use opus_engine::GameIdentity;
use opus_platform::OpusPaths;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

const ACCOUNTS_FILE: &str = "accounts-v1.json";
const ACCOUNT_SCHEMA_VERSION: u8 = 1;
const OFFICIAL_BADGE: &str = "official";
const PREMIUM_BADGE: &str = "premium";
const UNOFFICIAL_BADGE: &str = "unofficial";
static ACCOUNT_FILE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AccountKind {
    Microsoft,
    Offline,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountRecord {
    pub id: String,
    pub username: String,
    pub uuid: String,
    pub kind: AccountKind,
    pub badge: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AccountFile {
    schema_version: u8,
    selected_account_id: Option<String>,
    accounts: Vec<AccountRecord>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummary {
    pub id: String,
    pub username: String,
    pub uuid: Option<String>,
    pub kind: AccountKind,
    pub badge: String,
    pub ready: bool,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub enum ResolvedAccount {
    Microsoft(AccountRecord),
    Offline(AccountRecord),
}

impl Default for AccountFile {
    fn default() -> Self {
        Self {
            schema_version: ACCOUNT_SCHEMA_VERSION,
            selected_account_id: None,
            accounts: Vec::new(),
        }
    }
}

pub fn accounts_path(paths: &OpusPaths) -> PathBuf {
    paths.root.join(ACCOUNTS_FILE)
}

pub fn load(paths: &OpusPaths) -> Result<AccountFileView, String> {
    let path = accounts_path(paths);
    let mut file = if path.exists() {
        let bytes = fs::read(&path).map_err(display_error)?;
        serde_json::from_slice::<AccountFile>(&bytes)
            .map_err(|_| "Opus account list is invalid".to_owned())?
    } else {
        AccountFile::default()
    };
    normalize_file(&mut file)?;
    Ok(AccountFileView { file })
}

#[derive(Debug, Clone)]
pub struct AccountFileView {
    file: AccountFile,
}

impl AccountFileView {
    pub fn summaries(&self) -> Vec<AccountSummary> {
        let selected = self.file.selected_account_id.as_deref();
        self.file
            .accounts
            .iter()
            .map(|account| AccountSummary {
                id: account.id.clone(),
                username: account.username.clone(),
                uuid: Some(account.uuid.clone()),
                kind: account.kind.clone(),
                badge: account.badge.clone(),
                ready: account_ready(account),
                selected: selected == Some(account.id.as_str()),
            })
            .collect()
    }

    pub fn has_id(&self, id: &str) -> bool {
        self.file.accounts.iter().any(|account| account.id == id)
    }
}

pub fn selected_id(view: &AccountFileView) -> Option<String> {
    view.file
        .selected_account_id
        .clone()
        .filter(|id| view.has_id(id))
        .or_else(|| {
            view.summaries()
                .into_iter()
                .next()
                .map(|account| account.id)
        })
}

pub fn select(paths: &OpusPaths, id: &str) -> Result<AccountSummary, String> {
    let _catalog_lock = lock_catalog();
    let mut view = load(paths)?;
    if !view.has_id(id) {
        return Err("The selected account no longer exists".to_owned());
    }
    view.file.selected_account_id = Some(id.to_owned());
    save(paths, &view.file)?;
    view.file.selected_account_id = Some(id.to_owned());
    view.summaries()
        .into_iter()
        .find(|account| account.id == id)
        .ok_or_else(|| "The selected account could not be loaded".to_owned())
}

#[cfg(not(feature = "qa-edition"))]
pub fn upsert_microsoft(
    paths: &OpusPaths,
    username: &str,
    uuid: &str,
) -> Result<AccountSummary, String> {
    let _catalog_lock = lock_catalog();
    let normalized_uuid = normalize_uuid(uuid)?;
    if username.trim().is_empty() || username.chars().count() > 32 {
        return Err("Microsoft profile name is invalid".to_owned());
    }
    let mut view = load(paths)?;
    let id = format!("microsoft:{normalized_uuid}");
    let badge = retained_microsoft_badge(&view.file, &id);
    let record = AccountRecord {
        id: id.clone(),
        username: username.to_owned(),
        uuid: normalized_uuid,
        kind: AccountKind::Microsoft,
        badge,
    };
    if let Some(existing) = view
        .file
        .accounts
        .iter_mut()
        .find(|account| account.id == id)
    {
        *existing = record;
    } else {
        view.file.accounts.push(record);
    }
    view.file.selected_account_id = Some(id.clone());
    save(paths, &view.file)?;
    view.summaries()
        .into_iter()
        .find(|account| account.id == id)
        .ok_or_else(|| "Microsoft account was not added".to_owned())
}

pub fn upsert_offline(paths: &OpusPaths, username: &str) -> Result<AccountSummary, String> {
    let _catalog_lock = lock_catalog();
    let identity = GameIdentity::offline(username).map_err(display_error)?;
    let uuid = identity.uuid.clone();
    let id = format!("offline:{uuid}");
    let mut view = load(paths)?;
    let record = AccountRecord {
        id: id.clone(),
        username: username.to_owned(),
        uuid,
        kind: AccountKind::Offline,
        badge: UNOFFICIAL_BADGE.to_owned(),
    };
    if let Some(existing) = view
        .file
        .accounts
        .iter_mut()
        .find(|account| account.id == id)
    {
        *existing = record;
    } else {
        view.file.accounts.push(record);
    }
    view.file.selected_account_id = Some(id.clone());
    save(paths, &view.file)?;
    view.summaries()
        .into_iter()
        .find(|account| account.id == id)
        .ok_or_else(|| "Offline profile was not added".to_owned())
}

pub fn remove(paths: &OpusPaths, id: &str) -> Result<bool, String> {
    let _catalog_lock = lock_catalog();
    let mut view = load(paths)?;
    let Some(index) = view
        .file
        .accounts
        .iter()
        .position(|account| account.id == id)
    else {
        return Ok(false);
    };
    let account = view.file.accounts.remove(index);
    if matches!(account.kind, AccountKind::Microsoft) {
        RefreshTokenStore::for_profile(&account.uuid)
            .map_err(display_error)?
            .delete()
            .map_err(display_error)?;
    }
    if view.file.selected_account_id.as_deref() == Some(id) {
        view.file.selected_account_id = view.file.accounts.first().map(|next| next.id.clone());
    }
    save(paths, &view.file)?;
    Ok(true)
}

pub fn resolve(view: &AccountFileView, id: &str) -> Result<ResolvedAccount, String> {
    let account = view
        .file
        .accounts
        .iter()
        .find(|account| account.id == id)
        .cloned()
        .ok_or_else(|| "The selected account no longer exists".to_owned())?;
    match account.kind {
        AccountKind::Microsoft => Ok(ResolvedAccount::Microsoft(account)),
        AccountKind::Offline => Ok(ResolvedAccount::Offline(account)),
    }
}

fn save(paths: &OpusPaths, file: &AccountFile) -> Result<(), String> {
    let path = accounts_path(paths);
    let parent = path
        .parent()
        .ok_or_else(|| "Opus account path has no parent directory".to_owned())?;
    fs::create_dir_all(parent).map_err(display_error)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Opus account path has no file name".to_owned())?;
    let part = path.with_file_name(format!(".{name}-{}.part", std::process::id()));
    if part.exists() {
        fs::remove_file(&part).map_err(display_error)?;
    }
    let result = (|| -> Result<(), String> {
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&part)
            .map_err(display_error)?;
        serde_json::to_writer_pretty(
            &mut output,
            &AccountFile {
                schema_version: ACCOUNT_SCHEMA_VERSION,
                selected_account_id: file.selected_account_id.clone(),
                accounts: file.accounts.clone(),
            },
        )
        .map_err(display_error)?;
        output.write_all(b"\n").map_err(display_error)?;
        output.sync_all().map_err(display_error)?;
        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(&path).map_err(display_error)?;
        }
        fs::rename(&part, &path).map_err(display_error)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

fn normalize_file(file: &mut AccountFile) -> Result<(), String> {
    if file.schema_version != ACCOUNT_SCHEMA_VERSION {
        return Err("Opus account list schema is unsupported".to_owned());
    }
    for account in &mut file.accounts {
        account.badge = match account.kind {
            AccountKind::Microsoft if account.badge.eq_ignore_ascii_case(OFFICIAL_BADGE) => {
                OFFICIAL_BADGE.to_owned()
            }
            AccountKind::Microsoft => PREMIUM_BADGE.to_owned(),
            AccountKind::Offline => UNOFFICIAL_BADGE.to_owned(),
        };
    }
    file.accounts
        .retain(|account| validate_record(account).is_ok());
    if file
        .selected_account_id
        .as_ref()
        .is_some_and(|id| !file.accounts.iter().any(|account| &account.id == id))
    {
        file.selected_account_id = None;
    }
    Ok(())
}

fn validate_record(account: &AccountRecord) -> Result<(), String> {
    if account.id.trim().is_empty() || account.username.trim().is_empty() {
        return Err("account metadata is empty".to_owned());
    }
    normalize_uuid(&account.uuid)?;
    match account.kind {
        AccountKind::Microsoft
            if account.id == format!("microsoft:{}", account.uuid)
                && matches!(account.badge.as_str(), OFFICIAL_BADGE | PREMIUM_BADGE) =>
        {
            Ok(())
        }
        AccountKind::Offline if account.id == format!("offline:{}", account.uuid) => {
            GameIdentity::offline(&account.username).map_err(display_error)?;
            if account.badge == UNOFFICIAL_BADGE {
                Ok(())
            } else {
                Err("offline account badge is invalid".to_owned())
            }
        }
        _ => Err("account metadata does not match its identity kind".to_owned()),
    }
}

fn account_ready(account: &AccountRecord) -> bool {
    match account.kind {
        AccountKind::Offline => GameIdentity::offline(&account.username).is_ok(),
        AccountKind::Microsoft => RefreshTokenStore::for_profile(&account.uuid)
            .and_then(|store| store.load())
            .ok()
            .flatten()
            .is_some(),
    }
}

#[cfg(any(not(feature = "qa-edition"), test))]
fn retained_microsoft_badge(file: &AccountFile, id: &str) -> String {
    file.accounts
        .iter()
        .find(|account| account.id == id && matches!(account.kind, AccountKind::Microsoft))
        .map(|account| account.badge.as_str())
        .filter(|badge| matches!(*badge, OFFICIAL_BADGE | PREMIUM_BADGE))
        .unwrap_or(OFFICIAL_BADGE)
        .to_owned()
}

fn normalize_uuid(uuid: &str) -> Result<String, String> {
    let normalized = uuid.replace('-', "").to_ascii_lowercase();
    if normalized.len() != 32 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Minecraft profile identifier is invalid".to_owned());
    }
    Ok(normalized)
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn lock_catalog() -> MutexGuard<'static, ()> {
    ACCOUNT_FILE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offline_record(username: &str) -> AccountRecord {
        let identity = GameIdentity::offline(username).unwrap();
        AccountRecord {
            id: format!("offline:{}", identity.uuid),
            username: username.to_owned(),
            uuid: identity.uuid,
            kind: AccountKind::Offline,
            badge: UNOFFICIAL_BADGE.to_owned(),
        }
    }

    #[test]
    fn offline_profiles_persist_in_one_selected_catalog() {
        let directory = tempfile::tempdir().unwrap();
        let paths = OpusPaths::from_root(directory.path().join("opus")).unwrap();
        let first = offline_record("AlphaOne");
        let second = offline_record("BetaTwo");
        save(
            &paths,
            &AccountFile {
                schema_version: ACCOUNT_SCHEMA_VERSION,
                selected_account_id: Some(second.id.clone()),
                accounts: vec![first.clone(), second.clone()],
            },
        )
        .unwrap();

        let view = load(&paths).unwrap();
        let summaries = view.summaries();
        assert_eq!(summaries.len(), 2);
        assert_eq!(selected_id(&view), Some(second.id));
        assert!(summaries.iter().all(|account| account.ready));
        assert!(
            summaries
                .iter()
                .all(|account| account.badge == UNOFFICIAL_BADGE)
        );
        assert_eq!(
            summaries.iter().filter(|account| account.selected).count(),
            1
        );
    }

    #[test]
    fn invalid_records_and_missing_selection_are_normalized() {
        let valid = offline_record("ValidName");
        let mut file = AccountFile {
            schema_version: ACCOUNT_SCHEMA_VERSION,
            selected_account_id: Some("offline:missing".to_owned()),
            accounts: vec![
                valid.clone(),
                AccountRecord {
                    id: "offline:not-a-uuid".to_owned(),
                    username: "broken".to_owned(),
                    uuid: "not-a-uuid".to_owned(),
                    kind: AccountKind::Offline,
                    badge: "invalid".to_owned(),
                },
            ],
        };

        normalize_file(&mut file).unwrap();
        assert_eq!(file.accounts, vec![valid]);
        assert_eq!(file.selected_account_id, None);
    }

    #[test]
    fn multiple_microsoft_profiles_keep_names_and_supported_badges() {
        let mut file = AccountFile {
            schema_version: ACCOUNT_SCHEMA_VERSION,
            selected_account_id: Some("microsoft:22222222222222222222222222222222".to_owned()),
            accounts: vec![
                AccountRecord {
                    id: "microsoft:11111111111111111111111111111111".to_owned(),
                    username: "FirstPlayer".to_owned(),
                    uuid: "11111111111111111111111111111111".to_owned(),
                    kind: AccountKind::Microsoft,
                    badge: OFFICIAL_BADGE.to_owned(),
                },
                AccountRecord {
                    id: "microsoft:22222222222222222222222222222222".to_owned(),
                    username: "SecondPlayer".to_owned(),
                    uuid: "22222222222222222222222222222222".to_owned(),
                    kind: AccountKind::Microsoft,
                    badge: PREMIUM_BADGE.to_owned(),
                },
            ],
        };

        normalize_file(&mut file).unwrap();

        assert_eq!(file.accounts.len(), 2);
        assert_eq!(file.accounts[0].username, "FirstPlayer");
        assert_eq!(file.accounts[0].badge, OFFICIAL_BADGE);
        assert_eq!(file.accounts[1].username, "SecondPlayer");
        assert_eq!(file.accounts[1].badge, PREMIUM_BADGE);
    }

    #[test]
    fn microsoft_profile_ids_are_canonicalized() {
        assert_eq!(
            normalize_uuid("12345678-1234-ABCD-9876-1234567890AB").unwrap(),
            "123456781234abcd98761234567890ab"
        );
        assert!(normalize_uuid("../../keychain").is_err());
    }

    #[test]
    fn microsoft_accounts_default_to_official_and_preserve_premium_status() {
        let id = "microsoft:11111111111111111111111111111111";
        assert_eq!(
            retained_microsoft_badge(&AccountFile::default(), id),
            OFFICIAL_BADGE
        );

        let file = AccountFile {
            schema_version: ACCOUNT_SCHEMA_VERSION,
            selected_account_id: Some(id.to_owned()),
            accounts: vec![AccountRecord {
                id: id.to_owned(),
                username: "PremiumPlayer".to_owned(),
                uuid: "11111111111111111111111111111111".to_owned(),
                kind: AccountKind::Microsoft,
                badge: PREMIUM_BADGE.to_owned(),
            }],
        };
        assert_eq!(retained_microsoft_badge(&file, id), PREMIUM_BADGE);
    }
}
