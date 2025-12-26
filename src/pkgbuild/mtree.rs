use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use walkdir::WalkDir;

pub fn create_mtree(pkgdir: &Path) -> Result<()> {
    // Collect all files in the package directory
    let mut files: Vec<String> = Vec::new();

    for entry in WalkDir::new(pkgdir)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        let relative = path
            .strip_prefix(pkgdir)
            .context("Failed to get relative path")?;

        // Skip .MTREE itself
        let name = relative.to_string_lossy();
        if name == ".MTREE" {
            continue;
        }

        files.push(format!("./{}", name));
    }

    // Sort files for reproducibility
    files.sort();

    // Create file list for bsdtar
    let file_list = files.join("\0");

    // Run bsdtar to generate mtree
    let mut bsdtar = Command::new("bsdtar")
        .current_dir(pkgdir)
        .args([
            "--create",
            "--file",
            "-",
            "--format=mtree",
            "--options",
            "!all,use-set,type,uid,gid,mode,time,size,sha256,link",
            "--null",
            "--files-from",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn bsdtar")?;

    // Write file list to stdin
    if let Some(mut stdin) = bsdtar.stdin.take() {
        stdin
            .write_all(file_list.as_bytes())
            .context("Failed to write to bsdtar stdin")?;
    }

    let output = bsdtar.wait_with_output().context("Failed to wait for bsdtar")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("bsdtar failed: {}", stderr);
    }

    // Compress with gzip and write to .MTREE
    let mtree_path = pkgdir.join(".MTREE");
    let file = File::create(&mtree_path).context("Failed to create .MTREE")?;
    let mut encoder = GzEncoder::new(file, Compression::best());
    encoder
        .write_all(&output.stdout)
        .context("Failed to write .MTREE")?;
    encoder.finish().context("Failed to finish .MTREE compression")?;

    Ok(())
}
