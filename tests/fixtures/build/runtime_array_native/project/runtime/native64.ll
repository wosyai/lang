declare ptr @malloc(i64)
declare void @free(ptr)
declare void @exit(i32)

define ptr @__wosy_core_alloc(i64 %size, i64 %alignment) {
entry:
  %allocation = call ptr @malloc(i64 %size)
  ret ptr %allocation
}

define void @__wosy_core_free(ptr %pointer) {
entry:
  call void @free(ptr %pointer)
  ret void
}

define void @__wosy_core_system_panic() {
entry:
  call void @exit(i32 1)
  unreachable
}
