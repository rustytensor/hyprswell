# Hyprswell

A dock for Hyprland, written in Rust with GTK 4 and gtk4-layer-shell: pinned and running apps on the screens you
choose, icons that swell under the pointer, reflections, an app grid, minimized apps, auto-hide with a thin lip, and a
reveal when an app asks for attention.

## Build

```
cargo build --release
./target/release/hyprswell
```

Needs GTK 4 and gtk4-layer-shell (Fedora: `gtk4-devel gtk4-layer-shell-devel`) and a running Hyprland.

## Use

- `hyprswell [--output NAME] [--no-swell]` starts the dock; `hyprswell --show` shows a hidden dock.
- Settings: `~/.config/nwg-dock-hyprland/hyprswell.conf` (`SCREENS`, `ICON_SIZE`, `REFLECTIONS`, `LAUNCHER`, `AUTOHIDE`).
- Look: `~/.config/hyprswell/style.css` (GTK CSS; `style.example.css` is a starting point).
- Pinned apps: right-click an icon; they are kept in `~/.cache/nwg-dock-pinned`.

`cargo test` runs the hand-worked checks of its geometry, motion, auto-hide and matching rules.

## Licence

GPL-3.0-or-later. See `LICENSE`.
