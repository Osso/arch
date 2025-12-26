use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use super::fakeroot::run_sandboxed_with_fakeroot;

pub struct Sandbox<'a> {
    source_dir: &'a Path,
    dest_dir: Option<&'a Path>,
}

impl<'a> Sandbox<'a> {
    pub fn new(source_dir: &'a Path) -> Self {
        Self {
            source_dir,
            dest_dir: None,
        }
    }

    /// Add a writable destination directory for package output
    pub fn with_dest_dir(mut self, dest_dir: &'a Path) -> Self {
        self.dest_dir = Some(dest_dir);
        self
    }

    fn build_bwrap_command(&self) -> Command {
        let mut cmd = Command::new("bwrap");

        // Read-only system directories
        cmd.args(["--ro-bind", "/usr", "/usr"]);
        cmd.args(["--ro-bind", "/etc", "/etc"]);

        // Handle /lib and /lib64 (may be symlinks or real dirs)
        if Path::new("/lib").exists() {
            if Path::new("/lib").is_symlink() {
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

        // Handle /bin and /sbin (may be symlinks on modern systems)
        if Path::new("/bin").is_symlink() {
            if let Ok(target) = std::fs::read_link("/bin") {
                cmd.args(["--symlink", target.to_str().unwrap_or("usr/bin"), "/bin"]);
            }
        }
        if Path::new("/sbin").is_symlink() {
            if let Ok(target) = std::fs::read_link("/sbin") {
                cmd.args(["--symlink", target.to_str().unwrap_or("usr/bin"), "/sbin"]);
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

        // Bind build tools read-only under /opt (not /home)
        let home = std::env::var("HOME").unwrap_or_default();
        if !home.is_empty() {
            // Rustup toolchains -> /opt/rustup
            let rustup = format!("{}/.rustup", home);
            if Path::new(&rustup).exists() {
                cmd.args(["--ro-bind", &rustup, "/opt/rustup"]);
            }

            // Cargo bin directory -> /opt/cargo/bin
            let cargo_bin = format!("{}/.cargo/bin", home);
            if Path::new(&cargo_bin).exists() {
                cmd.args(["--ro-bind", &cargo_bin, "/opt/cargo/bin"]);
            }

            // Cargo registry (read-only cache) -> /opt/cargo/registry
            let cargo_registry = format!("{}/.cargo/registry", home);
            if Path::new(&cargo_registry).exists() {
                cmd.args(["--ro-bind", &cargo_registry, "/opt/cargo/registry"]);
            }

            // Cargo git checkouts (read-only cache) -> /opt/cargo/git
            let cargo_git = format!("{}/.cargo/git", home);
            if Path::new(&cargo_git).exists() {
                cmd.args(["--ro-bind", &cargo_git, "/opt/cargo/git"]);
            }
        }

        // Source directory -> /src (writable for build artifacts)
        let source_dir_str = self.source_dir.to_string_lossy();
        cmd.args(["--bind", &source_dir_str, "/src"]);

        // Optional destination directory -> /dest for package output
        if let Some(dest_dir) = self.dest_dir {
            let dest_dir_str = dest_dir.to_string_lossy();
            cmd.args(["--bind", &dest_dir_str, "/dest"]);
        }

        // Set working directory to /src
        cmd.args(["--chdir", "/src"]);

        // Die when parent dies (cleanup on error)
        cmd.arg("--die-with-parent");

        cmd
    }

    pub fn run(&self, script: &str) -> Result<()> {
        let bwrap_cmd = self.build_bwrap_command();
        run_sandboxed_with_fakeroot(bwrap_cmd, script)
            .context("Sandboxed command failed")?;
        Ok(())
    }
}
