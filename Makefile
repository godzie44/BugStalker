build:
	cargo build

build-test:
	cargo build --features "int_test, no_libunwind"

build-all: build
	cargo build --bin calc
	cargo build --bin hello_world
	cargo build --bin mt
	cargo build --bin vars
	cargo build --bin recursion
	cd examples ; cargo build -p calc_lib ; cargo build

cargo-test:
	cargo test --features "int_test"

cargo-test-no-libunwind:
	cargo test --features "int_test, no_libunwind"

int-test: build-test
	python3 -m unittest discover ./tests/integration/ -v

test: build-all cargo-test int-test