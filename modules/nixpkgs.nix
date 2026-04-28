{ inputs, ... }: {
  perSystem = { system, ... }: {
    # Override the default 'pkgs' for all perSystem scopes to allow unfree
    _module.args.pkgs = import inputs.nixpkgs {
      inherit system;
      config.allowUnfree = true;
      # If some electron app requires an insecure package, you can add it here too:
      # config.permittedInsecurePackages =[ "electron-27.3.11" ];
    };
  };
}