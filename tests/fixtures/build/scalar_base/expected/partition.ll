; ModuleID = 'src/main.w'
source_filename = "src/main.w"

define i32 @add(i32 %left, i32 %right) {
entry:
  %left1 = alloca i32, align 4
  store i32 %left, ptr %left1, align 4
  %right2 = alloca i32, align 4
  store i32 %right, ptr %right2, align 4
  %left3 = load i32, ptr %left1, align 4
  %right4 = load i32, ptr %right2, align 4
  %add = add i32 %left3, %right4
  ret i32 %add
}

define i32 @main() {
entry:
  br i1 true, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  %call = call i32 @add(i32 20, i32 22)
  br label %if.merge

if.else:                                          ; preds = %entry
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.then
  %if = phi i32 [ %call, %if.then ], [ 0, %if.else ]
  ret i32 0
}
