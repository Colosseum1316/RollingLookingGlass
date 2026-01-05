CARGO ?= cargo
PROFILE ?= release
FEATURES ?= --all-features

CLIPPY_FLAGS = $(FEATURES) -- -D warnings
TARPAULIN_FLAGS = --run-types AllTargets --out lcov --out stdout

.PHONY: all build test lint lint-fix fmt fmt-check clean ci help

all: lint build

build:
	$(CARGO) build --$(PROFILE)

test:
	$(CARGO) tarpaulin --$(PROFILE) $(TARPAULIN_FLAGS)

lint:
	$(CARGO) clippy $(CLIPPY_FLAGS)

lint-fix:
	$(CARGO) clippy $(FEATURES) --fix

fmt:
	$(CARGO) fmt

fmt-check:
	$(CARGO) fmt -- --check

clean:
	$(CARGO) clean

ci: fmt-check lint test build

help:
	@echo "Targets:"
	@echo "  build       Build project ($(PROFILE))"
	@echo "  test        Run tests with tarpaulin"
	@echo "  lint        Run clippy (deny warnings)"
	@echo "  lint-fix    Auto-fix clippy warnings"
	@echo "  fmt         Format code"
	@echo "  fmt-check   Check formatting"
	@echo "  clean       Clean build artifacts"
	@echo "  ci          Run all CI checks"
