
; wasm-ld places this symbol after static data and the reserved stack. Its address is the heap start.
@__heap_base = external global i8
@__wosy_core_heap_cursor = internal global i64 0
@__wosy_core_free_head = internal global i32 0
@__wosy_wasm_tail = internal global i32 0

declare i32 @llvm.wasm.memory.size.i32(i32)
declare i32 @llvm.wasm.memory.grow.i32(i32, i32)
