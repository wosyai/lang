target triple = "wasm32-wasi"

declare i32 @main()

define void @_start() {
entry:
  call i32 @main()
  ret void
}
