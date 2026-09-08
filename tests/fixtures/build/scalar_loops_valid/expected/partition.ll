; ModuleID = 'src/main.w'
source_filename = "wosy"

define i32 @loop(i32 %start) {
entry:
  %v0 = alloca i32
  store i32 %start, ptr %v0
  %v1 = load i32, ptr %v0
  %v2 = alloca i32
  store i32 %v1, ptr %v2
  br label %while.cond.0

while.cond.0:
  %v3 = load i32, ptr %v2
  %v4 = icmp slt i32 %v3, 3
  br i1 %v4, label %while.body.1, label %while.exit.2

while.body.1:
  %v5 = load i32, ptr %v2
  %v6 = add i32 %v5, 1
  %v7 = load i32, ptr %v2
  store i32 %v6, ptr %v2
  br label %while.cond.0

while.exit.2:
  %v8 = load i32, ptr %v2
  ret i32 %v8
}

define i32 @main() {
entry:
  %v0 = call i32 @loop(i32 0)
  ret i32 0
}
