{ ... }: {
  perSystem = { ... }: {
    flatpakDefs."kcalc" = {
      nixpkgsAttr   = "kdePackages.kcalc";
      appId         = "org.kde.kcalc";
      runtime       = "org.kde.Platform/6.10";
      skipAbiChecks = true;
      permissions   = {
        share   = [ "ipc" ];
        sockets = [ "fallback-x11" "wayland" "pulseaudio" ];
        devices = [ "dri" ];
      };
    };
  };
}