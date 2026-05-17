use std::thread;
use std::time::Duration;

use alpm::{Alpm, SigLevel};
use anyhow::{Context, Result};
use pacmanconf::Config;

const MAX_LOCK_RETRIES: u32 = 5;
const INITIAL_RETRY_DELAY_MS: u64 = 500;

fn is_lock_error(err: &alpm::Error) -> bool {
    err.to_string().to_lowercase().contains("lock")
}

fn is_last_retry_attempt(attempt: u32) -> bool {
    attempt >= MAX_LOCK_RETRIES - 1
}

fn wait_for_retry(attempt: u32) {
    let delay = INITIAL_RETRY_DELAY_MS * 2u64.pow(attempt);
    eprintln!(
        "Database locked, retrying in {}ms (attempt {}/{})",
        delay,
        attempt + 1,
        MAX_LOCK_RETRIES
    );
    thread::sleep(Duration::from_millis(delay));
}

fn handle_alpm_init_error(err: alpm::Error, attempt: u32) -> Result<alpm::Error> {
    if !is_lock_error(&err) {
        return Err(err).context("Failed to initialize alpm");
    }

    if !is_last_retry_attempt(attempt) {
        wait_for_retry(attempt);
    }

    Ok(err)
}

fn try_create_alpm_once(
    root: &str,
    db_path: &str,
    attempt: u32,
    last_error: &mut Option<alpm::Error>,
) -> Result<Option<Alpm>> {
    match Alpm::new(root, db_path) {
        Ok(handle) => Ok(Some(handle)),
        Err(err) => {
            let lock_error = handle_alpm_init_error(err, attempt)?;
            *last_error = Some(lock_error);
            Ok(None)
        }
    }
}

fn require_last_lock_error(last_error: Option<alpm::Error>) -> Result<alpm::Error> {
    last_error.ok_or_else(|| anyhow::anyhow!("Failed to initialize alpm without an error"))
}

/// Try to create alpm handle with retries for database lock
fn create_alpm_with_retry(root: &str, db_path: &str) -> Result<Alpm> {
    let mut last_error = None;

    for attempt in 0..MAX_LOCK_RETRIES {
        if let Some(handle) = try_create_alpm_once(root, db_path, attempt, &mut last_error)? {
            return Ok(handle);
        }
    }

    let last_error = require_last_lock_error(last_error)?;
    Err(last_error).context("Failed to initialize alpm after retries (database locked)")
}

fn configure_handle(handle: &mut Alpm, config: &Config) -> Result<()> {
    if config.cache_dir.is_empty() {
        handle
            .add_cachedir("/var/cache/pacman/pkg/")
            .context("Failed to add default cache directory")?;
    } else {
        for dir in &config.cache_dir {
            handle
                .add_cachedir(dir.as_str())
                .context("Failed to add cache directory")?;
        }
    }

    for repo in &config.repos {
        let db = handle
            .register_syncdb_mut(repo.name.as_str(), SigLevel::USE_DEFAULT)
            .with_context(|| format!("Failed to register database: {}", repo.name))?;

        for server in &repo.servers {
            db.add_server(server.as_str())
                .with_context(|| format!("Failed to add server {} to {}", server, repo.name))?;
        }
    }

    Ok(())
}

/// Initialize alpm handle with system configuration
pub fn init() -> Result<Alpm> {
    let config = Config::new().context("Failed to read pacman.conf")?;
    let mut handle = create_alpm_with_retry(config.root_dir.as_str(), config.db_path.as_str())?;
    configure_handle(&mut handle, &config)?;
    Ok(handle)
}

/// Initialize alpm handle for read-only operations (no mutable db access needed)
pub fn init_readonly() -> Result<Alpm> {
    let config = Config::new().context("Failed to read pacman.conf")?;

    let handle = create_alpm_with_retry(config.root_dir.as_str(), config.db_path.as_str())?;

    // Register sync databases from pacman.conf (read-only)
    for repo in &config.repos {
        let db = handle
            .register_syncdb(repo.name.as_str(), SigLevel::USE_DEFAULT)
            .with_context(|| format!("Failed to register database: {}", repo.name))?;

        // Note: Can't add servers to read-only db, but they should already be in the local cache
        let _ = db; // Silence unused warning
    }

    Ok(handle)
}

/// Initialize alpm handle for file database operations (sync)
pub fn init_files() -> Result<Alpm> {
    let config = Config::new().context("Failed to read pacman.conf")?;
    let mut handle = create_alpm_with_retry(config.root_dir.as_str(), config.db_path.as_str())?;
    handle.set_dbext(".files");
    configure_handle(&mut handle, &config)?;
    Ok(handle)
}

/// Initialize alpm handle for file database read operations
pub fn init_files_readonly() -> Result<Alpm> {
    let config = Config::new().context("Failed to read pacman.conf")?;

    let mut handle = create_alpm_with_retry(config.root_dir.as_str(), config.db_path.as_str())?;

    // Set database extension to .files BEFORE registering dbs
    handle.set_dbext(".files");

    // Register sync databases from pacman.conf (read-only)
    for repo in &config.repos {
        let db = handle
            .register_syncdb(repo.name.as_str(), SigLevel::USE_DEFAULT)
            .with_context(|| format!("Failed to register database: {}", repo.name))?;

        let _ = db;
    }

    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_readonly() {
        // This test requires a real Arch system with pacman.conf
        let result = init_readonly();
        assert!(result.is_ok(), "Failed to init: {:?}", result.err());

        let handle = result.unwrap();
        // Should have local db
        let local = handle.localdb();
        assert_eq!(local.name(), "local");
    }
}
