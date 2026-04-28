{ inputs, lib, ... }:
let
  discoveredFile = ../discovered.json;
  discovered = if builtins.pathExists discoveredFile then builtins.fromJSON (builtins.readFile discoveredFile) else {};

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
      defs = config.flatpakDefs; 

      # Robustly fetch a package, returning null if it 'throws' or doesn't exist
      safeGetPkg = attrPath: let
        pathList = lib.splitString "." attrPath;
        # First check if the attribute exists at all to avoid unnecessary tryEval
        exists = lib.hasAttrByPath pathList pkgs;
        # Then tryEval to catch 'throw'/removed aliases
        attempt = if exists then builtins.tryEval (lib.getAttrFromPath pathList pkgs) else { success = false; };
      in if attempt.success then attempt.value else null;

      makeEntry = name: info: let
        hasCurated = defs ? ${name};
        def = if hasCurated then defs.${name} else {
            nixpkgsAttr     = info.attrPath;
            appId           = info.appId;
            runtime         = if lib.hasInfix "kde" info.runtimeHint then "org.kde.Platform/6.10" else "org.gnome.Platform/49";
            permissions     = defaultPermissions;
            extraEnv = {}; extraLibs = []; skipAbiChecks = true; packageOverride = null; command = null;
        };

        pkg = if def.packageOverride != null then def.packageOverride else safeGetPkg def.nixpkgsAttr;

        flatpakArgs = {
          inherit (def) appId runtime permissions skipAbiChecks;
          package = pkg;
        } // lib.optionalAttrs (def.extraEnv != {}) { inherit (def) extraEnv; }
          // lib.optionalAttrs (def.extraLibs != []) { inherit (def) extraLibs; }
          // lib.optionalAttrs (def.command != null) { inherit (def) command; };
      in {
        inherit pkg;
        # Standard version
        standard = if pkg == null then null else mkFlatpak flatpakArgs;
        # Fallback version (No Icon) to bypass size/squareness errors
        noicon   = if pkg == null then null else mkFlatpak (flatpakArgs // { icon = null; });
      };

      allNames = lib.unique (builtins.attrNames discovered ++ builtins.attrNames defs);
      
      # Generate a massive attribute set of { "name" = ...; "name-noicon" = ...; }
      processed = lib.genAttrs allNames (name: makeEntry name (discovered.${name} or {}));
      
      finalPackages = lib.foldl' (acc: name: let
        entry = processed.${name};
      in acc // (lib.optionalAttrs (entry.standard != null) {
        "${name}" = entry.standard;
        "${name}-noicon" = entry.noicon;
      })) {} allNames;

    in {
      packages = finalPackages;
    };
}