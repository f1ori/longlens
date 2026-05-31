# Long Lens

[![CI](https://github.com/f1ori/longlens/actions/workflows/flatpak.yml/badge.svg)](https://github.com/f1ori/longlens/actions/workflows/flatpak.yml)

> **Note**: Even though basic RDP sessions work, this is still work in progress

A simple, minimal, and modern RDP client

* Minimal UI to manage a list of favorite connections
* Minimal UI for remote sessions
* Passwords stored in keyring
* Clipboard synchronization
* Based on GTK4/libadwaita/Rust/IronRDP


## How to build

Also can be built with GNOME Builder

### VSCode and devcontainer

    meson setup builddir --prefix=/workspaces/longlens/builddir/install
    meson compile -C builddir
    meson install -C builddir
    ./builddir/install/bin/longlens


### flatpak

    # Build using flatpak-builder
    flatpak-builder --user --install --force-clean _flatpak_build de.f1ori.longlens.json

    # Run
    flatpak run de.f1ori.longlens
