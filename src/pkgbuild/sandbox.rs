use std::path::Path;
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result};

/// Fakeroot daemon handle - kills daemon on drop
struct Fakeroot {
    key: String,
    daemon: Child,
}

impl Fakeroot {
    fn start() -> Result<Self> {
        let mut daemon = Command::new("faked")
            .stdout(Stdio::piped())
            .spawn()
            .context("Failed to start faked daemon")?;

        // Read KEY:PID from stdout
        let stdout = daemon.stdout.take().context("No stdout from faked")?;
        let mut output = String::new();
        std::io::Read::read_to_string(&mut std::io::BufReader::new(stdout), &mut output)?;

        let key = output
            .split(':')
            .next()
            .context("Invalid faked output")?
            .trim()
            .to_string();

        Ok(Self { key, daemon })
    }
}

impl Drop for Fakeroot {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

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

    pub fn command(&self, script: &str, fakeroot_key: &str) -> Command {
        let mut cmd = Command::new("bwrap");

        // Fakeroot environment (set inside sandbox)
        cmd.args(["--setenv", "FAKEROOTKEY", fakeroot_key]);
        cmd.args(["--setenv", "LD_LIBRARY_PATH", "/usr/lib/libfakeroot"]);
        cmd.args(["--setenv", "LD_PRELOAD", "libfakeroot.so"]);

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

        // Run bash with script (fakeroot env is set by run())
        cmd.args(["--", "bash", "-c", script]);

        cmd
    }

    pub fn run(&self, script: &str) -> Result<()> {
        // Start faked daemon (runs on host, communicates via SysV message queue)
        let fakeroot = Fakeroot::start()?;

        let status = self
            .command(script, &fakeroot.key)
            .status()
            .context("Failed to run sandboxed command")?;

        // fakeroot daemon is killed on drop

        if !status.success() {
            anyhow::bail!(
                "Sandboxed command failed with exit code {}",
                status.code().unwrap_or(-1)
            );
        }

        Ok(())
    }
}
