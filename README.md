# pkgshelf

A tiny pacman-integrated tracker for the few packages you choose: some from the AUR, some of your own (e.g. built with
[cratepkg](https://github.com/lazybrar/cratepkg)). Not a package manager and not an AUR helper that searches or resolves
the whole AUR: it builds your picks into one local pacman repo, and plain `pacman -Syu` installs and updates them.
Zero dependencies, written in Rust. It never installs anything itself.

## Setup (one shared setup for every user of the machine)

    cd pkg && makepkg -si        # creates the `pkgshelf` group and /var/lib/pkgshelf, /var/cache/pkgshelf
    sudo usermod -aG pkgshelf <you>      # then log in again
    pkgshelf doctor              # checks tools, directories and /etc/pacman.conf; prints what is missing

Add the block `doctor` prints to `/etc/pacman.conf` once (`Server = file:///var/lib/pkgshelf/repo`), then `sudo pacman -Sy`.

Everything is shared: one tracked list, one set of reviewed PKGBUILDs and one repo for all users, so `pacman -Syu` updates the
same packages for everyone. Members of the `pkgshelf` group can add, build and remove packages; **treat the group like root**,
because pacman installs whatever is in the repo as root. `doctor` warns if the directories are world-writable.
Upgrading from a version that kept data per user (`~/.config/pkgshelf`, `~/.local/share/pkgshelf`)? Run `pkgshelf migrate`
as the user who owns that data, update the `Server` path in `/etc/pacman.conf`, then delete the old directories.
`PKGSHELF_ROOT` (and `PKGSHELF_CACHE`) override the location, e.g. for a private per-user setup.

## Use

    pkgshelf add aur brave-bin claude-code     # track AUR packages
    pkgshelf add local ~/work/projects/small/clip   # track a local project (PKGBUILD or pkg/PKGBUILD)
    pkgshelf check                             # installed / built / latest, exit 100 if updates are pending
    pkgshelf update                            # build outdated ones into the repo (AUR: short summary, then confirm)
    sudo pacman -Syu                           # pacman installs and updates everything, as usual

Also: `list`, `diff <name>`, `hold`/`unhold <name>`, `rm <name> [--purge]`, `update <name> --force`, `doctor`
(lists installed foreign packages you don't track yet).

## How it works

- Tracked entries live in `/var/lib/pkgshelf/packages` (`aur NAME [hold]` / `local PATH [hold]`), plain text you can edit.
- Built packages go to `/var/lib/pkgshelf/repo` via `makepkg` + `repo-add`; versions are compared with `vercmp`.
- AUR: the git repo is cloned to `/var/cache/pkgshelf/aur`. `update` prints only a short summary (changed files, diffstat) and asks before building;
  `pkgshelf diff <name>` shows the PKGBUILD (full the first time, then only changes since your approval). Nothing is built without your `y` (`--yes` skips the prompt; use with care).
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
