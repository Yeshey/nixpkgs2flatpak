{ inputs, lib, ... }:
let
  discoveredFile = ../discovered.json;

  discovered =
    if builtins.pathExists discoveredFile
    then builtins.fromJSON (builtins.readFile discoveredFile)
    else {};

  defaultPermissions = {
    share       =[ "network" "ipc" ];
    sockets     =[ "x11" "wayland" "fallback-x11" "pulseaudio" "session-bus" "system-bus" ];
    devices     =[ "all" ];
    filesystems = [ "host" ];
    talk-names  =[ "*" ];
  };
in
{
  perSystem = { pkgs, config, system, ... }:
    let
      mkFlatpak = inputs.nix2flatpak.lib.${system}.mkFlatpak;
      defs      = config.flatpakDefs; 

      allNames = lib.unique (builtins.attrNames discovered ++ builtins.attrNames defs);

      mkEntry = name: info:
        let
          hasCurated = defs ? ${name};
          def = if hasCurated then defs.${name} else {
              nixpkgsAttr     = info.attrPath;
              appId           = info.appId;
              runtime         = if lib.hasInfix "kde" info.runtimeHint then "org.kde.Platform/6.10" else "org.gnome.Platform/49";
              permissions     = defaultPermissions;
              extraEnv        = {};
              extraLibs       =[];
              skipAbiChecks   = true;
              packageOverride = null;
              command         = null;
          };

          attrPathList = lib.splitString "." def.nixpkgsAttr;
          pkgAttempt = builtins.tryEval (
            if def.packageOverride != null then def.packageOverride
            else if lib.hasAttrByPath attrPathList pkgs then lib.getAttrFromPath attrPathList pkgs
            else null
          );
          pkg = if pkgAttempt.success then pkgAttempt.value else null;

          # Basic arguments
          baseArgs = {
            inherit (def) appId runtime permissions skipAbiChecks;
            package = pkg;
          } // lib.optionalAttrs (def.extraEnv != {}) { inherit (def) extraEnv; }
            // lib.optionalAttrs (def.extraLibs != []) { inherit (def) extraLibs; }
            // lib.optionalAttrs (def.command != null) { inherit (def) command; };

          # Strategy: Try building with icon detection. 
          # If that fails (likely non-square icon), try again with icon forced to null.
          attemptNormal = builtins.tryEval (mkFlatpak baseArgs);
          attemptNoIcon = builtins.tryEval (mkFlatpak (baseArgs // { icon = null; }));
        in
          if pkg == null then { ok = false; value = null; }
          else if attemptNormal.success then { ok = true; value = attemptNormal.value; }
          else if attemptNoIcon.success then { ok = true; value = attemptNoIcon.value; }
          else { ok = false; value = null; };

      mkAttempt = name:
        let 
          info = discovered.${name} or {
             attrPath = defs.${name}.nixpkgsAttr;
             appId = defs.${name}.appId;
             runtimeHint = defs.${name}.runtime or "org.gnome.Platform/49";
          };
        in mkEntry name info;

      allAttempts = lib.genAttrs allNames mkAttempt;
      successfulOnly = lib.filterAttrs (_: e: e.ok) allAttempts;
    in {
      packages = lib.mapAttrs (_: e: e.value) successfulOnly;
    };
}