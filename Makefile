build:
	cargo build

build-test:
	cargo build --features "int_test"

build-all: build
	cd examples ; cargo build -p calc_lib ; cargo build

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
