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
        systemd.services.nixpkgs2flatpak-update-summary = {
          description = "Regenerate nixpkgs2flatpak Flatpak repo summary";
          after       = [ "remote-fs.target" ];
          requires    = [ "remote-fs.target" ];
          
          path        = with pkgs;[ util-linux coreutils findutils flatpak rclone rsync ];
          
          serviceConfig = {
            Type                 = "oneshot";
            TimeoutStartSec      = "infinity";
            User                 = "root";
            Nice                 = 10;
            IOSchedulingClass    = "best-effort";
            IOSchedulingPriority = 5;
          };
          
          # Cleaned up script!
          script = ''
            set -euo pipefail
            
            REPO="${cfg.repoPath}"

            echo "Updating OSTree summary at $REPO ..."
            flatpak build-update-repo \
              ${lib.optionalString (cfg.gpgKeyId != null) ''--gpg-sign="${cfg.gpgKeyId}"''} \
              "$REPO"

            echo "Summary successfully updated."
          '';
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
              sendfile off;
              aio threads;
              send_timeout 600s;
              keepalive_timeout 300s;

              autoindex on;
              disable_symlinks off;
              add_header Access-Control-Allow-Origin "*";

              location ^~ /objects/ {
                directio 512k;
                aio threads;
                output_buffers 2 512k;
                add_header Access-Control-Allow-Origin "*";
                add_header Cache-Control "public, max-age=31536000";
                add_header Content-Type application/octet-stream;
              }

              location = /summary {
                add_header Access-Control-Allow-Origin "*";
                add_header Cache-Control "no-cache, no-store, must-revalidate";
                add_header Content-Type application/octet-stream;
              }
              location = /summary.sig {
                add_header Access-Control-Allow-Origin "*";
                add_header Cache-Control "no-cache, no-store, must-revalidate";
                add_header Content-Type application/octet-stream;
              }
              location ^~ /refs/ {
                add_header Access-Control-Allow-Origin "*";
                add_header Cache-Control "no-cache, no-store, must-revalidate";
              }
              
              location / {
                add_header Access-Control-Allow-Origin "*";
                add_header Cache-Control "public, max-age=300";
              }

              location ~* \.(bak|save|lock|template|env|env\.[a-z]+|dockerenv)$ {
                return 444;
              }
              location ~* ^/\.(env|dockerenv|git|ssh) {
                return 444;
              }
            '';
          };
        };

        systemd.services.nginx = {
          # Clear stale ports if workers got stuck in D-state
          preStart = lib.mkBefore ''
            ${pkgs.psmisc}/bin/fuser -k 80/tcp 443/tcp || true
          '';
          
          serviceConfig = {
            TimeoutStopSec = "10s";
            KillMode       = "mixed";
          };
        };

        systemd.timers.nixpkgs2flatpak-update-summary = {
          description = "Periodically regenerate nixpkgs2flatpak Flatpak repo summary";
          wantedBy    = [ "timers.target" ];
          timerConfig = {
            OnCalendar = "*-*-* 00:00:00";
            OnBootSec  = "10min";
            Persistent = false;
          };
        };

        # ── Static Delta Generation Timer ───────────────────────────────────────
        systemd.services.nixpkgs2flatpak-generate-deltas = {
          description = "Generate static deltas for nixpkgs2flatpak Flatpak repo";
          after       = [ "remote-fs.target" ];
          requires    = [ "remote-fs.target" ];
          
          path        = with pkgs;[ util-linux coreutils findutils flatpak rclone rsync ];
          
          serviceConfig = {
            Type                 = "oneshot";
            User                 = "root";
            Nice                 = 19;
            IOSchedulingClass    = "idle";
            TimeoutStartSec      = "infinity";
          };
          
          # Cleaned up script!
          script = ''
            set -euo pipefail

            REPO="${cfg.repoPath}"

            echo "Generating static deltas at $REPO ..."
            flatpak build-update-repo \
              --generate-static-deltas \
              ${lib.optionalString (cfg.gpgKeyId != null) ''--gpg-sign="${cfg.gpgKeyId}"''} \
              "$REPO"

            echo "Delta generation complete."
          '';
        };

        systemd.timers.nixpkgs2flatpak-generate-deltas = {
          description = "Weekly static delta generation for nixpkgs2flatpak";
          wantedBy    = [ "timers.target" ];
          timerConfig = {
            OnCalendar         = "Sun 03:00";
            Persistent         = false;
            RandomizedDelaySec = "30min";
          };
        };
      };
    };
}