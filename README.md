# NixOS Builder Monitor

A real-time web interface for monitoring NixOS build activity with system metrics.

## Features

- **Live Build Output**: Streams nix-daemon journal output in real-time
- **System Metrics**: CPU, memory, disk, network monitoring with sparkline graphs
- **Lightweight**: Minimal resource usage with Rust backend
- **Dark/Light Mode**: Toggle between themes
- **Collapsible Widgets**: Expand/collapse system information
- **Zero External Dependencies**: All assets bundled locally

## Quick Start

### Option 1: Use the NixOS Module (Recommended)

Add to your `flake.nix`:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nixos-builder-mon = {
      url = "github:yourusername/nixos-builder-mon";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, nixos-builder-mon }: {
    nixosConfigurations.your-host = nixpkgs.lib.nixosSystem {
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

### Option 2: Manual Import in configuration.nix

If using a local path:

```nix
{ pkgs, ... }:
{
  imports = [
    /path/to/nixos-builder-mon/flake.nix
  ];

  services.nixos-builder-mon = {
    enable = true;
    port = 80;
    openFirewall = true;
  };
}
```

## Development

### Prerequisites

- Node.js 18+ and pnpm
- Rust 1.70+

### Build

```bash
# Install dependencies
pnpm install

# Build web assets
pnpm build

# Build Rust server
cargo build --release
```

### Test Locally

```bash
# Build
pnpm build

# Run server
DEMO_MODE=true cargo run

# Open http://localhost:8080
```

## Architecture

```
nixos-builder-mon/
├── src/
│   └── index.html          # Main web interface
├── server-src/
│   └── main.rs             # Rust HTTP server
├── public/
│   └── logo.png            # Assets
├── dist/                   # Build output
│   ├── index.html
│   └── assets/
│       ├── xterm.js
│       ├── xterm.css
│       └── logo.png
├── flake.nix              # Nix flake with NixOS module
├── package.json           # Node dependencies
└── Cargo.toml            # Rust dependencies
```

## Configuration Options

### `services.nixos-builder-mon.enable`
- **Type**: boolean
- **Default**: `false`
- **Description**: Enable the NixOS build monitor service

### `services.nixos-builder-mon.port`
- **Type**: port (integer)
- **Default**: `80`
- **Description**: Port for the web interface

### `services.nixos-builder-mon.openFirewall`
- **Type**: boolean
- **Default**: `true`
- **Description**: Automatically open firewall port

## Credits

- Build output monitoring powered by [nix-output-monitor](https://github.com/maralorn/nix-output-monitor)
- Terminal emulation via [xterm.js](https://github.com/xtermjs/xterm.js)
- System metrics via [sysinfo](https://github.com/GuillaumeGomez/sysinfo)

## License

MIT
