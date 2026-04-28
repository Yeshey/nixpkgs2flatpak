{ ... }: {
  perSystem = { pkgs, ... }: {
    devShells.default = pkgs.mkShell {
      name = "nixpkgs2flatpak";
      packages = with pkgs; [
        # Rust
        rustc cargo clippy rust-analyzer

        # nix-index — provides nix-locate, used by `scanner discover`
        nix-index

        # Flatpak / OSTree tooling
        flatpak ostree

        # Handy utilities
        jq git
      ];

      shellHook = ''
        echo "nixpkgs2flatpak dev shell"
        echo ""
        echo "First-time setup:"
        echo "  nix-index          # build the nix-index database (~15 min, once)"
        echo ""
        echo "Workflow:"
        echo "  cargo run -- discover   # regenerate discovered.json"
        echo "  cargo run -- stats      # summarise discovered.json"
        echo "  nix build .#<name>      # build one Flatpak bundle"
      '';
    };
  };
}