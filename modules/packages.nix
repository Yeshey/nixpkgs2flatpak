{ inputs, lib, ... }:
let
  discoveredFile = ../discovered.json;
  discovered = if builtins.pathExists discoveredFile then builtins.fromJSON (builtins.readFile discoveredFile) else {};

  defaultPermissions = {
    share       = [ "network" "ipc" ];
    sockets     =[ "x11" "wayland" "pulseaudio" "session-bus" "system-bus" ];
    devices     =[ "all" ];
    filesystems = [ "host" ];
    talk-names  = [ "*" ];
  };

  # Instruct GUI frameworks to attempt Wayland first, then fallback to X11.
  defaultExtraEnv = {
    "QT_QPA_PLATFORM" = "wayland;xcb";
    "SDL_VIDEODRIVER" = "wayland,x11";
    "GDK_BACKEND"     = "wayland,x11";
  };
in
{
  perSystem = { pkgs, config, system, ... }:
    let
      mkFlatpak = inputs.nix2flatpak.lib.${system}.mkFlatpak;
      defs = config.flatpakDefs; 

      # NEW: The Icon Sanitizer logic
      fixIcon = pkg: appId: pkgs.runCommand "fixed-icon-${appId}.png" {
        nativeBuildInputs = [ pkgs.imagemagick ];
      } ''
        # 1. Try to find the icon by appId, then by any name
        # We look for PNGs first, then SVGs
        SRC=$(find ${pkg}/share/icons -name "${appId}.png" -o -name "${appId}.svg" | head -n 1)
        if [ -z "$SRC" ]; then
          SRC=$(find ${pkg}/share/icons -name "*.png" -o -name "*.svg" | head -n 1)
        fi

        if [ -n "$SRC" ]; then
          echo "Processing icon: $SRC"
          # convert: resize to fit in 512x512, center it, pad with transparency to make it square
          magick "$SRC" -resize 512x512 -background none -gravity center -extent 512x512 $out
        else
          echo "No icon found in package."
          exit 1
        fi
      '';

      safeGetPkg = attrPath: let
        pathList = lib.splitString "." attrPath;
        exists = lib.hasAttrByPath pathList pkgs;
        attempt = if exists then builtins.tryEval (lib.getAttrFromPath pathList pkgs) else { success = false; };
      in if attempt.success then attempt.value else null;

      makeEntry = name: info: let
        hasCurated = defs ? ${name};
        def = if hasCurated then defs.${name} else {
            nixpkgsAttr     = info.attrPath;
            appId           = info.appId;
            runtime         = if lib.hasInfix "kde" info.runtimeHint then "org.kde.Platform/6.10" else "org.gnome.Platform/49";
            permissions     = defaultPermissions;
            extraEnv        = defaultExtraEnv;
            extraLibs       =[]; skipAbiChecks = true; packageOverride = null; command = null;
        };

        pkg = if def.packageOverride != null then def.packageOverride else safeGetPkg def.nixpkgsAttr;

        flatpakArgs = {
          inherit (def) appId runtime permissions skipAbiChecks;
          package = pkg;
        } // lib.optionalAttrs (def.extraEnv != {}) { inherit (def) extraEnv; }
          // lib.optionalAttrs (def.extraLibs != []) { inherit (def) extraLibs; }
          // lib.optionalAttrs (def.command != null) { inherit (def) command; };

        # Logic for "Fixed" version
        fixedIconDerivation = if pkg != null then (builtins.tryEval (fixIcon pkg def.appId)) else { success = false; };
      in {
        inherit pkg;
        # Standard: nix2flatpak auto-detects
        standard = if pkg == null then null else mkFlatpak flatpakArgs;
        
        # Fixed: Force the ImageMagick-processed icon
        fixed    = if pkg == null || !fixedIconDerivation.success then null 
                   else mkFlatpak (flatpakArgs // { icon = fixedIconDerivation.value; });
      };

      allNames = lib.unique (builtins.attrNames discovered ++ builtins.attrNames defs);
      processed = lib.genAttrs allNames (name: makeEntry name (discovered.${name} or {}));
      
      finalPackages = lib.foldl' (acc: name: let
        entry = processed.${name};
      in acc // (lib.optionalAttrs (entry.standard != null) {
        "${name}" = entry.standard;
        # We rename the fallback to -fixed
        "${name}-fixed" = entry.fixed;
      })) {} allNames;

    in {
      packages = finalPackages;
    };
}