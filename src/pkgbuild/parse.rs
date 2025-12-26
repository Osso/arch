use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

use super::types::Pkgbuild;

const BRIDGE_SCRIPT: &str = r#"
set -e

emit_string() {
    local name="$1"
    local val="${!name}"
    if [[ -n "$val" ]]; then
        printf 'STRING %s %s\n' "$name" "$val"
    fi
}

emit_array() {
    local name="$1"
    local -n arr="$name" 2>/dev/null || return 0
    if [[ ${#arr[@]} -gt 0 ]]; then
        printf 'ARRAY %s' "$name"
        for val in "${arr[@]}"; do
            printf '\t%s' "$val"
        done
        printf '\n'
    fi
}

emit_function() {
    local name="$1"
    if declare -f "$name" >/dev/null 2>&1; then
        printf 'FUNCTION %s\n' "$name"
    fi
}

source "$1"

# pkgbase defaults to first pkgname
if [[ -z "$pkgbase" ]]; then
    if [[ -n "${pkgname[0]}" ]]; then
        pkgbase="${pkgname[0]}"
    elif [[ -n "$pkgname" ]]; then
        pkgbase="$pkgname"
    fi
fi

# String variables
for var in pkgbase pkgver pkgrel epoch pkgdesc url install changelog; do
    emit_string "$var"
done

# Array variables
for var in pkgname arch license depends makedepends checkdepends optdepends \
           provides conflicts replaces backup options source noextract \
           validpgpkeys sha256sums sha512sums b2sums md5sums; do
    emit_array "$var"
done

# Functions
for func in prepare build check package; do
    emit_function "$func"
done
"#;

pub fn parse_pkgbuild(path: &Path) -> Result<Pkgbuild> {
    let output = Command::new("bash")
        .arg("-c")
        .arg(BRIDGE_SCRIPT)
        .arg("--")
        .arg(path)
        .output()
        .context("Failed to run PKGBUILD parser")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to parse PKGBUILD: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_bridge_output(&stdout)
}

fn parse_bridge_output(output: &str) -> Result<Pkgbuild> {
    let mut pkg = Pkgbuild::default();
    let mut checksums: HashMap<String, Vec<String>> = HashMap::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("STRING ") {
            let (key, value) = rest.split_once(' ').unwrap_or((rest, ""));
            match key {
                "pkgbase" => pkg.pkgbase = value.to_string(),
                "pkgver" => pkg.pkgver = value.to_string(),
                "pkgrel" => pkg.pkgrel = value.to_string(),
                "epoch" => pkg.epoch = value.parse().ok(),
                "pkgdesc" => pkg.pkgdesc = Some(value.to_string()),
                "url" => pkg.url = Some(value.to_string()),
                "install" => pkg.install = Some(value.to_string()),
                "changelog" => pkg.changelog = Some(value.to_string()),
                _ => {}
            }
        } else if let Some(rest) = line.strip_prefix("ARRAY ") {
            let mut parts = rest.split('\t');
            let key = parts.next().unwrap_or("");
            let values: Vec<String> = parts.map(|s| s.to_string()).collect();

            match key {
                "pkgname" => pkg.pkgname = values,
                "arch" => pkg.arch = values,
                "license" => pkg.license = values,
                "depends" => pkg.depends = values,
                "makedepends" => pkg.makedepends = values,
                "checkdepends" => pkg.checkdepends = values,
                "optdepends" => pkg.optdepends = values,
                "provides" => pkg.provides = values,
                "conflicts" => pkg.conflicts = values,
                "replaces" => pkg.replaces = values,
                "backup" => pkg.backup = values,
                "options" => pkg.options = values,
                "source" => pkg.source = values,
                "sha256sums" | "sha512sums" | "b2sums" | "md5sums" => {
                    checksums.insert(key.to_string(), values);
                }
                _ => {}
            }
        } else if let Some(rest) = line.strip_prefix("FUNCTION ") {
            match rest {
                "prepare" => pkg.has_prepare = true,
                "build" => pkg.has_build = true,
                "check" => pkg.has_check = true,
                "package" => pkg.has_package = true,
                _ => {}
            }
        }
    }

    pkg.checksums = checksums;

    // Validate required fields
    if pkg.pkgbase.is_empty() {
        bail!("PKGBUILD missing pkgbase/pkgname");
    }
    if pkg.pkgver.is_empty() {
        bail!("PKGBUILD missing pkgver");
    }
    if pkg.pkgrel.is_empty() {
        bail!("PKGBUILD missing pkgrel");
    }
    if !pkg.has_package {
        bail!("PKGBUILD missing package() function");
    }

    Ok(pkg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bridge_output() {
        let output = r#"
STRING pkgbase mypackage
STRING pkgver 1.0.0
STRING pkgrel 1
STRING pkgdesc A test package
ARRAY pkgname	mypackage
ARRAY arch	x86_64
ARRAY depends	glibc	openssl
FUNCTION build
FUNCTION package
"#;
        let pkg = parse_bridge_output(output).unwrap();
        assert_eq!(pkg.pkgbase, "mypackage");
        assert_eq!(pkg.pkgver, "1.0.0");
        assert_eq!(pkg.pkgrel, "1");
        assert_eq!(pkg.pkgdesc, Some("A test package".to_string()));
        assert_eq!(pkg.pkgname, vec!["mypackage"]);
        assert_eq!(pkg.arch, vec!["x86_64"]);
        assert_eq!(pkg.depends, vec!["glibc", "openssl"]);
        assert!(pkg.has_build);
        assert!(pkg.has_package);
        assert!(!pkg.has_prepare);
    }
}
