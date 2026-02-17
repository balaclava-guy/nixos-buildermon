#!/usr/bin/env bash
set -euo pipefail

# Script to migrate the original project to use nixos-buildermon

ORIGINAL_PROJECT="/Users/hassan/projects/nixos-builder/lxc/pve-lxc-nixos"
NIXOS_BUILDER_MON="/Users/hassan/projects/nixos-buildermon"

echo "This script will update your original project to use nixos-buildermon"
echo "Original project: $ORIGINAL_PROJECT"
echo "nixos-buildermon: $NIXOS_BUILDER_MON"
echo ""
read -p "Continue? (y/n) " -n 1 -r
echo ""
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
  echo "Aborted."
  exit 1
fi

echo "Step 1: Updating flake.nix to add nixos-buildermon input..."

# Backup original flake
cp "$ORIGINAL_PROJECT/flake.nix" "$ORIGINAL_PROJECT/flake.nix.backup"

# Add nixos-buildermon input to flake.nix
# This would require more sophisticated text processing
echo "⚠️  Manual step required:"
echo "Add to your flake.nix inputs:"
echo ""
echo "  nixos-buildermon = {"
echo "    url = \"path:$NIXOS_BUILDER_MON\";"
echo "    inputs.nixpkgs.follows = \"nixpkgs\";"
echo "  };"
echo ""

echo "Step 2: Update configuration-x86-lxc.nix..."

# This would replace the manual mon-server and service definitions
# with the nixos-buildermon module
echo "⚠️  Manual step required:"
echo "Replace the nom-server and systemd service definitions in configuration-x86-lxc.nix"
echo "with the nixos-buildermon module import."
echo ""
echo "See INTEGRATION.md for examples."

echo ""
echo "✅ Migration guide complete!"
echo "Backup saved at: $ORIGINAL_PROJECT/flake.nix.backup"
