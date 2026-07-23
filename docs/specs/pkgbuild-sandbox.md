# PKGBUILD Cargo sandbox

The PKGBUILD sandbox provides hermetic Cargo builds while reusing the invoking user's cached dependencies. This contract is implemented in `src/pkgbuild/sandbox.rs` and exercised by the tests listed below.

## What it must do

- [ ] Detect a Cargo PKGBUILD source by finding a `Cargo.toml`.
- [ ] Make cached Rust dependencies available from the invoking user's Cargo `registry` and `git` directories.
- [ ] Mount available Cargo caches read-only under an isolated `CARGO_HOME`.
- [ ] Set `CARGO_NET_OFFLINE=true` for Cargo builds.
- [x] Disable sandbox networking.
- [x] Build a Rust package from the host Cargo cache without network access.

## How it works

Implementation details are maintained in `src/pkgbuild/sandbox.rs`; this spec records only the contract.

## Implementation inventory

- `src/pkgbuild/sandbox.rs` — detects Cargo sources, mounts Cargo caches, configures Cargo isolation, and disables networking.
- `tests/build_test.rs` — end-to-end Rust package build using the host Cargo cache.

## Tests asserting this spec

- `src/pkgbuild/sandbox.rs::tests::sandbox_network_is_disabled`
- `tests/build_test.rs::test_builds_rust_package_from_host_cargo_cache`

## Known gaps (current cycle)

- [ ] Add direct assertions for read-only cache mounts, isolated `CARGO_HOME`, `CARGO_NET_OFFLINE`, and recursive manifest detection.

## Out of scope

- Downloading missing Cargo dependencies during a hermetic build.
- Allowing network access from the PKGBUILD sandbox.
