use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use walkdir::WalkDir;

use super::fakeroot::run_sandboxed_with_fakeroot;

fn add_symlink_mount(cmd: &mut Command, path: &str, fallback_target: &str) {
    let path_obj = Path::new(path);
    if !path_obj.is_symlink() {
        return;
    }

    if let Ok(target) = std::fs::read_link(path_obj) {
        let target_str = target.to_str().unwrap_or(fallback_target);
        cmd.args(["--symlink", target_str, path]);
    }
}

fn add_lib_mount(cmd: &mut Command, path: &str, fallback_target: &str) {
    let path_obj = Path::new(path);
    if !path_obj.exists() {
        return;
    }

    if path_obj.is_symlink() {
        add_symlink_mount(cmd, path, fallback_target);
        return;
    }

    cmd.args(["--ro-bind", path, path]);
}

fn add_ro_bind_if_exists(cmd: &mut Command, source: &str, destination: &str) {
    if Path::new(source).exists() {
        cmd.args(["--ro-bind", source, destination]);
    }
}

fn cargo_home() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .map(|home| home.join(".cargo"))
        })
}

fn source_contains_cargo_manifest(source_dir: &Path) -> bool {
    WalkDir::new(source_dir)
        .into_iter()
        .filter_entry(|entry| {
            !matches!(entry.file_name().to_str(), Some(".git" | "pkg" | "target"))
        })
        .filter_map(Result::ok)
        .any(|entry| entry.file_type().is_file() && entry.file_name() == "Cargo.toml")
}

fn add_cargo_cache_mounts(cmd: &mut Command, source_dir: &Path) {
    if !source_contains_cargo_manifest(source_dir) {
        return;
    }
    let Some(host_cargo_home) = cargo_home() else {
        return;
    };

    let sandbox_cargo_home = Path::new("/tmp/arch-cargo-home");
    cmd.arg("--dir").arg(sandbox_cargo_home);
    for cache_name in ["registry", "git"] {
        let host_cache = host_cargo_home.join(cache_name);
        if host_cache.exists() {
            let sandbox_cache = sandbox_cargo_home.join(cache_name);
            cmd.arg("--ro-bind").arg(host_cache).arg(sandbox_cache);
        }
    }
    cmd.arg("--setenv")
        .arg("CARGO_HOME")
        .arg(sandbox_cargo_home);
    cmd.args(["--setenv", "CARGO_NET_OFFLINE", "true"]);
}

fn add_rustup_bind(cmd: &mut Command) {
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return;
    }

    let rustup_path = format!("{}/.rustup", home);
    if Path::new(&rustup_path).exists() {
        cmd.args(["--ro-bind", &rustup_path, &rustup_path]);
    }
}

fn bind_build_directories(cmd: &mut Command, source_dir: &Path, dest_dir: Option<&Path>) {
    let source_dir_str = source_dir.to_string_lossy();
    cmd.args(["--bind", source_dir_str.as_ref(), "/src"]);

    if let Some(dest_dir) = dest_dir {
        let dest_dir_str = dest_dir.to_string_lossy();
        cmd.args(["--bind", dest_dir_str.as_ref(), "/dest"]);
    }
}

fn add_system_mounts(cmd: &mut Command) {
    cmd.args(["--ro-bind", "/usr", "/usr"]);
    cmd.args(["--ro-bind", "/etc", "/etc"]);
    add_lib_mount(cmd, "/lib", "usr/lib");
    add_lib_mount(cmd, "/lib64", "usr/lib");
    add_symlink_mount(cmd, "/bin", "usr/bin");
    add_symlink_mount(cmd, "/sbin", "usr/bin");
    add_ro_bind_if_exists(cmd, "/var/lib/pacman", "/var/lib/pacman");
}

fn add_matching_makepkg_bind(cmd: &mut Command) {
    let makepkg = std::env::var_os("ARCH_MAKEPKG_BIN")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()?
                .parent()
                .map(|binary_dir| binary_dir.join("arch-makepkg"))
        });
    if let Some(makepkg) = makepkg.filter(|path| path.exists()) {
        cmd.arg("--ro-bind")
            .arg(makepkg)
            .arg("/usr/bin/arch-makepkg");
    }
}

fn add_runtime_mounts(cmd: &mut Command, source_dir: &Path) {
    cmd.args(["--dev", "/dev"]);
    cmd.args(["--proc", "/proc"]);
    cmd.args(["--tmpfs", "/tmp"]);
    cmd.args(["--tmpfs", "/home"]);
    add_rustup_bind(cmd);
    add_cargo_cache_mounts(cmd, source_dir);
}

fn add_execution_options(cmd: &mut Command) {
    cmd.args(["--chdir", "/src"]);
    cmd.arg("--unshare-net");
    cmd.arg("--die-with-parent");
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

    fn build_bwrap_command(&self) -> Command {
        let mut cmd = Command::new("bwrap");

        add_system_mounts(&mut cmd);
        add_runtime_mounts(&mut cmd, self.source_dir);
        add_matching_makepkg_bind(&mut cmd);
        bind_build_directories(&mut cmd, self.source_dir, self.dest_dir);
        add_execution_options(&mut cmd);

        cmd
    }

    pub fn run(&self, script: &str) -> Result<()> {
        let bwrap_cmd = self.build_bwrap_command();
        run_sandboxed_with_fakeroot(bwrap_cmd, script).context("Sandboxed command failed")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn command_args(command: &Command) -> Vec<OsString> {
        command.get_args().map(|arg| arg.to_os_string()).collect()
    }

    #[test]
    fn sandbox_network_is_disabled() {
        let mut command = Command::new("bwrap");

        add_execution_options(&mut command);

        assert!(command_args(&command).contains(&OsString::from("--unshare-net")));
    }
}
