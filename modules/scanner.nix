{ ... }: {
  perSystem = { pkgs, ... }:
    let
      scanner = pkgs.rustPlatform.buildRustPackage {
        pname   = "nixpkgs2flatpak-scanner";
        version = "0.1.0";
        src     = pkgs.lib.cleanSource ../.;

        cargoLock.lockFile = ../Cargo.lock;

        # Add makeWrapper so we can inject dependencies
        nativeBuildInputs = [ pkgs.makeWrapper ];
        
        # Inject nix-index, rclone, flatpak, and ostree directly into the Rust binary's PATH!
        postInstall = ''
          wrapProgram $out/bin/scanner \
            --prefix PATH : ${pkgs.lib.makeBinPath[ pkgs.nix-index pkgs.rclone pkgs.flatpak pkgs.ostree ]}
        '';

        meta.description = "Discover nixpkgs packages with .desktop files";
      };
    in {
      packages.scanner = scanner;

      apps.scanner = {
        type    = "app";
        program = "${scanner}/bin/scanner";
      };
    };
}