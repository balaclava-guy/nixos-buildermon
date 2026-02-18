{
  description = "NixOS Buildermon - Web interface for monitoring nix-daemon builds";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    devshell.url = "github:numtide/devshell";
    devshell.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = inputs@{ self, nixpkgs, flake-parts, devshell, ... }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];

      makePackages = pkgs: rec {
        nixos-buildermon-server = pkgs.rustPlatform.buildRustPackage {
          pname = "nixos-buildermon";
          version = "0.1.0";

          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          buildInputs = pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.libiconv
          ];

          buildFeatures = [ "server" ];

          meta = {
            description = "NixOS Buildermon - Fullstack Dioxus application";
          };
        };

        web-assets = pkgs.stdenv.mkDerivation {
          name = "nixos-buildermon-assets";
          src = ./.;

          cargoDeps = pkgs.rustPlatform.importCargoLock {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = with pkgs; [
            rustc
            cargo
            wasm-bindgen-cli
            rustPlatform.cargoSetupHook
            llvmPackages.lld
          ];

          buildPhase = ''
            export CARGO_TARGET_DIR=$PWD/target

            cargo build --release --target wasm32-unknown-unknown --no-default-features --features web

            mkdir -p wasm
            wasm-bindgen --target web --out-dir wasm \
              target/wasm32-unknown-unknown/release/nixos-buildermon.wasm
          '';

          installPhase = ''
            mkdir -p $out
            cp -r assets/* $out/
            cp -r wasm $out/wasm
          '';

          meta = {
            description = "NixOS Buildermon WASM assets";
          };
        };
      };
    in
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [ devshell.flakeModule ];
      inherit systems;

      perSystem = { pkgs, lib, ... }:
        let
          packages = makePackages pkgs;

          dx-build = pkgs.writeShellApplication {
            name = "dx-build";
            runtimeInputs = with pkgs; [
              dioxus-cli
              binaryen
              rustc
              cargo
              wasm-bindgen-cli
            ] ++ lib.optionals pkgs.stdenv.isDarwin [
              libiconv
            ] ++ lib.optionals pkgs.stdenv.isLinux [
              gcc
              llvmPackages.lld
            ];
            text = ''
              ${lib.optionalString pkgs.stdenv.isDarwin ''
                export LIBRARY_PATH="${pkgs.libiconv}/lib:''${LIBRARY_PATH:-}"
                export CPATH="${pkgs.libiconv}/include:''${CPATH:-}"
                export RUSTFLAGS="-L native=${pkgs.libiconv}/lib ''${RUSTFLAGS:-}"
              ''}
              exec dx build --platform web --release --fullstack true --features web "$@"
            '';
          };

          dx-serve = pkgs.writeShellApplication {
            name = "dx-serve";
            runtimeInputs = with pkgs; [
              dioxus-cli
              binaryen
              rustc
              cargo
              wasm-bindgen-cli
            ] ++ lib.optionals pkgs.stdenv.isDarwin [
              libiconv
            ] ++ lib.optionals pkgs.stdenv.isLinux [
              gcc
              llvmPackages.lld
            ];
            text = ''
              ${lib.optionalString pkgs.stdenv.isDarwin ''
                export LIBRARY_PATH="${pkgs.libiconv}/lib:''${LIBRARY_PATH:-}"
                export CPATH="${pkgs.libiconv}/include:''${CPATH:-}"
                export RUSTFLAGS="-L native=${pkgs.libiconv}/lib ''${RUSTFLAGS:-}"
              ''}
              exec dx serve --platform web --fullstack true --features web "$@"
            '';
          };
        in {
          packages = {
            default = packages.nixos-buildermon-server;
            server = packages.nixos-buildermon-server;
            web = packages.web-assets;
            inherit dx-build dx-serve;
          };

          apps = {
            dx-build = {
              type = "app";
              program = "${dx-build}/bin/dx-build";
            };
            dx-serve = {
              type = "app";
              program = "${dx-serve}/bin/dx-serve";
            };
          };

          devshells.default = {
            packages = with pkgs; [
              rustc
              cargo
              dioxus-cli
              wasm-bindgen-cli
              binaryen
              llvmPackages.lld
            ] ++ lib.optionals pkgs.stdenv.isDarwin [
              libiconv
            ];

            env = lib.optionals pkgs.stdenv.isDarwin [
              {
                name = "LIBRARY_PATH";
                value = "${pkgs.libiconv}/lib";
              }
              {
                name = "CPATH";
                value = "${pkgs.libiconv}/include";
              }
              {
                name = "RUSTFLAGS";
                value = "-L native=${pkgs.libiconv}/lib";
              }
            ];

            commands = [
              {
                name = "dx-build";
                command = "dx build --platform web --release --fullstack true --features web";
                help = "Build fullstack app with Dioxus";
              }
              {
                name = "dx-serve";
                command = "dx serve --platform web --fullstack true --features web";
                help = "Run fullstack app with Dioxus dev server";
              }
              {
                name = "check-server";
                command = "cargo check --features server";
                help = "Check server target";
              }
              {
                name = "check-web";
                command = "cargo check --no-default-features --features web --target wasm32-unknown-unknown";
                help = "Check web target";
              }
            ];
          };
        };

      flake = {
        nixosModules.default = { config, lib, pkgs, ... }:
          let
            packages = makePackages pkgs;
          in
          with lib;
          let
            cfg = config.services.nixos-buildermon;
          in {
            options.services.nixos-buildermon = {
              enable = mkEnableOption "NixOS Buildermon";

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
              environment.etc."nixos-buildermon/index.html".source = "${packages.web-assets}/index.html";
              environment.etc."nixos-buildermon/assets".source = "${packages.web-assets}/assets";

              environment.etc."nixos-buildermon/monitor-daemon.sh" = {
                text = ''
                  #!/usr/bin/env bash
                  set -euo pipefail
                  OUTPUT_FILE="/var/log/nom-output.log"
                  touch "$OUTPUT_FILE"

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

              systemd.services.nixos-buildermon-daemon = {
                description = "Monitor nix-daemon builds";
                wantedBy = [ "multi-user.target" ];
                after = [ "nix-daemon.service" ];
                requires = [ "nix-daemon.service" ];

                path = with pkgs; [ nix coreutils systemd expect nix-output-monitor ];

                serviceConfig = {
                  Type = "simple";
                  ExecStart = "/etc/nixos-buildermon/monitor-daemon.sh";
                  Restart = "always";
                  RestartSec = "5";
                  Nice = 19;
                  CPUSchedulingPolicy = "idle";
                  IOSchedulingClass = "idle";
                  MemoryHigh = "100M";
                  MemoryMax = "200M";
                };
              };

              systemd.services.nixos-buildermon = {
                description = "NixOS Buildermon (Dioxus Fullstack)";
                wantedBy = [ "multi-user.target" ];
                after = [ "network.target" ];

                path = with pkgs; [ iproute2 ];

                environment = {
                  DIOXUS_ASSET_ROOT = "${packages.web-assets}";
                  DIOXUS_PUBLIC_PATH = "${packages.web-assets}";
                  IP = "0.0.0.0";
                  PORT = toString cfg.port;
                };

                serviceConfig = {
                  Type = "simple";
                  ExecStart = "${packages.nixos-buildermon-server}/bin/nixos-buildermon";
                  Restart = "always";
                  RestartSec = "5";
                  User = "root";
                  Nice = 19;
                  CPUSchedulingPolicy = "idle";
                  IOSchedulingClass = "idle";
                  MemoryHigh = "100M";
                  MemoryMax = "200M";
                };
              };

              system.activationScripts.nixos-buildermon-setup = ''
                touch /var/log/nom-output.log
                chmod 644 /var/log/nom-output.log
              '';

              networking.firewall.allowedTCPPorts = mkIf cfg.openFirewall [ cfg.port ];
            };
          };
      };
    };
}
