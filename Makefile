# BLAM! build system
# Ensures the pinned Rust toolchain is used regardless of system PATH

CARGO := ./scripts/cargo.sh

.PHONY: build release test run clean

build:
	$(CARGO) build

release:
	$(CARGO) build --release

test:
	$(CARGO) test

run:
	$(CARGO) run

clean:
	$(CARGO) clean
