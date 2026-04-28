# NeoChat needs insecure olm allowed, and benefits from dropping QtWebEngine
{ inputs, ... }: {
  perSystem = { system, ... }:
    let
      # Import nixpkgs with the insecure olm package allowed and opencv
      # stripped from kquickimageeditor (saves ~44 MB, same as the official Flatpak)
      pkgsForNeochat = import inputs.nixpkgs {
        inherit system;
        config.permittedInsecurePackages = [ "olm-3.2.16" ];
        overlays = [
          (_final: prev: {
            kdePackages = prev.kdePackages.overrideScope (_kfinal: kprev: {
              kquickimageeditor = kprev.kquickimageeditor.overrideAttrs (old: {
                buildInputs = builtins.filter
                  (dep: !(dep ? pname && dep.pname == "opencv"))
                  old.buildInputs;
              });
            });
          })
        ];
      };
    in {
      flatpakDefs."neochat" = {
        nixpkgsAttr = "kdePackages.neochat";
        appId       = "org.kde.neochat";
        runtime     = "org.kde.Platform/6.10";
        # Dropping QtWebView saves ~375 MB (no Chromium), same as the official Flatpak
        packageOverride = pkgsForNeochat.kdePackages.neochat.override {
          qtwebview = null;
        };
        skipAbiChecks = true;
        permissions = {
          share       = [ "network" "ipc" ];
          sockets     = [ "fallback-x11" "wayland" "pulseaudio" ];
          devices     = [ "dri" ];
          filesystems = [ "xdg-download" ];
        };
      };
    };
}