{ ... }: {
  perSystem = { pkgs, ... }:
    let
      scanner = pkgs.rustPlatform.buildRustPackage {
        pname   = "nixpkgs2flatpak-scanner";
        version = "0.1.0";
        src     = pkgs.lib.cleanSource ../.;

        cargoLock.lockFile = ../Cargo.lock;

        # nix-locate must be available at runtime for `scanner discover`
        nativeBuildInputs = [ pkgs.makeWrapper ];
        postInstall = ''
          wrapProgram $out/bin/scanner \
            --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.nix-index ]}
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