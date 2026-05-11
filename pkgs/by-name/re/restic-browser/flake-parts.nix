{ ... }: {
  perSystem = { pkgs, ... }: {
    flatpakDefs."restic-browser" = {
      nixpkgsAttr = "restic-browser";
      packageOverride = pkgs.restic-browser.overrideAttrs (old: {
        nativeBuildInputs = (old.nativeBuildInputs or []) ++ [ pkgs.makeWrapper ];
        postInstall = (old.postInstall or "") + ''
          wrapProgram $out/bin/Restic-Browser \
            --prefix PATH : ${pkgs.restic}/bin \
            --prefix PATH : ${pkgs.rclone}/bin
        '';
      });
      appId   = "org.nixpkgs.resticbrowser";
      runtime = "org.gnome.Platform/49";
      command = "Restic-Browser";
      skipAbiChecks = false;
      permissions = {
        share       = [ "ipc" "network" ];
        sockets     = [ "fallback-x11" "wayland" ];
        devices     = [ "dri" ];
        filesystems = [
          "xdg-config/rclone"
          "home"
        ];
      };
    };
  };
}