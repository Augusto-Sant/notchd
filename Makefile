# Development entry points. `make check` is the gate: run it before every
# commit. See DEVELOPMENT.md.

QMLLINT ?= $(firstword $(wildcard /usr/lib/qt6/bin/qmllint /usr/lib64/qt6/bin/qmllint) \
	$(shell command -v qmllint6 2>/dev/null) $(shell command -v qmllint 2>/dev/null))

# The Omarchy host imports qs.Commons/qs.Ui, which only exist inside the
# Omarchy shell, so lint skips it; `make restart` and the shell log cover it.
LINT_FILES = shell.qml core/*.qml core/settings/*.qml core/settings/previews/*.qml \
	core/modules/*.qml
# Not core/terminal/: it imports QMLTermWidget, whose package ships no type
# information for qmllint. The QML tests load it where it is installed.

# Dev config for `make dev`, so it never touches ~/.config/notchd.
DEV_CONFIG ?= $(CURDIR)/build/dev-config.json

.PHONY: check test qmltest visual hero icons bench lint validate dev restart deps live-deps local-live-deps live-models clean

check: test qmltest lint validate visual

# Node tests of the pure-JS rules (core/*.js, core/settings/*.js) and the
# live bridge's that need no GStreamer.
test:
	node --test tests/*.test.mjs
	python3 tests/codex_live_test.py
	python3 tests/local_live_test.py

# QML behaviour tests (tests/qml/shell.qml), offscreen: bindings, signals
# and animations the Node tests can't reach. Regression tests live here.
qmltest:
	scripts/qml-test.sh

# Renders the settings window offscreen into build/visual/; fails on any QML
# warning, so it also proves the core loads.
visual:
	scripts/visual-test.sh

# The README image, docs/images/hero.webp: the real notch offscreen, with a
# demo media player on a private D-Bus, over Omarchy's Gruvbox wallpaper
# (WALLPAPER=<image> for another). Rerun after a visual change to the notch
# and commit the image.
hero:
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

# The repo root is the Omarchy plugin.
validate:
	omarchy plugin validate .

# The standalone host (shell.qml) against a scratch config; hot-reloads.
dev:
	@mkdir -p build
	NOTCHD_CONFIG="$(DEV_CONFIG)" quickshell -p .

# The Omarchy shell does not hot-reload plugin QML. Installs the optional
# packages first (deps).
restart: deps
	omarchy restart shell

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

# What a spoken conversation (the codex-live module, core/live/codex-live.py)
# needs: GStreamer's WebRTC (gst-plugins-bad, libnice), Opus over RTP
# (gst-plugins-good) and the microphone and speakers through pipewire-pulse
# (pulsesrc, pulsesink), Python's GObject.
# About 8 MB; not part of deps, as only that module uses them.
LIVE_PACKAGES = python-gobject gst-plugins-base gst-plugins-good gst-plugins-bad libnice libpulse
live-deps:
	@if ! command -v pacman >/dev/null 2>&1; then \
	  echo "notchd: install $(LIVE_PACKAGES) (or their equivalents) for Codex Live" >&2; exit 1; fi
	@missing=; for pkg in $(LIVE_PACKAGES); do pacman -Q "$$pkg" >/dev/null 2>&1 || missing="$$missing $$pkg"; done; \
	if [ -z "$$missing" ]; then echo "notchd: Codex Live's packages are installed"; \
	else echo "notchd: installing$$missing (Codex Live: WebRTC and the microphone)"; \
	  sudo pacman -S --needed$$missing; fi

# What the local voice (the local-live module, core/live/local-live.py)
# needs: pacat and parec (libpulse), whisper.cpp's whisper-cli (whisper-cpp,
# official; CPU) and Piper (`piper`, PyPI's piper-tts, installed with uv or
# pipx into ~/.local/bin: the AUR's piper-tts-bin is the archived 2023
# release, and the `piper` package is a mouse tool). Models: `make live-models`.
LOCAL_LIVE_PACKAGES = libpulse whisper-cpp
local-live-deps:
	@if ! command -v pacman >/dev/null 2>&1; then \
	  echo "notchd: install $(LOCAL_LIVE_PACKAGES) (or their equivalents) and piper-tts (PyPI) for the local voice" >&2; exit 1; fi
	@missing=; for pkg in $(LOCAL_LIVE_PACKAGES); do pacman -Q "$$pkg" >/dev/null 2>&1 || missing="$$missing $$pkg"; done; \
	if [ -n "$$missing" ]; then echo "notchd: installing$$missing (the local voice)"; sudo pacman -S --needed$$missing; fi
	@if command -v piper >/dev/null 2>&1 || [ -x "$(HOME)/.local/bin/piper" ]; then :; \
	elif command -v uv >/dev/null 2>&1; then echo "notchd: installing piper-tts (uv tool)"; uv tool install piper-tts; \
	elif command -v pipx >/dev/null 2>&1; then echo "notchd: installing piper-tts (pipx)"; pipx install piper-tts; \
	else echo "notchd: install uv or pipx, then piper-tts (the local voice's Piper)" >&2; exit 1; fi
	@echo "notchd: the local voice's packages are installed"

# The local voice's default models, into $XDG_DATA_HOME/notchd/models (not the
# repo): whisper.cpp's ggml-small (about 470 MB) and Piper's pt_BR faber voice
# (about 60 MB). Existing files are kept; WHISPER_MODEL=base|small|medium.
MODELS_DIR ?= $(or $(XDG_DATA_HOME),$(HOME)/.local/share)/notchd/models
WHISPER_MODEL ?= small
WHISPER_URL = https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-$(WHISPER_MODEL).bin
VOICE_URL = https://huggingface.co/rhasspy/piper-voices/resolve/main/pt/pt_BR/faber/medium/pt_BR-faber-medium
live-models:
	@mkdir -p "$(MODELS_DIR)"
	@for f in "ggml-$(WHISPER_MODEL).bin $(WHISPER_URL)" "pt_BR-faber-medium.onnx $(VOICE_URL).onnx" "pt_BR-faber-medium.onnx.json $(VOICE_URL).onnx.json"; do \
	  set -- $$f; \
	  if [ -s "$(MODELS_DIR)/$$1" ]; then echo "notchd: $$1 is there"; \
	  else echo "notchd: downloading $$1"; curl -fL --progress-bar -o "$(MODELS_DIR)/$$1.part" "$$2" && mv "$(MODELS_DIR)/$$1.part" "$(MODELS_DIR)/$$1" || { rm -f "$(MODELS_DIR)/$$1.part"; exit 1; }; fi; \
	done

clean:
	rm -rf build
