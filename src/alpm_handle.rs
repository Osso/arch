use std::thread;
use std::time::Duration;

use alpm::{Alpm, SigLevel};
use anyhow::{Context, Result};
use pacmanconf::Config;

const MAX_LOCK_RETRIES: u32 = 5;
const INITIAL_RETRY_DELAY_MS: u64 = 500;

/// Try to create alpm handle with retries for database lock
fn create_alpm_with_retry(root: &str, db_path: &str) -> Result<Alpm> {
    let mut last_error = None;

    for attempt in 0..MAX_LOCK_RETRIES {
        match Alpm::new(root, db_path) {
            Ok(handle) => return Ok(handle),
            Err(e) => {
                let err_str = format!("{:?}", e);
                // Check if it's a lock error
                if err_str.contains("lock") || err_str.contains("Lock") {
                    if attempt < MAX_LOCK_RETRIES - 1 {
                        let delay = INITIAL_RETRY_DELAY_MS * 2u64.pow(attempt);
                        eprintln!(
                            "Database locked, retrying in {}ms (attempt {}/{})",
                            delay,
                            attempt + 1,
                            MAX_LOCK_RETRIES
                        );
                        thread::sleep(Duration::from_millis(delay));
                    }
                    last_error = Some(e);
                } else {
                    // Not a lock error, fail immediately
                    return Err(e).context("Failed to initialize alpm");
                }
            }
        }
    }

    Err(last_error.unwrap()).context("Failed to initialize alpm after retries (database locked)")
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
