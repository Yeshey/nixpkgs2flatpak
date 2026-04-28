{ ... }: {
  perSystem = { ... }: {
    flatpakDefs."gnome-calculator" = {
      nixpkgsAttr   = "gnome-calculator";
      appId         = "org.gnome.Calculator";
      runtime       = "org.gnome.Platform/49";
      skipAbiChecks = false;
      permissions   = {
        share   = [ "ipc" ];
        sockets = [ "fallback-x11" "wayland" ];
        devices = [ "dri" ];
      };
    };
  };
}