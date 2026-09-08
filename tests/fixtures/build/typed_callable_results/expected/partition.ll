; ModuleID = 'src/main.w'
source_filename = "wosy"

define i1 @flag(i32 %value) {
entry:
  %v0 = alloca i32
  store i32 %value, ptr %v0
  %v1 = load i32, ptr %v0
  %v2 = icmp eq i32 %v1, 42
  ret i1 %v2
}

define i32 @value(i32 %input) {
entry:
  %v0 = alloca i32
  store i32 %input, ptr %v0
  %v1 = load i32, ptr %v0
  ret i32 %v1
}

define void @touch(i32 %input) {
entry:
  %v0 = alloca i32
  store i32 %input, ptr %v0
  %v1 = load i32, ptr %v0
  %v2 = call i32 @value(i32 %v1)
  %v3 = alloca i32
  store i32 %v2, ptr %v3
  ret void
}

define i32 @main() {
entry:
  %v0 = call i1 @flag(i32 42)
  call void @touch(i32 42)
  %v1 = call i32 @value(i32 42)
  ret i32 0
}
