# Makefile for tests - wrapper around CMake

BUILD_DIR := build

# Only show warnings and errors from cmake/make output.
# Use PIPESTATUS to propagate the real exit code.
FILTER := 2>&1 | grep -E 'warning:|error:' || true

.PHONY: all clean

all:
	@mkdir -p $(BUILD_DIR) && cd $(BUILD_DIR) && cmake .. > /dev/null 2>&1; \
		EXIT=$${PIPESTATUS[0]}; [ $$EXIT -ne 0 ] && exit $$EXIT; \
		$(MAKE) $(FILTER); \
		EXIT=$${PIPESTATUS[0]}; [ $$EXIT -ne 0 ] && exit $$EXIT; \
		true

clean:
	rm -rf $(BUILD_DIR)
