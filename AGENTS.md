# Agent Build Rules

These rules are mandatory for all automated agents working in this repository.

## Nix-First Execution

- Always run builds, checks, tests, and formatting through a Nix dev shell.
- Do not run `cargo`, `rustc`, `clippy`, `rustfmt`, or test commands directly on the host.

Use:

```bash
nix develop -c <command>
```

If `nix` is not on `PATH`, use the absolute binary:

```bash
/nix/var/nix/profiles/default/bin/nix develop -c <command>
```

## Required Command Forms

- Server check: `nix develop -c cargo check --features server`
- Web check (wasm): `nix develop -c cargo check --no-default-features --features web --target wasm32-unknown-unknown`
- Format: `nix develop -c cargo fmt`

## Why

- Ensures deterministic toolchains.
- Avoids host linker/toolchain mismatches (including macOS linker/libiconv issues).
