<div id="longlens-logo" align="center">
    <br />
    <img src="data/icons/hicolor/scalable/apps/de.f1ori.longlens.svg" width="128" height="128" alt="Long Lens icon">
    <h1>Long Lens</h1>
</div>

[![CI](https://github.com/f1ori/longlens/actions/workflows/flatpak.yml/badge.svg)](https://github.com/f1ori/longlens/actions/workflows/flatpak.yml)

A simple, minimal, and modern RDP client

<div align="center">
  <img src="data/screenshots/screenshot1.png" width="45%" alt="Screenshot 1">
  <img src="data/screenshots/screenshot2.png" width="45%" alt="Screenshot 2">
</div>

* Clean, minimal UI for managing a list of favorite connections
* Minimal UI for remote sessions
* One window per active session
* Passwords stored in the system keyring
* GNOME Search Provider for destinations (can be enabled in GNOME Settings)
* High-DPI display support
* Fullscreen mode
* Text clipboard sync between local and remote sessions
* Based on FreeRDP

## How to install

Get it from [this flatpak repository on github pages](https://f1ori.github.io/longlens/)

The flatpak repository contains released versions and nightly development builds.

## How to build

Also can be built with GNOME Builder

### VSCode and devcontainer

FreeRDP 3.27.1 development libraries are required for system builds.

    meson setup builddir --prefix=/workspaces/longlens/builddir/install/ -Dprofile=development
    meson compile -C builddir
    meson install -C builddir
    ./builddir/install/bin/longlens


### flatpak

    # Build using flatpak-builder
    flatpak-builder --user --install --force-clean _flatpak_build de.f1ori.longlens.Devel.json

    # Run
    flatpak run de.f1ori.longlens.Devel
