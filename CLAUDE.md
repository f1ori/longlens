# Long Lens

A minimal RDP client based on Rust, GTK4 and IronRDP.

We use the ironrdp-client library. A reference application for this library is in ironrdp-viewer.

It uses blueprint files (blp) to describe the UI.

# Build

Use meson to build, not cargo directly:

```
meson compile -C builddir
```
