# Makefile for QEMU E2E Testing Framework
# Basic skeleton for kernel feature testing

-include .env

QEMU_TEST_DIR := infra
QEMU_TIMEOUT ?= 0
AUTO_TEST ?= 1

# Auto-detect KERNEL_PATH (one level above qemu-e2e/), or use .env override
PROJECT_ROOT := $(shell pwd)
KERNEL_PATH_ABS ?= $(shell cd $(PROJECT_ROOT)/.. && pwd)

# Skill installation path (project-scoped, discovered when Claude runs from kernel tree)
CLAUDE_SKILLS_DIR := $(KERNEL_PATH_ABS)/.claude/skills
SKILL_NAME := kernel-dev
SKILL_DST := $(CLAUDE_SKILLS_DIR)/$(SKILL_NAME)/SKILL.md

.PHONY: all help verify busybox qemu qemu-kvm qemu-debug disk initrd qemu-test clean install-skill uninstall-skill

all: help

help:
	@echo "QEMU E2E Test Environment"
	@echo ""
	@echo "Available targets:"
	@echo "  verify        - Check prerequisites before building/running"
	@echo "  busybox       - Ensure static BusyBox for ARCH (release download / source fallback)"
	@echo "  qemu          - Start QEMU VM"
	@echo "  qemu-kvm      - Start QEMU VM with KVM acceleration"
	@echo "  qemu-debug    - Start QEMU with GDB stub"
	@echo "  qemu-test     - Start QEMU with timeout (for CI)"
	@echo "  disk          - Create disk.qcow2 (block device for testing)"
	@echo "  initrd        - Rebuild initrd.img + rootfs.img (two-stage boot pair)"
	@echo "  install-skill - Install kernel-dev skill for Claude Code"
	@echo "  uninstall-skill - Remove kernel-dev skill from Claude Code"
	@echo "  clean         - Remove generated images"
	@echo ""
	@echo "Variables:"
	@echo "  KERNEL_IMAGE   - Path to kernel image (default: $(KERNEL_IMAGE))"
	@echo "  QEMU_TIMEOUT   - Timeout for qemu-test in seconds (0 = no timeout)"
	@echo ""
	@echo "Examples:"
	@echo "  make qemu                       # Run interactively"
	@echo "  make qemu-kvm                   # Run with KVM acceleration"
	@echo "  make qemu-debug                 # Debug with GDB"
	@echo "  make qemu-test QEMU_TIMEOUT=60  # Auto-test with 60s timeout"
	@echo "  make install-skill              # Install skill to \$$KERNEL_PATH/.claude/skills/$(SKILL_NAME)/"

verify:
	@cd $(QEMU_TEST_DIR) && ./verify.sh

busybox:
	@echo "Ensuring per-arch static BusyBox (release download / source fallback)..."
	@cd $(QEMU_TEST_DIR) && ./fetch-busybox.sh

qemu:
	@echo "Starting QEMU..."
	cd $(QEMU_TEST_DIR) && ./run-qemu.sh $(KERNEL_IMAGE)

qemu-kvm:
	@echo "Starting QEMU with KVM acceleration..."
	cd $(QEMU_TEST_DIR) && QEMU_KVM=1 ./run-qemu.sh $(KERNEL_IMAGE)

qemu-debug:
	@echo "Starting QEMU with GDB stub on port 1234..."
	cd $(QEMU_TEST_DIR) && QEMU_DEBUG=1 ./run-qemu.sh $(KERNEL_IMAGE)

qemu-test: initrd
	@if [ "$(QEMU_TIMEOUT)" = "0" ]; then \
		echo "ERROR: Set QEMU_TIMEOUT (e.g., make qemu-test QEMU_TIMEOUT=60)"; \
		exit 1; \
	fi
	@echo "Running QEMU test with $(QEMU_TIMEOUT)s timeout..."
	@cd $(QEMU_TEST_DIR) && { \
		PID_FILE=".qemu_test.pid"; rm -f "$$PID_FILE"; \
		AUTO_TEST=$(AUTO_TEST) timeout --signal=KILL $(QEMU_TIMEOUT) bash -c "echo \$$\$$ > '$$PID_FILE'; exec ./run-qemu.sh '$(KERNEL_IMAGE)'" & \
		QEMU_PGID=$$!; \
		wait $$QEMU_PGID; \
		EXIT_CODE=$$?; \
		rm -f "$$PID_FILE"; \
		if [ $$EXIT_CODE -eq 124 ] || [ $$EXIT_CODE -eq 137 ]; then \
			echo "ERROR: Test timed out after $(QEMU_TIMEOUT) seconds"; \
			kill -- -$$QEMU_PGID 2>/dev/null || true; \
			exit 124; \
		elif [ $$EXIT_CODE -ne 0 ]; then \
			echo "ERROR: Test failed with exit code $$EXIT_CODE"; \
			exit $$EXIT_CODE; \
		else \
			echo "Test completed successfully!"; \
		fi; \
	}

disk:
	@if [ -f $(QEMU_TEST_DIR)/disk.qcow2 ]; then \
		echo "disk.qcow2 already exists."; \
	else \
		echo "Creating disk.qcow2 (512MB block device)"; \
		qemu-img create -f qcow2 $(QEMU_TEST_DIR)/disk.qcow2 512M; \
	fi

initrd:
	@echo "Rebuilding initrd.img (minimal initramfs) + rootfs.img (ext4 rootfs)..."
	cd $(QEMU_TEST_DIR) && ./build-initrd.sh

install-skill:
	@if [ -z "$(KERNEL_PATH)" ]; then \
		echo "ERROR: KERNEL_PATH is not set."; \
		echo "  Copy .env.example to .env and set KERNEL_PATH to your kernel tree."; \
		exit 1; \
	fi
	@echo "Installing $(SKILL_NAME) skill for Claude Code..."
	@echo "  Source: $(PROJECT_ROOT)/skills/$(SKILL_NAME)/SKILL.md"
	@echo "  Destination: $(SKILL_DST)"
	@mkdir -p "$(CLAUDE_SKILLS_DIR)/$(SKILL_NAME)"
	@cp "$(PROJECT_ROOT)/skills/$(SKILL_NAME)/SKILL.md" "$(SKILL_DST)"
	@echo "Done. Claude Code will now recognize the $(SKILL_NAME) skill when running from $(KERNEL_PATH_ABS)/"

uninstall-skill:
	@if [ -d "$(CLAUDE_SKILLS_DIR)/$(SKILL_NAME)" ]; then \
		rm -rf "$(CLAUDE_SKILLS_DIR)/$(SKILL_NAME)"; \
		echo "Removed: $(CLAUDE_SKILLS_DIR)/$(SKILL_NAME)/"; \
	else \
		echo "Skill not installed at $(CLAUDE_SKILLS_DIR)/$(SKILL_NAME)/"; \
	fi

clean:
	rm -f $(QEMU_TEST_DIR)/disk.qcow2 $(QEMU_TEST_DIR)/initrd.img $(QEMU_TEST_DIR)/rootfs.img
	rm -rf $(QEMU_TEST_DIR)/testcases/build
