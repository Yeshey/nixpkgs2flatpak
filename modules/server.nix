{ ... }: {
  flake.nixosModules.flatpakServer =
    { config, pkgs, lib, ... }:
    let
      cfg = config.services.nixpkgs2flatpak;
    in {
      options.services.nixpkgs2flatpak = {
        enable = lib.mkEnableOption "nixpkgs2flatpak Flatpak repository HTTPS server";

        repoPath = lib.mkOption {
          type        = lib.types.str;
          default     = "/var/lib/nixpkgs2flatpak/repo";
          description = "Absolute path to the OSTree repository on disk.";
        };

        domain = lib.mkOption {
          type        = lib.types.str;
          example     = "flatpak.example.com";
          description = "Public domain name to serve the repo from.";
        };

        gpgKeyId = lib.mkOption {
          type    = lib.types.nullOr lib.types.str;
          default = null;
          description = "GPG key ID used to sign the repo (optional but recommended).";
        };

        acmeEmail = lib.mkOption {
          type        = lib.types.str;
          description = "Email address for Let's Encrypt certificate notifications.";
        };
      };

      config = lib.mkIf cfg.enable {
        users.users.nixpkgs2flatpak = {
          isSystemUser = true;
          group        = "nixpkgs2flatpak";
          home         = "/var/lib/nixpkgs2flatpak";
          createHome   = true;
        };
        users.groups.nixpkgs2flatpak = {};

        systemd.tmpfiles.rules = [
          "d ${cfg.repoPath} 0755 nixpkgs2flatpak nixpkgs2flatpak -"
        ];

        security.acme = {
          acceptTerms = true;
          defaults.email = cfg.acmeEmail;
        };

        services.nginx = {
          enable = true;
          virtualHosts.${cfg.domain} = {
            enableACME = true;
            forceSSL   = true;
            root       = cfg.repoPath;
            extraConfig = ''
              autoindex on;
              add_header Access-Control-Allow-Origin "*";
              add_header Cache-Control "public, max-age=300";

              # Flatpak clients expect these MIME types
              location ~* \.flatpak$ {
                add_header Content-Type application/octet-stream;
              }
              location = /summary {
                add_header Content-Type application/octet-stream;
              }
            '';
          };
        };
      };
    };
}