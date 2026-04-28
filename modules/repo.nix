{ ... }: {
  perSystem = { pkgs, ... }: {
    packages.repo-builder = pkgs.writeShellApplication {
      name = "build-nixpkgs2flatpak-repo";
      runtimeInputs = with pkgs;[ flatpak ostree ];
      text = ''
        set -euo pipefail

        REPO_PATH="''${1:-./flatpak-repo}"
        BUNDLES_DIR="''${2:-./bundles}"

        echo "Initialising OSTree repo at $REPO_PATH …"
        mkdir -p "$REPO_PATH"
        ostree init --mode=archive-z2 --repo="$REPO_PATH"

        echo "Importing bundles from $BUNDLES_DIR …"
        find "$BUNDLES_DIR" -name '*.flatpak' -print0 | while IFS= read -r -d "" bundle; do
          echo "  → $bundle"
          flatpak build-import-bundle "$REPO_PATH" "$bundle" \
            ''${GPG_KEY_ID:+--gpg-sign="$GPG_KEY_ID"} \
            || echo "  WARNING: failed to import $bundle, skipping"
        done

        echo "Generating static deltas and updating summary …"
        flatpak build-update-repo "$REPO_PATH" \
          --generate-static-deltas \
          ''${GPG_KEY_ID:+--gpg-sign="$GPG_KEY_ID"}

        echo "Done. Users can add the repo with:"
        echo "  flatpak remote-add --user nixpkgs2flatpak https://YOUR_DOMAIN"
      '';
    };
  };
}