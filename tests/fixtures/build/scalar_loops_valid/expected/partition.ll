; ModuleID = 'src/main.w'
source_filename = "src/main.w"

@result = internal global i32 0

define i32 @loop(i32 %start) {
entry:
  %start1 = alloca i32, align 4
  store i32 %start, ptr %start1, align 4
  br label %self_tail.header

self_tail.header:                                 ; preds = %self_tail.then, %entry
  %start2 = load i32, ptr %start1, align 4
  %lt = icmp slt i32 %start2, 3
  br i1 %lt, label %self_tail.then, label %self_tail.else

self_tail.then:                                   ; preds = %self_tail.header
  %start3 = load i32, ptr %start1, align 4
  %add = add i32 %start3, 1
  store i32 %add, ptr %start1, align 4
  br label %self_tail.header

self_tail.else:                                   ; preds = %self_tail.header
  %start4 = load i32, ptr %start1, align 4
  ret i32 %start4
}

define i32 @main() {
entry:
  %call = call i32 @loop(i32 0)
  store i32 %call, ptr @result, align 4
  ret i32 0
}
