# pkgshelf

A tiny pacman-integrated tracker for the few packages you choose: some from the AUR, some of your own (e.g. built with
[cratepkg](https://github.com/lazybrar/cratepkg)). Not a package manager and not an AUR helper that searches or resolves
the whole AUR: it builds your picks into one local pacman repo, and plain `pacman -Syu` installs and updates them.
Zero dependencies, written in Rust. It never installs anything itself.

## Setup

    cargo build --release        # or: cd pkg && makepkg -si
    pkgshelf doctor              # checks tools; prints the [pkgshelf] block to add to /etc/pacman.conf

Add the printed block to `/etc/pacman.conf` once (it points at `~/.local/share/pkgshelf/repo`), then `sudo pacman -Sy`.

## Use

    pkgshelf add aur brave-bin claude-code     # track AUR packages
    pkgshelf add local ~/work/projects/small/clip   # track a local project (PKGBUILD or pkg/PKGBUILD)
    pkgshelf check                             # installed / built / latest, exit 100 if updates are pending
    pkgshelf update                            # build outdated ones into the repo (AUR: you review the PKGBUILD first)
    sudo pacman -Syu                           # pacman installs and updates everything, as usual

Also: `list`, `diff <name>`, `hold`/`unhold <name>`, `rm <name> [--purge]`, `update <name> --force`, `doctor`
(lists installed foreign packages you don't track yet).

## How it works

- Tracked entries live in `~/.config/pkgshelf/packages` (`aur NAME [hold]` / `local PATH [hold]`), plain text you can edit.
- Built packages go to `~/.local/share/pkgshelf/repo` via `makepkg` + `repo-add`; versions are compared with `vercmp`.
- AUR: the git repo is cloned to `~/.cache/pkgshelf/aur`. The first time you see the full PKGBUILD; afterwards only the diff
  since the commit you approved. Nothing is built without your `y` (`--yes` skips the prompt; use with care).
- Local: the PKGBUILD is built as is; bump the version (e.g. `cratepkg init --force` after a Cargo.toml version bump)
  so pacman sees an upgrade. `update --force <name>` rebuilds the same version.
- Once a package is in the `[pkgshelf]` repo, `pacman -Qm` no longer lists it as foreign.

## Limits

- AUR dependencies are not resolved: if a build needs another AUR package, `pkgshelf add aur <dep>` it first.
- VCS packages (`-git`) report the PKGBUILD's stale `pkgver`, so `check` can't see new commits; use `update --force`.
- Split AUR packages: only the tracked package name is added to the repo.
- Uses `curl` (AUR RPC) and `git`, `makepkg`, `repo-add`, `vercmp` from pacman. `makepkg -s` may ask for sudo to install build deps.

## License

GPL-2.0-or-later. See `LICENSE`.
