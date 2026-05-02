# Installation

This project installs a single binary named `wonder-of-u`.

## Requirements

- Linux, macOS, or another Unix-like shell environment
- Rust toolchain matching the workspace (`rustc 1.85+`)
- `cargo` available in `PATH`

Install Rust with [rustup](https://rustup.rs/) if needed:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Recommended install

From the repository root:

```bash
./install.sh
```

The default prefix is `$HOME/.local`, so the binary is usually copied to:

```text
$HOME/.local/bin/wonder-of-u
```

If that directory is not in `PATH`, add it to your shell profile:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## Custom prefix

```bash
./install.sh --prefix /usr/local
./install.sh --prefix "$HOME/.local"
```

The script always installs into `<prefix>/bin/wonder-of-u`.

## Debug install

Useful while developing:

```bash
./install.sh --debug --force
```

## Skip rebuild

If you already built the binary:

```bash
cargo build --release -p wonder-of-u-cli
./install.sh --skip-build --force
```

## Cargo-only install

You can also let Cargo install directly:

```bash
cargo install --path crates/wonder-of-u-cli --locked
```

## Uninstall

Use the same prefix you installed with:

```bash
./install.sh --uninstall
./install.sh --prefix /usr/local --uninstall
```

## Verify

```bash
wonder-of-u --version
wonder-of-u doctor
```
