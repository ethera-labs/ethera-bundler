.DEFAULT_GOAL := help

CARGO ?= cargo
BIN   := ethera-bundler

##@ Help

.PHONY: help
help: ## Show this message.
	@awk 'BEGIN {FS = ":.*##"} /^[a-zA-Z_-]+:.*##/ { printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2 } /^##@/ { printf "\n\033[1m%s\033[0m\n", substr($$0, 5) }' $(MAKEFILE_LIST)

##@ Build

.PHONY: build
build: ## Build all workspace crates (release).
	$(CARGO) build --workspace --release

.PHONY: build-debug
build-debug: ## Build all workspace crates (debug).
	$(CARGO) build --workspace

.PHONY: install
install: ## Install the bundler binary to ~/.cargo/bin.
	$(CARGO) install --path bin/$(BIN) --locked --force

.PHONY: clean
clean: ## Remove build artifacts.
	$(CARGO) clean

##@ Run

.PHONY: run
run: ## Run the bundler in debug mode (loads .env if present).
	@if [ -f .env ]; then set -a && . ./.env && set +a; fi; \
		$(CARGO) run --bin $(BIN)

.PHONY: run-release
run-release: build ## Run the release binary (loads .env if present).
	@if [ -f .env ]; then set -a && . ./.env && set +a; fi; \
		./target/release/$(BIN)

##@ Quality

.PHONY: test
test: ## Run all tests across the workspace.
	$(CARGO) test --workspace --all-targets

.PHONY: fmt
fmt: ## Format all sources in place.
	$(CARGO) fmt --all

.PHONY: fmt-check
fmt-check: ## Check formatting without modifying files.
	$(CARGO) fmt --all -- --check

.PHONY: lint
lint: ## Run clippy across the workspace, fail on warnings.
	$(CARGO) clippy --workspace --all-targets -- -D warnings

.PHONY: lint-fix
lint-fix: ## Auto-apply clippy suggestions where possible.
	$(CARGO) clippy --fix --allow-dirty --allow-staged --workspace --all-targets -- -D warnings

.PHONY: check
check: ## Type-check the workspace without producing binaries.
	$(CARGO) check --workspace --all-targets

.PHONY: deny
deny: ## Run cargo-deny supply-chain checks (requires cargo-deny installed).
	$(CARGO) deny check

.PHONY: machete
machete: ## Find unused workspace dependencies (requires cargo-machete installed).
	$(CARGO) machete

.PHONY: pr
pr: fmt-check lint test ## Run the full pre-PR gate (fmt-check + lint + test).

##@ Tooling

.PHONY: install-tools
install-tools: ## Install cargo-deny, cargo-machete, and pre-commit.
	$(CARGO) install --locked cargo-deny cargo-machete
	@command -v pre-commit >/dev/null 2>&1 || pip install --user pre-commit

.PHONY: install-hooks
install-hooks: ## Wire pre-commit into this clone's .git/hooks.
	pre-commit install

##@ Docker

.PHONY: docker-build
docker-build: ## Build the Docker image.
	docker build -t ethera-bundler:latest .

.PHONY: docker-run
docker-run: ## Run the Docker image with .env mounted.
	docker run --rm -p 8082:8082 --env-file .env ethera-bundler:latest
