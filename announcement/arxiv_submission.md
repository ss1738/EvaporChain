# Publishing on arXiv

## Steps:
1. Go to https://arxiv.org/user/register — create account
2. Go to https://arxiv.org/submit
3. Category: cs.CR (Cryptography and Security) or cs.DC (Distributed Computing)
4. Title: "EvaporChain: Thermodynamic State Decay with Recursive Proof Folding for Sustainable Blockchain Architecture"
5. Authors: Satyawan Singh
6. Abstract: Copy from whitepaper
7. Upload: Convert EVAPORCHAIN_WHITEPAPER.md to PDF first
8. Submit

## Converting whitepaper to PDF:
Use pandoc:
```
pandoc EVAPORCHAIN_WHITEPAPER.md -o evaporchain_whitepaper.pdf \
  --pdf-engine=xelatex \
  -V geometry:margin=1in \
  -V fontsize=11pt \
  -V documentclass=article
```

Or use an online Markdown-to-PDF converter.
