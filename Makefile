build:
	cargo build

build-all: build
	cargo build --bin calc
	cargo build --bin hello_world
	cargo build --bin mt
	cargo build --bin vars

func-test:
	cargo test

int-test:
	python3 -m unittest discover ./tests/integration/ -v

test: build-all func-test int-test