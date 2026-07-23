# arch

Sane Arch Linux package manager wrapping libalpm.

- **Safe installs**: Local installs use existing package databases, do not sync repositories, and never perform a system upgrade; use `upgrade` explicitly for that
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

`arch install <directory>` builds a directory containing `PKGBUILD` in the sandbox, copies the archive produced by that build into the directory, and installs that current archive without syncing repositories; stale package archives already in the directory are ignored. Supported PKGBUILD arrays—`arch`, `license`, `depends`, `makedepends`, `checkdepends`, `optdepends`, `provides`, `conflicts`, `replaces`, and `backup`—retain element boundaries, including spaces within individual elements, when passed into package creation. `arch=('any')` remains architecture-independent; otherwise the current host architecture is selected when listed, and the first declared architecture is used when it is not.

Cargo PKGBUILD builds detect a `Cargo.toml`, expose the invoking user's Cargo `registry` and `git` caches read-only through an isolated `CARGO_HOME`, set `CARGO_NET_OFFLINE=true`, and disable sandbox networking. See [`docs/specs/pkgbuild-sandbox.md`](docs/specs/pkgbuild-sandbox.md).

A directory without `PKGBUILD` but with `deploy.sh` is also supported: `arch install <directory>` runs the script, captures files written to supported install roots, synthesizes a local package, and installs it through ALPM. If both files exist, `PKGBUILD` takes precedence.

## Testing

Run the full formatting, clippy, and test checks with:

```bash
./run-tests.sh
```

Run selective unit tests with:

```bash
./run-tests.sh unit <filter>
```

## Usage

```bash
arch --help
arch <command> --help
```
