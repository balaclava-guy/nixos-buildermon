# NixOS Builder Monitor

Single-binary Dioxus 0.7 monitor for NixOS builders.

It provides:
- live build output from `/var/log/nom-output.log` via SSE
- sysinfo-only system metrics (CPU, RAM, swap, disks, network)
- SSE fan-out with server-side cached snapshots
- browser-tab aware update coordination to reduce duplicate refresh work

## Highlights

- Dioxus `0.7` fullstack app in one binary
- Uses `sysinfo` exclusively for system metrics
- Forwards nix-daemon logs through `nix-output-monitor` (`nom`)
- Preserves ANSI output and renders it in the web terminal
- Includes per-core and network sparklines
- Nerd Font stack for glyph-heavy terminal output
- Bundles JetBrainsMono Nerd Font (webfont) so the terminal works without a local Nerd Font; still prefers local Nerd Fonts when available

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

In your system flake:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nixos-builder-mon.url = "github:balaclava-guy/nixos-builder-mon";
    nixos-builder-mon.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, nixos-builder-mon, ... }: {
    nixosConfigurations.builder = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        nixos-builder-mon.nixosModules.default
        {
          services.nixos-builder-mon = {
            enable = true;
            port = 80;
            openFirewall = true;
          };
        }
      ];
    };
  };
}
```

The module configures:
- `nixos-builder-mon` service (web UI)
- `nixos-builder-mon-daemon` service (`journalctl nix-daemon | nom | tee /var/log/nom-output.log`)
- Nerd fonts (`JetBrains Mono Nerd Font`, `Iosevka Nerd Font`)
- `nix-output-monitor` package on system

## Development

Because this environment may have linker quirks, use `nix develop`:

```bash
nix develop -c cargo check --features server
nix develop -c cargo check --features web --target wasm32-unknown-unknown
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
