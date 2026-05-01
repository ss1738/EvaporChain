// serialize_seed.js — Maestro runScript helper.
//
// capture_seed.js wrote 24 separate output.seedN entries. This script
// rolls them up into a single JSON-encoded array on output.seedWords so
// later steps can pass it as an env var to lookup_confirm_word.js.
const arr = [];
for (let i = 1; i <= 24; i++) {
  arr.push(output['seed' + i] || '');
}
output.seedWords = JSON.stringify(arr);
