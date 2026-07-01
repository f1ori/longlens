# Long Lens

A minimal RDP client based on Rust, GTK4 and FreeRDP.

We use the installed FreeRDP 3 client libraries through the C adapter in
`src/rdp/freerdp_bridge.c`.

It uses blueprint files (blp) to describe the UI.

# Build

Use meson to build, not cargo directly:

```
meson compile -C builddir
```

# Translations

Translatable strings are wrapped in `gettext()` (Rust) or marked in the blueprint
files. The source files containing them are listed in `po/POTFILES.in` (keep it
sorted alphabetically). After adding or changing translatable strings:

```
# Regenerate the po/longlens.pot template from the sources
ninja -C builddir longlens-pot

# Merge the new strings into every po/*.po catalog (e.g. de.po)
ninja -C builddir longlens-update-po
```

Then fill in the empty `msgstr ""` entries in the affected `po/*.po` files.
