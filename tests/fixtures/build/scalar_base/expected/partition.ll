; ModuleID = 'src/main.w'
source_filename = "wosy"

define i32 @add(i32 %left, i32 %right) {
entry:
  %v0 = add i32 %left, %right
  ret i32 %v0
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
