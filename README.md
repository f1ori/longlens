<div id="longlens-logo" align="center">
    <br />
    <img src="data/icons/hicolor/scalable/apps/de.f1ori.longlens.svg" width="128" height="128" alt="Long Lens icon">
    <h1>Long Lens</h1>
</div>

[![CI](https://github.com/f1ori/longlens/actions/workflows/flatpak.yml/badge.svg)](https://github.com/f1ori/longlens/actions/workflows/flatpak.yml)

A simple, minimal, and modern RDP client

<div align="center">
  <img src="data/screenshots/screenshot-windows-light.png" width="30%" alt="Screenshot of connection to Windows host">
  <img src="data/screenshots/screenshot-overview-light.png" width="30%" alt="Screenshot of initial overview">
  <img src="data/screenshots/screenshot-bluefin-dark.png" width="30%" alt="Screenshot of connection to GNOME remote desktop">
</div>

* Clean, minimal UI for managing a list of favorite connections
* Minimal UI for remote sessions
* One window per active session
* Passwords stored in the system keyring
* GNOME Search Provider for destinations (can be enabled in GNOME Settings)
* High-DPI display support
* Fullscreen mode
* Text and file clipboard sync between local and remote sessions
* Based on FreeRDP

## How to install

Get it from [this flatpak repository on github pages](https://f1ori.github.io/longlens/)

The flatpak repository contains released versions and nightly development builds.

## How to build

### GNOME builder

Should work out of the box.

### VSCode and devcontainer

All dependencies are included in the devcontainer.

A few caveats when using the devcontainer:

* localhost refers to the container, not your host system. With Podman, use hosts.containers.internal to reach the host.
* File copy and paste may not work because the filesystem is not shared and the FileTransfer portal is not configured.

    meson setup builddir --prefix=/workspaces/longlens/builddir/install/ -Dprofile=development
    meson compile -C builddir
    meson install -C builddir
    ./builddir/install/bin/longlens


### flatpak

    # Build using flatpak-builder
    flatpak-builder --user --install --force-clean _flatpak_build de.f1ori.longlens.Devel.json

    # Run
    flatpak run de.f1ori.longlens.Devel
