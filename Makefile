PREFIX ?= $(HOME)/.local

.PHONY: build release install verify clean

build:
	cargo build -p rebotica-cli

release:
	cargo build --release -p rebotica-cli

install:
	scripts/install.sh "$(PREFIX)"

verify:
	cargo build --workspace
	bin/rbtc help >/dev/null

clean:
	rm -rf target
