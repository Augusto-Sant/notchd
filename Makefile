# Development entry points. See DEVELOPMENT.md, "Tests".
#
# The tests are a pyramid: many cheap ones at the bottom, few costly ones on
# top, the costly ones run less often.
#   make test         unit: pure rules, seconds. After every change.
#   make integration  bridges and installer as processes, with fakes.
#   make qmltest      QML behaviour, offscreen.
#   make visual       offscreen renders.
#   make check        all of it, the cheap layers first: before every commit.
#   make times        what each layer and test cost in its last run.
#   make gnome-test   the GNOME Shell extension in a headless GNOME Shell (needs
#                     gnome-shell; not in check). gnome-test-46: the same on
#                     GNOME 46 (Ubuntu 24.04) in a podman container.

QMLLINT ?= $(firstword $(wildcard /usr/lib/qt6/bin/qmllint /usr/lib64/qt6/bin/qmllint) \
	$(shell command -v qmllint6 2>/dev/null) $(shell command -v qmllint 2>/dev/null))

# The Omarchy host imports qs.Commons/qs.Ui, which only exist inside the
# Omarchy shell, so lint skips it; `make restart` and the shell log cover it.
LINT_FILES = shell.qml core/*.qml core/settings/*.qml core/settings/previews/*.qml \
	$(wildcard core/client/*.qml core/client/prims/*.qml)
# Not core/terminal/: it imports QMLTermWidget, whose package ships no type
# information for qmllint. The QML tests load it where it is installed.

# Dev config for `make dev`, so it never touches ~/.config/notchd.
DEV_CONFIG ?= $(CURDIR)/build/dev-config.json

# The daemon `make dev` and `make restart` build and the hosts start
# (scripts/ensure-daemon.sh finds it through NOTCHD_BIN).
DAEMON_BIN = $(CURDIR)/native/target/release/notchd
# `make dev` runs its own daemon instance, never the user's `default` one.
DEV_INSTANCE ?= dev

# Each layer records what it cost in build/times/ (scripts/timed.py).
TIMED = scripts/timed.py

.PHONY: check test rust integration qmltest visual times hero icons bench lint lint-run validate daemon dev restart deps clean gnome-dev gnome-dev-46 gnome-test gnome-test-46 gnome-image-46 gnome-zip

# The cheap layers first, one after another: a broken rule fails in seconds,
# before anything costly starts. Then the costly ones at once.
check: lint validate test rust
	@$(MAKE) --no-print-directory -j3 -O integration qmltest visual

# Unit layer: the UI-side JS (core/*.js, core/settings/*.js, core/client/) in
# Node, the Rust core's rules and modules (native/, cargo test), and
# the Python bridges' and installer's rules as functions (tests/*_test.py,
# every TestCase without `tier = "integration"`). No processes, no servers.
test:
	@mkdir -p build/times
	@$(TIMED) node node --test --test-reporter=dot --test-reporter-destination=stdout \
	  --test-reporter=./scripts/node-times.mjs --test-reporter-destination=build/times/node.tsv \
	  tests/*.test.mjs
	@$(TIMED) py-unit scripts/pytests.py unit
	@$(TIMED) rust cargo test --manifest-path native/Cargo.toml --quiet

# The Rust workspace (native/): format, clippy with warnings as errors, every
# crate's tests. Cheap, so it sits with the first layers of `make check`.
rust:
	@$(TIMED) rust-fmt cargo fmt --all --check --manifest-path native/Cargo.toml
	@$(TIMED) rust-clippy cargo clippy --workspace --all-targets --manifest-path native/Cargo.toml --quiet -- -D warnings
	@$(TIMED) rust-test cargo test --workspace --manifest-path native/Cargo.toml --quiet

# Integration layer: the live bridges and the components' installer run as
# processes against fakes (a fake LocalAI, pacman, downloads from a local
# server: nothing real is installed), on parallel workers. TestCases with
# `tier = "integration"`.
integration: daemon
	@$(TIMED) py-integration scripts/pytests.py integration

# QML behaviour tests (tests/qml/shell.qml), offscreen, in parallel shards:
# bindings, signals and animations the Node tests can't reach. Regression
# tests live here. QMLTEST_ONLY=<part of a name> runs only those cases.
qmltest: daemon
	@$(TIMED) qml scripts/qml-test.sh

# Renders the settings window offscreen into build/visual/; fails on any QML
# warning, so it also proves the core loads.
visual: daemon
	@$(TIMED) visual scripts/visual-test.sh

# The pyramid from the last run of each layer: tests, wall and CPU time per
# layer, the slowest tests and those over their layer's budget. Also in
# build/test-times.md.
times:
	@scripts/test-times.py

# The README image, docs/images/hero.webp: the real notch offscreen, with a
# demo media player on a private D-Bus, over Omarchy's Gruvbox wallpaper
# (WALLPAPER=<image> for another). Rerun after a visual change to the notch
# and commit the image.
hero: daemon
	scripts/hero.sh

# The app icon at every hicolor size, plus notchd.ico, from
# assets/icons/hicolor/scalable/apps/notchd.svg. Rerun after editing the SVG
# and commit the PNGs.
icons:
	scripts/icons.sh

# What notchd costs (CPU, wakeups, memory) per scenario, on your real
# Wayland session with a demo player on a private D-Bus; a second notch
# shows while it runs. Not part of `check`: it needs a display and the
# numbers depend on the machine. Report in build/bench.md.
bench:
	scripts/bench.sh

# --unqualified disable: delegates read outer ids by design.
# The filtered line is a qmllint false positive: it ignores a qmldir without
# a `module` line when linting the singleton itself.
lint:
	@$(TIMED) lint $(MAKE) -s --no-print-directory lint-run

lint-run:
	@test -x "$(QMLLINT)" || { echo "Qt 6 qmllint not found" >&2; exit 1; }
	@out=$$("$(QMLLINT)" --unqualified disable $(LINT_FILES) 2>&1 \
	  | grep -v 'Type Theme not declared as singleton in qmldir'); \
	if printf '%s\n' "$$out" | grep -qE '^(Warning|Error)'; then \
	  printf '%s\n' "$$out"; exit 1; fi; \
	echo "qmllint: clean"
	@python3 -c 'import ast, sys; ast.parse(open(sys.argv[1]).read(), sys.argv[1])' core/live/codex-live.py \
	  && echo "codex-live.py: parses"
	@python3 -c 'import ast, sys; ast.parse(open(sys.argv[1]).read(), sys.argv[1])' core/live/local-live.py \
	  && echo "local-live.py: parses"
	@python3 -c 'import ast, sys; ast.parse(open(sys.argv[1]).read(), sys.argv[1])' core/components/localai.py \
	  && echo "localai.py: parses"
	@python3 -c 'import ast, sys; ast.parse(open(sys.argv[1]).read(), sys.argv[1])' core/components/install.py \
	  && echo "install.py: parses"

# The repo root is the Omarchy plugin.
validate:
	@$(TIMED) validate omarchy plugin validate .

# The daemon, release build: what the hosts start with the shell.
daemon:
	cargo build --release -p notchd-daemon --manifest-path native/Cargo.toml

# The standalone host (shell.qml) against a scratch config and its own daemon
# instance (DEV_INSTANCE, the dev config): the user's `default` daemon and
# ~/.config/notchd stay untouched. A dev daemon of an earlier build is stopped
# first, since the daemon outlives the shell; hot-reloads.
dev: daemon
	@mkdir -p build
	@NOTCHD_INSTANCE="$(DEV_INSTANCE)" NOTCHD_CONFIG="$(DEV_CONFIG)" "$(DAEMON_BIN)" ipc shutdown >/dev/null 2>&1 || true
	NOTCHD_INSTANCE="$(DEV_INSTANCE)" NOTCHD_CONFIG="$(DEV_CONFIG)" NOTCHD_BIN="$(DAEMON_BIN)" quickshell -p .

# The GNOME Shell extension (hosts/gnome/) in a GNOME session in a window of
# this desktop (gnome-shell --devkit), with its own daemon, config, D-Bus and
# runtime dir. Extensions do not hot-reload: close the window and run it again.
gnome-dev: daemon
	scripts/gnome-dev.sh

# The extension in a headless GNOME Shell, driven through its test hooks.
gnome-test: daemon
	python3 tests/gnome_test.py

# GNOME 46, the oldest supported (Ubuntu 24.04 LTS), from a podman image
# (tests/containers/gnome-46.Containerfile). The daemon built here runs
# there (its glibc floor is 24.04's). gnome-dev-46 shows the session in an
# X11 window through Xwayland (DISPLAY): GNOME 46 has no devkit.
GNOME46_IMAGE = notchd-gnome-46
GNOME46_RUN = podman run --rm --tmpfs /tmp -v "$(CURDIR)":/src $(GNOME46_IMAGE) bash -c 'dbus-daemon --system --fork &&
gnome-image-46:
	podman build -t $(GNOME46_IMAGE) -f tests/containers/gnome-46.Containerfile tests/containers

gnome-test-46: daemon gnome-image-46
	$(GNOME46_RUN) python3 tests/gnome_test.py'

gnome-dev-46: daemon gnome-image-46
	podman run --rm --ipc=host -v "$(CURDIR)":/src -v /tmp/.X11-unix:/tmp/.X11-unix -e DISPLAY="$(DISPLAY)" \
	  $(GNOME46_IMAGE) bash -c 'dbus-daemon --system --fork && scripts/gnome-dev.sh --nested'

# The extension as one folder and the zip extensions.gnome.org takes, with
# the UI-side core/ files copied in as ES modules (build/gnome-ext/).
gnome-zip:
	@dir=$$(scripts/gnome-build.sh build/gnome-ext) && rm -f "$$dir/testhook.js" && \
	  (cd "$$dir" && rm -f ../notchd-gnome.zip && zip -qr ../notchd-gnome.zip .) && echo "build/gnome-ext/notchd-gnome.zip"

# The Omarchy shell does not hot-reload plugin QML. Builds the daemon, stops
# the running one (it outlives the shell, so it would keep the old build) and
# copies the build to $XDG_DATA_HOME/notchd/bin/notchd, where the plugin looks
# when NOTCHD_BIN does not reach the shell. Installs the optional packages
# first (deps).
# The daemon is replaced only when the build differs from the installed one:
# an unchanged daemon keeps running, with its terminal and agent sessions.
restart: deps daemon
	@installed="$${XDG_DATA_HOME:-$$HOME/.local/share}/notchd/bin/notchd"; \
	if cmp -s "$(DAEMON_BIN)" "$$installed"; then \
	  echo "notchd unchanged: the daemon and its sessions stay"; \
	else \
	  "$(DAEMON_BIN)" ipc shutdown >/dev/null 2>&1 || true; \
	  install -D -m 755 "$(DAEMON_BIN)" "$$installed"; \
	  echo "notchd rebuilt: the daemon restarts (its sessions end)"; \
	fi
	NOTCHD_BIN="$(DAEMON_BIN)" omarchy restart shell

# Optional packages the modules use: qmltermwidget, the codex module's
# terminal. Installed with pacman only when missing (sudo asks then); without
# pacman, says what to install.
OPTIONAL_PACKAGES = qmltermwidget
deps:
	@for pkg in $(OPTIONAL_PACKAGES); do \
	  if ! command -v pacman >/dev/null 2>&1; then \
	    echo "notchd: install $$pkg with your package manager for the codex module" >&2; \
	  elif ! pacman -Q "$$pkg" >/dev/null 2>&1; then \
	    echo "notchd: installing $$pkg (the codex module's terminal)"; \
	    sudo pacman -S --needed --noconfirm "$$pkg"; \
	  fi; \
	done

# Nothing else is installed from here. What the voice modules need beyond the
# app (LocalAI and its voices, Voxtype, system packages) is installed, removed
# and configured from notchd's settings, Components page (core/components/).

clean:
	rm -rf build
