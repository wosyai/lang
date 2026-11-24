# Recursion fixture stacks

The Wasm linear-memory stack and Wasmtime’s execution stack are separate.
Adjust them in [build-wasm](project/scripts/build-wasm) and
[run-wasmtime](project/scripts/run-wasmtime), respectively. The source’s logical
depth counter does not measure either stack.
