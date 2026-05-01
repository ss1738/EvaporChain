// lookup_confirm_word.js — Maestro runScript helper.
//
// At the seed-confirmation step, three random "Word #N" prompts are rendered.
// Each TextInput has testID `create-confirm-word-input-{N-1}` (0-indexed).
// We can't enumerate which 3 indices were picked from a flow, so the parent
// flow tries every i in 0..23 — this script returns the matching word from
// the previously-captured `seedN` outputs, defaulting to empty string if the
// input for that index isn't on screen.
//
// Inputs (set via `env:` in the flow):
//   INDEX        — 0..23 zero-based seed-word index to look up
//   SEED_WORDS   — JSON array of all 24 words, captured by capture_seed.js
//
// Output:
//   word         — the matching word, or "" if INDEX is out of range
const idx = parseInt(INDEX, 10);
const words = JSON.parse(SEED_WORDS);
output.word = (idx >= 0 && idx < words.length) ? words[idx] : '';
