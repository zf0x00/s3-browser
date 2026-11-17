//! Saved connections.
//!
//! Profiles live in `~/Library/Application Support/S3 Browser/connections.json`
//! (or the platform equivalent). Secret keys never go in that file: they are
//! kept in the macOS keychain. Credentials can also be imported from the same
//! places s3fs reads them: `~/.aws/credentials` and `~/.passwd-s3fs`.

use std::path::PathBuf;

use anyhow::{Context as _, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::providers::ProviderKind;

const KEYCHAIN_SERVICE: &str = "S3 Browser";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub provider: ProviderKind,
    pub endpoint: String,
    pub region: String,
    pub access_key: String,
    pub path_style: bool,
    /// Optional bucket for keys that can't call ListBuckets (R2/B2 scoped tokens).
    #[serde(default)]
    pub bucket: Option<String>,
}

/// The secret half of a connection, held only in memory and the keychain.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Secret {
    pub secret_key: String,
    #[serde(default)]
    pub session_token: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    connections: Vec<Connection>,
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("S3 Browser")
}

fn connections_path() -> PathBuf {
    config_dir().join("connections.json")
}

pub fn load() -> Vec<Connection> {
    std::fs::read(connections_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<File>(&bytes).ok())
        .map(|f| f.connections)
        .unwrap_or_default()
}

pub fn save(connections: &[Connection]) -> Result<()> {
    std::fs::create_dir_all(config_dir())?;
    let file = File { connections: connections.to_vec() };
    let path = connections_path();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&file)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ---- keychain ---------------------------------------------------------------

// All secret keys live in ONE keychain item ("S3 Browser" / "connections"),
// read once per launch and kept in memory. So macOS asks at most once, and
// with the stable code signature (scripts/sign.sh) "Always Allow" sticks
// across rebuilds. Older builds kept one item per connection; those are
// migrated into the vault the first time they're read.

#[cfg(target_os = "macos")]
const VAULT_ACCOUNT: &str = "connections";
#[cfg(target_os = "macos")]
const ERR_NOT_FOUND: i32 = -25300; // errSecItemNotFound

#[cfg(target_os = "macos")]
static VAULT: std::sync::Mutex<Option<std::collections::HashMap<String, Secret>>> = std::sync::Mutex::new(None);

#[cfg(target_os = "macos")]
fn with_vault<R>(f: impl FnOnce(&mut std::collections::HashMap<String, Secret>) -> Result<R>) -> Result<R> {
    use security_framework::passwords::get_generic_password;
    let mut guard = VAULT.lock().unwrap();
    if guard.is_none() {
        let map = match get_generic_password(KEYCHAIN_SERVICE, VAULT_ACCOUNT) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("keychain entry is corrupt")?,
            Err(e) if e.code() == ERR_NOT_FOUND => Default::default(),
            // Denied or locked: don't pretend the vault is empty (we'd overwrite it).
            Err(e) => return Err(anyhow!("Couldn't read the keychain ({e})")),
        };
        *guard = Some(map);
    }
    f(guard.as_mut().unwrap())
}

#[cfg(target_os = "macos")]
fn write_vault(map: &std::collections::HashMap<String, Secret>) -> Result<()> {
    security_framework::passwords::set_generic_password(KEYCHAIN_SERVICE, VAULT_ACCOUNT, &serde_json::to_vec(map)?)
        .map_err(|e| anyhow!("keychain: {e}"))
}

/// `s3-browser --check-keychain`: read the vault and report (diagnostics).
#[cfg(target_os = "macos")]
pub fn check_keychain() -> Result<usize> {
    with_vault(|map| Ok(map.len()))
}

#[cfg(target_os = "macos")]
pub fn save_secret(id: &str, secret: &Secret) -> Result<()> {
    with_vault(|map| {
        map.insert(id.to_string(), secret.clone());
        write_vault(map)
    })
}

#[cfg(target_os = "macos")]
pub fn load_secret(id: &str) -> Result<Secret> {
    with_vault(|map| {
        if let Some(secret) = map.get(id) {
            return Ok(secret.clone());
        }
        // A per-connection item from an older build: move it into the vault.
        let bytes = security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, id)
            .map_err(|e| anyhow!("No saved secret key in the keychain ({e})"))?;
        let secret: Secret = serde_json::from_slice(&bytes).context("keychain entry is corrupt")?;
        map.insert(id.to_string(), secret.clone());
        write_vault(map)?;
        Ok(secret)
    })
}

#[cfg(target_os = "macos")]
pub fn delete_secret(id: &str) {
    let _ = with_vault(|map| {
        if map.remove(id).is_some() {
            write_vault(map)?;
        }
        Ok(())
    });
    let _ = security_framework::passwords::delete_generic_password(KEYCHAIN_SERVICE, id);
}

// Other platforms: a private file next to the profiles (0600).
#[cfg(not(target_os = "macos"))]
fn secret_path(id: &str) -> PathBuf {
    config_dir().join("secrets").join(format!("{id}.json"))
}

#[cfg(not(target_os = "macos"))]
pub fn save_secret(id: &str, secret: &Secret) -> Result<()> {
    let path = secret_path(id);
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_vec(secret)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn load_secret(id: &str) -> Result<Secret> {
    Ok(serde_json::from_slice(&std::fs::read(secret_path(id))?)?)
}

#[cfg(not(target_os = "macos"))]
pub fn delete_secret(id: &str) {
    let _ = std::fs::remove_file(secret_path(id));
}

// ---- imports (same sources as s3fs) ----------------------------------------

/// A credential found on disk, ready to prefill the connection form.
#[derive(Clone, Debug)]
pub struct ImportedCredential {
    pub label: String,
    pub access_key: String,
    pub secret_key: String,
    pub session_token: Option<String>,
    pub region: Option<String>,
    pub endpoint: Option<String>,
    /// s3fs's `bucket:ACCESS:SECRET` form names the bucket.
    pub bucket: Option<String>,
}

/// Profiles from `~/.aws/credentials`, merged with regions/endpoints from `~/.aws/config`.
pub fn import_aws_profiles() -> Vec<ImportedCredential> {
    let Some(home) = dirs::home_dir() else { return Vec::new() };
    let creds = parse_ini(&std::fs::read_to_string(home.join(".aws/credentials")).unwrap_or_default());
    let config = parse_ini(&std::fs::read_to_string(home.join(".aws/config")).unwrap_or_default());
    creds
        .into_iter()
        .filter_map(|(profile, kv)| {
            let get = |k: &str| kv.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
            let access_key = get("aws_access_key_id")?;
            let secret_key = get("aws_secret_access_key")?;
            let section = if profile == "default" { "default".to_string() } else { format!("profile {profile}") };
            let cfg = config.iter().find(|(name, _)| *name == section).map(|(_, kv)| kv.clone()).unwrap_or_default();
            let cfg_get = |k: &str| cfg.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
            Some(ImportedCredential {
                label: format!("AWS profile “{profile}”"),
                access_key,
                secret_key,
                session_token: get("aws_session_token"),
                region: get("region").or_else(|| cfg_get("region")),
                endpoint: get("endpoint_url").or_else(|| cfg_get("endpoint_url")),
                bucket: None,
            })
        })
        .collect()
}

/// Lines from `~/.passwd-s3fs`: `ACCESS:SECRET` or `bucket:ACCESS:SECRET`.
pub fn import_s3fs_passwd() -> Vec<ImportedCredential> {
    let Some(home) = dirs::home_dir() else { return Vec::new() };
    let text = std::fs::read_to_string(home.join(".passwd-s3fs")).unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(':').collect();
            match parts.as_slice() {
                [ak, sk] => Some(ImportedCredential {
                    label: "~/.passwd-s3fs (default)".into(),
                    access_key: ak.to_string(),
                    secret_key: sk.to_string(),
                    session_token: None,
                    region: None,
                    endpoint: None,
                    bucket: None,
                }),
                [bucket, ak, sk] => Some(ImportedCredential {
                    label: format!("~/.passwd-s3fs bucket “{bucket}”"),
                    access_key: ak.to_string(),
                    secret_key: sk.to_string(),
                    session_token: None,
                    region: None,
                    endpoint: None,
                    bucket: Some(bucket.to_string()),
                }),
                _ => None,
            }
        })
        .collect()
}

pub fn import_all() -> Vec<ImportedCredential> {
    let mut all = import_aws_profiles();
    all.extend(import_s3fs_passwd());
    all
}

type Section = (String, Vec<(String, String)>);

fn parse_ini(text: &str) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            out.push((line[1..line.len() - 1].trim().to_string(), Vec::new()));
        } else if let Some((k, v)) = line.split_once('=') {
            if let Some((_, kv)) = out.last_mut() {
                kv.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_sections() {
        let ini = parse_ini("[default]\naws_access_key_id = A\n# c\n[profile x]\nregion=eu-west-1\n");
        assert_eq!(ini.len(), 2);
        assert_eq!(ini[0].1[0], ("aws_access_key_id".into(), "A".into()));
        assert_eq!(ini[1].0, "profile x");
    }
}
