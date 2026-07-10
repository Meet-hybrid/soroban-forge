use soroban_sdk::{Address, Env, String};

pub fn normalize_address(env: &Env, address: Address) -> Address {
    address
}

pub fn validate_url(url: &str) -> anyhow::Result<()> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(())
    } else {
        Err(anyhow::anyhow!("invalid URL: {}", url))
    }
}
