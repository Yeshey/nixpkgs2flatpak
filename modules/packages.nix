{ inputs, lib, ... }:
let
  discoveredFile = ../discovered.json;

  # Read at evaluation time; returns {} if the file doesn't exist yet
  discovered =
    if builtins.pathExists discoveredFile
    then builtins.fromJSON (builtins.readFile discoveredFile)
    else {};

  # Applied when no flake-parts.nix exists for a package.
  # All permissions = the app is guaranteed to at least launch.
  # Contributors progressively tighten these by adding a flake-parts.nix.
  defaultPermissions = {
    share       = [ "network" "ipc" ];
    sockets     = [ "x11" "wayland" "fallback-x11" "pulseaudio" "session-bus" "system-bus" ];
    devices     = [ "all" ];
    filesystems = [ "host" ];
    talk-names  = [ "*" ];
  };
in
{
  perSystem = { pkgs, config, system, ... }:
    let
      mkFlatpak = inputs.nix2flatpak.lib.${system}.mkFlatpak;
      defs      = config.flatpakDefs;  # merged from all flake-parts.nix files

      # Resolve a dot-separated attribute path against pkgs:
      # "kdePackages.kcalc" → pkgs.kdePackages.kcalc
      getPkg = attrPath:
        lib.getAttrFromPath (lib.splitString "." attrPath) pkgs;

      mkEntry = name: info:
        let
          hasCurated = defs ? ${name};

          # For uncurated packages, synthesise a permissive def from discovered.json
          def =
            if hasCurated
            then defs.${name}
            else {
              nixpkgsAttr     = info.attrPath;
              appId           = info.appId;
              runtime         = info.runtimeHint;
              permissions     = defaultPermissions;
              extraEnv        = {};
              extraLibs       = [];
              skipAbiChecks   = true;
              packageOverride = null;
              command         = null;
            };

          pkg =
            if def.packageOverride != null
            then def.packageOverride
            else getPkg def.nixpkgsAttr;

          # Build the attrset that mkFlatpak accepts, only including optional
          # keys when they carry a non-default value (mkFlatpak is strict about unknowns)
          flatpakArgs =
            {
              inherit (def) appId runtime permissions skipAbiChecks;
              package = pkg;
            }
            // lib.optionalAttrs (def.extraEnv  != {}) { inherit (def) extraEnv;  }
            // lib.optionalAttrs (def.extraLibs != []) { inherit (def) extraLibs; }
            // lib.optionalAttrs (def.command   != null) { inherit (def) command; };

          attempt = builtins.tryEval (mkFlatpak flatpakArgs);
        in
          if attempt.success
          then { ok = true;  value = attempt.value; }
          else { ok = false; value = null; };

      allNames = lib.unique (builtins.attrNames discovered ++ builtins.attrNames defs);

      mkAttempt = name:
        let 
          info = discovered.${name} or {
             attrPath = defs.${name}.nixpkgsAttr;
             appId = defs.${name}.appId;
             runtimeHint = defs.${name}.runtime or "org.freedesktop.Platform/24.08";
          };
        in mkEntry name info;

      # Create an attempt for every combined package
      allAttempts = lib.genAttrs allNames mkAttempt;
      successfulOnly = lib.filterAttrs (_: e: e.ok) allAttempts;
    in {
      packages = lib.mapAttrs (_: e: e.value) successfulOnly;
    };
}