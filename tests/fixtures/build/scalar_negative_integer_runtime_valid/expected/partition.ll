; ModuleID = 'src/main.w'
source_filename = "src/main.w"

@pair = internal global [16 x i8] zeroinitializer, align 8
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
  %struct_literal = alloca [16 x i8], align 8
  %address = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i64 0, ptr %address, align 4
  %count = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i32 42, ptr %count, align 4
  call void @llvm.memmove.p0.p0.i64(ptr align 1 @pair, ptr align 1 %struct_literal, i64 16, i1 false)
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

; Function Attrs: nocallback nofree nounwind willreturn memory(argmem: readwrite)
declare void @llvm.memmove.p0.p0.i64(ptr nocapture writeonly, ptr nocapture readonly, i64, i1 immarg) #0

attributes #0 = { nocallback nofree nounwind willreturn memory(argmem: readwrite) }
