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
          description = "Public domain name or IP to serve the repo from. (Do NOT include http://)";
        };
        
        enableSSL = lib.mkOption {
          type        = lib.types.bool;
          default     = true;
          description = "Enable HTTPS and auto-fetch Let's Encrypt certs. Set to false if using a raw IP.";
        };

        isDefault = lib.mkOption {
          type        = lib.types.bool;
          default     = false;
          description = "Whether this virtual host should be the default for the server's IP address.";
        };

        openFirewall = lib.mkOption {
          type        = lib.types.bool;
          default     = false;
          description = "Whether to automatically open the required ports (80 and 443) in the firewall.";
        };

        gpgKeyId = lib.mkOption {
          type    = lib.types.nullOr lib.types.str;
          default = null;
          description = "GPG key ID used to sign the repo (optional but recommended).";
        };

        acmeEmail = lib.mkOption {
          type        = lib.types.nullOr lib.types.str;
          default     = null;
          description = "Email address for Let's Encrypt certificate notifications.";
        };
      };

      config = lib.mkIf cfg.enable {
        # ── Firewall ──
        networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall (
          [ 80 ] ++ (lib.optional cfg.enableSSL 443)
        );

        systemd.tmpfiles.rules = [
          "d ${cfg.repoPath} 0755 nixpkgs2flatpak nixpkgs2flatpak -"
        ];

        security.acme = lib.mkIf cfg.enableSSL {
          acceptTerms = true;
          defaults.email = cfg.acmeEmail;
        };

        # ── Summary regeneration timer ──────────────────────────────────────────
        # CI runners are ephemeral and only see a slice of the full package set,
        # so they never touch the summary file. The server, which has the complete
        # OneDrive repo mounted, owns the summary and regenerates it here.
        # Hourly: fast summary update — what flatpak remote-ls and installs need.
        # No static deltas here; those are expensive and handled separately below.
        systemd.services.nixpkgs2flatpak-update-summary = {
          description = "Regenerate nixpkgs2flatpak Flatpak repo summary";
          after       = [ "remote-fs.target" ];
          requires    = [ "remote-fs.target" ];
          serviceConfig = {
            Type                 = "oneshot";
            User                 = "root";
            Nice                 = 10;
            IOSchedulingClass    = "best-effort";
            IOSchedulingPriority = 5;
          };
          script = ''
            set -euo pipefail
            echo "Updating OSTree summary at ${cfg.repoPath} ..."
            ${pkgs.flatpak}/bin/flatpak build-update-repo \
              ${lib.optionalString (cfg.gpgKeyId != null) ''--gpg-sign="${cfg.gpgKeyId}"''} \
              ${cfg.repoPath}
            echo "Summary updated."
          '';
        };

        systemd.timers.nixpkgs2flatpak-update-summary = {
          description = "Periodically regenerate nixpkgs2flatpak Flatpak repo summary";
          wantedBy    = [ "timers.target" ];
          timerConfig = {
            OnCalendar         = "hourly";
            Persistent         = true;
            RandomizedDelaySec = "5min";
          };
        };

        # Weekly: expensive static delta generation — improves update download
        # size for existing users but not required for installs or remote-ls.
        systemd.services.nixpkgs2flatpak-generate-deltas = {
          description = "Generate static deltas for nixpkgs2flatpak Flatpak repo";
          after       = [ "remote-fs.target" ];
          requires    = [ "remote-fs.target" ];
          serviceConfig = {
            Type                 = "oneshot";
            User                 = "root";
            Nice                 = 19;
            IOSchedulingClass    = "idle";
            # Delta generation can take hours on a large repo; don't let
            # systemd kill it on a default timeout.
            TimeoutStartSec      = "infinity";
          };
          script = ''
            set -euo pipefail
            echo "Generating static deltas at ${cfg.repoPath} ..."
            ${pkgs.flatpak}/bin/flatpak build-update-repo \
              --generate-static-deltas \
              ${lib.optionalString (cfg.gpgKeyId != null) ''--gpg-sign="${cfg.gpgKeyId}"''} \
              ${cfg.repoPath}
            echo "Delta generation complete."
          '';
        };

        systemd.timers.nixpkgs2flatpak-generate-deltas = {
          description = "Weekly static delta generation for nixpkgs2flatpak";
          wantedBy    = [ "timers.target" ];
          timerConfig = {
            OnCalendar         = "Sun 03:00";
            Persistent         = true;
            RandomizedDelaySec = "30min";
          };
        };

        # ── Nginx ──
        services.nginx = {
          enable = true;
          virtualHosts.${cfg.domain} = {
            enableACME = cfg.enableSSL;
            forceSSL   = cfg.enableSSL;
            default    = cfg.isDefault;
            root       = cfg.repoPath;
            extraConfig = ''
              autoindex on;
              disable_symlinks off;
              
              add_header Access-Control-Allow-Origin "*";
              add_header Cache-Control "public, max-age=300";

              location ~* \.flatpak$ {
                add_header Access-Control-Allow-Origin "*";
                add_header Cache-Control "public, max-age=300";
                add_header Content-Type application/octet-stream;
              }
              location = /summary {
                add_header Access-Control-Allow-Origin "*";
                add_header Cache-Control "public, max-age=300";
                add_header Content-Type application/octet-stream;
              }
            '';
          };
        };
      };
    };
}