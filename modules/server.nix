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
          
          path        = with pkgs;[ util-linux coreutils findutils flatpak rclone rsync ];
          
          serviceConfig = {
            Type                 = "oneshot";
            TimeoutStartSec      = "infinity";
            User                 = "root";
            Nice                 = 10;
            IOSchedulingClass    = "best-effort";
            IOSchedulingPriority = 5;
          };
          script = ''
            set -euo pipefail
            
            REPO="${cfg.repoPath}"
            CACHE_DIR="/var/cache/nixpkgs2flatpak-overlay"
            UPPER="$CACHE_DIR/upper"
            WORK="$CACHE_DIR/work"
            MERGED="$CACHE_DIR/merged"

            echo "Preparing OverlayFS Trapdoor..."
            umount -l "$MERGED" 2>/dev/null || true
            rm -rf "$CACHE_DIR"
            mkdir -p "$UPPER" "$WORK" "$MERGED"

            mount -t overlay overlay -o lowerdir="$REPO",upperdir="$UPPER",workdir="$WORK" "$MERGED"

            cleanup() {
              echo "Cleaning up OverlayFS..."
              umount -l "$MERGED" 2>/dev/null || true
              rm -rf "$CACHE_DIR"
            }
            trap cleanup EXIT

            echo "Updating OSTree summary at $MERGED ..."
            flatpak build-update-repo \
              ${lib.optionalString (cfg.gpgKeyId != null) ''--gpg-sign="${cfg.gpgKeyId}"''} \
              "$MERGED"

            echo "Removing OverlayFS whiteout devices..."
            find "$UPPER" -type c -delete

            echo "Pushing compiled metadata DIRECTLY into the active mount..."
            # Using rclone local-to-local copy directly into the mount folder.
            # This passes through FUSE, guaranteeing the VFS cache is instantly consistent!
            rclone copy "$UPPER" "$REPO" \
              --config /root/.config/rclone/rclone.conf \
              --fast-list --transfers 4

            echo "Summary successfully updated."
          '';
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

              # 1. Objects are immutable (they never change hashes). Cache them for a year!
              location ^~ /objects/ {
                directio 512k;
                aio threads;
                output_buffers 2 512k;
                add_header Access-Control-Allow-Origin "*";
                add_header Cache-Control "public, max-age=31536000";
                add_header Content-Type application/octet-stream;
              }

              # 2. Summary and Refs change constantly. NEVER cache them!
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
              
              # Fallback for anything else
              location / {
                add_header Access-Control-Allow-Origin "*";
                add_header Cache-Control "public, max-age=300";
              }

              # ─────────────────────────────────────────────────────────────────
              # Block vulnerability scanners & bots to save OneDrive API calls!
              # ─────────────────────────────────────────────────────────────────
              
              # 1. Browsers always ask for this. Return an empty success to save I/O.
              location = /favicon.ico {
                return 204;
                access_log off;
                log_not_found off;
              }

              # 2. Block ANY hidden files or directories (e.g. /.env, /.git, /.ssh, /.bash_history)
              location ~ /\. {
                return 444;
              }

              # 3. Block ANY PHP files (even with trailing paths) or known PHP test directories
              location ~* (\.php|/vendor/|/phpunit/) {
                return 444;
              }

              # 4. Block common web-app routes probed by bots (login, admin, wordpress, etc.)
              location ~* ^/(login|admin|wp-|api|containers|test|tests|demo|backup|workspace|blog|cms|crm|panel|v[1-9]|zend|yii|laravel|lib|www|ws|app|apps|public)(/|$) {
                return 444;
              }

              # 5. Block configuration, database dumps, and environment files
              location ~* \.(py|json|js|yaml|yml|ini|xml|bak|save|lock|template|env|env\.[a-z]+|dockerenv|sql|tar|gz|zip|db)$ {
                return 444;
              }
            '';
          };
        };

        systemd.services.nginx = {
          # Use mkBefore so this port-clearing script runs BEFORE nginx does its built-in configuration syntax check
          preStart = lib.mkBefore ''
            ${pkgs.psmisc}/bin/fuser -k 80/tcp 443/tcp || true
          '';
          
          serviceConfig = {
            TimeoutStopSec = "10s";
            KillMode       = "mixed";
          };
        };

        # Weekly: expensive static delta generation — improves update download
        # size for existing users but not required for installs or remote-ls.
        systemd.services.nixpkgs2flatpak-generate-deltas = {
          description = "Generate static deltas for nixpkgs2flatpak Flatpak repo";
          after       = [ "remote-fs.target" ];
          requires    = [ "remote-fs.target" ];
          
          # Inject required commands into the systemd environment
          path        = with pkgs;[ util-linux coreutils findutils flatpak rclone rsync ];
          
          serviceConfig = {
            Type                 = "oneshot";
            User                 = "root";
            Nice                 = 19;
            IOSchedulingClass    = "idle";
            TimeoutStartSec      = "infinity";
          };
          script = ''
            set -euo pipefail

            REPO="${cfg.repoPath}"
            CACHE_DIR="/var/cache/nixpkgs2flatpak-delta-overlay"
            UPPER="$CACHE_DIR/upper"
            WORK="$CACHE_DIR/work"
            MERGED="$CACHE_DIR/merged"

            echo "Preparing OverlayFS Trapdoor..."
            umount -l "$MERGED" 2>/dev/null || true
            rm -rf "$CACHE_DIR"
            mkdir -p "$UPPER" "$WORK" "$MERGED"

            mount -t overlay overlay -o lowerdir="$REPO",upperdir="$UPPER",workdir="$WORK" "$MERGED"

            cleanup() {
              echo "Cleaning up OverlayFS..."
              umount -l "$MERGED" 2>/dev/null || true
              rm -rf "$CACHE_DIR"
            }
            trap cleanup EXIT

            echo "Generating static deltas at $MERGED ..."
            flatpak build-update-repo \
              --generate-static-deltas \
              ${lib.optionalString (cfg.gpgKeyId != null) ''--gpg-sign="${cfg.gpgKeyId}"''} \
              "$MERGED"

            echo "Removing OverlayFS whiteout devices..."
            find "$UPPER" -type c -delete

            echo "Pushing deltas DIRECTLY into the active mount..."
            # Using rclone local-to-local copy directly into the mount folder.
            # This passes through FUSE, guaranteeing the VFS cache is instantly consistent!
            rclone copy "$UPPER" "$REPO" \
              --config /root/.config/rclone/rclone.conf \
              --fast-list --transfers 2 --tpslimit 3 --tpslimit-burst 5 --checkers 8

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