use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct Pkgbuild {
    pub pkgbase: String,
    pub pkgname: Vec<String>,
    pub pkgver: String,
    pub pkgrel: String,
    pub epoch: Option<u32>,
    pub pkgdesc: Option<String>,
    pub url: Option<String>,
    pub license: Vec<String>,
    pub arch: Vec<String>,
    pub depends: Vec<String>,
    pub makedepends: Vec<String>,
    pub checkdepends: Vec<String>,
    pub optdepends: Vec<String>,
    pub provides: Vec<String>,
    pub conflicts: Vec<String>,
    pub replaces: Vec<String>,
    pub backup: Vec<String>,
    pub options: Vec<String>,
    pub source: Vec<String>,
    pub checksums: HashMap<String, Vec<String>>,
    pub install: Option<String>,
    pub changelog: Option<String>,

    // Functions present
    pub has_prepare: bool,
    pub has_build: bool,
    pub has_check: bool,
    pub has_package: bool,
}

impl Pkgbuild {
    pub fn full_version(&self) -> String {
        match self.epoch {
            Some(e) if e > 0 => format!("{}:{}-{}", e, self.pkgver, self.pkgrel),
            _ => format!("{}-{}", self.pkgver, self.pkgrel),
        }
    }

    pub fn package_name(&self) -> &str {
        self.pkgname.first().map(|s| s.as_str()).unwrap_or(&self.pkgbase)
    }
}
