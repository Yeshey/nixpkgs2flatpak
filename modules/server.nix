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
            TimeoutStartSec = "infinity";
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

            # Warm the rclone VFS directory cache before mounting the OverlayFS.
            #
            # Root cause of "Stale file handle" errors:
            #   rclone's default --dir-cache-time is 5 minutes. flatpak build-update-repo
            #   takes 10+ minutes to walk all refs. Halfway through, rclone silently
            #   invalidates and rebuilds its directory cache — but OverlayFS is still
            #   holding the old inode handles into the FUSE lowerdir → ESTALE.
            #
            # The rclone mount now uses --dir-cache-time 2h (see rclone-mount-onedrive.nix)
            # so invalidation can't happen mid-run. This find forces rclone to eagerly
            # populate fresh directory entries right before the overlay mounts, so the
            # 2-hour clock starts from a known-good state.
            # echo "Warming rclone VFS directory cache for refs/..."
            # find "$REPO/refs" -follow -type f >/dev/null 2>&1 || true

            echo "Preparing OverlayFS Trapdoor..."
            umount -q "$MERGED" 2>/dev/null || true
            rm -rf "$CACHE_DIR"
            mkdir -p "$UPPER" "$WORK" "$MERGED"

            mount -t overlay overlay -o lowerdir="$REPO",upperdir="$UPPER",workdir="$WORK" "$MERGED"

            cleanup() {
              echo "Cleaning up OverlayFS..."
              umount -q "$MERGED" 2>/dev/null || true
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

              # Silently discard requests from vulnerability scanners probing common paths.
              # These generate noise in the logs but are otherwise harmless.
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
          # Use mkBefore so this port-clearing script runs BEFORE nginx does its built-in configuration syntax check
          preStart = lib.mkBefore ''
            ${pkgs.psmisc}/bin/fuser -k 80/tcp 443/tcp || true
          '';
          
          serviceConfig = {
            TimeoutStopSec = "10s";
          };
        };

        systemd.timers.nixpkgs2flatpak-update-summary = {
          description = "Periodically regenerate nixpkgs2flatpak Flatpak repo summary";
          wantedBy    = [ "timers.target" ];
          timerConfig = {
            OnCalendar = "hourly";
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

            # Same VFS warm-up as the summary service — delta generation takes even
            # longer, so a fresh directory cache is essential.
            # echo "Warming rclone VFS directory cache for refs/..."
            # find "$REPO/refs" -follow -maxdepth 3 -type f >/dev/null 2>&1 || true

            echo "Preparing OverlayFS Trapdoor..."
            umount -q "$MERGED" 2>/dev/null || true
            rm -rf "$CACHE_DIR"
            mkdir -p "$UPPER" "$WORK" "$MERGED"

            mount -t overlay overlay -o lowerdir="$REPO",upperdir="$UPPER",workdir="$WORK" "$MERGED"

            cleanup() {
              echo "Cleaning up OverlayFS..."
              umount -q "$MERGED" 2>/dev/null || true
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
              --fast-list --transfers 4 --checkers 8

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
      };
    };
}