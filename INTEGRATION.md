# Integration Guide

This guide shows how to integrate nixos-builder-mon into your NixOS configuration.

## Option 1: Using as a Flake Input (Recommended)

### Step 1: Add to your flake inputs

In your `flake.nix`:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # Add nixos-builder-mon
    nixos-builder-mon = {
      url = "path:/Users/hassan/projects/nixos-builder-mon";
      # Or from GitHub once published:
      # url = "github:yourusername/nixos-builder-mon";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, nixos-builder-mon }: {
    nixosConfigurations.your-builder = nixpkgs.lib.nixosSystem {
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

### Step 2: Build your system

```bash
nix build .#nixosConfigurations.your-builder.config.system.build.toplevel
```

## Option 2: Direct Import in configuration.nix

If you're not using flakes, you can import the module directly:

```nix
{ config, pkgs, ... }:

let
  nixos-builder-mon = import /path/to/nixos-builder-mon/flake.nix;
in
{
  imports = [
    nixos-builder-mon.nixosModules.default
  ];

  services.nixos-builder-mon = {
    enable = true;
    port = 80;
    openFirewall = true;
  };
}
```

## Option 3: Manual Installation (for existing configuration)

If you want to manually integrate into an existing configuration without using the module:

```nix
{ pkgs, ... }:

let
  # Build the web assets
  nixos-builder-mon = pkgs.callPackage /path/to/nixos-builder-mon {};

  # Or reference the built server directly
  nom-server = pkgs.rustPlatform.buildRustPackage {
    pname = "nom-server";
    version = "1.0.0";
    src = /path/to/nixos-builder-mon;
    cargoLock = {
      lockFile = /path/to/nixos-builder-mon/Cargo.lock;
    };
  };
in
{
  # Install web assets
  environment.etc."nixos-builder-mon/index.html".source =
    /path/to/nixos-builder-mon/dist/index.html;
  environment.etc."nixos-builder-mon/assets".source =
    /path/to/nixos-builder-mon/dist/assets;

  # Monitoring script
  environment.etc."nixos-builder-mon/monitor-daemon.sh" = {
    text = ''
      #!/usr/bin/env bash
      OUTPUT_FILE="/var/log/nom-output.log"
      : > "$OUTPUT_FILE"
      ${pkgs.systemd}/bin/journalctl -u nix-daemon -n 100 --no-pager --no-hostname -o cat -f 2>&1 | tee "$OUTPUT_FILE"
    '';
    mode = "0755";
  };

  # Systemd services
  systemd.services.nixos-builder-mon-daemon = {
    description = "Monitor nix-daemon builds";
    wantedBy = [ "multi-user.target" ];
    after = [ "nix-daemon.service" ];
    requires = [ "nix-daemon.service" ];

    path = with pkgs; [ nix coreutils systemd ];

    serviceConfig = {
      Type = "simple";
      ExecStart = "/etc/nixos-builder-mon/monitor-daemon.sh";
      Restart = "always";
      RestartSec = "5";
    };
  };

  systemd.services.nixos-builder-mon-web = {
    description = "NixOS Build Monitor Web Interface";
    wantedBy = [ "multi-user.target" ];
    after = [ "network.target" ];

    path = with pkgs; [ iproute2 ];

    serviceConfig = {
      Type = "simple";
      ExecStart = "${nom-server}/bin/nom-server";
      Restart = "always";
      RestartSec = "5";
      User = "root";
    };
  };

  networking.firewall.allowedTCPPorts = [ 80 ];
}
```

## Verifying the Installation

After rebuilding your system, verify the services are running:

```bash
# Check service status
systemctl status nixos-builder-mon-web
systemctl status nixos-builder-mon-daemon

# Check logs
journalctl -u nixos-builder-mon-web -f

# Access the web interface
curl http://localhost
# Or in a browser: http://<your-ip>
```

## Configuration Options

See the [README.md](./README.md) for all available configuration options.
