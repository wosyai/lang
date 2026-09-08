; ModuleID = 'src/main.w'
source_filename = "wosy"

define i32 @add(i32 %left, i32 %right) {
entry:
  %v0 = alloca i32
  store i32 %left, ptr %v0
  %v1 = alloca i32
  store i32 %right, ptr %v1
  %v2 = load i32, ptr %v0
  %v3 = load i32, ptr %v1
  %v4 = add i32 %v2, %v3
  ret i32 %v4
}

define i32 @main() {
entry:
  br i1 1, label %if.then.0, label %if.else.1

if.then.0:
  %v0 = call i32 @add(i32 20, i32 22)
  br label %if.merge.2

if.else.1:
  br label %if.merge.2

if.merge.2:
  %v1 = phi i32 [ %v0, %if.then.0 ], [ 0, %if.else.1 ]
  ret i32 0
}
