use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::{collections::BTreeMap, env, fs, path::PathBuf};

const DEFAULT_PROFILE_PATH: &str = ".config/soroban-forge/networks.toml";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileFile {
    #[serde(default)]
    pub profiles: BTreeMap<String, NetworkProfile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkProfile {
    pub rpc_url: Option<String>,
    pub network_passphrase: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNetwork {
    pub name: String,
    pub rpc_url: Option<String>,
    pub network_passphrase: Option<String>,
}

pub fn default_profile_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("SOROBAN_FORGE_NETWORK_PROFILES") {
        return Ok(PathBuf::from(path));
    }
    let home = env::var_os("HOME").map(PathBuf::from).context(
        "cannot determine the network profile path: set HOME or SOROBAN_FORGE_NETWORK_PROFILES",
    )?;
    Ok(home.join(DEFAULT_PROFILE_PATH))
}

pub fn resolve_network(
    explicit_name: Option<&str>,
    profile_name: Option<&str>,
    explicit_rpc_url: Option<&str>,
    explicit_passphrase: Option<&str>,
) -> Result<ResolvedNetwork> {
    if explicit_name.is_none()
        && profile_name.is_none()
        && explicit_rpc_url.is_none()
        && explicit_passphrase.is_none()
    {
        return Ok(ResolvedNetwork {
            name: "testnet".to_owned(),
            rpc_url: None,
            network_passphrase: None,
        });
    }

    let mut profile: Option<NetworkProfile> = None;
    if let Some(selected) = profile_name {
        if selected.trim().is_empty() {
            bail!("network profile name cannot be empty; pass --network-profile <name>");
        }
        let path = default_profile_path()?;
        let contents = fs::read_to_string(&path).with_context(|| {
            format!(
                "could not read network profile file '{}'; set SOROBAN_FORGE_NETWORK_PROFILES or create the file",
                path.display()
            )
        })?;
        let file: ProfileFile = toml::from_str(&contents)
            .with_context(|| format!("malformed network profile file '{}'", path.display()))?;
        let profiles = file.profiles;
        profile = Some(
            profiles
                .get(selected)
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown network profile '{selected}' in '{}'; define it under [profiles.{selected}]",
                        path.display()
                    )
                })?,
        );
    }

    let name = explicit_name
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| profile_name.map(str::to_owned))
        .unwrap_or_else(|| "testnet".to_owned());

    let rpc_url = explicit_rpc_url
        .map(str::to_owned)
        .or_else(|| profile.as_ref().and_then(|value| value.rpc_url.clone()));
    let passphrase = explicit_passphrase.map(str::to_owned).or_else(|| {
        profile
            .as_ref()
            .and_then(|value| value.network_passphrase.clone())
    });

    if profile_name.is_some() {
        validate_profile(&name, rpc_url.as_deref(), passphrase.as_deref())?;
    }

    if let Some(rpc) = rpc_url.as_deref() {
        validate_rpc_url(rpc)?;
    }
    if rpc_url.is_some() != passphrase.is_some() {
        bail!(
            "incomplete network configuration: both --rpc-url and --network-passphrase are required together"
        );
    }

    Ok(ResolvedNetwork {
        name,
        rpc_url,
        network_passphrase: passphrase,
    })
}

fn validate_profile(name: &str, rpc_url: Option<&str>, passphrase: Option<&str>) -> Result<()> {
    if name.trim().is_empty() {
        bail!("network profile is missing a name");
    }
    let rpc = rpc_url.ok_or_else(|| {
        anyhow::anyhow!("network profile '{name}' is incomplete: missing rpc_url")
    })?;
    let passphrase = passphrase.ok_or_else(|| {
        anyhow::anyhow!("network profile '{name}' is incomplete: missing network_passphrase")
    })?;
    validate_rpc_url(rpc)?;
    if passphrase.trim().is_empty() {
        bail!("network profile '{name}' is incomplete: network_passphrase cannot be empty");
    }
    Ok(())
}

fn validate_rpc_url(url: &str) -> Result<()> {
    let trimmed = url.trim();
    if trimmed.is_empty()
        || !(trimmed.starts_with("http://") || trimmed.starts_with("https://"))
        || trimmed == "http://"
        || trimmed == "https://"
    {
        bail!("malformed RPC URL '{url}'; use an absolute http:// or https:// URL");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_profiles(contents: &str) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().expect("temp profile file");
        fs::write(file.path(), contents).expect("write profile file");
        file
    }

    #[test]
    fn resolves_named_profile() {
        let file = temp_profiles(
            "[profiles.local]\nrpc_url = \"http://127.0.0.1:8000\"\nnetwork_passphrase = \"Standalone Network ; February 2017\"\n",
        );
        env::set_var("SOROBAN_FORGE_NETWORK_PROFILES", file.path());
        let resolved = resolve_network(None, Some("local"), None, None).expect("resolve profile");
        assert_eq!(resolved.name, "local");
        assert_eq!(resolved.rpc_url.as_deref(), Some("http://127.0.0.1:8000"));
        env::remove_var("SOROBAN_FORGE_NETWORK_PROFILES");
    }

    #[test]
    fn explicit_flags_override_profile() {
        let file = temp_profiles(
            "[profiles.custom]\nrpc_url = \"https://profile.example\"\nnetwork_passphrase = \"Profile passphrase\"\n",
        );
        env::set_var("SOROBAN_FORGE_NETWORK_PROFILES", file.path());
        let resolved = resolve_network(
            Some("override"),
            Some("custom"),
            Some("https://override.example"),
            Some("Override passphrase"),
        )
        .expect("resolve overrides");
        assert_eq!(resolved.name, "override");
        assert_eq!(
            resolved.rpc_url.as_deref(),
            Some("https://override.example")
        );
        assert_eq!(
            resolved.network_passphrase.as_deref(),
            Some("Override passphrase")
        );
        env::remove_var("SOROBAN_FORGE_NETWORK_PROFILES");
    }

    #[test]
    fn unknown_profile_is_actionable() {
        let file = temp_profiles("[profiles.local]\nrpc_url = \"https://local.example\"\nnetwork_passphrase = \"Local\"\n");
        env::set_var("SOROBAN_FORGE_NETWORK_PROFILES", file.path());
        let error =
            resolve_network(None, Some("missing"), None, None).expect_err("unknown profile");
        assert!(error
            .to_string()
            .contains("unknown network profile 'missing'"));
        env::remove_var("SOROBAN_FORGE_NETWORK_PROFILES");
    }

    #[test]
    fn incomplete_profile_is_rejected() {
        let file = temp_profiles("[profiles.local]\nrpc_url = \"https://local.example\"\n");
        env::set_var("SOROBAN_FORGE_NETWORK_PROFILES", file.path());
        let error =
            resolve_network(None, Some("local"), None, None).expect_err("incomplete profile");
        assert!(error.to_string().contains("missing network_passphrase"));
        env::remove_var("SOROBAN_FORGE_NETWORK_PROFILES");
    }

    #[test]
    fn malformed_profile_file_is_rejected() {
        let file = temp_profiles("[profiles.local\nrpc_url =");
        env::set_var("SOROBAN_FORGE_NETWORK_PROFILES", file.path());
        let error = resolve_network(None, Some("local"), None, None).expect_err("malformed file");
        assert!(error.to_string().contains("malformed network profile file"));
        env::remove_var("SOROBAN_FORGE_NETWORK_PROFILES");
    }

    #[test]
    fn no_profile_preserves_testnet_default() {
        let resolved = resolve_network(None, None, None, None).expect("default network");
        assert_eq!(resolved.name, "testnet");
        assert_eq!(resolved.rpc_url, None);
        assert_eq!(resolved.network_passphrase, None);
    }

    #[test]
    fn secret_fields_are_rejected() {
        let file = temp_profiles(
            "[profiles.local]\nrpc_url = \"https://local.example\"\nnetwork_passphrase = \"Local\"\nsecret_key = \"SAAAA\"\n",
        );
        env::set_var("SOROBAN_FORGE_NETWORK_PROFILES", file.path());
        let error = resolve_network(None, Some("local"), None, None).expect_err("secret field");
        assert!(error.to_string().contains("malformed network profile file"));
        env::remove_var("SOROBAN_FORGE_NETWORK_PROFILES");
    }
}
