//! Private child-process protocol. Credentials travel through stdin/stdout pipes only;
//! they never enter the Client's HTTP bridge, browser, arguments or diagnostic logs.
use super::{accounts, microsoft_authenticator};
use opus_auth::RefreshTokenStore;
use opus_engine::GameIdentity;
use opus_platform::OpusPaths;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    root: PathBuf,
    id: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Identity {
    username: String,
    uuid: String,
    access_token: String,
    user_type: String,
}

fn resolve(request: Request) -> Result<Identity, ()> {
    if !request.root.is_absolute() || request.id.is_empty() || request.id.len() > 256 {
        return Err(());
    }
    let paths = OpusPaths::from_root(request.root).map_err(|_| ())?;
    let catalog = accounts::load(&paths).map_err(|_| ())?;
    let identity = match accounts::resolve(&catalog, &request.id).map_err(|_| ())? {
        accounts::ResolvedAccount::Offline(record) => {
            GameIdentity::offline(&record.username).map_err(|_| ())?
        }
        accounts::ResolvedAccount::Microsoft(record) => {
            let store = RefreshTokenStore::for_profile(&record.uuid).map_err(|_| ())?;
            if store.load().map_err(|_| ())?.is_none() {
                return Err(());
            }
            let account = microsoft_authenticator()
                .map_err(|_| ())?
                .refresh_session(&store)
                .map_err(|_| ())?;
            account.save_refresh_token_for_profile().map_err(|_| ())?;
            // Catalog selection is committed by Client only after its session changed.
            GameIdentity::authenticated(
                &account.session.username,
                &account.session.uuid,
                account.session.access_token.clone(),
                &account.session.user_type,
            )
            .map_err(|_| ())?
        }
    };
    Ok(Identity {
        username: identity.username,
        uuid: identity.uuid,
        access_token: identity.access_token,
        user_type: identity.user_type,
    })
}

pub fn mode() -> Option<i32> {
    if std::env::args_os().nth(1).as_deref()
        != Some(std::ffi::OsStr::new("--resolve-client-account"))
    {
        return None;
    }
    let mut input = String::new();
    let result = (|| {
        std::io::stdin()
            .take(8193)
            .read_to_string(&mut input)
            .map_err(|_| ())?;
        if input.len() > 8192 {
            return Err(());
        }
        let identity = resolve(serde_json::from_str(&input).map_err(|_| ())?)?;
        serde_json::to_writer(std::io::stdout().lock(), &identity).map_err(|_| ())?;
        std::io::stdout().flush().map_err(|_| ())?;
        Ok(())
    })();
    // Deliberately no auth-provider error, username, response or token in stderr.
    Some(if result.is_ok() { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_switch_resolves_live_identity_without_changing_catalog_selection() {
        let dir = tempfile::tempdir().unwrap();
        let paths = OpusPaths::from_root(dir.path().to_path_buf()).unwrap();
        let first = accounts::upsert_offline(&paths, "First").unwrap();
        let second = accounts::upsert_offline(&paths, "Second").unwrap();
        accounts::select(&paths, &first.id).unwrap();
        let identity = resolve(Request {
            root: dir.path().to_path_buf(),
            id: second.id,
        })
        .unwrap();
        assert_eq!(identity.username, "Second");
        assert_eq!(identity.access_token, "0");
        assert_eq!(identity.user_type, "legacy");
        assert_eq!(
            accounts::selected_id(&accounts::load(&paths).unwrap()),
            Some(first.id)
        );
    }
    #[test]
    fn unknown_account_and_relative_root_are_rejected() {
        assert!(
            resolve(Request {
                root: PathBuf::from("relative"),
                id: "offline:missing".into()
            })
            .is_err()
        );
        let dir = tempfile::tempdir().unwrap();
        assert!(
            resolve(Request {
                root: dir.path().into(),
                id: "missing".into()
            })
            .is_err()
        );
    }
}
