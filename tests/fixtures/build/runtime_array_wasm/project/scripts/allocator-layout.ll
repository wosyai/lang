target triple = "wasm32-wasi"

; Keep a large static allocation in the linked module and check the heap layout
; without accessing the allocation's payload.
@static_storage = internal global [131073 x i8] zeroinitializer

declare ptr @__wosy_core_alloc(i64, i64)
declare void @__wosy_core_free(ptr)
declare void @llvm.trap()

define void @_start() {
entry:
  %storage_end = getelementptr inbounds [131073 x i8], ptr @static_storage, i32 1
  %payload = call ptr @__wosy_core_alloc(i64 1, i64 8)
  %address = ptrtoint ptr %payload to i32
  %end_address = ptrtoint ptr %storage_end to i32
  %above_statics = icmp uge i32 %address, %end_address
  %align_bits = and i32 %address, 7
  %aligned = icmp eq i32 %align_bits, 0
  %valid = and i1 %above_statics, %aligned
  br i1 %valid, label %complete, label %fail

complete:
  call void @__wosy_core_free(ptr %payload)
  ret void

fail:
  call void @llvm.trap()
  unreachable
}
