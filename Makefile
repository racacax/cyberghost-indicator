# cyberghost-indicator — `make help` lists the targets.
# Requirements: rustup (the toolchain is pinned in rust-toolchain.toml and
# installed automatically), zip; dbus-daemon for the end-to-end tests;
# python3 + Pillow + fonts-noto-color-emoji only for `make icons`.

SHELL   := bash
CARGO   ?= cargo
TARGET  ?= x86_64-unknown-linux-musl
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
NAME    := cyberghost-indicator-$(VERSION)-x86_64-linux
ZIP     := dist/$(NAME).zip
export PATH := $(HOME)/.cargo/bin:$(PATH)

.DEFAULT_GOAL := help
.PHONY: help setup build run test test-unit test-e2e test-package test-install test-all check fmt icons package install uninstall clean

help: ## Show this help
	@grep -E '^[a-z0-9-]+:.*## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*## "} {printf "  \033[36m%-13s\033[0m %s\n", $$1, $$2}'

setup: ## Install the pinned Rust toolchain and targets (rustup)
	@command -v rustup >/dev/null || { echo "rustup is required: https://rustup.rs" >&2; exit 1; }
	rustup toolchain install

build: ## Static release binary (TARGET=x86_64-unknown-linux-gnu for glibc)
	TARGET=$(TARGET) scripts/build.sh

run: ## Run the indicator from the source tree
	$(CARGO) run

test: ## Unit + end-to-end tests
	$(CARGO) test

test-unit: ## Unit tests only
	$(CARGO) test --lib

test-e2e: ## End-to-end tests only (private D-Bus, mocked CLI)
	$(CARGO) test --test 'e2e_*'

test-package: package ## Check the release ZIP
	tests/package_test.sh $(ZIP)

test-install: package ## Installer / uninstaller tests (sandboxed, touches nothing)
	tests/install_test.sh dist/$(NAME)

test-all: check test test-package test-install ## Everything (what CI should run)

check: ## rustfmt + clippy
	$(CARGO) fmt --check
	$(CARGO) clippy --all-targets -- -D warnings

fmt: ## Format the code
	$(CARGO) fmt

icons: ## Regenerate assets/icons from Noto Color Emoji
	python3 tools/render-icons.py

package: ## Build the release ZIP in dist/
	TARGET=$(TARGET) scripts/package.sh

install: package ## Install for the current user from the freshly built release
	dist/$(NAME)/install.sh

uninstall: ## Uninstall (per-user install)
	$(HOME)/.local/share/cyberghost-indicator/uninstall.sh

clean: ## Remove build outputs
	$(CARGO) clean
	rm -rf dist
