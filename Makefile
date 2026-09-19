# Makefile for Virtuoso (Phase 2)
#
# Phase 2 起 Rust workspace 是唯一行为权威；本文件只是转发壳，
# 保留 make 旧习惯。实际逻辑见 xtask/ 与 crates/（AGENTS.md 有 Code Map）。

-include .env

QEMU_TIMEOUT ?= 0

.PHONY: all help verify busybox qemu qemu-kvm qemu-debug initrd qemu-test clean install-skill uninstall-skill

all: help

help:
	@echo "Virtuoso — kernel E2E virtualization test harness (Phase 2: Rust authoritative)"
	@echo ""
	@echo "All targets forward to 'cargo xtask' (single behavior source)."
	@echo ""
	@echo "Available targets:"
	@echo "  verify        - Check prerequisites before building/running"
	@echo "  busybox       - Ensure static BusyBox for ARCH (release download / source fallback)"
	@echo "  qemu          - Start QEMU VM"
	@echo "  qemu-kvm      - Start QEMU VM with KVM acceleration"
	@echo "  qemu-debug    - Start QEMU with GDB stub"
	@echo "  qemu-test     - Start QEMU with timeout (for CI)"
	@echo "  initrd        - Rebuild initrd.img + rootfs.img + tools.img (two-stage boot pair + tools disk)"
	@echo "  install-skill - Install kernel-dev skill for Claude Code"
	@echo "  uninstall-skill - Remove kernel-dev skill from Claude Code"
	@echo "  clean         - Remove generated images"
	@echo ""
	@echo "Examples:"
	@echo "  make qemu                       # Run interactively"
	@echo "  make qemu-kvm                   # Run with KVM acceleration"
	@echo "  make qemu-debug                 # Debug with GDB"
	@echo "  make qemu-test QEMU_TIMEOUT=60  # Auto-test with 60s timeout"
	@echo "  make install-skill              # Install skill into kernel tree"

verify:
	@cargo xtask verify

busybox:
	@cargo xtask busybox

qemu:
	@cargo xtask shell

qemu-kvm:
	@cargo xtask shell --kvm

qemu-debug:
	@cargo xtask debug

qemu-test:
	@if [ "$(QEMU_TIMEOUT)" = "0" ] || [ -z "$(QEMU_TIMEOUT)" ]; then \
		echo "ERROR: Set QEMU_TIMEOUT (e.g., make qemu-test QEMU_TIMEOUT=60)"; \
		exit 1; \
	fi
	@cargo xtask test --timeout $(QEMU_TIMEOUT)

initrd:
	@cargo xtask build

install-skill:
	@cargo xtask skill install

uninstall-skill:
	@cargo xtask skill uninstall

clean:
	@cargo xtask clean
