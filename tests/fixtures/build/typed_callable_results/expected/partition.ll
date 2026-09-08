; ModuleID = 'src/main.w'
source_filename = "src/main.w"

@observed = internal global i1 false
@result = internal global i32 0

define i1 @flag(i32 %value) {
entry:
  %value1 = alloca i32, align 4
  store i32 %value, ptr %value1, align 4
  %value2 = load i32, ptr %value1, align 4
  %eq = icmp eq i32 %value2, 42
  ret i1 %eq
}

define i32 @value(i32 %input) {
entry:
  %input1 = alloca i32, align 4
  store i32 %input, ptr %input1, align 4
  %input2 = load i32, ptr %input1, align 4
  ret i32 %input2
}

define void @touch(i32 %input) {
entry:
  %input1 = alloca i32, align 4
  store i32 %input, ptr %input1, align 4
  %input2 = load i32, ptr %input1, align 4
  %call = call i32 @value(i32 %input2)
  %ignored = alloca i32, align 4
  store i32 %call, ptr %ignored, align 4
  ret void
}

define i32 @main() {
entry:
  %call = call i1 @flag(i32 42)
  store i1 %call, ptr @observed, align 1
  call void @touch(i32 42)
  %call1 = call i32 @value(i32 42)
  store i32 %call1, ptr @result, align 4
  ret i32 0
}
