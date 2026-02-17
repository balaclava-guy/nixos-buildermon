{
  description = "NixOS Build Monitor - Web interface for monitoring nix-daemon builds";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};

      # Build the web assets
      web-assets = pkgs.stdenv.mkDerivation {
        name = "nixos-builder-mon-web";
        src = ./.;

        buildInputs = [ pkgs.nodejs pkgs.nodePackages.pnpm ];

        buildPhase = ''
          export HOME=$TMPDIR
          pnpm install --frozen-lockfile
          pnpm build
        '';

        installPhase = ''
          mkdir -p $out
          cp -r dist/* $out/
        '';
      };

      # Build the Rust server
      nom-server = pkgs.rustPlatform.buildRustPackage {
        pname = "nom-server";
        version = "1.0.0";

        src = ./.;

        cargoLock = {
          lockFile = ./Cargo.lock;
        };

        buildType = "release";

        meta = {
          description = "Minimal HTTP server for nixos-builder-mon web interface";
        };
      };

    in {
      packages.${system} = {
        default = nom-server;
        server = nom-server;
        web = web-assets;
      };

      # NixOS module for easy integration
      nixosModules.default = { config, lib, pkgs, ... }:
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
            environment.etc."nixos-builder-mon/index.html".source = "${web-assets}/index.html";
            environment.etc."nixos-builder-mon/assets".source = "${web-assets}/assets";

            # Monitoring script for nix-daemon
            environment.etc."nixos-builder-mon/monitor-daemon.sh" = {
              text = ''
                #!/usr/bin/env bash
                OUTPUT_FILE="/var/log/nom-output.log"
                : > "$OUTPUT_FILE"
                ${pkgs.systemd}/bin/journalctl -u nix-daemon -n 100 --no-pager --no-hostname -o cat -f 2>&1 | tee "$OUTPUT_FILE"
              '';
              mode = "0755";
            };

            # Systemd service to monitor nix-daemon
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
                Nice = 19;
                CPUSchedulingPolicy = "idle";
                IOSchedulingClass = "idle";
                MemoryHigh = "100M";
                MemoryMax = "200M";
              };
            };

            # Systemd service for web interface
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
                User = "root"; # Needed for port 80
                Nice = 19;
                CPUSchedulingPolicy = "idle";
                IOSchedulingClass = "idle";
                MemoryHigh = "50M";
                MemoryMax = "100M";
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
