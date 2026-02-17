# Integration Guide

This project ships a NixOS module that installs and runs the monitor as a service.

## Flake Integration

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nixos-buildermon.url = "github:balaclava-guy/nixos-buildermon";
    # For local development instead of GitHub:
    # nixos-buildermon.url = "path:/path/to/nixos-buildermon";
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

Apply configuration:

```bash
sudo nixos-rebuild switch --flake .#builder
```

## What The Module Configures

- `systemd.services.nixos-buildermon`
  - serves the Dioxus fullstack binary
- `systemd.services.nixos-buildermon-daemon`
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
systemctl status nixos-buildermon
systemctl status nixos-buildermon-daemon
journalctl -u nixos-buildermon -f
journalctl -u nixos-buildermon-daemon -f
```
