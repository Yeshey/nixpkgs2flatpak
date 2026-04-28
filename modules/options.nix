{ lib, ... }: {
  perSystem = { ... }: {
    options.flatpakDefs = lib.mkOption {
      default     = {};
      description = ''
        Per-package Flatpak definitions contributed by pkgs/by-name/<2l>/<name>/flake-parts.nix.
        Each entry overrides the auto-discovered defaults from discovered.json.
      '';
      type = lib.types.attrsOf (lib.types.submodule {
        options = {
          nixpkgsAttr = lib.mkOption {
            type        = lib.types.str;
            description = "Full nixpkgs attribute path, e.g. 'gnome-calculator' or 'kdePackages.kcalc'";
          };
          appId = lib.mkOption {
            type        = lib.types.str;
            description = "Flatpak application ID, e.g. 'org.gnome.Calculator'";
          };
          runtime = lib.mkOption {
            type    = lib.types.str;
            default = "org.freedesktop.Platform/24.08";
          };
          permissions = lib.mkOption {
            type    = lib.types.attrs;
            default = {};
          };
          extraEnv = lib.mkOption {
            type    = lib.types.attrs;
            default = {};
          };
          extraLibs = lib.mkOption {
            type    = lib.types.listOf lib.types.package;
            default = [];
          };
          skipAbiChecks = lib.mkOption {
            type    = lib.types.bool;
            default = false;
          };
          # When set, this derivation is passed to mkFlatpak instead of pkgs.${nixpkgsAttr}.
          # Use this for packages that need overlays, insecure permissions, overrides, etc.
          packageOverride = lib.mkOption {
            type    = lib.types.nullOr lib.types.raw;
            default = null;
          };
          # Override the launch command (default: meta.mainProgram or package name)
          command = lib.mkOption {
            type    = lib.types.nullOr lib.types.str;
            default = null;
          };
        };
      });
    };
  };
}