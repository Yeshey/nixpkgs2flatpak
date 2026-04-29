# nixpkgs2flatpak

> [!WARNING]
> This is still just a **Proof of Concept.**

This project aims to mass-convert the massive library of applications in [nixpkgs](https://github.com/NixOS/nixpkgs) into [Flatpak](https://flatpak.org/). 

Ideally, we'll have manually created flatpak definitions for all pkgs, for now it employs a conversion automatically.

## How it works
1. [**nix-index-database**](https://github.com/nix-community/nix-index-database) is used to identify every package in nixpkgs that provides a `.desktop` file, only those are converted to flatpaks.
2. [**nix2flatpak by neobrain**](https://github.com/neobrain/nix2flatpak) is used for converting nix expressions to flatpak bundles.
3. Packages that don't have a manual definition are automatically converted, very primitive checks are done to decide whether to use the `org.kde.Platform/6.10` or the `org.gnome.Platform/49` runtime. Lax permissions are given to the flatpaks to guarantee they work.
4. `x86_64-linux` and `aarch64-linux` flatpak bundles are built with GitHub Actions and provided.

## Using the Repository

I'm hosting it in my own server (with no domain name yet 😔), you may add it locally:

```bash
flatpak remote-add --user --no-gpg-verify nixpkgs2flatpak http://143.47.53.175/
```

And check already available packages (they're being built and added slowly):

```bash
❯ flatpak remote-ls nixpkgs2flatpak
Name                                   Application ID                               Version                                Branch
Akira                                  com.github.akiraux.akira                     0.0.16                                 stable
Agenda                                 com.github.dahenson.agenda                   1.2.1                                  stable
Airshipper                             net.veloren.airshipper                       0.17.0                                 stable
Agent Configuration Dialog             org.kde.akonadi.configdialog                 26.04.0                                stable
```

### Install an application
```bash
flatpak install --user nixpkgs2flatpak com.github.akiraux.akira
```

---

## Contributing
The project uses a **Dendritic Pattern** (via `flake-parts` and `import-tree`). Each package has its own folder in `pkgs/by-name/`. 

---

## To-Do
- [ ] Currently, `nix-index` only sees what is in the official binary cache (Hydra). Since unfree packages are not cached, they aren't automatically discovered.
- [ ] Test packages, and don't expose them if they don't even launch.
- [ ] Move away from `--no-gpg-verify`.
---

Shoutout to [neobrain](https://github.com/neobrain) and their [**nix2flatpak**](https://github.com/neobrain/nix2flatpak) converter that inspired this project.
