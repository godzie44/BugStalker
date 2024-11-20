build:
	cargo build

build-test:
	cargo build --features "int_test"

build-examples:
	cd examples; \
	cargo build -p calc_lib; \
	$(SHLIB_SO_PATH) cargo build; \
	cargo build --manifest-path tokio_tcp/tokio_tcp_1_40/Cargo.toml; \
	cargo build --manifest-path tokio_tcp/tokio_tcp_1_41/Cargo.toml; \

build-all: build build-examples

cargo-test:
	cargo test --features "int_test"

cargo-test-no-libunwind:
	cargo test --no-default-features --features "int_test"

int-test: build-test
	sudo python3 -m unittest discover ./tests/integration/ -v

int-test-external: build-test
	sudo python3 -m unittest discover ./tests/integration/ -v -p "*external*"

# for local usage, note that test_external.py requires a root privileges
test: build-all cargo-test int-test
