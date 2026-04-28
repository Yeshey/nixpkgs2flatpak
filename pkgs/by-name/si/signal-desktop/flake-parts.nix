{ ... }: {
  perSystem = { ... }: {
    flatpakDefs."signal-desktop" = {
      nixpkgsAttr   = "signal-desktop";
      appId         = "org.signal.Signal";
      runtime       = "org.gnome.Platform/49";
      command       = "signal-desktop";
      skipAbiChecks = true;
      extraEnv = {
        # The SUID sandbox does not work inside the Flatpak sandbox
        ELECTRON_DISABLE_SANDBOX = "1";
      };
      permissions = {
        share       = [ "network" "ipc" ];
        sockets     = [ "x11" "wayland" "pulseaudio" ];
        devices     = [ "all" ];
        filesystems = [ "xdg-download" ];
        talk-names  = [
          "org.freedesktop.Notifications"
          "org.freedesktop.secrets"
        ];
      };
    };
  };
}