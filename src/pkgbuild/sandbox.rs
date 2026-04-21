use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

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

fn add_runtime_mounts(cmd: &mut Command) {
    cmd.args(["--dev", "/dev"]);
    cmd.args(["--proc", "/proc"]);
    cmd.args(["--tmpfs", "/tmp"]);
    cmd.args(["--tmpfs", "/home"]);
    add_rustup_bind(cmd);
}

fn add_execution_options(cmd: &mut Command) {
    cmd.args(["--chdir", "/src"]);
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
        add_runtime_mounts(&mut cmd);
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
