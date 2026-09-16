# Zava — build, install and package.
#
#   make            debug build
#   make release    optimized build
#   make test       run the test suite
#   make check      fmt + clippy (-D warnings) + tests
#   make install    install under $(PREFIX) (default /usr/local)
#   make uninstall
#   make dist       source tarball used by the PKGBUILD
#
# Staged installs: every install path honours DESTDIR, so packagers can use
#   make install DESTDIR=$pkgdir PREFIX=/usr

PREFIX      ?= /usr/local
BINDIR      ?= $(PREFIX)/bin
DATADIR     ?= $(PREFIX)/share
DOCDIR      ?= $(DATADIR)/doc/zava
MANDIR      ?= $(DATADIR)/man

CARGO       ?= cargo
INSTALL     ?= install
VERSION     := $(shell sed -n 's/^version[[:space:]]*=[[:space:]]*"\(.*\)"/\1/p' Cargo.toml | head -n1)
TARBALL     := zava-$(VERSION).tar.gz

DIST_FILES  := Cargo.toml Cargo.lock src tests config zava.1 \
               README.md LICENSE .gitignore

.PHONY: all build release test fmt clippy check install uninstall dist clean

all: build

build:
	$(CARGO) build

release:
	$(CARGO) build --release

test:
	$(CARGO) test

fmt:
	$(CARGO) fmt

clippy:
	$(CARGO) clippy --all-targets -- -D warnings

check: fmt clippy test

install: release
	$(INSTALL) -Dm755 target/release/zava $(DESTDIR)$(BINDIR)/zava
	$(INSTALL) -Dm644 config/zava.conf $(DESTDIR)$(DATADIR)/zava/config.example
	$(INSTALL) -Dm644 zava.1 $(DESTDIR)$(MANDIR)/man1/zava.1
	$(INSTALL) -Dm644 README.md $(DESTDIR)$(DOCDIR)/README.md
	$(INSTALL) -Dm644 LICENSE $(DESTDIR)$(DOCDIR)/LICENSE
	@echo "Installed zava $(VERSION). Reference config: $(DATADIR)/zava/config.example"

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/zava
	rm -f $(DESTDIR)$(MANDIR)/man1/zava.1
	rm -rf $(DESTDIR)$(DOCDIR)
	rm -rf $(DESTDIR)$(DATADIR)/zava

dist: clean-dist
	rm -rf .dist
	mkdir -p .dist/zava-$(VERSION)
	cp -r $(DIST_FILES) .dist/zava-$(VERSION)/
	tar -C .dist -czf $(TARBALL) zava-$(VERSION)
	rm -rf .dist
	@echo "Created $(TARBALL)"

clean-dist:
	rm -f $(TARBALL)

clean: clean-dist
	$(CARGO) clean