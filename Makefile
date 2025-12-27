build:
	cargo build

build-rel:
	cargo build --release

build-test:
	cargo build --features "int_test"

build-test-rel:
	cargo build --release --features "int_test"

RUST_VERSION ?= stable

build-examples-for-func-test:
	cd examples; \
	cargo +$(RUST_VERSION) build -p calc_lib; \
	$(SHLIB_SO_PATH) cargo +$(RUST_VERSION) build; \

build-examples: build-examples-for-func-test
	cd examples; \
	cargo +$(RUST_VERSION) build --manifest-path tokio_tcp/tokio_1_40/Cargo.toml; \
	cargo +$(RUST_VERSION) build --manifest-path tokio_tcp/tokio_1_41/Cargo.toml; \
	cargo +$(RUST_VERSION) build --manifest-path tokio_tcp/tokio_1_42/Cargo.toml; \
	cargo +$(RUST_VERSION) build --manifest-path tokio_tcp/tokio_1_43/Cargo.toml; \
	cargo +$(RUST_VERSION) build --manifest-path tokio_tcp/tokio_1_44/Cargo.toml; \
	cargo +$(RUST_VERSION) build --manifest-path tokio_vars/tokio_1_40/Cargo.toml; \
	cargo +$(RUST_VERSION) build --manifest-path tokio_vars/tokio_1_41/Cargo.toml; \
	cargo +$(RUST_VERSION) build --manifest-path tokio_vars/tokio_1_42/Cargo.toml; \
	cargo +$(RUST_VERSION) build --manifest-path tokio_vars/tokio_1_43/Cargo.toml; \
	cargo +$(RUST_VERSION) build --manifest-path tokio_vars/tokio_1_44/Cargo.toml; \

build-all: build build-examples

build-all-rel: build-rel build-examples

cargo-test:
	cargo test --features "int_test"

dap-tests: build-examples-for-func-test
	cargo build --bin bs_dap --features "int_test"
	cargo test --test debugger --features "int_test"

int-test-external: build-test
	sudo python3 -m unittest discover ./tests/integration/ -v -p "*external*"

int-test-async: build-test
	python3 -m unittest discover ./tests/integration/ -v -p "*async*"

int-test: build-test
	sudo python3 -m unittest discover ./tests/integration/ -v

int-test-rel: build-test-rel
	sudo python3 -m unittest discover ./tests/integration/ -v

# for local usage, note that test_external.py requires a root privileges
test: build-all cargo-test int-test

test-rel: build-all-rel cargo-test int-test-rel

install:
	cargo install --path .
