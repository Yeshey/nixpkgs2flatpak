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

          def =
            if hasCurated
            then defs.${name}
            else {
              nixpkgsAttr     = info.attrPath;
              appId           = info.appId;
              runtime         = info.runtimeHint;
              permissions     = defaultPermissions;
              extraEnv        = {};
              extraLibs       =[];
              skipAbiChecks   = true;
              packageOverride = null;
              command         = null;
            };

          attrPathList = lib.splitString "." def.nixpkgsAttr;

          # FIX: Wrap the package fetch in tryEval to catch "throw" errors 
          # from removed or broken packages (like chatmcp)
          pkgAttempt = builtins.tryEval (
            if def.packageOverride != null
            then def.packageOverride
            else if lib.hasAttrByPath attrPathList pkgs
            then lib.getAttrFromPath attrPathList pkgs
            else null
          );

          pkg = if pkgAttempt.success then pkgAttempt.value else null;

          flatpakArgs =
            {
              inherit (def) appId runtime permissions skipAbiChecks;
              package = pkg;
            }
            // lib.optionalAttrs (def.extraEnv  != {}) { inherit (def) extraEnv;  }
            // lib.optionalAttrs (def.extraLibs !=[]) { inherit (def) extraLibs; }
            // lib.optionalAttrs (def.command   != null) { inherit (def) command; };

          attempt = builtins.tryEval (mkFlatpak flatpakArgs);
        in
          if pkg != null && attempt.success
          then { ok = true;  value = attempt.value; }
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