// capture_seed.js — Maestro runScript helper.
//
// Reads all 24 seed words from the on-screen `create-seed-word-{i}` testIDs
// and writes each to `output.seedN` (1-indexed). The confirm-step flow reads
// these back when prompted for "Word #N".
//
// Maestro's runScript has access to `maestro.copyText(testId)` which returns
// the accessibility text of the matching node (or null if not present).
//
// Reference: https://maestro.mobile.dev/api-reference/commands/runscript
for (let i = 0; i < 24; i++) {
  const word = maestro.copyText('create-seed-word-' + i);
  output['seed' + (i + 1)] = word;
}
