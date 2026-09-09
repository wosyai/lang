target triple = "wasm32-wasi"

declare i32 @__original_main()

define void @_start() {
entry:
  %result = call i32 @__original_main()
  ret void
}
