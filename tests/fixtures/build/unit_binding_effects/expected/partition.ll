; ModuleID = 'src/main.w'
source_filename = "src/main.w"

define void @touch() {
entry:
  call void @touch()
  ret void
}

define void @use() {
entry:
  call void @touch()
  call void @touch()
  call void @touch()
  ret void
}

define i32 @main() {
entry:
  call void @touch()
  call void @use()
  ret i32 0
}
