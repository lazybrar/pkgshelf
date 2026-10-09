# Changelog

## 0.4.0
- not in the `pkgshelf` group? `add/rm/update/hold/rollback` re-run themselves through `sudo -g pkgshelf`; clear permission errors; `update` stops at the first permission failure
- after building, `update` offers `sudo pacman -Syu --needed <built packages>`
- missing AUR dependencies are tracked, built and installed first (asks); missing PGP keys from `validpgpkeys` are offered for import
- `update --devel` rebuilds -git/-svn/-hg/-bzr packages; `check` flags orphaned AUR packages
- `rollback <name>` restores the previous built version (the repo keeps the newest two) and holds the package
- `setup` also adds the sudo user to the group and the `[pkgshelf]` repo to /etc/pacman.conf
- packaged: daily `pkgshelf-check` user timer with a desktop notification (`systemctl --user enable --now pkgshelf-check.timer`), fish/bash/zsh completions

## 0.3.0
- setup command prints the root script for cargo-install users; doctor points to it; crates.io metadata and README

## 0.2.0
- shared global data in /var/lib/pkgshelf with a pkgshelf group; doctor checks permissions; migrate command

## 0.1.1
- `update` no longer prints PKGBUILD contents: a short summary and a prompt; read it on demand with `pkgshelf diff <name>`.

## 0.1.0
- add/rm/list/check/diff/update/hold/doctor; AUR and local packages in one local pacman repo; PKGBUILD review gate.
