# arch

Sane Arch Linux package manager wrapping libalpm.

- **Safe installs**: Local installs are hermetic and never upgrade the system; use `upgrade` explicitly for that
- **Simple commands**: `install`, `remove`, `upgrade` instead of `-Syu`, `-Rs`, `-Qu`
- **Structured logging**: Operations logged to journald with queryable fields

## Build and install

Build and install `arch`, `arch-fakeroot`, and `arch-makepkg` to `/usr/local/bin` with:

```bash
./deploy.sh
```

For a pacman-tracked local install, build the package from this directory instead:

```bash
arch install .
```

## Local package sources

`arch install <directory>` builds a directory containing `PKGBUILD` in the sandbox and installs the resulting package without syncing repositories. Supported PKGBUILD arrays—`arch`, `license`, `depends`, `makedepends`, `checkdepends`, `optdepends`, `provides`, `conflicts`, `replaces`, and `backup`—retain element boundaries, including spaces within individual elements, when passed into package creation. `arch=('any')` remains architecture-independent; otherwise the current host architecture is selected when listed.

A directory without `PKGBUILD` but with `deploy.sh` is also supported: `arch install <directory>` runs the script, captures files written to supported install roots, synthesizes a local package, and installs it through ALPM. If both files exist, `PKGBUILD` takes precedence.

## Usage

```bash
arch --help
arch <command> --help
```
