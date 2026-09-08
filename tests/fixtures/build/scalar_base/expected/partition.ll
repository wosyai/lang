; ModuleID = 'src/main.w'
source_filename = "src/main.w"

@seed = internal global i32 0
@increment = internal global i32 0
@choose_sum = internal global i1 false
@result = internal global i32 0

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
  store i32 20, ptr @seed, align 4
  store i32 22, ptr @increment, align 4
  store i1 true, ptr @choose_sum, align 1
  %choose_sum = load i1, ptr @choose_sum, align 1
  br i1 %choose_sum, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  %seed = load i32, ptr @seed, align 4
  %increment = load i32, ptr @increment, align 4
  %call = call i32 @add(i32 %seed, i32 %increment)
  br label %if.merge

if.else:                                          ; preds = %entry
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.then
  %if = phi i32 [ %call, %if.then ], [ 0, %if.else ]
  store i32 %if, ptr @result, align 4
  ret i32 0
}
