PREFIX ?= $(HOME)/.local

.PHONY: build release install verify clean

build:
	cargo build -p atelier-cli

release:
	cargo build --release -p atelier-cli

install:
	scripts/install.sh "$(PREFIX)"

verify:
	cargo build --workspace
	bin/atelier help >/dev/null

clean:
	rm -rf target
