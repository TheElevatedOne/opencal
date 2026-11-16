# opencal: build and install
#   make                     build (release)
#   make install PREFIX=/usr DESTDIR=pkgdir
#   make check               unit tests + self-test (77 = no display, skipped)

PREFIX  ?= /usr/local
DESTDIR ?=
CARGO   ?= cargo
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
MAJOR   := $(firstword $(subst ., ,$(VERSION)))
TARGET  ?= target/release

BINDIR  = $(DESTDIR)$(PREFIX)/bin
LIBDIR  = $(DESTDIR)$(PREFIX)/lib
INCDIR  = $(DESTDIR)$(PREFIX)/include
DATADIR = $(DESTDIR)$(PREFIX)/share

.PHONY: all build check install uninstall clean

all: build

build:
	$(CARGO) build --release --workspace --locked

check:
	$(CARGO) test --workspace --locked
	$(TARGET)/opencal --self-test || [ $$? -eq 77 ]

install:
	install -Dm755 $(TARGET)/opencal $(BINDIR)/opencal
	install -Dm755 $(TARGET)/libopencal.so $(LIBDIR)/libopencal.so.$(VERSION)
	ln -sf libopencal.so.$(VERSION) $(LIBDIR)/libopencal.so.$(MAJOR)
	ln -sf libopencal.so.$(MAJOR) $(LIBDIR)/libopencal.so
	install -Dm644 core/include/opencal.h $(INCDIR)/opencal.h
	install -d $(LIBDIR)/pkgconfig
	sed -e 's|@PREFIX@|$(PREFIX)|' -e 's|@VERSION@|$(VERSION)|' core/opencal.pc.in > $(LIBDIR)/pkgconfig/opencal.pc
	chmod 644 $(LIBDIR)/pkgconfig/opencal.pc
	install -Dm644 data/org.opencal.OpenCal.desktop $(DATADIR)/applications/org.opencal.OpenCal.desktop
	install -Dm644 data/org.opencal.OpenCal.metainfo.xml $(DATADIR)/metainfo/org.opencal.OpenCal.metainfo.xml
	install -Dm644 data/icons/org.opencal.OpenCal.svg $(DATADIR)/icons/hicolor/scalable/apps/org.opencal.OpenCal.svg
	install -d $(DATADIR)/doc/opencal/examples/sync $(DATADIR)/doc/opencal/examples/c
	install -m755 examples/sync/*.sh $(DATADIR)/doc/opencal/examples/sync/
	install -m644 examples/c/* $(DATADIR)/doc/opencal/examples/c/

uninstall:
	rm -f $(BINDIR)/opencal $(LIBDIR)/libopencal.so* $(INCDIR)/opencal.h $(LIBDIR)/pkgconfig/opencal.pc
	rm -f $(DATADIR)/applications/org.opencal.OpenCal.desktop $(DATADIR)/metainfo/org.opencal.OpenCal.metainfo.xml
	rm -f $(DATADIR)/icons/hicolor/scalable/apps/org.opencal.OpenCal.svg
	rm -rf $(DATADIR)/doc/opencal

clean:
	$(CARGO) clean
	rm -rf dist
