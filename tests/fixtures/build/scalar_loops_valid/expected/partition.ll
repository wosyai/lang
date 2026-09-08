; ModuleID = 'src/main.w'
source_filename = "src/main.w"

define i32 @loop(i32 %start) {
entry:
  %start1 = alloca i32, align 4
  store i32 %start, ptr %start1, align 4
  %start2 = load i32, ptr %start1, align 4
  %value = alloca i32, align 4
  store i32 %start2, ptr %value, align 4
  br label %while.cond.0

while.cond.0:                                     ; preds = %while.body.1, %entry
  %value3 = load i32, ptr %value, align 4
  %lt = icmp slt i32 %value3, 3
  br i1 %lt, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %value4 = load i32, ptr %value, align 4
  %add = add i32 %value4, 1
  %old = load i32, ptr %value, align 4
  store i32 %add, ptr %value, align 4
  br label %while.cond.0

while.exit.2:                                     ; preds = %while.cond.0
  %value5 = load i32, ptr %value, align 4
  ret i32 %value5
}

define i32 @main() {
entry:
  %call = call i32 @loop(i32 0)
  ret i32 0
}
