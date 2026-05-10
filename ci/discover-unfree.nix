# ci/discover-unfree.nix
#
# Returns a JSON list of { pname, stem } for every package in nixpkgs whose
# `desktopItems` attribute is a non-empty list, evaluated with allowUnfree=true.
#
# "stem" is the desktop file name without the .desktop suffix, derived from
# the first desktopItems derivation's name (makeDesktopItem convention).
#
# Run with:
#   nix eval --json --impure -f ci/discover-unfree.nix
#
# This covers packages that use makeDesktopItem (the standard nixpkgs helper),
# which is the pattern used by most unfree apps: Slack, Discord, Spotify, Steam,
# etc. all wrap a downloaded binary and add a desktop item via this helper.
#
# Packages that install .desktop files from their source tarball without using
# makeDesktopItem are handled by the grep pass in src/discover_unfree.rs.
let
  flake   = builtins.getFlake (toString ./..);
  nixpkgs = flake.inputs.nixpkgs;

  pkgs = import nixpkgs {
    system = builtins.currentSystem;
    config = {
      allowUnfree            = true;
      allowBroken            = true;
      allowUnsupportedSystem = true;
    };
  };

  # Helper: evaluate expr, return its value or null on any failure.
  # builtins.tryEval catches `throw` (the common failure mode in nixpkgs),
  # so this is safe to apply to arbitrary package attribute accesses.
  try = expr:
    let r = builtins.tryEval expr;
    in if r.success then r.value else null;

  # Given the attribute name and its value from the top-level pkgs set,
  # return { pname, stem } if the package has desktopItems, else null.
  inspect = attrName: val:
    # Guard 1: must be a plain attrset (not a function, string, int, etc.)
    let isSet = try (builtins.isAttrs val);
    in if isSet != true then null
    else
      # Guard 2: must expose a non-empty desktopItems list.
      # The `?` operator does NOT evaluate the attribute value; it only checks
      # presence, so this is safe even for attributes that would throw on access.
      let hasItems = try (
            val ? desktopItems &&
            builtins.isList val.desktopItems &&
            val.desktopItems != []
          );
      in if hasItems != true then null
      else
        # Extract the desktop file stem from the first desktopItems entry.
        # makeDesktopItem sets drv.name = the stem, so the desktop file ends up
        # at share/applications/${name}.desktop. Strip common packaging suffixes
        # that nixpkgs occasionally adds ("-desktop-item", "-desktopitem").
        let
          firstItem = try (builtins.head val.desktopItems);
          firstName =
            if firstItem == null then null
            else if builtins.isAttrs firstItem then try firstItem.name
            else
              # It's a string path like "../apvlv.desktop" — extract the stem from it
              let base = builtins.baseNameOf firstItem;
              in try (builtins.head (builtins.match "^(.+)\\.desktop$" base));

          stem =
            if firstName == null then attrName   # fallback: use the attr name
            else
              let
                m1 = builtins.match "^(.+)-desktop-item$" firstName;
                m2 = builtins.match "^(.+)-desktopitem$"  firstName;
              in
                if      m1 != null then builtins.head m1
                else if m2 != null then builtins.head m2
                else firstName;
        in
        { pname = attrName; inherit stem; };

  # Map over every top-level nixpkgs attribute, wrapping each call in a
  # tryEval so that a single broken package can never abort the whole eval.
  results = builtins.concatMap
    (attrName:
      let r = builtins.tryEval (inspect attrName pkgs.${attrName});
      in if r.success && r.value != null then [ r.value ] else []
    )
    (builtins.attrNames pkgs);

in results
