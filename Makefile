CARGO ?= cargo
PREFIX ?= $(HOME)/.local
DESTDIR ?=

BIN := mntk
RELEASE_BIN := target/release/$(BIN)

.PHONY: all build release install uninstall check test clippy fmt fmt-check clean

all: build

build:
	$(CARGO) build --locked

release:
	$(CARGO) build --release --locked

install: release
	install -d $(DESTDIR)$(PREFIX)/bin
	install -m 0755 $(RELEASE_BIN) $(DESTDIR)$(PREFIX)/bin/$(BIN)

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/$(BIN)

check:
	$(CARGO) check --workspace --locked

test:
	$(CARGO) test --workspace --locked

clippy:
	$(CARGO) clippy --workspace --all-targets --locked

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all --check

clean:
	$(CARGO) clean
