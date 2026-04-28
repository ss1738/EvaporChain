# EvaporChain — Coq mechanization
#
# Build:    make
# Check:    make check  (alias of `make`)
# Clean:    make clean
#
# Requires Coq 8.18+ (for [Lia] and modern tactic library). Runs against
# `_CoqProject` so file-list updates only need editing there.

COQMAKEFILE = CoqMakefile
COQPROJECT  = _CoqProject

all: $(COQMAKEFILE)
	@$(MAKE) -f $(COQMAKEFILE) $(MAKEFLAGS)

check: all

$(COQMAKEFILE): $(COQPROJECT)
	coq_makefile -f $(COQPROJECT) -o $(COQMAKEFILE)

clean:
	@if [ -f $(COQMAKEFILE) ]; then $(MAKE) -f $(COQMAKEFILE) cleanall; fi
	@rm -f $(COQMAKEFILE) $(COQMAKEFILE).conf .CoqMakefile.d

.PHONY: all check clean
