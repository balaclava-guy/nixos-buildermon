<h1><img src="assets/logo.png" alt="NixOS Buildermon logo" width="34" style="vertical-align: middle; margin-right: 8px;" /> NixOS Buildermon</h1>

Single-binary monitor for NixOS builders. See what your nixos buiilders are building, by simply visiting their url.


It provides:
- live build output from `/var/log/nom-output.log` via SSE
- sysinfo-only system metrics (CPU, RAM, swap, disks, network)
- SSE fan-out with server-side cached snapshots
- browser-tab aware update coordination to reduce duplicate refresh work

> [!WARNING]
>  Build logs are exposed as-is via HTTP -- no auth or encryption. This is fine for my risk appetite on a local network with low-stakes packages, it may not be for yours!

## Highlights

- Modern light/dark theme
- Uses `sysinfo` exclusively for system metrics
- Forwards nix-daemon logs through `nix-output-monitor` (`nom`)
- Preserves ANSI output and renders it in the web terminal
- Includes per-core and network data

## Recommended Build Commands

For best NOM detail, run builds through `nom` directly:

```bash
nom build .#your-target
```

This follows the NOM "easy way" (`nom build`) and keeps NOM as the primary parser/output layer.

If you need a manual JSON pipeline, use:

```bash
nix build .#your-target --log-format internal-json -v |& nom --json
```

## NixOS Integration

In your system flake (Git input example):

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nixos-buildermon.url = "github:balaclava-guy/nixos-buildermon";
    nixos-buildermon.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, nixos-buildermon, ... }: {
    nixosConfigurations.builder = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        nixos-buildermon.nixosModules.default
        {
          services.nixos-buildermon = {
            enable = true;
            port = 8091;
            openFirewall = true;
          };
        }
      ];
    };
  };
}
```

Then apply it:

```bash
sudo nixos-rebuild switch --flake .#builder
```

The module configures:
- `nixos-buildermon` service (web UI)
- `nixos-buildermon-daemon` service (`journalctl nix-daemon | nom | tee /var/log/nom-output.log`)
- Nerd fonts (`JetBrains Mono Nerd Font`, `Iosevka Nerd Font`)
- `nix-output-monitor` package on system

Useful checks:

```bash
systemctl status nixos-buildermon
systemctl status nixos-buildermon-daemon
journalctl -u nixos-buildermon -f
journalctl -u nixos-buildermon-daemon -f
```

## Development

Use Dioxus commands through Nix:

```bash
nix run .#dx-serve -- --port 8091
nix run .#dx-build
```

Or from the dev shell:

```bash
nix develop -c dx serve --port 8091
nix develop -c dx build --platform web --release --fullstack true --features web
nix develop -c cargo check --features server
nix develop -c cargo check --no-default-features --features web --target wasm32-unknown-unknown
```

## Architecture

- `src/main.rs`: Dioxus app, server functions, and server-side metrics collector
- `assets/style.css`: UI styles, terminal/Nerd Font stack, responsive layout
- `flake.nix`: package + NixOS module wiring

## Notes on Resource Use

- metrics are collected in a single shared background task
- server exposes SSE streams for metrics and build logs
- background/hidden tabs pause active stream ownership; a visible tab renews ownership
- server functions still return cached snapshots for fallback reads
- CPU refresh follows sysinfo delta semantics (`MINIMUM_CPU_UPDATE_INTERVAL` warmup)
- disk list refresh is throttled

## License

MIT
