; ModuleID = 'src/main.w'
source_filename = "src/main.w"

@quotient = internal global i32 0
@remainder = internal global i32 0
@computed_quotient = internal global i32 0
@computed_remainder = internal global i32 0

define i32 @negate(i32 %value) {
entry:
  %value1 = alloca i32, align 4
  store i32 %value, ptr %value1, align 4
  %value2 = load i32, ptr %value1, align 4
  %neg = sub i32 0, %value2
  ret i32 %neg
}

define i32 @main() {
entry:
  store i32 -2, ptr @quotient, align 4
  store i32 -1, ptr @remainder, align 4
  %call = call i32 @negate(i32 7)
  %div = sdiv i32 %call, 3
  store i32 %div, ptr @computed_quotient, align 4
  %call1 = call i32 @negate(i32 7)
  %rem = srem i32 %call1, 3
  store i32 %rem, ptr @computed_remainder, align 4
  ret i32 0
}
