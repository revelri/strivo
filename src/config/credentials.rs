use anyhow::{Context, Result};

const SERVICE_NAME: &str = "streavo";

pub fn store_secret(key: &str, value: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE_NAME, key)
        .context("Failed to create keyring entry")?;
    entry
        .set_password(value)
        .context("Failed to store secret in keyring")?;
    Ok(())
}

pub fn get_secret(key: &str) -> Result<Option<String>> {
    let entry = keyring::Entry::new(SERVICE_NAME, key)
        .context("Failed to create keyring entry")?;
    match entry.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(anyhow::anyhow!("Failed to retrieve secret: {}", e)),
    }
}

pub fn delete_secret(key: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE_NAME, key)
        .context("Failed to create keyring entry")?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(anyhow::anyhow!("Failed to delete secret: {}", e)),
    }
}
