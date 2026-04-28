nixpkgs2flatpak/
├── flake.nix
├── Cargo.toml
├── discovered.json
├── .gitignore
├── src/
│   ├── main.rs
│   ├── types.rs
│   ├── discover.rs
│   ├── desktop_parser.rs
│   └── runtime_detector.rs
├── modules/
│   ├── options.nix
│   ├── packages.nix
│   ├── repo.nix
│   ├── server.nix
│   ├── devshell.nix
│   └── scanner.nix
├── pkgs/
│   └── by-name/
│       ├── gn/gnome-calculator/flake-parts.nix
│       ├── kc/kcalc/flake-parts.nix
│       ├── ne/neochat/flake-parts.nix
│       └── si/signal-desktop/flake-parts.nix
└── ci/
    ├── update-discovered.sh
    └── build.yml              ← copy to .github/workflows/