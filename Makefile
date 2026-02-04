# BLAM! build system
# Ensures correct Rust toolchain is used regardless of system PATH

CARGO := $(HOME)/.cargo/bin/cargo
RUSTC := $(HOME)/.cargo/bin/rustc
export RUSTC

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
