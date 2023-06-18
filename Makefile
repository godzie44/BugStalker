build:
	cargo build

build-test:
	cargo build --features "int_test"

build-all: build
	cargo build --bin calc
	cargo build --bin hello_world
	cargo build --bin mt
	cargo build --bin vars
	cargo build --bin recursion
	cd examples && cargo build

cargo-test:
	cargo test --features "int_test"

int-test:
	python3 -m unittest discover ./tests/integration/ -v

test: build-all build-test cargo-test int-test