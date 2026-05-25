# Long Lens

A simple, modern RDP client

## How to build

The easiest thing to use is GNOME Builder.

### VSCode and devcontainer

    meson setup builddir --reconfigure --prefix=/workspaces/longlens/builddir/install
    meson compile -C builddir
    meson install -C builddir
    ./builddir/install/bin/longlens


### flatpak

    # Build using flatpak-builder
    flatpak-builder --user --install --force-clean _flatpak_build de.f1ori.longlens.json

    # Run
    flatpak run de.f1ori.longlens
