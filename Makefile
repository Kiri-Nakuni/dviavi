# make            build dviavi
# make test       Rust and TeX tests
# make install    the TeX files into TEXMFLOCAL (as links to this repository,
#                 so later edits need no reinstall) and mktexlsr; needs root:
#                   sudo env "PATH=$PATH" make install
# make link-bin   ~/.local/bin/dviavi -> target/release/dviavi (no root)
TEXMFLOCAL ?= $(shell kpsewhich -var-value TEXMFLOCAL)
BINDIR ?= $(HOME)/.local/bin

.PHONY: all test install uninstall link-bin
all:
	cargo build --release

test: all
	cargo test --release
	python3 tests/tex/run.py

install:
	test -n "$(TEXMFLOCAL)"
	mkdir -p $(TEXMFLOCAL)/tex/generic/dviavi $(TEXMFLOCAL)/tex/latex/dviavi
	ln -sfn $(CURDIR)/tex/dviavi.tex $(TEXMFLOCAL)/tex/generic/dviavi/dviavi.tex
	ln -sfn $(CURDIR)/tex/dviavi.sty $(TEXMFLOCAL)/tex/latex/dviavi/dviavi.sty
	mktexlsr $(TEXMFLOCAL)

uninstall:
	$(RM) $(TEXMFLOCAL)/tex/generic/dviavi/dviavi.tex $(TEXMFLOCAL)/tex/latex/dviavi/dviavi.sty
	rmdir $(TEXMFLOCAL)/tex/generic/dviavi $(TEXMFLOCAL)/tex/latex/dviavi
	mktexlsr $(TEXMFLOCAL)

link-bin: all
	mkdir -p $(BINDIR)
	ln -sfn $(CURDIR)/target/release/dviavi $(BINDIR)/dviavi
