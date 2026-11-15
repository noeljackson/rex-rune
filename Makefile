.DEFAULT_GOAL := all

CARGO ?= cargo
RUSTC ?= rustc
ZIG ?= zig
CARGO_TARGET_DIR ?= target
REX_BIN ?= $(CARGO_TARGET_DIR)/$(if $(CARGO_BUILD_TARGET),$(CARGO_BUILD_TARGET)/)release/rex-linux
TEST_ARGS ?= --test-threads=1
RUN_ARGS ?=

# Empty PREFIX preserves the installer's XDG paths (normally ~/.local).
# Explicit PREFIX selects PREFIX/bin and PREFIX/share; BINDIR/DATADIR override them.
export CARGO RUSTC ZIG CARGO_TARGET_DIR REX_BIN PREFIX BINDIR DATADIR DESTDIR
# Cargo rejects an empty target, so do not export this optional override unset.
ifneq ($(strip $(CARGO_BUILD_TARGET)),)
export CARGO_BUILD_TARGET
else
unexport CARGO_BUILD_TARGET
endif

.PHONY: all build release toolchain install install-only uninstall run clean \
	test fmt clippy check test-rexrs test-install help
.NOTPARALLEL: check

all: build
release: build

toolchain:
	@bash scripts/check-toolchain.sh

build: toolchain
	"$$CARGO" build --locked --release --bin rex-linux

install: build
install install-only:
	bash scripts/install-desktop.sh --copy "$$REX_BIN"

uninstall:
	bash scripts/install-desktop.sh --uninstall

run: build
	"$$REX_BIN" $(RUN_ARGS)

clean:
	"$$CARGO" clean

test: toolchain
	"$$CARGO" test --locked --all-features -- $(TEST_ARGS)

fmt:
	"$$CARGO" fmt --all -- --check

clippy: toolchain
	"$$CARGO" clippy --locked --all-targets --all-features -- -D warnings

test-rexrs: toolchain
	"$$CARGO" test --manifest-path vendor/rexrs/Cargo.toml --all-features -- $(TEST_ARGS)

test-install:
	python3 scripts/test-make-install.py
	python3 scripts/test-install-receipt.py

check: fmt clippy test test-rexrs test-install

help:
	@printf '%s\n' \
	  'make [build|release]   Build the locked release application (default)' \
	  'make install           Build and install binary, launcher and icons' \
	  'make install-only      Install an already-built REX_BIN; no toolchain needed' \
	  'make uninstall         Remove installed files; preserve preferences' \
	  'make run RUN_ARGS=...  Build and launch the application' \
	  'make test|test-rexrs   Run tests serially; override TEST_ARGS as needed' \
	  'make fmt|clippy|check  Check formatting, lint, or run all checks' \
	  'make test-install      Exercise installation in isolated temporary paths' \
	  'make clean             Remove Cargo build outputs only' \
	  '' \
	  'Install defaults to XDG paths, normally ~/.local (no root required).' \
	  'Overrides: PREFIX, BINDIR, DATADIR, DESTDIR (absolute paths).' \
	  'Build overrides: CARGO, RUSTC, ZIG, CARGO_TARGET_DIR, CARGO_BUILD_TARGET, REX_BIN.'
