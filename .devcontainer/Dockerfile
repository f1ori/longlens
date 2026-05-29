FROM registry.fedoraproject.org/fedora-minimal:44
RUN microdnf install -y --refresh \
    git \
    cargo \
    rust \
    meson \
    msgfmt \
    blueprint-compiler \
    gtk-update-icon-cache \
    update-desktop-database \
    pango-devel \
    gtk4-devel \
    libadwaita-devel \
    rust-analyzer \
    && microdnf clean all \
    && useradd -m -u 1000 vscode