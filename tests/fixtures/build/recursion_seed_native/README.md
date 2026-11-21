# Native64 recursion seed (Windows x64)

The runner inherits stdin. The fixture builder resolves the single Wosy extern
for `fixture_stdin.getchar` to the CRT `getchar` with LLD's COFF alias. It finds
the fingerprinted symbol in the object so edits, including line-ending changes,
do not invalidate the binding. The production CLI supplies `runtime.ll`.

Clang compiles at `-O0`. LLD pins the executable's main-thread stack to 1,048,576
bytes reserved and 65,536 bytes initially committed. Use `llvm-readobj
--file-headers .wosy/artifacts/app/dev/main/main` from the project directory to
check the PE fields. The fixture shares the workload, effect trace, and logical
depth oracle described in the sibling `recursion_seed_wasm/README.md`.
