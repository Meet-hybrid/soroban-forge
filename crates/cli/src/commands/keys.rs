use crate::cli::{KeysArgs, KeysCommand};
use anyhow::{bail, Context, Result};
use rand::{rngs::OsRng, RngCore};
use serde::Serialize;
use std::io::IsTerminal;
use stellar_strkey::{ed25519, Strkey};

#[derive(Debug, Serialize, PartialEq, Eq)]
struct GeneratedKeys {
    public_key: String,
    secret_seed: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct InspectedKey {
    input: String,
    kind: String,
    version_byte: String,
    payload_hex: String,
    checksum_valid: bool,
}

pub fn run(args: KeysArgs) -> Result<()> {
    match args.command {
        KeysCommand::Generate { json } => generate(json),
        KeysCommand::Inspect { address_or_key } => inspect(&address_or_key),
    }
}

fn generate(json: bool) -> Result<()> {
    let mut seed = [0_u8; 32];
    OsRng.fill_bytes(&mut seed);
    let private = ed25519::PrivateKey(seed);
    let public = ed25519::PublicKey(
        ed25519_dalek::SigningKey::from_bytes(&seed)
            .verifying_key()
            .to_bytes(),
    );
    let result = GeneratedKeys {
        public_key: public.to_string().to_string(),
        secret_seed: private.as_unredacted().to_string().to_string(),
    };

    if !std::io::stdout().is_terminal() {
        eprintln!("warning: generated secret seed is being written to non-terminal output");
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Public key: {}", result.public_key);
        println!("Secret seed: {}", result.secret_seed);
    }
    Ok(())
}

fn inspect(input: &str) -> Result<()> {
    let result = if input.starts_with('S') {
        let key = ed25519::PrivateKey::from_string(input).with_context(|| {
            format!("invalid secret seed '{input}': checksum or payload is invalid")
        })?;
        InspectedKey {
            input: input.to_owned(),
            kind: "secret_seed".to_owned(),
            version_byte: "0xB0".to_owned(),
            payload_hex: hex(&key.0),
            checksum_valid: true,
        }
    } else {
        let key = Strkey::from_string(input).with_context(|| {
            format!("invalid StrKey '{input}': expected a valid G..., C..., or S... value")
        })?;
        match key {
            Strkey::PublicKeyEd25519(key) => inspected(input, "public_key", "0x30", &key.0),
            Strkey::Contract(key) => inspected(input, "contract_id", "0x10", &key.0),
            _ => bail!("unsupported StrKey '{}': only public keys, contract ids, and secret seeds are supported", input),
        }
    };

    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn inspected(input: &str, kind: &str, version: &str, payload: &[u8; 32]) -> InspectedKey {
    InspectedKey {
        input: input.to_owned(),
        kind: kind.to_owned(),
        version_byte: version.to_owned(),
        payload_hex: hex(payload),
        checksum_valid: true,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_round_trips() {
        let mut seed = [0_u8; 32];
        seed[0] = 7;
        let private = ed25519::PrivateKey(seed);
        let encoded = private.as_unredacted().to_string();
        let decoded = ed25519::PrivateKey::from_string(encoded.as_str()).unwrap();
        assert_eq!(decoded.0, seed);
    }

    #[test]
    fn known_public_key_is_decoded() {
        let input = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";
        let key = Strkey::from_string(input).unwrap();
        let Strkey::PublicKeyEd25519(key) = key else {
            panic!("expected public key")
        };
        assert_eq!(hex(&key.0), "00".repeat(32));
    }

    #[test]
    fn malformed_key_returns_actionable_error() {
        let error = inspect("Gnot-a-key").unwrap_err().to_string();
        assert!(error.contains("invalid StrKey"));
    }
}
