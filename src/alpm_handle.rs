use alpm::{Alpm, SigLevel};
use anyhow::{Context, Result};
use pacmanconf::Config;

/// Initialize alpm handle with system configuration
pub fn init() -> Result<Alpm> {
    let config = Config::new().context("Failed to read pacman.conf")?;

    let mut handle = Alpm::new(config.root_dir.as_str(), config.db_path.as_str())
        .context("Failed to initialize alpm")?;

    // Register sync databases from pacman.conf
    for repo in &config.repos {
        let db = handle
            .register_syncdb_mut(repo.name.as_str(), SigLevel::USE_DEFAULT)
            .with_context(|| format!("Failed to register database: {}", repo.name))?;

        for server in &repo.servers {
            db.add_server(server.as_str())
                .with_context(|| format!("Failed to add server {} to {}", server, repo.name))?;
        }
    }

    Ok(handle)
}

/// Initialize alpm handle for read-only operations (no mutable db access needed)
pub fn init_readonly() -> Result<Alpm> {
    let config = Config::new().context("Failed to read pacman.conf")?;

    let handle = Alpm::new(config.root_dir.as_str(), config.db_path.as_str())
        .context("Failed to initialize alpm")?;

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

    let mut handle = Alpm::new(config.root_dir.as_str(), config.db_path.as_str())
        .context("Failed to initialize alpm")?;

    // Set database extension to .files BEFORE registering dbs
    handle.set_dbext(".files");

    // Register sync databases from pacman.conf
    for repo in &config.repos {
        let db = handle
            .register_syncdb_mut(repo.name.as_str(), SigLevel::USE_DEFAULT)
            .with_context(|| format!("Failed to register database: {}", repo.name))?;

        for server in &repo.servers {
            db.add_server(server.as_str())
                .with_context(|| format!("Failed to add server {} to {}", server, repo.name))?;
        }
    }

    Ok(handle)
}

/// Initialize alpm handle for file database read operations
pub fn init_files_readonly() -> Result<Alpm> {
    let config = Config::new().context("Failed to read pacman.conf")?;

    let mut handle = Alpm::new(config.root_dir.as_str(), config.db_path.as_str())
        .context("Failed to initialize alpm")?;

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
