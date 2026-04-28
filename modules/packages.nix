{ inputs, lib, ... }:
let
  discoveredFile = ../discovered.json;
  discovered = if builtins.pathExists discoveredFile then builtins.fromJSON (builtins.readFile discoveredFile) else {};
in
{
  perSystem = { pkgs, config, system, ... }:
    let
      mkFlatpak = inputs.nix2flatpak.lib.${system}.mkFlatpak;
      defs = config.flatpakDefs; 

      mkEntry = name: info:
        let
          hasCurated = defs ? ${name};
          def = if hasCurated then defs.${name} else {
              nixpkgsAttr     = info.attrPath;
              appId           = info.appId;
              runtime         = if lib.hasInfix "kde" info.runtimeHint then "org.kde.Platform/6.10" else "org.gnome.Platform/49";
              permissions     = {
                share       = [ "network" "ipc" ];
                sockets     = [ "x11" "wayland" "fallback-x11" "pulseaudio" "session-bus" "system-bus" ];
                devices     = [ "all" ];
                filesystems = [ "host" ];
                talk-names  = [ "*" ];
              };
              extraEnv = {}; extraLibs = []; skipAbiChecks = true; packageOverride = null; command = null;
          };

          attrPathList = lib.splitString "." def.nixpkgsAttr;
          
          # Attempt to get the package, catching 'throw' errors (removed packages)
          pkgAttempt = builtins.tryEval (
            if def.packageOverride != null then def.packageOverride
            else if lib.hasAttrByPath attrPathList pkgs then lib.getAttrFromPath attrPathList pkgs
            else null
          );
          
          pkg = if pkgAttempt.success then pkgAttempt.value else null;

          # Heuristic: If name starts with _, it's often a problematic alias or large icon app.
          # We'll default icons to null for these to prevent the "1024px" crash.
          forceNoIcon = lib.hasPrefix "_" name;

          flatpakArgs = {
            inherit (def) appId runtime permissions skipAbiChecks;
            package = pkg;
            icon = if forceNoIcon then null else (def.icon or (info.icon or null));
          } // lib.optionalAttrs (def.extraEnv != {}) { inherit (def) extraEnv; }
            // lib.optionalAttrs (def.extraLibs != []) { inherit (def) extraLibs; }
            // lib.optionalAttrs (def.command != null) { inherit (def) command; };

          # We can't catch build failures, only eval failures.
          attempt = builtins.tryEval (mkFlatpak flatpakArgs);
        in
          if pkg != null && attempt.success
          then { ok = true; value = attempt.value; }
          else { ok = false; value = null; };

      allNames = lib.unique (builtins.attrNames discovered ++ builtins.attrNames defs);
      allAttempts = lib.genAttrs allNames (name: mkEntry name (discovered.${name} or {}));
      successfulOnly = lib.filterAttrs (_: e: e.ok) allAttempts;
    in {
      packages = lib.mapAttrs (_: e: e.value) successfulOnly;
    };
}