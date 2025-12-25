# arch

Sane Arch Linux package manager wrapping libalpm.

- **Safe installs**: Always syncs and upgrades before installing (no partial upgrades)
- **Simple commands**: `install`, `remove`, `upgrade` instead of `-Syu`, `-Rs`, `-Qu`
- **Structured logging**: Operations logged to journald with queryable fields

## Build

```bash
cargo build --release
sudo cp target/release/arch /usr/local/bin/
```

## Usage

```bash
arch --help
arch <command> --help
```
