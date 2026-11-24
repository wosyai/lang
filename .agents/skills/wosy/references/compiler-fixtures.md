# Recursion fixtures

## Windows

From the built fixture's project directory:

```sh
llvm-readobj --file-headers .wosy/artifacts/app/dev/main/main
```

Compare the reserve and commit sizes with [build-native](../../../../tests/fixtures/build/recursion_seed_native/project/scripts/build-native).

## WebAssembly

Set the linear-memory stack in [build-wasm](../../../../tests/fixtures/build/recursion_seed_wasm/project/scripts/build-wasm) and the execution stack in [run-wasmtime](../../../../tests/fixtures/build/recursion_seed_wasm/project/scripts/run-wasmtime). The logical recursion counter measures neither.
