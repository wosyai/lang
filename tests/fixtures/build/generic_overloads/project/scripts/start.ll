target triple = "wasm32-wasi"

declare void @wosy_start()

define void @_start() {
entry:
  call void @wosy_start()
  ret void
}
