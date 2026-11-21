# Wasm32 recursion seed

The runner inherits stdin. The capsule reads one byte with WASI Preview 1
`fd_read`. Clang compiles at `-O0`, `wasm-ld` reserves a 32,768-byte
linear-memory stack, and Wasmtime sets `max-wasm-stack=524288` bytes for
engine-managed Wasm execution. These are separate stacks. The production CLI
supplies `runtime.ll`.

## Shared workload and runtime oracles

Each executable receives one raw stdin byte: decimal `3` (shallow) or `64`
(bounded). Its value is the number of recursive edges taken in each of the
three subcases. Success requires exit 0 and empty stdout/stderr. Failed source
oracles call `core.system_panic()`.

| Oracle | Input `3` | Input `64` |
| --- | ---: | ---: |
| Direct tail sum | `6` | `2080` |
| Direct non-tail pair, in order | `(11, 20)` | `(2085, 203)` |
| Forward mutual pair, in order | `(110, 220)` | `(261, 585)` |
| Tail / non-tail / mutual effect counts | `4 / 7 / 4` | `65 / 129 / 65` |
| Peak logical depth in each subcase | `4` | `65` |

`record_event` checks the exact `(event, remaining)` at each trace position
before advancing the global position. Tail enters at `n, n-1, …, 0`. Non-tail
enters in the same order, then records returns at `1, 2, …, n`. Mutual
alternates `a, b, a, …` while descending at `n, n-1, …, 0`, starting with
the function that refers forward to its partner in the same module.

The test-only `logical_depth` argument starts at 1, grows by 1 for each
recursive edge, and feeds a peak counter independent of the trace position.
It includes the base call and excludes the driver and instrumentation helpers.
This is a source-level path measurement. Machine-stack consumption and maximum
safe depth require separate measurements.
