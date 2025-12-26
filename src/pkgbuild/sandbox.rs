use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

pub struct Sandbox<'a> {
    build_dir: &'a Path,
}

impl<'a> Sandbox<'a> {
    pub fn new(build_dir: &'a Path) -> Self {
        Self { build_dir }
    }

    pub fn command(&self, script: &str) -> Command {
        let mut cmd = Command::new("bwrap");

        // Read-only system directories
        cmd.args(["--ro-bind", "/usr", "/usr"]);
        cmd.args(["--ro-bind", "/etc", "/etc"]);

        // Handle /lib and /lib64 (may be symlinks or real dirs)
        if Path::new("/lib").exists() {
            if Path::new("/lib").is_symlink() {
                // It's a symlink, create the same symlink in sandbox
                if let Ok(target) = std::fs::read_link("/lib") {
                    cmd.args([
                        "--symlink",
                        target.to_str().unwrap_or("usr/lib"),
                        "/lib",
                    ]);
                }
            } else {
                cmd.args(["--ro-bind", "/lib", "/lib"]);
            }
        }

        if Path::new("/lib64").exists() {
            if Path::new("/lib64").is_symlink() {
                if let Ok(target) = std::fs::read_link("/lib64") {
                    cmd.args([
                        "--symlink",
                        target.to_str().unwrap_or("usr/lib"),
                        "/lib64",
                    ]);
                }
            } else {
                cmd.args(["--ro-bind", "/lib64", "/lib64"]);
            }
        }

        // Package database (for pkgver() functions that query installed packages)
        if Path::new("/var/lib/pacman").exists() {
            cmd.args(["--ro-bind", "/var/lib/pacman", "/var/lib/pacman"]);
        }

        // Essential mounts
        cmd.args(["--dev", "/dev"]);
        cmd.args(["--proc", "/proc"]);
        cmd.args(["--tmpfs", "/tmp"]);

        // Empty /home - no access to user data
        cmd.args(["--tmpfs", "/home"]);

        // Bind build tools read-only, use env vars for writable state
        let home = std::env::var("HOME").unwrap_or_default();
        if !home.is_empty() {
            // Rustup toolchains (read-only, toolchains don't need writes during build)
            let rustup = format!("{}/.rustup", home);
            if Path::new(&rustup).exists() {
                cmd.args(["--ro-bind", &rustup, &rustup]);
            }

            // Cargo bin directory only (read-only)
            let cargo_bin = format!("{}/.cargo/bin", home);
            if Path::new(&cargo_bin).exists() {
                cmd.args(["--ro-bind", &cargo_bin, &cargo_bin]);
            }

            // Cargo registry (read-only cache of downloaded crates)
            let cargo_registry = format!("{}/.cargo/registry", home);
            if Path::new(&cargo_registry).exists() {
                cmd.args(["--ro-bind", &cargo_registry, &cargo_registry]);
            }

            // Cargo git checkouts (read-only cache)
            let cargo_git = format!("{}/.cargo/git", home);
            if Path::new(&cargo_git).exists() {
                cmd.args(["--ro-bind", &cargo_git, &cargo_git]);
            }
        }

        // Writable build directory
        let build_dir_str = self.build_dir.to_string_lossy();
        cmd.args(["--bind", &build_dir_str, &build_dir_str]);

        // Set working directory
        cmd.args(["--chdir", &build_dir_str]);

        // Die when parent dies (cleanup on error)
        cmd.arg("--die-with-parent");

        // Run bash with script
        cmd.args(["--", "bash", "-c", script]);

        cmd
    }

    pub fn run(&self, script: &str) -> Result<()> {
        let status = self
            .command(script)
            .status()
            .context("Failed to run sandboxed command")?;

        if !status.success() {
            anyhow::bail!(
                "Sandboxed command failed with exit code {}",
                status.code().unwrap_or(-1)
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_sandbox_command_args() {
        let build_dir = PathBuf::from("/tmp/test-build");
        let sandbox = Sandbox::new(&build_dir);
        let cmd = sandbox.command("echo hello");

        let args: Vec<_> = cmd.get_args().map(|s| s.to_string_lossy()).collect();

        assert!(args.contains(&"--ro-bind".into()));
        assert!(args.contains(&"/usr".into()));
        assert!(args.contains(&"--tmpfs".into()));
        assert!(args.contains(&"/home".into()));
        assert!(args.contains(&"--die-with-parent".into()));
    }
}
