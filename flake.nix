{
  description = "Automatically convert nixpkgs applications to Flatpak and host a repository";

  inputs = {
    nixpkgs.url      = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url  = "github:hercules-ci/flake-parts";
    import-tree.url  = "github:vic/import-tree";
    nix2flatpak.url  = "github:neobrain/nix2flatpak";
  };

  outputs = inputs:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [
        (inputs.import-tree ./modules)   # picks up every *.nix in modules/
        (inputs.import-tree ./pkgs)      # picks up every flake-parts.nix under pkgs/
      ];
      systems = [ "x86_64-linux" "aarch64-linux" ];
    };
}