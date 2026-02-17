# Integration Guide

This project ships a NixOS module that installs and runs the monitor as a service.

## Flake Integration

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nixos-builder-mon = {
      url = "path:/path/to/nixos-builder-mon";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, nixos-builder-mon, ... }: {
    nixosConfigurations.builder = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
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

## What The Module Configures

- `systemd.services.nixos-builder-mon`
  - serves the Dioxus fullstack binary
- `systemd.services.nixos-builder-mon-daemon`
  - follows nix-daemon logs and pipes through `nom`
  - appends to `/var/log/nom-output.log`
- installs `nix-output-monitor`
- installs Nerd Fonts for terminal glyphs:
  - `JetBrains Mono Nerd Font`
  - `Iosevka Nerd Font`

Daemon pipeline is configured as:

```bash
unbuffer journalctl -u nix-daemon -n 0 --no-pager --no-hostname -o cat -f 2>&1 | nom | tee -a /var/log/nom-output.log
```

## Best Practice For Manual Builds

For the richest output, run builds directly via NOM:

```bash
nom build .#target
```

Or explicit JSON mode:

```bash
nix build .#target --log-format internal-json -v |& nom --json
```

## Verify

```bash
systemctl status nixos-builder-mon
systemctl status nixos-builder-mon-daemon
journalctl -u nixos-builder-mon -f
journalctl -u nixos-builder-mon-daemon -f
```
