{
  description = "NixOS Build Monitor - Web interface for monitoring nix-daemon builds";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsFor = system: nixpkgs.legacyPackages.${system};

      makePackages = pkgs: rec {
        # Build the Dioxus Fullstack application
        nixos-builder-mon-server = pkgs.rustPlatform.buildRustPackage {
        pname = "nixos-builder-mon";
        version = "1.0.0";

        src = ./.;

        cargoLock = {
          lockFile = ./Cargo.lock;
        };

        buildInputs = pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.libiconv
        ];

        buildFeatures = [ "server" ];

        meta = {
          description = "NixOS Build Monitor - Fullstack Dioxus application";
        };
      };

      # Build WASM assets for the fullstack app
      web-assets = pkgs.stdenv.mkDerivation {
        name = "nixos-builder-mon-assets";
        src = ./.;

        nativeBuildInputs = with pkgs; [
          rustc
          cargo
          wasm-bindgen-cli
        ];

        buildPhase = ''
          export CARGO_HOME=$PWD/.cargo
          export CARGO_TARGET_DIR=$PWD/target

          # Build WASM
          cargo build --release --target wasm32-unknown-unknown --features web

          # Run wasm-bindgen
          mkdir -p wasm
          wasm-bindgen --target web --out-dir wasm \
            target/wasm32-unknown-unknown/release/nixos-builder-mon.wasm
        '';

        installPhase = ''
          mkdir -p $out
          cp -r assets/* $out/
          cp -r wasm $out/wasm
        '';

          meta = {
            description = "NixOS Build Monitor WASM assets";
          };
        };
      };

    in {
      packages = forAllSystems (system:
        let
          pkgs = pkgsFor system;
          packages = makePackages pkgs;
        in {
          default = packages.nixos-builder-mon-server;
          server = packages.nixos-builder-mon-server;
          web = packages.web-assets;
        }
      );

      # Development shell with dioxus-cli
      devShells = forAllSystems (system:
        let
          pkgs = pkgsFor system;
        in {
          default = pkgs.mkShell {
            buildInputs = with pkgs; [
              rustc
              cargo
              dioxus-cli
              llvmPackages.lld
            ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
              libiconv
            ];

            shellHook = pkgs.lib.optionalString pkgs.stdenv.isDarwin ''
              export LIBRARY_PATH="${pkgs.libiconv}/lib:''${LIBRARY_PATH:-}"
              export CPATH="${pkgs.libiconv}/include:''${CPATH:-}"
              export RUSTFLAGS="-L native=${pkgs.libiconv}/lib ''${RUSTFLAGS:-}"
            '';
          };
        }
      );

      # NixOS module for easy integration (Linux only)
      nixosModules.default = { config, lib, pkgs, ... }:
        let
          packages = makePackages pkgs;
        in
        with lib;
        let
          cfg = config.services.nixos-builder-mon;
        in {
          options.services.nixos-builder-mon = {
            enable = mkEnableOption "NixOS Build Monitor";

            port = mkOption {
              type = types.port;
              default = 80;
              description = "Port for the web interface";
            };

            openFirewall = mkOption {
              type = types.bool;
              default = true;
              description = "Open firewall port for the web interface";
            };
          };

          config = mkIf cfg.enable {
            # Install web assets
            environment.etc."nixos-builder-mon/index.html".source = "${packages.web-assets}/index.html";
            environment.etc."nixos-builder-mon/assets".source = "${packages.web-assets}/assets";

            # Monitoring script for nix-daemon
            environment.etc."nixos-builder-mon/monitor-daemon.sh" = {
              text = ''
                #!/usr/bin/env bash
                set -euo pipefail
                OUTPUT_FILE="/var/log/nom-output.log"
                touch "$OUTPUT_FILE"

                # Human-log fallback path: daemon output -> nom -> monitor log
                # For user-invoked builds, prefer: nom build ...
                exec ${pkgs.expect}/bin/unbuffer ${pkgs.systemd}/bin/journalctl \
                  -u nix-daemon -n 0 --no-pager --no-hostname -o cat -f \
                  2>&1 | ${pkgs.nix-output-monitor}/bin/nom | ${pkgs.coreutils}/bin/tee -a "$OUTPUT_FILE"
              '';
              mode = "0755";
            };

            environment.systemPackages = with pkgs; [
              nix-output-monitor
            ];

            fonts.packages = with pkgs; [
              nerd-fonts.jetbrains-mono
              nerd-fonts.iosevka
            ];

            # Systemd service to monitor nix-daemon
            systemd.services.nixos-builder-mon-daemon = {
              description = "Monitor nix-daemon builds";
              wantedBy = [ "multi-user.target" ];
              after = [ "nix-daemon.service" ];
              requires = [ "nix-daemon.service" ];

              path = with pkgs; [ nix coreutils systemd expect nix-output-monitor ];

              serviceConfig = {
                Type = "simple";
                ExecStart = "/etc/nixos-builder-mon/monitor-daemon.sh";
                Restart = "always";
                RestartSec = "5";
                Nice = 19;
                CPUSchedulingPolicy = "idle";
                IOSchedulingClass = "idle";
                MemoryHigh = "100M";
                MemoryMax = "200M";
              };
            };

            # Systemd service for fullstack application
            systemd.services.nixos-builder-mon = {
              description = "NixOS Build Monitor (Dioxus Fullstack)";
              wantedBy = [ "multi-user.target" ];
              after = [ "network.target" ];

              path = with pkgs; [ iproute2 ];

              environment = {
                DIOXUS_ASSET_ROOT = "${packages.web-assets}";
                IP = "0.0.0.0";
                PORT = toString cfg.port;
              };

              serviceConfig = {
                Type = "simple";
                ExecStart = "${packages.nixos-builder-mon-server}/bin/nixos-builder-mon";
                Restart = "always";
                RestartSec = "5";
                User = "root"; # Needed for port 80
                Nice = 19;
                CPUSchedulingPolicy = "idle";
                IOSchedulingClass = "idle";
                MemoryHigh = "100M";
                MemoryMax = "200M";
              };
            };

            # Setup activation script
            system.activationScripts.nixos-builder-mon-setup = ''
              touch /var/log/nom-output.log
              chmod 644 /var/log/nom-output.log
            '';

            # Open firewall if requested
            networking.firewall.allowedTCPPorts = mkIf cfg.openFirewall [ cfg.port ];
          };
        };
    };
}
