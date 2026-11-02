# GNOME Shell 46 (Ubuntu 24.04 LTS, the GNOME UI's oldest supported
# version) for `make gnome-test-46`: tests/gnome_test.py against the
# extension in a headless shell there. Software rendering, no GPU.
FROM docker.io/library/ubuntu:24.04

RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      gnome-shell gnome-shell-common libglib2.0-bin dbus dbus-bin \
      dconf-gsettings-backend dconf-service \
      python3 python3-gi gir1.2-glib-2.0 \
      libgl1-mesa-dri libegl1 libegl-mesa0 mesa-libgallium \
      procps fuse3 ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# No systemd in a container: logind cannot be activated, and GNOME Shell 46
# throws at startup when asking for it fails. With no activation file the
# name is simply absent, which the shell copes with.
RUN rm /usr/share/dbus-1/system-services/org.freedesktop.login1.service \
 && mkdir -p /run/dbus

WORKDIR /src
