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
	cd examples ; cargo build -p calc_lib ; cargo build

cargo-test:
	cargo test --features "int_test"

cargo-test-no-libunwind:
	cargo test --no-default-features --features "int_test"

int-test: build-test
	python3 -m unittest discover ./tests/integration/ -v -p "*test_command*"

int-test-external: build-test
	sudo python3 -m unittest discover ./tests/integration/ -v -p "*external*"

# for local usage, because test_external.py requires a root privileges
test: build-all cargo-test
	sudo python3 -m unittest discover ./tests/integration/ -v