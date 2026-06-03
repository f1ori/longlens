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
    alsa-lib-devel \
    gcc-c++ \
    cmake \
    sudo \
    git-lfs \
    && microdnf clean all \
    && useradd -m -u 1000 vscode \
    && echo "vscode ALL=(ALL) NOPASSWD:ALL" >> /etc/sudoers.d/vscode