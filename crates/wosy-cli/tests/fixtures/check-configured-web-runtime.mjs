// Run on the linked Wasm output from configured_web_export_keeps_checked_release_leaf_and_guarded_owner.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const wasm = await WebAssembly.compile(readFileSync(process.argv[2]));
assert.deepEqual(WebAssembly.Module.imports(wasm), []);
const { exports } = await WebAssembly.instantiate(wasm);
for (const [index, value] of [[0n, 7n], [1n, 8n], [2n, 9n]]) {
    assert.equal(exports.observe_value(index), value);
}
assert.equal(exports.observe, undefined);
assert.equal(exports.stop, undefined);
assert.throws(() => exports.stop_now(), WebAssembly.RuntimeError);
assert.equal(exports.observe_value(1n), 8n);
