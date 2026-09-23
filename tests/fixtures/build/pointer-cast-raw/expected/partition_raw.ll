; ModuleID = 'src/main.w'
source_filename = "src/main.w"

declare i32 @wosy_extern__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f77617369__776173695f736e617073686f745f7072657669657731__5f66645f72656164(i32, i32, i32, i32) #0
declare i32 @wosy_extern__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f77617369__776173695f736e617073686f745f7072657669657731__66645f7772697465(i32, i32, i32, i32) #1
declare i32 @wosy_extern__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f77617369__776173695f736e617073686f745f7072657669657731__5f706f6c6c5f6f6e656f6666(i32, i32, i32, i32) #2
declare void @wosy_extern__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f77617369__776173695f736e617073686f745f7072657669657731__5f70726f635f65786974(i32) #3

attributes #0 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="fd_read" }
attributes #1 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="fd_write" }
attributes #2 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="poll_oneoff" }
attributes #3 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="proc_exit" }

@wosy_utf8_literal_0 = private constant [1 x i8] c"\0A"
@wosy_utf8_literal_1 = private constant [1 x i8] c"\0A"

define void @wosy_fn__706f696e7465722d636173742d726177__7372632f6d61696e2e77__66666637306432326135346365616161303661656666326632666637636634316635633333636234663761653938643433353161363033373136343463386663__72756e() {
entry:
  %utf8_literal34 = alloca [16 x i8], align 1
  %utf8_literal = alloca [16 x i8], align 1
  %fifth = alloca i8, align 1
  %first = alloca i8, align 1
  %back = alloca i32, align 4
  %assignment_value19 = alloca i32, align 4
  %viewed = alloca i32, align 4
  %assignment_value16 = alloca i32, align 4
  %assignment_value8 = alloca i8, align 1
  %w4 = alloca i32, align 4
  %at4 = alloca i32, align 4
  %assignment_value5 = alloca i32, align 4
  %assignment_value = alloca i8, align 1
  %w0 = alloca i32, align 4
  %storage = alloca i32, align 4
  %allocation = call ptr @__wosy_core_alloc(i64 8, i64 4)
  %allocation_address = ptrtoint ptr %allocation to i32
  store i32 %allocation_address, ptr %storage, align 4
  %storage1 = load i32, ptr %storage, align 4
  store i32 %storage1, ptr %w0, align 4
  store i8 7, ptr %assignment_value, align 1
  %assignment_value2 = load i8, ptr %assignment_value, align 1
  %w03 = load i32, ptr %w0, align 4
  %deref_is_null = icmp eq i32 %w03, 0
  br i1 %deref_is_null, label %deref_null_panic, label %deref_null_continue

deref_null_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

deref_null_continue:                              ; preds = %entry
  %deref = inttoptr i32 %w03 to ptr
  store i8 %assignment_value2, ptr %deref, align 1
  %storage4 = load i32, ptr %storage, align 4
  %offset_base = inttoptr i32 %storage4 to ptr
  %offset = getelementptr inbounds i8, ptr %offset_base, i64 4
  %offset_address = ptrtoint ptr %offset to i32
  store i32 %offset_address, ptr %assignment_value5, align 4
  %assignment_value6 = load i32, ptr %assignment_value5, align 4
  store i32 %assignment_value6, ptr %at4, align 4
  %at47 = load i32, ptr %at4, align 4
  store i32 %at47, ptr %w4, align 4
  store i8 42, ptr %assignment_value8, align 1
  %assignment_value9 = load i8, ptr %assignment_value8, align 1
  %w410 = load i32, ptr %w4, align 4
  %deref_is_null11 = icmp eq i32 %w410, 0
  br i1 %deref_is_null11, label %deref_null_panic12, label %deref_null_continue13

deref_null_panic12:                               ; preds = %deref_null_continue
  call void @__wosy_core_system_panic()
  unreachable

deref_null_continue13:                            ; preds = %deref_null_continue
  %deref14 = inttoptr i32 %w410 to ptr
  store i8 %assignment_value9, ptr %deref14, align 1
  %storage15 = load i32, ptr %storage, align 4
  store i32 %storage15, ptr %assignment_value16, align 4
  %assignment_value17 = load i32, ptr %assignment_value16, align 4
  store i32 %assignment_value17, ptr %viewed, align 4
  %viewed18 = load i32, ptr %viewed, align 4
  store i32 %viewed18, ptr %assignment_value19, align 4
  %assignment_value20 = load i32, ptr %assignment_value19, align 4
  store i32 %assignment_value20, ptr %back, align 4
  %back21 = load i32, ptr %back, align 4
  %load_base = inttoptr i32 %back21 to ptr
  %load = load i8, ptr %load_base, align 1
  store i8 %load, ptr %first, align 1
  %back22 = load i32, ptr %back, align 4
  %offset_base23 = inttoptr i32 %back22 to ptr
  %offset24 = getelementptr inbounds i8, ptr %offset_base23, i64 4
  %offset_address25 = ptrtoint ptr %offset24 to i32
  %load_base26 = inttoptr i32 %offset_address25 to ptr
  %load27 = load i8, ptr %load_base26, align 1
  store i8 %load27, ptr %fifth, align 1
  %first28 = load i8, ptr %first, align 1
  %call = call ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333334333333363336333233333335333333313333333633333339333333353333333333363336333333343333333433333331333333373336333233333338333633363333333133333333333333323333333533333332333333353333333933363332333633353336333633363332333633353333333633363334333633353333333333363333333333343333333133363334333633353333333733333331333333363333333633363335333333313333333933333333333333393333333933333334333633333333333233333338333333333336333633333332333333383333333733333338333333373333333433333330333633343333333633333331356635663336333633363636333733323336363433363331333733345f5f3337__(i8 %first28)
  %call29 = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__7072696e74(ptr %call)
  %data = getelementptr inbounds i8, ptr %utf8_literal, i8 0
  store i32 ptrtoint (ptr @wosy_utf8_literal_0 to i32), ptr %data, align 4
  %length = getelementptr inbounds i8, ptr %utf8_literal, i8 8
  store i64 1, ptr %length, align 4
  %call30 = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__7072696e74(ptr %utf8_literal)
  %fifth31 = load i8, ptr %fifth, align 1
  %call32 = call ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333334333333363336333233333335333333313333333633333339333333353333333333363336333333343333333433333331333333373336333233333338333633363333333133333333333333323333333533333332333333353333333933363332333633353336333633363332333633353333333633363334333633353333333333363333333333343333333133363334333633353333333733333331333333363333333633363335333333313333333933333333333333393333333933333334333633333333333233333338333333333336333633333332333333383333333733333338333333373333333433333330333633343333333633333331356635663336333633363636333733323336363433363331333733345f5f3337__(i8 %fifth31)
  %call33 = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__7072696e74(ptr %call32)
  %data35 = getelementptr inbounds i8, ptr %utf8_literal34, i8 0
  store i32 ptrtoint (ptr @wosy_utf8_literal_1 to i32), ptr %data35, align 4
  %length36 = getelementptr inbounds i8, ptr %utf8_literal34, i8 8
  store i64 1, ptr %length36, align 4
  %call37 = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__7072696e74(ptr %utf8_literal34)
  %storage38 = load i32, ptr %storage, align 4
  %free_pointer = inttoptr i32 %storage38 to ptr
  call void @__wosy_core_free(ptr %free_pointer)
  ret void
}

define { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__7072696e74(ptr %text) {
entry:
  %assignment_value50 = alloca i1, align 1
  %assignment_value44 = alloca i64, align 8
  %assignment_value42 = alloca i1, align 1
  %assignment_value40 = alloca i64, align 8
  %assignment_value38 = alloca i1, align 1
  %assignment_value32 = alloca i64, align 8
  %assignment_value22 = alloca i32, align 4
  %assignment_value = alloca i32, align 4
  %count = alloca i32, align 4
  %result = alloca i32, align 4
  %byte_count = alloca [4 x i8], align 1
  %struct_literal10 = alloca [4 x i8], align 1
  %iovec = alloca [8 x i8], align 1
  %struct_literal = alloca [8 x i8], align 1
  %success = alloca i32, align 4
  %maximum = alloca i64, align 8
  %maximum_u32 = alloca i32, align 4
  %complete = alloca i1, align 1
  %zero = alloca i64, align 8
  %reported = alloca i64, align 8
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  store i64 0, ptr %reported, align 4
  store i64 0, ptr %zero, align 4
  %length = getelementptr inbounds i8, ptr %text1, i8 8
  %place = load i64, ptr %length, align 4
  %zero2 = load i64, ptr %zero, align 4
  %eq = icmp eq i64 %place, %zero2
  store i1 %eq, ptr %complete, align 1
  store i32 -1, ptr %maximum_u32, align 4
  %maximum_u323 = load i32, ptr %maximum_u32, align 4
  %int_extend = zext i32 %maximum_u323 to i64
  store i64 %int_extend, ptr %maximum, align 4
  store i32 0, ptr %success, align 4
  %data = getelementptr inbounds i8, ptr %text1, i8 0
  %place4 = load i32, ptr %data, align 4
  %length5 = getelementptr inbounds i8, ptr %text1, i8 8
  %place6 = load i64, ptr %length5, align 4
  %int_trunc = trunc i64 %place6 to i32
  %data7 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %place4, ptr %data7, align 4
  %length8 = getelementptr inbounds i8, ptr %struct_literal, i8 4
  store i32 %int_trunc, ptr %length8, align 4
  %struct_value9 = load [8 x i8], ptr %struct_literal, align 1
  store [8 x i8] %struct_value9, ptr %iovec, align 1
  %value = getelementptr inbounds i8, ptr %struct_literal10, i8 0
  store i32 0, ptr %value, align 4
  %struct_value11 = load [4 x i8], ptr %struct_literal10, align 1
  store [4 x i8] %struct_value11, ptr %byte_count, align 1
  store i32 0, ptr %result, align 4
  store i32 0, ptr %count, align 4
  %length12 = getelementptr inbounds i8, ptr %text1, i8 8
  %place13 = load i64, ptr %length12, align 4
  %zero14 = load i64, ptr %zero, align 4
  %ne = icmp ne i64 %place13, %zero14
  br i1 %ne, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %length15 = getelementptr inbounds i8, ptr %text1, i8 8
  %place16 = load i64, ptr %length15, align 4
  %maximum17 = load i64, ptr %maximum, align 4
  %le = icmp ule i64 %place16, %maximum17
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %le, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  %address = ptrtoint ptr %iovec to i32
  %address18 = ptrtoint ptr %byte_count to i32
  %call = call i32 @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__66645f77726974655f6f6e6365(i32 1, i32 %address, i32 %address18)
  store i32 %call, ptr %assignment_value, align 4
  %assignment_value19 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value19, ptr %result, align 4
  %value20 = getelementptr inbounds i8, ptr %byte_count, i8 0
  %place21 = load i32, ptr %value20, align 4
  store i32 %place21, ptr %assignment_value22, align 4
  %assignment_value23 = load i32, ptr %assignment_value22, align 4
  store i32 %assignment_value23, ptr %count, align 4
  %result24 = load i32, ptr %result, align 4
  %success25 = load i32, ptr %success, align 4
  %eq26 = icmp eq i32 %result24, %success25
  br i1 %eq26, label %if.then27, label %if.else28

if.else:                                          ; preds = %short_circuit.merge
  store i64 0, ptr %assignment_value44, align 4
  %assignment_value45 = load i64, ptr %assignment_value44, align 4
  store i64 %assignment_value45, ptr %reported, align 4
  %length46 = getelementptr inbounds i8, ptr %text1, i8 8
  %place47 = load i64, ptr %length46, align 4
  %zero48 = load i64, ptr %zero, align 4
  %eq49 = icmp eq i64 %place47, %zero48
  store i1 %eq49, ptr %assignment_value50, align 1
  %assignment_value51 = load i1, ptr %assignment_value50, align 1
  store i1 %assignment_value51, ptr %complete, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.merge29
  %reported52 = load i64, ptr %reported, align 4
  %complete53 = load i1, ptr %complete, align 1
  %output = insertvalue { i64, i1 } zeroinitializer, i64 %reported52, 0
  %output54 = insertvalue { i64, i1 } %output, i1 %complete53, 1
  ret { i64, i1 } %output54

if.then27:                                        ; preds = %if.then
  %count30 = load i32, ptr %count, align 4
  %int_extend31 = zext i32 %count30 to i64
  store i64 %int_extend31, ptr %assignment_value32, align 4
  %assignment_value33 = load i64, ptr %assignment_value32, align 4
  store i64 %assignment_value33, ptr %reported, align 4
  %reported34 = load i64, ptr %reported, align 4
  %length35 = getelementptr inbounds i8, ptr %text1, i8 8
  %place36 = load i64, ptr %length35, align 4
  %eq37 = icmp eq i64 %reported34, %place36
  store i1 %eq37, ptr %assignment_value38, align 1
  %assignment_value39 = load i1, ptr %assignment_value38, align 1
  store i1 %assignment_value39, ptr %complete, align 1
  br label %if.merge29

if.else28:                                        ; preds = %if.then
  store i64 0, ptr %assignment_value40, align 4
  %assignment_value41 = load i64, ptr %assignment_value40, align 4
  store i64 %assignment_value41, ptr %reported, align 4
  store i1 false, ptr %assignment_value42, align 1
  %assignment_value43 = load i1, ptr %assignment_value42, align 1
  store i1 %assignment_value43, ptr %complete, align 1
  br label %if.merge29

if.merge29:                                       ; preds = %if.else28, %if.then27
  br label %if.merge
}

define { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__657072696e74(ptr %text) {
entry:
  %assignment_value50 = alloca i1, align 1
  %assignment_value44 = alloca i64, align 8
  %assignment_value42 = alloca i1, align 1
  %assignment_value40 = alloca i64, align 8
  %assignment_value38 = alloca i1, align 1
  %assignment_value32 = alloca i64, align 8
  %assignment_value22 = alloca i32, align 4
  %assignment_value = alloca i32, align 4
  %count = alloca i32, align 4
  %result = alloca i32, align 4
  %byte_count = alloca [4 x i8], align 1
  %struct_literal10 = alloca [4 x i8], align 1
  %iovec = alloca [8 x i8], align 1
  %struct_literal = alloca [8 x i8], align 1
  %success = alloca i32, align 4
  %maximum = alloca i64, align 8
  %maximum_u32 = alloca i32, align 4
  %complete = alloca i1, align 1
  %zero = alloca i64, align 8
  %reported = alloca i64, align 8
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  store i64 0, ptr %reported, align 4
  store i64 0, ptr %zero, align 4
  %length = getelementptr inbounds i8, ptr %text1, i8 8
  %place = load i64, ptr %length, align 4
  %zero2 = load i64, ptr %zero, align 4
  %eq = icmp eq i64 %place, %zero2
  store i1 %eq, ptr %complete, align 1
  store i32 -1, ptr %maximum_u32, align 4
  %maximum_u323 = load i32, ptr %maximum_u32, align 4
  %int_extend = zext i32 %maximum_u323 to i64
  store i64 %int_extend, ptr %maximum, align 4
  store i32 0, ptr %success, align 4
  %data = getelementptr inbounds i8, ptr %text1, i8 0
  %place4 = load i32, ptr %data, align 4
  %length5 = getelementptr inbounds i8, ptr %text1, i8 8
  %place6 = load i64, ptr %length5, align 4
  %int_trunc = trunc i64 %place6 to i32
  %data7 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %place4, ptr %data7, align 4
  %length8 = getelementptr inbounds i8, ptr %struct_literal, i8 4
  store i32 %int_trunc, ptr %length8, align 4
  %struct_value9 = load [8 x i8], ptr %struct_literal, align 1
  store [8 x i8] %struct_value9, ptr %iovec, align 1
  %value = getelementptr inbounds i8, ptr %struct_literal10, i8 0
  store i32 0, ptr %value, align 4
  %struct_value11 = load [4 x i8], ptr %struct_literal10, align 1
  store [4 x i8] %struct_value11, ptr %byte_count, align 1
  store i32 0, ptr %result, align 4
  store i32 0, ptr %count, align 4
  %length12 = getelementptr inbounds i8, ptr %text1, i8 8
  %place13 = load i64, ptr %length12, align 4
  %zero14 = load i64, ptr %zero, align 4
  %ne = icmp ne i64 %place13, %zero14
  br i1 %ne, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %length15 = getelementptr inbounds i8, ptr %text1, i8 8
  %place16 = load i64, ptr %length15, align 4
  %maximum17 = load i64, ptr %maximum, align 4
  %le = icmp ule i64 %place16, %maximum17
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %le, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  %address = ptrtoint ptr %iovec to i32
  %address18 = ptrtoint ptr %byte_count to i32
  %call = call i32 @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__66645f77726974655f6f6e6365(i32 2, i32 %address, i32 %address18)
  store i32 %call, ptr %assignment_value, align 4
  %assignment_value19 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value19, ptr %result, align 4
  %value20 = getelementptr inbounds i8, ptr %byte_count, i8 0
  %place21 = load i32, ptr %value20, align 4
  store i32 %place21, ptr %assignment_value22, align 4
  %assignment_value23 = load i32, ptr %assignment_value22, align 4
  store i32 %assignment_value23, ptr %count, align 4
  %result24 = load i32, ptr %result, align 4
  %success25 = load i32, ptr %success, align 4
  %eq26 = icmp eq i32 %result24, %success25
  br i1 %eq26, label %if.then27, label %if.else28

if.else:                                          ; preds = %short_circuit.merge
  store i64 0, ptr %assignment_value44, align 4
  %assignment_value45 = load i64, ptr %assignment_value44, align 4
  store i64 %assignment_value45, ptr %reported, align 4
  %length46 = getelementptr inbounds i8, ptr %text1, i8 8
  %place47 = load i64, ptr %length46, align 4
  %zero48 = load i64, ptr %zero, align 4
  %eq49 = icmp eq i64 %place47, %zero48
  store i1 %eq49, ptr %assignment_value50, align 1
  %assignment_value51 = load i1, ptr %assignment_value50, align 1
  store i1 %assignment_value51, ptr %complete, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.merge29
  %reported52 = load i64, ptr %reported, align 4
  %complete53 = load i1, ptr %complete, align 1
  %output = insertvalue { i64, i1 } zeroinitializer, i64 %reported52, 0
  %output54 = insertvalue { i64, i1 } %output, i1 %complete53, 1
  ret { i64, i1 } %output54

if.then27:                                        ; preds = %if.then
  %count30 = load i32, ptr %count, align 4
  %int_extend31 = zext i32 %count30 to i64
  store i64 %int_extend31, ptr %assignment_value32, align 4
  %assignment_value33 = load i64, ptr %assignment_value32, align 4
  store i64 %assignment_value33, ptr %reported, align 4
  %reported34 = load i64, ptr %reported, align 4
  %length35 = getelementptr inbounds i8, ptr %text1, i8 8
  %place36 = load i64, ptr %length35, align 4
  %eq37 = icmp eq i64 %reported34, %place36
  store i1 %eq37, ptr %assignment_value38, align 1
  %assignment_value39 = load i1, ptr %assignment_value38, align 1
  store i1 %assignment_value39, ptr %complete, align 1
  br label %if.merge29

if.else28:                                        ; preds = %if.then
  store i64 0, ptr %assignment_value40, align 4
  %assignment_value41 = load i64, ptr %assignment_value40, align 4
  store i64 %assignment_value41, ptr %reported, align 4
  store i1 false, ptr %assignment_value42, align 1
  %assignment_value43 = load i1, ptr %assignment_value42, align 1
  store i1 %assignment_value43, ptr %complete, align 1
  br label %if.merge29

if.merge29:                                       ; preds = %if.else28, %if.then27
  br label %if.merge
}

define { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__726561645f696e746f(i32 %destination, i64 %capacity) {
entry:
  %destination1 = alloca i32, align 4
  store i32 %destination, ptr %destination1, align 4
  %capacity2 = alloca i64, align 8
  store i64 %capacity, ptr %capacity2, align 4
  %destination3 = load i32, ptr %destination1, align 4
  %capacity4 = load i64, ptr %capacity2, align 4
  %call = call { i64, i1 } @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__726561645f696e746f(i32 %destination3, i64 %capacity4)
  ret { i64, i1 } %call
}

define void @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__65786974(i32 %code) {
entry:
  %code1 = alloca i32, align 4
  store i32 %code, ptr %code1, align 4
  %code2 = load i32, ptr %code1, align 4
  call void @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__65786974(i32 %code2)
  ret void
}

define void @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__736c6565705f6e73(i64 %nanoseconds) {
entry:
  %nanoseconds1 = alloca i64, align 8
  store i64 %nanoseconds, ptr %nanoseconds1, align 4
  %nanoseconds2 = load i64, ptr %nanoseconds1, align 4
  call void @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__736c6565705f6e73(i64 %nanoseconds2)
  ret void
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f726561645f6c696e655f656f665f737461747573(i1 %saw_input, i32 %status) {
entry:
  %saw_input1 = alloca i1, align 1
  store i1 %saw_input, ptr %saw_input1, align 1
  %status2 = alloca i32, align 4
  store i32 %status, ptr %status2, align 4
  %saw_input3 = load i1, ptr %saw_input1, align 1
  %not = xor i1 %saw_input3, true
  br i1 %not, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %status4 = load i32, ptr %status2, align 4
  %eq = icmp eq i32 %status4, 0
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %eq, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  br label %if.merge

if.else:                                          ; preds = %short_circuit.merge
  %status5 = load i32, ptr %status2, align 4
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.then
  %if = phi i32 [ 1, %if.then ], [ %status5, %if.else ]
  ret i32 %if
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f726561645f6c696e655f63617061636974795f737461747573(i64 %length, i64 %maximum, i32 %status) {
entry:
  %length1 = alloca i64, align 8
  store i64 %length, ptr %length1, align 4
  %maximum2 = alloca i64, align 8
  store i64 %maximum, ptr %maximum2, align 4
  %status3 = alloca i32, align 4
  store i32 %status, ptr %status3, align 4
  %length4 = load i64, ptr %length1, align 4
  %maximum5 = load i64, ptr %maximum2, align 4
  %ge = icmp uge i64 %length4, %maximum5
  br i1 %ge, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  br label %if.merge

if.else:                                          ; preds = %entry
  %status6 = load i32, ptr %status3, align 4
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.then
  %if = phi i32 [ 4, %if.then ], [ %status6, %if.else ]
  ret i32 %if
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f726561645f6c696e655f757466385f737461747573(ptr %bytes, i64 %length, i32 %status) {
entry:
  %assignment_value127 = alloca i32, align 4
  %assignment_value120 = alloca i64, align 8
  %assignment_value116 = alloca i32, align 4
  %assignment_value109 = alloca i8, align 1
  %assignment_value102 = alloca i8, align 1
  %assignment_value95 = alloca i64, align 8
  %assignment_value83 = alloca i8, align 1
  %assignment_value76 = alloca i8, align 1
  %assignment_value70 = alloca i64, align 8
  %assignment_value57 = alloca i64, align 8
  %assignment_value43 = alloca i32, align 4
  %assignment_value31 = alloca i8, align 1
  %assignment_value28 = alloca i8, align 1
  %assignment_value25 = alloca i64, align 8
  %assignment_value = alloca i32, align 4
  %byte = alloca i8, align 1
  %result = alloca i32, align 4
  %next_maximum = alloca i8, align 1
  %next_minimum = alloca i8, align 1
  %remaining = alloca i64, align 8
  %index = alloca i64, align 8
  %f4 = alloca i8, align 1
  %f0 = alloca i8, align 1
  %ed = alloca i8, align 1
  %e0 = alloca i8, align 1
  %four_byte_limit = alloca i8, align 1
  %four_byte_minimum = alloca i8, align 1
  %three_byte_minimum = alloca i8, align 1
  %two_byte_minimum = alloca i8, align 1
  %continuation_maximum = alloca i8, align 1
  %continuation_minimum = alloca i8, align 1
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %bytes1 = alloca ptr, align 8
  store ptr %bytes, ptr %bytes1, align 8
  %length2 = alloca i64, align 8
  store i64 %length, ptr %length2, align 4
  %status3 = alloca i32, align 4
  store i32 %status, ptr %status3, align 4
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store i8 -128, ptr %continuation_minimum, align 1
  store i8 -65, ptr %continuation_maximum, align 1
  store i8 -62, ptr %two_byte_minimum, align 1
  store i8 -32, ptr %three_byte_minimum, align 1
  store i8 -16, ptr %four_byte_minimum, align 1
  store i8 -12, ptr %four_byte_limit, align 1
  store i8 -32, ptr %e0, align 1
  store i8 -19, ptr %ed, align 1
  store i8 -16, ptr %f0, align 1
  store i8 -12, ptr %f4, align 1
  %zero4 = load i64, ptr %zero, align 4
  store i64 %zero4, ptr %index, align 4
  %zero5 = load i64, ptr %zero, align 4
  store i64 %zero5, ptr %remaining, align 4
  %continuation_minimum6 = load i8, ptr %continuation_minimum, align 1
  store i8 %continuation_minimum6, ptr %next_minimum, align 1
  %continuation_maximum7 = load i8, ptr %continuation_maximum, align 1
  store i8 %continuation_maximum7, ptr %next_maximum, align 1
  %status8 = load i32, ptr %status3, align 4
  store i32 %status8, ptr %result, align 4
  br label %while.cond.0

while.cond.0:                                     ; preds = %if.merge, %entry
  %index9 = load i64, ptr %index, align 4
  %length10 = load i64, ptr %length2, align 4
  %lt = icmp ult i64 %index9, %length10
  br i1 %lt, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %bytes11 = load ptr, ptr %bytes1, align 8
  %index12 = load i64, ptr %index, align 4
  %array_element = getelementptr inbounds i8, ptr %bytes11, i64 %index12
  %place = load i8, ptr %array_element, align 1
  store i8 %place, ptr %byte, align 1
  %remaining13 = load i64, ptr %remaining, align 4
  %zero14 = load i64, ptr %zero, align 4
  %ne = icmp ne i64 %remaining13, %zero14
  br i1 %ne, label %if.then, label %if.else

while.exit.2:                                     ; preds = %while.cond.0
  %remaining122 = load i64, ptr %remaining, align 4
  %zero123 = load i64, ptr %zero, align 4
  %ne124 = icmp ne i64 %remaining122, %zero123
  br i1 %ne124, label %if.then125, label %if.merge126

if.then:                                          ; preds = %while.body.1
  %byte15 = load i8, ptr %byte, align 1
  %next_minimum16 = load i8, ptr %next_minimum, align 1
  %lt17 = icmp ult i8 %byte15, %next_minimum16
  br i1 %lt17, label %short_circuit.merge, label %short_circuit.rhs

if.else:                                          ; preds = %while.body.1
  %byte33 = load i8, ptr %byte, align 1
  %continuation_minimum34 = load i8, ptr %continuation_minimum, align 1
  %ge = icmp uge i8 %byte33, %continuation_minimum34
  br i1 %ge, label %short_circuit.rhs35, label %short_circuit.merge36

if.merge:                                         ; preds = %if.merge115, %if.merge21
  %index118 = load i64, ptr %index, align 4
  %one119 = load i64, ptr %one, align 4
  %add = add i64 %index118, %one119
  store i64 %add, ptr %assignment_value120, align 4
  %assignment_value121 = load i64, ptr %assignment_value120, align 4
  store i64 %assignment_value121, ptr %index, align 4
  br label %while.cond.0

short_circuit.rhs:                                ; preds = %if.then
  %byte18 = load i8, ptr %byte, align 1
  %next_maximum19 = load i8, ptr %next_maximum, align 1
  %gt = icmp ugt i8 %byte18, %next_maximum19
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %if.then
  %short_circuit = phi i1 [ true, %if.then ], [ %gt, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then20, label %if.merge21

if.then20:                                        ; preds = %short_circuit.merge
  store i32 3, ptr %assignment_value, align 4
  %assignment_value22 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value22, ptr %result, align 4
  br label %if.merge21

if.merge21:                                       ; preds = %if.then20, %short_circuit.merge
  %remaining23 = load i64, ptr %remaining, align 4
  %one24 = load i64, ptr %one, align 4
  %sub = sub i64 %remaining23, %one24
  store i64 %sub, ptr %assignment_value25, align 4
  %assignment_value26 = load i64, ptr %assignment_value25, align 4
  store i64 %assignment_value26, ptr %remaining, align 4
  %continuation_minimum27 = load i8, ptr %continuation_minimum, align 1
  store i8 %continuation_minimum27, ptr %assignment_value28, align 1
  %assignment_value29 = load i8, ptr %assignment_value28, align 1
  store i8 %assignment_value29, ptr %next_minimum, align 1
  %continuation_maximum30 = load i8, ptr %continuation_maximum, align 1
  store i8 %continuation_maximum30, ptr %assignment_value31, align 1
  %assignment_value32 = load i8, ptr %assignment_value31, align 1
  store i8 %assignment_value32, ptr %next_maximum, align 1
  br label %if.merge

short_circuit.rhs35:                              ; preds = %if.else
  %byte37 = load i8, ptr %byte, align 1
  %two_byte_minimum38 = load i8, ptr %two_byte_minimum, align 1
  %lt39 = icmp ult i8 %byte37, %two_byte_minimum38
  br label %short_circuit.merge36

short_circuit.merge36:                            ; preds = %short_circuit.rhs35, %if.else
  %short_circuit40 = phi i1 [ false, %if.else ], [ %lt39, %short_circuit.rhs35 ]
  br i1 %short_circuit40, label %if.then41, label %if.merge42

if.then41:                                        ; preds = %short_circuit.merge36
  store i32 3, ptr %assignment_value43, align 4
  %assignment_value44 = load i32, ptr %assignment_value43, align 4
  store i32 %assignment_value44, ptr %result, align 4
  br label %if.merge42

if.merge42:                                       ; preds = %if.then41, %short_circuit.merge36
  %byte45 = load i8, ptr %byte, align 1
  %two_byte_minimum46 = load i8, ptr %two_byte_minimum, align 1
  %ge47 = icmp uge i8 %byte45, %two_byte_minimum46
  br i1 %ge47, label %short_circuit.rhs48, label %short_circuit.merge49

short_circuit.rhs48:                              ; preds = %if.merge42
  %byte50 = load i8, ptr %byte, align 1
  %three_byte_minimum51 = load i8, ptr %three_byte_minimum, align 1
  %lt52 = icmp ult i8 %byte50, %three_byte_minimum51
  br label %short_circuit.merge49

short_circuit.merge49:                            ; preds = %short_circuit.rhs48, %if.merge42
  %short_circuit53 = phi i1 [ false, %if.merge42 ], [ %lt52, %short_circuit.rhs48 ]
  br i1 %short_circuit53, label %if.then54, label %if.merge55

if.then54:                                        ; preds = %short_circuit.merge49
  %one56 = load i64, ptr %one, align 4
  store i64 %one56, ptr %assignment_value57, align 4
  %assignment_value58 = load i64, ptr %assignment_value57, align 4
  store i64 %assignment_value58, ptr %remaining, align 4
  br label %if.merge55

if.merge55:                                       ; preds = %if.then54, %short_circuit.merge49
  %byte59 = load i8, ptr %byte, align 1
  %three_byte_minimum60 = load i8, ptr %three_byte_minimum, align 1
  %ge61 = icmp uge i8 %byte59, %three_byte_minimum60
  br i1 %ge61, label %short_circuit.rhs62, label %short_circuit.merge63

short_circuit.rhs62:                              ; preds = %if.merge55
  %byte64 = load i8, ptr %byte, align 1
  %four_byte_minimum65 = load i8, ptr %four_byte_minimum, align 1
  %lt66 = icmp ult i8 %byte64, %four_byte_minimum65
  br label %short_circuit.merge63

short_circuit.merge63:                            ; preds = %short_circuit.rhs62, %if.merge55
  %short_circuit67 = phi i1 [ false, %if.merge55 ], [ %lt66, %short_circuit.rhs62 ]
  br i1 %short_circuit67, label %if.then68, label %if.merge69

if.then68:                                        ; preds = %short_circuit.merge63
  store i64 2, ptr %assignment_value70, align 4
  %assignment_value71 = load i64, ptr %assignment_value70, align 4
  store i64 %assignment_value71, ptr %remaining, align 4
  %byte72 = load i8, ptr %byte, align 1
  %e073 = load i8, ptr %e0, align 1
  %eq = icmp eq i8 %byte72, %e073
  br i1 %eq, label %if.then74, label %if.merge75

if.merge69:                                       ; preds = %if.merge82, %short_circuit.merge63
  %byte85 = load i8, ptr %byte, align 1
  %four_byte_minimum86 = load i8, ptr %four_byte_minimum, align 1
  %ge87 = icmp uge i8 %byte85, %four_byte_minimum86
  br i1 %ge87, label %short_circuit.rhs88, label %short_circuit.merge89

if.then74:                                        ; preds = %if.then68
  store i8 -96, ptr %assignment_value76, align 1
  %assignment_value77 = load i8, ptr %assignment_value76, align 1
  store i8 %assignment_value77, ptr %next_minimum, align 1
  br label %if.merge75

if.merge75:                                       ; preds = %if.then74, %if.then68
  %byte78 = load i8, ptr %byte, align 1
  %ed79 = load i8, ptr %ed, align 1
  %eq80 = icmp eq i8 %byte78, %ed79
  br i1 %eq80, label %if.then81, label %if.merge82

if.then81:                                        ; preds = %if.merge75
  store i8 -97, ptr %assignment_value83, align 1
  %assignment_value84 = load i8, ptr %assignment_value83, align 1
  store i8 %assignment_value84, ptr %next_maximum, align 1
  br label %if.merge82

if.merge82:                                       ; preds = %if.then81, %if.merge75
  br label %if.merge69

short_circuit.rhs88:                              ; preds = %if.merge69
  %byte90 = load i8, ptr %byte, align 1
  %four_byte_limit91 = load i8, ptr %four_byte_limit, align 1
  %le = icmp ule i8 %byte90, %four_byte_limit91
  br label %short_circuit.merge89

short_circuit.merge89:                            ; preds = %short_circuit.rhs88, %if.merge69
  %short_circuit92 = phi i1 [ false, %if.merge69 ], [ %le, %short_circuit.rhs88 ]
  br i1 %short_circuit92, label %if.then93, label %if.merge94

if.then93:                                        ; preds = %short_circuit.merge89
  store i64 3, ptr %assignment_value95, align 4
  %assignment_value96 = load i64, ptr %assignment_value95, align 4
  store i64 %assignment_value96, ptr %remaining, align 4
  %byte97 = load i8, ptr %byte, align 1
  %f098 = load i8, ptr %f0, align 1
  %eq99 = icmp eq i8 %byte97, %f098
  br i1 %eq99, label %if.then100, label %if.merge101

if.merge94:                                       ; preds = %if.merge108, %short_circuit.merge89
  %byte111 = load i8, ptr %byte, align 1
  %four_byte_limit112 = load i8, ptr %four_byte_limit, align 1
  %gt113 = icmp ugt i8 %byte111, %four_byte_limit112
  br i1 %gt113, label %if.then114, label %if.merge115

if.then100:                                       ; preds = %if.then93
  store i8 -112, ptr %assignment_value102, align 1
  %assignment_value103 = load i8, ptr %assignment_value102, align 1
  store i8 %assignment_value103, ptr %next_minimum, align 1
  br label %if.merge101

if.merge101:                                      ; preds = %if.then100, %if.then93
  %byte104 = load i8, ptr %byte, align 1
  %f4105 = load i8, ptr %f4, align 1
  %eq106 = icmp eq i8 %byte104, %f4105
  br i1 %eq106, label %if.then107, label %if.merge108

if.then107:                                       ; preds = %if.merge101
  store i8 -113, ptr %assignment_value109, align 1
  %assignment_value110 = load i8, ptr %assignment_value109, align 1
  store i8 %assignment_value110, ptr %next_maximum, align 1
  br label %if.merge108

if.merge108:                                      ; preds = %if.then107, %if.merge101
  br label %if.merge94

if.then114:                                       ; preds = %if.merge94
  store i32 3, ptr %assignment_value116, align 4
  %assignment_value117 = load i32, ptr %assignment_value116, align 4
  store i32 %assignment_value117, ptr %result, align 4
  br label %if.merge115

if.merge115:                                      ; preds = %if.then114, %if.merge94
  br label %if.merge

if.then125:                                       ; preds = %while.exit.2
  store i32 3, ptr %assignment_value127, align 4
  %assignment_value128 = load i32, ptr %assignment_value127, align 4
  store i32 %assignment_value128, ptr %result, align 4
  br label %if.merge126

if.merge126:                                      ; preds = %if.then125, %while.exit.2
  %result129 = load i32, ptr %result, align 4
  ret i32 %result129
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f726561645f6c696e655f64697363617264() {
entry:
  %assignment_value34 = alloca i1, align 1
  %assignment_value19 = alloca i1, align 1
  %assignment_value12 = alloca i1, align 1
  %assignment_value10 = alloca i32, align 4
  %assignment_value7 = alloca i1, align 1
  %assignment_value = alloca i64, align 8
  %complete = alloca i1, align 1
  %count = alloca i64, align 8
  %status = alloca i32, align 4
  %reading = alloca i1, align 1
  %scratch = alloca [1 x i8], align 1
  %array_literal = alloca [1 x i8], align 1
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  %array_element = getelementptr inbounds [1 x i8], ptr %array_literal, i32 0, i32 0
  store i8 0, ptr %array_element, align 1
  %struct_value = load [1 x i8], ptr %array_literal, align 1
  store [1 x i8] %struct_value, ptr %scratch, align 1
  store i1 true, ptr %reading, align 1
  store i32 4, ptr %status, align 4
  br label %while.cond.0

while.cond.0:                                     ; preds = %if.merge33, %entry
  %reading1 = load i1, ptr %reading, align 1
  br i1 %reading1, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %zero2 = load i64, ptr %zero, align 4
  store i64 %zero2, ptr %count, align 4
  store i1 false, ptr %complete, align 1
  %array_element3 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i64 0
  %address = ptrtoint ptr %array_element3 to i32
  %one4 = load i64, ptr %one, align 4
  %call = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__726561645f696e746f(i32 %address, i64 %one4)
  %output = extractvalue { i64, i1 } %call, 0
  store i64 %output, ptr %assignment_value, align 4
  %assignment_value5 = load i64, ptr %assignment_value, align 4
  %output6 = extractvalue { i64, i1 } %call, 1
  store i1 %output6, ptr %assignment_value7, align 1
  %assignment_value8 = load i1, ptr %assignment_value7, align 1
  store i64 %assignment_value5, ptr %count, align 4
  store i1 %assignment_value8, ptr %complete, align 1
  %complete9 = load i1, ptr %complete, align 1
  %not = xor i1 %complete9, true
  br i1 %not, label %if.then, label %if.merge

while.exit.2:                                     ; preds = %while.cond.0
  %status36 = load i32, ptr %status, align 4
  ret i32 %status36

if.then:                                          ; preds = %while.body.1
  store i32 2, ptr %assignment_value10, align 4
  %assignment_value11 = load i32, ptr %assignment_value10, align 4
  store i32 %assignment_value11, ptr %status, align 4
  store i1 false, ptr %assignment_value12, align 1
  %assignment_value13 = load i1, ptr %assignment_value12, align 1
  store i1 %assignment_value13, ptr %reading, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %while.body.1
  %complete14 = load i1, ptr %complete, align 1
  br i1 %complete14, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %if.merge
  %count15 = load i64, ptr %count, align 4
  %zero16 = load i64, ptr %zero, align 4
  %eq = icmp eq i64 %count15, %zero16
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %if.merge
  %short_circuit = phi i1 [ false, %if.merge ], [ %eq, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then17, label %if.merge18

if.then17:                                        ; preds = %short_circuit.merge
  store i1 false, ptr %assignment_value19, align 1
  %assignment_value20 = load i1, ptr %assignment_value19, align 1
  store i1 %assignment_value20, ptr %reading, align 1
  br label %if.merge18

if.merge18:                                       ; preds = %if.then17, %short_circuit.merge
  %complete21 = load i1, ptr %complete, align 1
  br i1 %complete21, label %short_circuit.rhs22, label %short_circuit.merge23

short_circuit.rhs22:                              ; preds = %if.merge18
  %count24 = load i64, ptr %count, align 4
  %zero25 = load i64, ptr %zero, align 4
  %ne = icmp ne i64 %count24, %zero25
  br label %short_circuit.merge23

short_circuit.merge23:                            ; preds = %short_circuit.rhs22, %if.merge18
  %short_circuit26 = phi i1 [ false, %if.merge18 ], [ %ne, %short_circuit.rhs22 ]
  br i1 %short_circuit26, label %short_circuit.rhs27, label %short_circuit.merge28

short_circuit.rhs27:                              ; preds = %short_circuit.merge23
  %array_element29 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i64 0
  %place = load i8, ptr %array_element29, align 1
  %eq30 = icmp eq i8 %place, 10
  br label %short_circuit.merge28

short_circuit.merge28:                            ; preds = %short_circuit.rhs27, %short_circuit.merge23
  %short_circuit31 = phi i1 [ false, %short_circuit.merge23 ], [ %eq30, %short_circuit.rhs27 ]
  br i1 %short_circuit31, label %if.then32, label %if.merge33

if.then32:                                        ; preds = %short_circuit.merge28
  store i1 false, ptr %assignment_value34, align 1
  %assignment_value35 = load i1, ptr %assignment_value34, align 1
  store i1 %assignment_value35, ptr %reading, align 1
  br label %if.merge33

if.merge33:                                       ; preds = %if.then32, %short_circuit.merge28
  br label %while.cond.0
}

define { i32, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f726561645f6c696e655f616c6c6f6361746564(i64 %max_bytes, i8 %first, i1 %pending_cr) {
entry:
  %text = alloca [16 x i8], align 1
  %struct_literal = alloca [16 x i8], align 1
  %assignment_value184 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value171 = alloca i32, align 4
  %assignment_value162 = alloca i64, align 8
  %assignment_value155 = alloca i8, align 1
  %assignment_value146 = alloca i32, align 4
  %assignment_value117 = alloca i1, align 1
  %assignment_value95 = alloca i64, align 8
  %assignment_value88 = alloca i8, align 1
  %assignment_value81 = alloca i32, align 4
  %assignment_value55 = alloca i1, align 1
  %assignment_value53 = alloca i1, align 1
  %assignment_value37 = alloca i1, align 1
  %assignment_value30 = alloca i1, align 1
  %assignment_value28 = alloca i32, align 4
  %assignment_value22 = alloca i1, align 1
  %assignment_value19 = alloca i64, align 8
  %complete = alloca i1, align 1
  %count = alloca i64, align 8
  %assignment_value13 = alloca i64, align 8
  %assignment_value = alloca i8, align 1
  %status = alloca i32, align 4
  %reading = alloca i1, align 1
  %length = alloca i64, align 8
  %scratch = alloca [1 x i8], align 1
  %array_literal = alloca [1 x i8], align 1
  %cr = alloca i8, align 1
  %lf = alloca i8, align 1
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %max_bytes1 = alloca i64, align 8
  store i64 %max_bytes, ptr %max_bytes1, align 4
  %first2 = alloca i8, align 1
  store i8 %first, ptr %first2, align 1
  %pending_cr3 = alloca i1, align 1
  store i1 %pending_cr, ptr %pending_cr3, align 1
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store i8 10, ptr %lf, align 1
  store i8 13, ptr %cr, align 1
  %array_element = getelementptr inbounds [1 x i8], ptr %array_literal, i32 0, i32 0
  store i8 0, ptr %array_element, align 1
  %struct_value = load [1 x i8], ptr %array_literal, align 1
  store [1 x i8] %struct_value, ptr %scratch, align 1
  %max_bytes4 = load i64, ptr %max_bytes1, align 4
  %allocation_size = mul i64 %max_bytes4, 1
  %allocation = call ptr @__wosy_core_alloc(i64 %allocation_size, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  %zero5 = load i64, ptr %zero, align 4
  store i64 %zero5, ptr %length, align 4
  store i1 true, ptr %reading, align 1
  store i32 0, ptr %status, align 4
  %pending_cr6 = load i1, ptr %pending_cr3, align 1
  %not = xor i1 %pending_cr6, true
  br i1 %not, label %if.then, label %if.merge

if.then:                                          ; preds = %allocation_continue
  %first7 = load i8, ptr %first2, align 1
  store i8 %first7, ptr %assignment_value, align 1
  %assignment_value8 = load i8, ptr %assignment_value, align 1
  %length9 = load i64, ptr %length, align 4
  %array_element10 = getelementptr inbounds i8, ptr %allocation, i64 %length9
  store i8 %assignment_value8, ptr %array_element10, align 1
  %length11 = load i64, ptr %length, align 4
  %one12 = load i64, ptr %one, align 4
  %add = add i64 %length11, %one12
  store i64 %add, ptr %assignment_value13, align 4
  %assignment_value14 = load i64, ptr %assignment_value13, align 4
  store i64 %assignment_value14, ptr %length, align 4
  br label %if.merge

if.merge:                                         ; preds = %if.then, %allocation_continue
  br label %while.cond.0

while.cond.0:                                     ; preds = %if.merge141, %if.merge
  %reading15 = load i1, ptr %reading, align 1
  br i1 %reading15, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %zero16 = load i64, ptr %zero, align 4
  store i64 %zero16, ptr %count, align 4
  store i1 false, ptr %complete, align 1
  %array_element17 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i64 0
  %address = ptrtoint ptr %array_element17 to i32
  %one18 = load i64, ptr %one, align 4
  %call = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__726561645f696e746f(i32 %address, i64 %one18)
  %output = extractvalue { i64, i1 } %call, 0
  store i64 %output, ptr %assignment_value19, align 4
  %assignment_value20 = load i64, ptr %assignment_value19, align 4
  %output21 = extractvalue { i64, i1 } %call, 1
  store i1 %output21, ptr %assignment_value22, align 1
  %assignment_value23 = load i1, ptr %assignment_value22, align 1
  store i64 %assignment_value20, ptr %count, align 4
  store i1 %assignment_value23, ptr %complete, align 1
  %complete24 = load i1, ptr %complete, align 1
  %not25 = xor i1 %complete24, true
  br i1 %not25, label %if.then26, label %if.merge27

while.exit.2:                                     ; preds = %while.cond.0
  %status164 = load i32, ptr %status, align 4
  %eq165 = icmp eq i32 %status164, 0
  br i1 %eq165, label %if.then166, label %if.merge167

if.then26:                                        ; preds = %while.body.1
  store i32 2, ptr %assignment_value28, align 4
  %assignment_value29 = load i32, ptr %assignment_value28, align 4
  store i32 %assignment_value29, ptr %status, align 4
  store i1 false, ptr %assignment_value30, align 1
  %assignment_value31 = load i1, ptr %assignment_value30, align 1
  store i1 %assignment_value31, ptr %reading, align 1
  br label %if.merge27

if.merge27:                                       ; preds = %if.then26, %while.body.1
  %complete32 = load i1, ptr %complete, align 1
  br i1 %complete32, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %if.merge27
  %count33 = load i64, ptr %count, align 4
  %zero34 = load i64, ptr %zero, align 4
  %eq = icmp eq i64 %count33, %zero34
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %if.merge27
  %short_circuit = phi i1 [ false, %if.merge27 ], [ %eq, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then35, label %if.merge36

if.then35:                                        ; preds = %short_circuit.merge
  store i1 false, ptr %assignment_value37, align 1
  %assignment_value38 = load i1, ptr %assignment_value37, align 1
  store i1 %assignment_value38, ptr %reading, align 1
  br label %if.merge36

if.merge36:                                       ; preds = %if.then35, %short_circuit.merge
  %complete39 = load i1, ptr %complete, align 1
  br i1 %complete39, label %short_circuit.rhs40, label %short_circuit.merge41

short_circuit.rhs40:                              ; preds = %if.merge36
  %count42 = load i64, ptr %count, align 4
  %zero43 = load i64, ptr %zero, align 4
  %ne = icmp ne i64 %count42, %zero43
  br label %short_circuit.merge41

short_circuit.merge41:                            ; preds = %short_circuit.rhs40, %if.merge36
  %short_circuit44 = phi i1 [ false, %if.merge36 ], [ %ne, %short_circuit.rhs40 ]
  br i1 %short_circuit44, label %short_circuit.rhs45, label %short_circuit.merge46

short_circuit.rhs45:                              ; preds = %short_circuit.merge41
  %array_element47 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i64 0
  %place = load i8, ptr %array_element47, align 1
  %lf48 = load i8, ptr %lf, align 1
  %eq49 = icmp eq i8 %place, %lf48
  br label %short_circuit.merge46

short_circuit.merge46:                            ; preds = %short_circuit.rhs45, %short_circuit.merge41
  %short_circuit50 = phi i1 [ false, %short_circuit.merge41 ], [ %eq49, %short_circuit.rhs45 ]
  br i1 %short_circuit50, label %if.then51, label %if.merge52

if.then51:                                        ; preds = %short_circuit.merge46
  store i1 false, ptr %assignment_value53, align 1
  %assignment_value54 = load i1, ptr %assignment_value53, align 1
  store i1 %assignment_value54, ptr %pending_cr3, align 1
  store i1 false, ptr %assignment_value55, align 1
  %assignment_value56 = load i1, ptr %assignment_value55, align 1
  store i1 %assignment_value56, ptr %reading, align 1
  br label %if.merge52

if.merge52:                                       ; preds = %if.then51, %short_circuit.merge46
  %complete57 = load i1, ptr %complete, align 1
  br i1 %complete57, label %short_circuit.rhs58, label %short_circuit.merge59

short_circuit.rhs58:                              ; preds = %if.merge52
  %count60 = load i64, ptr %count, align 4
  %zero61 = load i64, ptr %zero, align 4
  %ne62 = icmp ne i64 %count60, %zero61
  br label %short_circuit.merge59

short_circuit.merge59:                            ; preds = %short_circuit.rhs58, %if.merge52
  %short_circuit63 = phi i1 [ false, %if.merge52 ], [ %ne62, %short_circuit.rhs58 ]
  br i1 %short_circuit63, label %short_circuit.rhs64, label %short_circuit.merge65

short_circuit.rhs64:                              ; preds = %short_circuit.merge59
  %array_element66 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i64 0
  %place67 = load i8, ptr %array_element66, align 1
  %lf68 = load i8, ptr %lf, align 1
  %ne69 = icmp ne i8 %place67, %lf68
  br label %short_circuit.merge65

short_circuit.merge65:                            ; preds = %short_circuit.rhs64, %short_circuit.merge59
  %short_circuit70 = phi i1 [ false, %short_circuit.merge59 ], [ %ne69, %short_circuit.rhs64 ]
  br i1 %short_circuit70, label %short_circuit.rhs71, label %short_circuit.merge72

short_circuit.rhs71:                              ; preds = %short_circuit.merge65
  %pending_cr73 = load i1, ptr %pending_cr3, align 1
  br label %short_circuit.merge72

short_circuit.merge72:                            ; preds = %short_circuit.rhs71, %short_circuit.merge65
  %short_circuit74 = phi i1 [ false, %short_circuit.merge65 ], [ %pending_cr73, %short_circuit.rhs71 ]
  br i1 %short_circuit74, label %if.then75, label %if.merge76

if.then75:                                        ; preds = %short_circuit.merge72
  %length77 = load i64, ptr %length, align 4
  %max_bytes78 = load i64, ptr %max_bytes1, align 4
  %status79 = load i32, ptr %status, align 4
  %call80 = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f726561645f6c696e655f63617061636974795f737461747573(i64 %length77, i64 %max_bytes78, i32 %status79)
  store i32 %call80, ptr %assignment_value81, align 4
  %assignment_value82 = load i32, ptr %assignment_value81, align 4
  store i32 %assignment_value82, ptr %status, align 4
  %length83 = load i64, ptr %length, align 4
  %max_bytes84 = load i64, ptr %max_bytes1, align 4
  %lt = icmp ult i64 %length83, %max_bytes84
  br i1 %lt, label %if.then85, label %if.merge86

if.merge76:                                       ; preds = %if.merge86, %short_circuit.merge72
  %complete97 = load i1, ptr %complete, align 1
  br i1 %complete97, label %short_circuit.rhs98, label %short_circuit.merge99

if.then85:                                        ; preds = %if.then75
  %cr87 = load i8, ptr %cr, align 1
  store i8 %cr87, ptr %assignment_value88, align 1
  %assignment_value89 = load i8, ptr %assignment_value88, align 1
  %length90 = load i64, ptr %length, align 4
  %array_element91 = getelementptr inbounds i8, ptr %allocation, i64 %length90
  store i8 %assignment_value89, ptr %array_element91, align 1
  %length92 = load i64, ptr %length, align 4
  %one93 = load i64, ptr %one, align 4
  %add94 = add i64 %length92, %one93
  store i64 %add94, ptr %assignment_value95, align 4
  %assignment_value96 = load i64, ptr %assignment_value95, align 4
  store i64 %assignment_value96, ptr %length, align 4
  br label %if.merge86

if.merge86:                                       ; preds = %if.then85, %if.then75
  br label %if.merge76

short_circuit.rhs98:                              ; preds = %if.merge76
  %count100 = load i64, ptr %count, align 4
  %zero101 = load i64, ptr %zero, align 4
  %ne102 = icmp ne i64 %count100, %zero101
  br label %short_circuit.merge99

short_circuit.merge99:                            ; preds = %short_circuit.rhs98, %if.merge76
  %short_circuit103 = phi i1 [ false, %if.merge76 ], [ %ne102, %short_circuit.rhs98 ]
  br i1 %short_circuit103, label %short_circuit.rhs104, label %short_circuit.merge105

short_circuit.rhs104:                             ; preds = %short_circuit.merge99
  %array_element106 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i64 0
  %place107 = load i8, ptr %array_element106, align 1
  %lf108 = load i8, ptr %lf, align 1
  %ne109 = icmp ne i8 %place107, %lf108
  br label %short_circuit.merge105

short_circuit.merge105:                           ; preds = %short_circuit.rhs104, %short_circuit.merge99
  %short_circuit110 = phi i1 [ false, %short_circuit.merge99 ], [ %ne109, %short_circuit.rhs104 ]
  br i1 %short_circuit110, label %if.then111, label %if.merge112

if.then111:                                       ; preds = %short_circuit.merge105
  %array_element113 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i64 0
  %place114 = load i8, ptr %array_element113, align 1
  %cr115 = load i8, ptr %cr, align 1
  %eq116 = icmp eq i8 %place114, %cr115
  store i1 %eq116, ptr %assignment_value117, align 1
  %assignment_value118 = load i1, ptr %assignment_value117, align 1
  store i1 %assignment_value118, ptr %pending_cr3, align 1
  br label %if.merge112

if.merge112:                                      ; preds = %if.then111, %short_circuit.merge105
  %complete119 = load i1, ptr %complete, align 1
  br i1 %complete119, label %short_circuit.rhs120, label %short_circuit.merge121

short_circuit.rhs120:                             ; preds = %if.merge112
  %count122 = load i64, ptr %count, align 4
  %zero123 = load i64, ptr %zero, align 4
  %ne124 = icmp ne i64 %count122, %zero123
  br label %short_circuit.merge121

short_circuit.merge121:                           ; preds = %short_circuit.rhs120, %if.merge112
  %short_circuit125 = phi i1 [ false, %if.merge112 ], [ %ne124, %short_circuit.rhs120 ]
  br i1 %short_circuit125, label %short_circuit.rhs126, label %short_circuit.merge127

short_circuit.rhs126:                             ; preds = %short_circuit.merge121
  %array_element128 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i64 0
  %place129 = load i8, ptr %array_element128, align 1
  %lf130 = load i8, ptr %lf, align 1
  %ne131 = icmp ne i8 %place129, %lf130
  br label %short_circuit.merge127

short_circuit.merge127:                           ; preds = %short_circuit.rhs126, %short_circuit.merge121
  %short_circuit132 = phi i1 [ false, %short_circuit.merge121 ], [ %ne131, %short_circuit.rhs126 ]
  br i1 %short_circuit132, label %short_circuit.rhs133, label %short_circuit.merge134

short_circuit.rhs133:                             ; preds = %short_circuit.merge127
  %array_element135 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i64 0
  %place136 = load i8, ptr %array_element135, align 1
  %cr137 = load i8, ptr %cr, align 1
  %ne138 = icmp ne i8 %place136, %cr137
  br label %short_circuit.merge134

short_circuit.merge134:                           ; preds = %short_circuit.rhs133, %short_circuit.merge127
  %short_circuit139 = phi i1 [ false, %short_circuit.merge127 ], [ %ne138, %short_circuit.rhs133 ]
  br i1 %short_circuit139, label %if.then140, label %if.merge141

if.then140:                                       ; preds = %short_circuit.merge134
  %length142 = load i64, ptr %length, align 4
  %max_bytes143 = load i64, ptr %max_bytes1, align 4
  %status144 = load i32, ptr %status, align 4
  %call145 = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f726561645f6c696e655f63617061636974795f737461747573(i64 %length142, i64 %max_bytes143, i32 %status144)
  store i32 %call145, ptr %assignment_value146, align 4
  %assignment_value147 = load i32, ptr %assignment_value146, align 4
  store i32 %assignment_value147, ptr %status, align 4
  %length148 = load i64, ptr %length, align 4
  %max_bytes149 = load i64, ptr %max_bytes1, align 4
  %lt150 = icmp ult i64 %length148, %max_bytes149
  br i1 %lt150, label %if.then151, label %if.merge152

if.merge141:                                      ; preds = %if.merge152, %short_circuit.merge134
  br label %while.cond.0

if.then151:                                       ; preds = %if.then140
  %array_element153 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i64 0
  %place154 = load i8, ptr %array_element153, align 1
  store i8 %place154, ptr %assignment_value155, align 1
  %assignment_value156 = load i8, ptr %assignment_value155, align 1
  %length157 = load i64, ptr %length, align 4
  %array_element158 = getelementptr inbounds i8, ptr %allocation, i64 %length157
  store i8 %assignment_value156, ptr %array_element158, align 1
  %length159 = load i64, ptr %length, align 4
  %one160 = load i64, ptr %one, align 4
  %add161 = add i64 %length159, %one160
  store i64 %add161, ptr %assignment_value162, align 4
  %assignment_value163 = load i64, ptr %assignment_value162, align 4
  store i64 %assignment_value163, ptr %length, align 4
  br label %if.merge152

if.merge152:                                      ; preds = %if.then151, %if.then140
  br label %if.merge141

if.then166:                                       ; preds = %while.exit.2
  %length168 = load i64, ptr %length, align 4
  %status169 = load i32, ptr %status, align 4
  %call170 = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f726561645f6c696e655f757466385f737461747573(ptr %allocation, i64 %length168, i32 %status169)
  store i32 %call170, ptr %assignment_value171, align 4
  %assignment_value172 = load i32, ptr %assignment_value171, align 4
  store i32 %assignment_value172, ptr %status, align 4
  br label %if.merge167

if.merge167:                                      ; preds = %if.then166, %while.exit.2
  %status173 = load i32, ptr %status, align 4
  %eq174 = icmp eq i32 %status173, 0
  br i1 %eq174, label %if.then175, label %if.else

if.then175:                                       ; preds = %if.merge167
  store i32 0, ptr %data, align 4
  %length177 = load i64, ptr %length, align 4
  %zero178 = load i64, ptr %zero, align 4
  %ne179 = icmp ne i64 %length177, %zero178
  br i1 %ne179, label %if.then180, label %if.merge181

if.else:                                          ; preds = %if.merge167
  %status195 = load i32, ptr %status, align 4
  %output196 = insertvalue { i32, i32 } zeroinitializer, i32 %status195, 1
  br label %if.merge176

if.merge176:                                      ; preds = %if.else, %if.merge181
  %if = phi { i32, i32 } [ %output194, %if.merge181 ], [ %output196, %if.else ]
  ret { i32, i32 } %if

if.then180:                                       ; preds = %if.then175
  %array_element182 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address183 = ptrtoint ptr %array_element182 to i32
  store i32 %address183, ptr %assignment_value184, align 4
  %assignment_value185 = load i32, ptr %assignment_value184, align 4
  store i32 %assignment_value185, ptr %data, align 4
  br label %if.merge181

if.merge181:                                      ; preds = %if.then180, %if.then175
  %data186 = load i32, ptr %data, align 4
  %length187 = load i64, ptr %length, align 4
  %data188 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data186, ptr %data188, align 4
  %length189 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length187, ptr %length189, align 4
  %struct_value190 = load [16 x i8], ptr %struct_literal, align 1
  store [16 x i8] %struct_value190, ptr %text, align 1
  %address191 = ptrtoint ptr %text to i32
  %status192 = load i32, ptr %status, align 4
  %output193 = insertvalue { i32, i32 } zeroinitializer, i32 %address191, 0
  %output194 = insertvalue { i32, i32 } %output193, i32 %status192, 1
  br label %if.merge176
}

define { i32, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__726561645f6c696e65(i64 %max_bytes) {
entry:
  %assignment_value111 = alloca i1, align 1
  %assignment_value109 = alloca i32, align 4
  %assignment_value106 = alloca i32, align 4
  %assignment_value75 = alloca i1, align 1
  %assignment_value73 = alloca i32, align 4
  %assignment_value49 = alloca i1, align 1
  %assignment_value47 = alloca i32, align 4
  %assignment_value45 = alloca i32, align 4
  %empty = alloca [16 x i8], align 1
  %struct_literal = alloca [16 x i8], align 1
  %assignment_value27 = alloca i1, align 1
  %assignment_value25 = alloca i32, align 4
  %assignment_value18 = alloca i1, align 1
  %assignment_value16 = alloca i32, align 4
  %assignment_value11 = alloca i1, align 1
  %assignment_value = alloca i64, align 8
  %complete = alloca i1, align 1
  %count = alloca i64, align 8
  %status = alloca i32, align 4
  %text = alloca i32, align 4
  %reading = alloca i1, align 1
  %scratch = alloca [1 x i8], align 1
  %array_literal = alloca [1 x i8], align 1
  %maximum_u32 = alloca i64, align 8
  %maximum_u32_value = alloca i32, align 4
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %max_bytes1 = alloca i64, align 8
  store i64 %max_bytes, ptr %max_bytes1, align 4
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store i32 -1, ptr %maximum_u32_value, align 4
  %maximum_u32_value2 = load i32, ptr %maximum_u32_value, align 4
  %int_extend = zext i32 %maximum_u32_value2 to i64
  store i64 %int_extend, ptr %maximum_u32, align 4
  %max_bytes3 = load i64, ptr %max_bytes1, align 4
  %maximum_u324 = load i64, ptr %maximum_u32, align 4
  %gt = icmp ugt i64 %max_bytes3, %maximum_u324
  br i1 %gt, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  br label %if.merge

if.else:                                          ; preds = %entry
  %array_element = getelementptr inbounds [1 x i8], ptr %array_literal, i32 0, i32 0
  store i8 0, ptr %array_element, align 1
  %struct_value = load [1 x i8], ptr %array_literal, align 1
  store [1 x i8] %struct_value, ptr %scratch, align 1
  store i1 true, ptr %reading, align 1
  store i32 0, ptr %text, align 4
  store i32 1, ptr %status, align 4
  br label %while.cond.0

if.merge:                                         ; preds = %while.exit.2, %if.then
  %if = phi { i32, i32 } [ { i32 0, i32 4 }, %if.then ], [ %output116, %while.exit.2 ]
  ret { i32, i32 } %if

while.cond.0:                                     ; preds = %if.merge97, %if.else
  %reading5 = load i1, ptr %reading, align 1
  br i1 %reading5, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %zero6 = load i64, ptr %zero, align 4
  store i64 %zero6, ptr %count, align 4
  store i1 false, ptr %complete, align 1
  %array_element7 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i64 0
  %address = ptrtoint ptr %array_element7 to i32
  %one8 = load i64, ptr %one, align 4
  %call = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__726561645f696e746f(i32 %address, i64 %one8)
  %output = extractvalue { i64, i1 } %call, 0
  store i64 %output, ptr %assignment_value, align 4
  %assignment_value9 = load i64, ptr %assignment_value, align 4
  %output10 = extractvalue { i64, i1 } %call, 1
  store i1 %output10, ptr %assignment_value11, align 1
  %assignment_value12 = load i1, ptr %assignment_value11, align 1
  store i64 %assignment_value9, ptr %count, align 4
  store i1 %assignment_value12, ptr %complete, align 1
  %complete13 = load i1, ptr %complete, align 1
  %not = xor i1 %complete13, true
  br i1 %not, label %if.then14, label %if.merge15

while.exit.2:                                     ; preds = %while.cond.0
  %text113 = load i32, ptr %text, align 4
  %status114 = load i32, ptr %status, align 4
  %output115 = insertvalue { i32, i32 } zeroinitializer, i32 %text113, 0
  %output116 = insertvalue { i32, i32 } %output115, i32 %status114, 1
  br label %if.merge

if.then14:                                        ; preds = %while.body.1
  store i32 2, ptr %assignment_value16, align 4
  %assignment_value17 = load i32, ptr %assignment_value16, align 4
  store i32 %assignment_value17, ptr %status, align 4
  store i1 false, ptr %assignment_value18, align 1
  %assignment_value19 = load i1, ptr %assignment_value18, align 1
  store i1 %assignment_value19, ptr %reading, align 1
  br label %if.merge15

if.merge15:                                       ; preds = %if.then14, %while.body.1
  %complete20 = load i1, ptr %complete, align 1
  br i1 %complete20, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %if.merge15
  %count21 = load i64, ptr %count, align 4
  %zero22 = load i64, ptr %zero, align 4
  %eq = icmp eq i64 %count21, %zero22
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %if.merge15
  %short_circuit = phi i1 [ false, %if.merge15 ], [ %eq, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then23, label %if.merge24

if.then23:                                        ; preds = %short_circuit.merge
  store i32 1, ptr %assignment_value25, align 4
  %assignment_value26 = load i32, ptr %assignment_value25, align 4
  store i32 %assignment_value26, ptr %status, align 4
  store i1 false, ptr %assignment_value27, align 1
  %assignment_value28 = load i1, ptr %assignment_value27, align 1
  store i1 %assignment_value28, ptr %reading, align 1
  br label %if.merge24

if.merge24:                                       ; preds = %if.then23, %short_circuit.merge
  %complete29 = load i1, ptr %complete, align 1
  br i1 %complete29, label %short_circuit.rhs30, label %short_circuit.merge31

short_circuit.rhs30:                              ; preds = %if.merge24
  %count32 = load i64, ptr %count, align 4
  %zero33 = load i64, ptr %zero, align 4
  %ne = icmp ne i64 %count32, %zero33
  br label %short_circuit.merge31

short_circuit.merge31:                            ; preds = %short_circuit.rhs30, %if.merge24
  %short_circuit34 = phi i1 [ false, %if.merge24 ], [ %ne, %short_circuit.rhs30 ]
  br i1 %short_circuit34, label %short_circuit.rhs35, label %short_circuit.merge36

short_circuit.rhs35:                              ; preds = %short_circuit.merge31
  %array_element37 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i64 0
  %place = load i8, ptr %array_element37, align 1
  %eq38 = icmp eq i8 %place, 10
  br label %short_circuit.merge36

short_circuit.merge36:                            ; preds = %short_circuit.rhs35, %short_circuit.merge31
  %short_circuit39 = phi i1 [ false, %short_circuit.merge31 ], [ %eq38, %short_circuit.rhs35 ]
  br i1 %short_circuit39, label %if.then40, label %if.merge41

if.then40:                                        ; preds = %short_circuit.merge36
  %zero42 = load i64, ptr %zero, align 4
  %data = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 0, ptr %data, align 4
  %length = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %zero42, ptr %length, align 4
  %struct_value43 = load [16 x i8], ptr %struct_literal, align 1
  store [16 x i8] %struct_value43, ptr %empty, align 1
  %address44 = ptrtoint ptr %empty to i32
  store i32 %address44, ptr %assignment_value45, align 4
  %assignment_value46 = load i32, ptr %assignment_value45, align 4
  store i32 %assignment_value46, ptr %text, align 4
  store i32 0, ptr %assignment_value47, align 4
  %assignment_value48 = load i32, ptr %assignment_value47, align 4
  store i32 %assignment_value48, ptr %status, align 4
  store i1 false, ptr %assignment_value49, align 1
  %assignment_value50 = load i1, ptr %assignment_value49, align 1
  store i1 %assignment_value50, ptr %reading, align 1
  br label %if.merge41

if.merge41:                                       ; preds = %if.then40, %short_circuit.merge36
  %complete51 = load i1, ptr %complete, align 1
  br i1 %complete51, label %short_circuit.rhs52, label %short_circuit.merge53

short_circuit.rhs52:                              ; preds = %if.merge41
  %count54 = load i64, ptr %count, align 4
  %zero55 = load i64, ptr %zero, align 4
  %ne56 = icmp ne i64 %count54, %zero55
  br label %short_circuit.merge53

short_circuit.merge53:                            ; preds = %short_circuit.rhs52, %if.merge41
  %short_circuit57 = phi i1 [ false, %if.merge41 ], [ %ne56, %short_circuit.rhs52 ]
  br i1 %short_circuit57, label %short_circuit.rhs58, label %short_circuit.merge59

short_circuit.rhs58:                              ; preds = %short_circuit.merge53
  %array_element60 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i64 0
  %place61 = load i8, ptr %array_element60, align 1
  %ne62 = icmp ne i8 %place61, 10
  br label %short_circuit.merge59

short_circuit.merge59:                            ; preds = %short_circuit.rhs58, %short_circuit.merge53
  %short_circuit63 = phi i1 [ false, %short_circuit.merge53 ], [ %ne62, %short_circuit.rhs58 ]
  br i1 %short_circuit63, label %short_circuit.rhs64, label %short_circuit.merge65

short_circuit.rhs64:                              ; preds = %short_circuit.merge59
  %max_bytes66 = load i64, ptr %max_bytes1, align 4
  %zero67 = load i64, ptr %zero, align 4
  %eq68 = icmp eq i64 %max_bytes66, %zero67
  br label %short_circuit.merge65

short_circuit.merge65:                            ; preds = %short_circuit.rhs64, %short_circuit.merge59
  %short_circuit69 = phi i1 [ false, %short_circuit.merge59 ], [ %eq68, %short_circuit.rhs64 ]
  br i1 %short_circuit69, label %if.then70, label %if.merge71

if.then70:                                        ; preds = %short_circuit.merge65
  %call72 = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f726561645f6c696e655f64697363617264()
  store i32 %call72, ptr %assignment_value73, align 4
  %assignment_value74 = load i32, ptr %assignment_value73, align 4
  store i32 %assignment_value74, ptr %status, align 4
  store i1 false, ptr %assignment_value75, align 1
  %assignment_value76 = load i1, ptr %assignment_value75, align 1
  store i1 %assignment_value76, ptr %reading, align 1
  br label %if.merge71

if.merge71:                                       ; preds = %if.then70, %short_circuit.merge65
  %complete77 = load i1, ptr %complete, align 1
  br i1 %complete77, label %short_circuit.rhs78, label %short_circuit.merge79

short_circuit.rhs78:                              ; preds = %if.merge71
  %count80 = load i64, ptr %count, align 4
  %zero81 = load i64, ptr %zero, align 4
  %ne82 = icmp ne i64 %count80, %zero81
  br label %short_circuit.merge79

short_circuit.merge79:                            ; preds = %short_circuit.rhs78, %if.merge71
  %short_circuit83 = phi i1 [ false, %if.merge71 ], [ %ne82, %short_circuit.rhs78 ]
  br i1 %short_circuit83, label %short_circuit.rhs84, label %short_circuit.merge85

short_circuit.rhs84:                              ; preds = %short_circuit.merge79
  %array_element86 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i64 0
  %place87 = load i8, ptr %array_element86, align 1
  %ne88 = icmp ne i8 %place87, 10
  br label %short_circuit.merge85

short_circuit.merge85:                            ; preds = %short_circuit.rhs84, %short_circuit.merge79
  %short_circuit89 = phi i1 [ false, %short_circuit.merge79 ], [ %ne88, %short_circuit.rhs84 ]
  br i1 %short_circuit89, label %short_circuit.rhs90, label %short_circuit.merge91

short_circuit.rhs90:                              ; preds = %short_circuit.merge85
  %max_bytes92 = load i64, ptr %max_bytes1, align 4
  %zero93 = load i64, ptr %zero, align 4
  %ne94 = icmp ne i64 %max_bytes92, %zero93
  br label %short_circuit.merge91

short_circuit.merge91:                            ; preds = %short_circuit.rhs90, %short_circuit.merge85
  %short_circuit95 = phi i1 [ false, %short_circuit.merge85 ], [ %ne94, %short_circuit.rhs90 ]
  br i1 %short_circuit95, label %if.then96, label %if.merge97

if.then96:                                        ; preds = %short_circuit.merge91
  %max_bytes98 = load i64, ptr %max_bytes1, align 4
  %array_element99 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i64 0
  %place100 = load i8, ptr %array_element99, align 1
  %array_element101 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i64 0
  %place102 = load i8, ptr %array_element101, align 1
  %eq103 = icmp eq i8 %place102, 13
  %call104 = call { i32, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f726561645f6c696e655f616c6c6f6361746564(i64 %max_bytes98, i8 %place100, i1 %eq103)
  %output105 = extractvalue { i32, i32 } %call104, 0
  store i32 %output105, ptr %assignment_value106, align 4
  %assignment_value107 = load i32, ptr %assignment_value106, align 4
  %output108 = extractvalue { i32, i32 } %call104, 1
  store i32 %output108, ptr %assignment_value109, align 4
  %assignment_value110 = load i32, ptr %assignment_value109, align 4
  store i32 %assignment_value107, ptr %text, align 4
  store i32 %assignment_value110, ptr %status, align 4
  store i1 false, ptr %assignment_value111, align 1
  %assignment_value112 = load i1, ptr %assignment_value111, align 1
  store i1 %assignment_value112, ptr %reading, align 1
  br label %if.merge97

if.merge97:                                       ; preds = %if.then96, %short_circuit.merge91
  br label %while.cond.0
}

define i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f64696769745f76616c7565(i8 %byte) {
entry:
  %assignment_value33 = alloca i8, align 1
  %assignment_value19 = alloca i8, align 1
  %assignment_value = alloca i8, align 1
  %upper_base = alloca i8, align 1
  %lower_base = alloca i8, align 1
  %zero_digit = alloca i8, align 1
  %value = alloca i8, align 1
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  store i8 -1, ptr %value, align 1
  store i8 48, ptr %zero_digit, align 1
  store i8 87, ptr %lower_base, align 1
  store i8 55, ptr %upper_base, align 1
  %byte2 = load i8, ptr %byte1, align 1
  %ge = icmp uge i8 %byte2, 48
  br i1 %ge, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %byte3 = load i8, ptr %byte1, align 1
  %le = icmp ule i8 %byte3, 57
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %le, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.merge

if.then:                                          ; preds = %short_circuit.merge
  %byte4 = load i8, ptr %byte1, align 1
  %zero_digit5 = load i8, ptr %zero_digit, align 1
  %sub = sub i8 %byte4, %zero_digit5
  store i8 %sub, ptr %assignment_value, align 1
  %assignment_value6 = load i8, ptr %assignment_value, align 1
  store i8 %assignment_value6, ptr %value, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %short_circuit.merge
  %byte7 = load i8, ptr %byte1, align 1
  %ge8 = icmp uge i8 %byte7, 97
  br i1 %ge8, label %short_circuit.rhs9, label %short_circuit.merge10

short_circuit.rhs9:                               ; preds = %if.merge
  %byte11 = load i8, ptr %byte1, align 1
  %le12 = icmp ule i8 %byte11, 102
  br label %short_circuit.merge10

short_circuit.merge10:                            ; preds = %short_circuit.rhs9, %if.merge
  %short_circuit13 = phi i1 [ false, %if.merge ], [ %le12, %short_circuit.rhs9 ]
  br i1 %short_circuit13, label %if.then14, label %if.merge15

if.then14:                                        ; preds = %short_circuit.merge10
  %byte16 = load i8, ptr %byte1, align 1
  %lower_base17 = load i8, ptr %lower_base, align 1
  %sub18 = sub i8 %byte16, %lower_base17
  store i8 %sub18, ptr %assignment_value19, align 1
  %assignment_value20 = load i8, ptr %assignment_value19, align 1
  store i8 %assignment_value20, ptr %value, align 1
  br label %if.merge15

if.merge15:                                       ; preds = %if.then14, %short_circuit.merge10
  %byte21 = load i8, ptr %byte1, align 1
  %ge22 = icmp uge i8 %byte21, 65
  br i1 %ge22, label %short_circuit.rhs23, label %short_circuit.merge24

short_circuit.rhs23:                              ; preds = %if.merge15
  %byte25 = load i8, ptr %byte1, align 1
  %le26 = icmp ule i8 %byte25, 70
  br label %short_circuit.merge24

short_circuit.merge24:                            ; preds = %short_circuit.rhs23, %if.merge15
  %short_circuit27 = phi i1 [ false, %if.merge15 ], [ %le26, %short_circuit.rhs23 ]
  br i1 %short_circuit27, label %if.then28, label %if.merge29

if.then28:                                        ; preds = %short_circuit.merge24
  %byte30 = load i8, ptr %byte1, align 1
  %upper_base31 = load i8, ptr %upper_base, align 1
  %sub32 = sub i8 %byte30, %upper_base31
  store i8 %sub32, ptr %assignment_value33, align 1
  %assignment_value34 = load i8, ptr %assignment_value33, align 1
  store i8 %assignment_value34, ptr %value, align 1
  br label %if.merge29

if.merge29:                                       ; preds = %if.then28, %short_circuit.merge24
  %value35 = load i8, ptr %value, align 1
  ret i8 %value35
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f64696769745f76616c6964(i8 %byte, i8 %radix) {
entry:
  %value = alloca i8, align 1
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  %radix2 = alloca i8, align 1
  store i8 %radix, ptr %radix2, align 1
  %byte3 = load i8, ptr %byte1, align 1
  %call = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f64696769745f76616c7565(i8 %byte3)
  store i8 %call, ptr %value, align 1
  %value4 = load i8, ptr %value, align 1
  %radix5 = load i8, ptr %radix2, align 1
  %lt = icmp ult i8 %value4, %radix5
  ret i1 %lt
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f756e64657273636f7265(i8 %byte) {
entry:
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  %byte2 = load i8, ptr %byte1, align 1
  %eq = icmp eq i8 %byte2, 95
  ret i1 %eq
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f7369676e(i8 %byte) {
entry:
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  %byte2 = load i8, ptr %byte1, align 1
  %eq = icmp eq i8 %byte2, 45
  ret i1 %eq
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f706c7573(i8 %byte) {
entry:
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  %byte2 = load i8, ptr %byte1, align 1
  %eq = icmp eq i8 %byte2, 43
  ret i1 %eq
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f7a65726f(i8 %byte) {
entry:
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  %byte2 = load i8, ptr %byte1, align 1
  %eq = icmp eq i8 %byte2, 48
  ret i1 %eq
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f78(i8 %byte) {
entry:
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  %byte2 = load i8, ptr %byte1, align 1
  %eq = icmp eq i8 %byte2, 120
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %byte3 = load i8, ptr %byte1, align 1
  %eq4 = icmp eq i8 %byte3, 88
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %eq4, %short_circuit.rhs ]
  ret i1 %short_circuit
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f62(i8 %byte) {
entry:
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  %byte2 = load i8, ptr %byte1, align 1
  %eq = icmp eq i8 %byte2, 98
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %byte3 = load i8, ptr %byte1, align 1
  %eq4 = icmp eq i8 %byte3, 66
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %eq4, %short_circuit.rhs ]
  ret i1 %short_circuit
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f6f(i8 %byte) {
entry:
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  %byte2 = load i8, ptr %byte1, align 1
  %eq = icmp eq i8 %byte2, 111
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %byte3 = load i8, ptr %byte1, align 1
  %eq4 = icmp eq i8 %byte3, 79
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %eq4, %short_circuit.rhs ]
  ret i1 %short_circuit
}

define i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f72616469785f66726f6d5f707265666978(i8 %first, i8 %second) {
entry:
  %assignment_value26 = alloca i8, align 1
  %assignment_value16 = alloca i8, align 1
  %assignment_value = alloca i8, align 1
  %leading = alloca i1, align 1
  %radix = alloca i8, align 1
  %first1 = alloca i8, align 1
  store i8 %first, ptr %first1, align 1
  %second2 = alloca i8, align 1
  store i8 %second, ptr %second2, align 1
  store i8 10, ptr %radix, align 1
  %first3 = load i8, ptr %first1, align 1
  %call = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f7a65726f(i8 %first3)
  store i1 %call, ptr %leading, align 1
  %leading4 = load i1, ptr %leading, align 1
  br i1 %leading4, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %second5 = load i8, ptr %second2, align 1
  %call6 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f78(i8 %second5)
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %call6, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.merge

if.then:                                          ; preds = %short_circuit.merge
  store i8 16, ptr %assignment_value, align 1
  %assignment_value7 = load i8, ptr %assignment_value, align 1
  store i8 %assignment_value7, ptr %radix, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %short_circuit.merge
  %leading8 = load i1, ptr %leading, align 1
  br i1 %leading8, label %short_circuit.rhs9, label %short_circuit.merge10

short_circuit.rhs9:                               ; preds = %if.merge
  %second11 = load i8, ptr %second2, align 1
  %call12 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f62(i8 %second11)
  br label %short_circuit.merge10

short_circuit.merge10:                            ; preds = %short_circuit.rhs9, %if.merge
  %short_circuit13 = phi i1 [ false, %if.merge ], [ %call12, %short_circuit.rhs9 ]
  br i1 %short_circuit13, label %if.then14, label %if.merge15

if.then14:                                        ; preds = %short_circuit.merge10
  store i8 2, ptr %assignment_value16, align 1
  %assignment_value17 = load i8, ptr %assignment_value16, align 1
  store i8 %assignment_value17, ptr %radix, align 1
  br label %if.merge15

if.merge15:                                       ; preds = %if.then14, %short_circuit.merge10
  %leading18 = load i1, ptr %leading, align 1
  br i1 %leading18, label %short_circuit.rhs19, label %short_circuit.merge20

short_circuit.rhs19:                              ; preds = %if.merge15
  %second21 = load i8, ptr %second2, align 1
  %call22 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f6f(i8 %second21)
  br label %short_circuit.merge20

short_circuit.merge20:                            ; preds = %short_circuit.rhs19, %if.merge15
  %short_circuit23 = phi i1 [ false, %if.merge15 ], [ %call22, %short_circuit.rhs19 ]
  br i1 %short_circuit23, label %if.then24, label %if.merge25

if.then24:                                        ; preds = %short_circuit.merge20
  store i8 8, ptr %assignment_value26, align 1
  %assignment_value27 = load i8, ptr %assignment_value26, align 1
  store i8 %assignment_value27, ptr %radix, align 1
  br label %if.merge25

if.merge25:                                       ; preds = %if.then24, %short_circuit.merge20
  %radix28 = load i8, ptr %radix, align 1
  ret i8 %radix28
}

define { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f616363756d756c6174655f75313238(i32 %data, i64 %start, i64 %end, i8 %radix) {
entry:
  %assignment_value84 = alloca i1, align 1
  %assignment_value79 = alloca i1, align 1
  %assignment_value74 = alloca i64, align 8
  %assignment_value69 = alloca i1, align 1
  %assignment_value67 = alloca i1, align 1
  %assignment_value61 = alloca i1, align 1
  %assignment_value52 = alloca i1, align 1
  %assignment_value42 = alloca i1, align 1
  %assignment_value40 = alloca i1, align 1
  %assignment_value38 = alloca i128, align 8
  %assignment_value33 = alloca i1, align 1
  %limit = alloca i128, align 8
  %radix_wide = alloca i128, align 8
  %digit_wide = alloca i128, align 8
  %digit_ok = alloca i1, align 1
  %digit = alloca i8, align 1
  %underscore = alloca i1, align 1
  %assignment_value = alloca i8, align 1
  %byte = alloca i8, align 1
  %position = alloca i64, align 8
  %wide_index = alloca i128, align 8
  %last_underscore = alloca i1, align 1
  %saw_digit = alloca i1, align 1
  %valid = alloca i1, align 1
  %index = alloca i64, align 8
  %maximum = alloca i128, align 8
  %result = alloca i128, align 8
  %one = alloca i64, align 8
  %data1 = alloca i32, align 4
  store i32 %data, ptr %data1, align 4
  %start2 = alloca i64, align 8
  store i64 %start, ptr %start2, align 4
  %end3 = alloca i64, align 8
  store i64 %end, ptr %end3, align 4
  %radix4 = alloca i8, align 1
  store i8 %radix, ptr %radix4, align 1
  store i64 1, ptr %one, align 4
  store i128 0, ptr %result, align 4
  store i128 -1, ptr %maximum, align 4
  %start5 = load i64, ptr %start2, align 4
  store i64 %start5, ptr %index, align 4
  store i1 true, ptr %valid, align 1
  store i1 false, ptr %saw_digit, align 1
  store i1 false, ptr %last_underscore, align 1
  br label %while.cond.0

while.cond.0:                                     ; preds = %if.merge, %entry
  %index6 = load i64, ptr %index, align 4
  %end7 = load i64, ptr %end3, align 4
  %lt = icmp ult i64 %index6, %end7
  br i1 %lt, label %short_circuit.rhs, label %short_circuit.merge

while.body.1:                                     ; preds = %short_circuit.merge
  %index9 = load i64, ptr %index, align 4
  %int_extend = zext i64 %index9 to i128
  store i128 %int_extend, ptr %wide_index, align 4
  %wide_index10 = load i128, ptr %wide_index, align 4
  %int_trunc = trunc i128 %wide_index10 to i64
  store i64 %int_trunc, ptr %position, align 4
  store i8 0, ptr %byte, align 1
  %data11 = load i32, ptr %data1, align 4
  %position12 = load i64, ptr %position, align 4
  %offset_base = inttoptr i32 %data11 to ptr
  %offset = getelementptr inbounds i8, ptr %offset_base, i64 %position12
  %offset_address = ptrtoint ptr %offset to i32
  %load_base = inttoptr i32 %offset_address to ptr
  %load = load i8, ptr %load_base, align 1
  store i8 %load, ptr %assignment_value, align 1
  %assignment_value13 = load i8, ptr %assignment_value, align 1
  store i8 %assignment_value13, ptr %byte, align 1
  %byte14 = load i8, ptr %byte, align 1
  %call = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f756e64657273636f7265(i8 %byte14)
  store i1 %call, ptr %underscore, align 1
  %byte15 = load i8, ptr %byte, align 1
  %call16 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f64696769745f76616c7565(i8 %byte15)
  store i8 %call16, ptr %digit, align 1
  %byte17 = load i8, ptr %byte, align 1
  %radix18 = load i8, ptr %radix4, align 1
  %call19 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f64696769745f76616c6964(i8 %byte17, i8 %radix18)
  store i1 %call19, ptr %digit_ok, align 1
  %digit_ok20 = load i1, ptr %digit_ok, align 1
  br i1 %digit_ok20, label %if.then, label %if.else

while.exit.2:                                     ; preds = %short_circuit.merge
  %saw_digit76 = load i1, ptr %saw_digit, align 1
  %not = xor i1 %saw_digit76, true
  br i1 %not, label %if.then77, label %if.merge78

short_circuit.rhs:                                ; preds = %while.cond.0
  %valid8 = load i1, ptr %valid, align 1
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %while.cond.0
  %short_circuit = phi i1 [ false, %while.cond.0 ], [ %valid8, %short_circuit.rhs ]
  br i1 %short_circuit, label %while.body.1, label %while.exit.2

if.then:                                          ; preds = %while.body.1
  %digit21 = load i8, ptr %digit, align 1
  %int_extend22 = zext i8 %digit21 to i128
  store i128 %int_extend22, ptr %digit_wide, align 4
  %radix23 = load i8, ptr %radix4, align 1
  %int_extend24 = zext i8 %radix23 to i128
  store i128 %int_extend24, ptr %radix_wide, align 4
  %maximum25 = load i128, ptr %maximum, align 4
  %digit_wide26 = load i128, ptr %digit_wide, align 4
  %sub = sub i128 %maximum25, %digit_wide26
  %radix_wide27 = load i128, ptr %radix_wide, align 4
  %div = udiv i128 %sub, %radix_wide27
  store i128 %div, ptr %limit, align 4
  %result28 = load i128, ptr %result, align 4
  %limit29 = load i128, ptr %limit, align 4
  %gt = icmp ugt i128 %result28, %limit29
  br i1 %gt, label %if.then30, label %if.else31

if.else:                                          ; preds = %while.body.1
  %underscore44 = load i1, ptr %underscore, align 1
  br i1 %underscore44, label %short_circuit.rhs45, label %short_circuit.merge46

if.merge:                                         ; preds = %if.merge66, %if.merge32
  %index71 = load i64, ptr %index, align 4
  %one72 = load i64, ptr %one, align 4
  %add73 = add i64 %index71, %one72
  store i64 %add73, ptr %assignment_value74, align 4
  %assignment_value75 = load i64, ptr %assignment_value74, align 4
  store i64 %assignment_value75, ptr %index, align 4
  br label %while.cond.0

if.then30:                                        ; preds = %if.then
  store i1 false, ptr %assignment_value33, align 1
  %assignment_value34 = load i1, ptr %assignment_value33, align 1
  store i1 %assignment_value34, ptr %valid, align 1
  br label %if.merge32

if.else31:                                        ; preds = %if.then
  %result35 = load i128, ptr %result, align 4
  %radix_wide36 = load i128, ptr %radix_wide, align 4
  %mul = mul i128 %result35, %radix_wide36
  %digit_wide37 = load i128, ptr %digit_wide, align 4
  %add = add i128 %mul, %digit_wide37
  store i128 %add, ptr %assignment_value38, align 4
  %assignment_value39 = load i128, ptr %assignment_value38, align 4
  store i128 %assignment_value39, ptr %result, align 4
  br label %if.merge32

if.merge32:                                       ; preds = %if.else31, %if.then30
  store i1 false, ptr %assignment_value40, align 1
  %assignment_value41 = load i1, ptr %assignment_value40, align 1
  store i1 %assignment_value41, ptr %last_underscore, align 1
  store i1 true, ptr %assignment_value42, align 1
  %assignment_value43 = load i1, ptr %assignment_value42, align 1
  store i1 %assignment_value43, ptr %saw_digit, align 1
  br label %if.merge

short_circuit.rhs45:                              ; preds = %if.else
  %index47 = load i64, ptr %index, align 4
  %start48 = load i64, ptr %start2, align 4
  %eq = icmp eq i64 %index47, %start48
  br label %short_circuit.merge46

short_circuit.merge46:                            ; preds = %short_circuit.rhs45, %if.else
  %short_circuit49 = phi i1 [ false, %if.else ], [ %eq, %short_circuit.rhs45 ]
  br i1 %short_circuit49, label %if.then50, label %if.merge51

if.then50:                                        ; preds = %short_circuit.merge46
  store i1 false, ptr %assignment_value52, align 1
  %assignment_value53 = load i1, ptr %assignment_value52, align 1
  store i1 %assignment_value53, ptr %valid, align 1
  br label %if.merge51

if.merge51:                                       ; preds = %if.then50, %short_circuit.merge46
  %underscore54 = load i1, ptr %underscore, align 1
  br i1 %underscore54, label %short_circuit.rhs55, label %short_circuit.merge56

short_circuit.rhs55:                              ; preds = %if.merge51
  %last_underscore57 = load i1, ptr %last_underscore, align 1
  br label %short_circuit.merge56

short_circuit.merge56:                            ; preds = %short_circuit.rhs55, %if.merge51
  %short_circuit58 = phi i1 [ false, %if.merge51 ], [ %last_underscore57, %short_circuit.rhs55 ]
  br i1 %short_circuit58, label %if.then59, label %if.merge60

if.then59:                                        ; preds = %short_circuit.merge56
  store i1 false, ptr %assignment_value61, align 1
  %assignment_value62 = load i1, ptr %assignment_value61, align 1
  store i1 %assignment_value62, ptr %valid, align 1
  br label %if.merge60

if.merge60:                                       ; preds = %if.then59, %short_circuit.merge56
  %underscore63 = load i1, ptr %underscore, align 1
  br i1 %underscore63, label %if.then64, label %if.else65

if.then64:                                        ; preds = %if.merge60
  store i1 true, ptr %assignment_value67, align 1
  %assignment_value68 = load i1, ptr %assignment_value67, align 1
  store i1 %assignment_value68, ptr %last_underscore, align 1
  br label %if.merge66

if.else65:                                        ; preds = %if.merge60
  store i1 false, ptr %assignment_value69, align 1
  %assignment_value70 = load i1, ptr %assignment_value69, align 1
  store i1 %assignment_value70, ptr %valid, align 1
  br label %if.merge66

if.merge66:                                       ; preds = %if.else65, %if.then64
  br label %if.merge

if.then77:                                        ; preds = %while.exit.2
  store i1 false, ptr %assignment_value79, align 1
  %assignment_value80 = load i1, ptr %assignment_value79, align 1
  store i1 %assignment_value80, ptr %valid, align 1
  br label %if.merge78

if.merge78:                                       ; preds = %if.then77, %while.exit.2
  %last_underscore81 = load i1, ptr %last_underscore, align 1
  br i1 %last_underscore81, label %if.then82, label %if.merge83

if.then82:                                        ; preds = %if.merge78
  store i1 false, ptr %assignment_value84, align 1
  %assignment_value85 = load i1, ptr %assignment_value84, align 1
  store i1 %assignment_value85, ptr %valid, align 1
  br label %if.merge83

if.merge83:                                       ; preds = %if.then82, %if.merge78
  %result86 = load i128, ptr %result, align 4
  %valid87 = load i1, ptr %valid, align 1
  %output = insertvalue { i128, i1 } zeroinitializer, i128 %result86, 0
  %output88 = insertvalue { i128, i1 } %output, i1 %valid87, 1
  ret { i128, i1 } %output88
}

define { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f616363756d756c6174655f693132385f6e6567(i32 %data, i64 %start, i64 %end, i8 %radix) {
entry:
  %assignment_value84 = alloca i1, align 1
  %assignment_value79 = alloca i1, align 1
  %assignment_value74 = alloca i64, align 8
  %assignment_value69 = alloca i1, align 1
  %assignment_value67 = alloca i1, align 1
  %assignment_value61 = alloca i1, align 1
  %assignment_value52 = alloca i1, align 1
  %assignment_value42 = alloca i1, align 1
  %assignment_value40 = alloca i1, align 1
  %assignment_value38 = alloca i128, align 8
  %assignment_value33 = alloca i1, align 1
  %limit = alloca i128, align 8
  %radix_wide = alloca i128, align 8
  %digit_wide = alloca i128, align 8
  %digit_ok = alloca i1, align 1
  %digit = alloca i8, align 1
  %underscore = alloca i1, align 1
  %assignment_value = alloca i8, align 1
  %byte = alloca i8, align 1
  %position = alloca i64, align 8
  %wide_index = alloca i128, align 8
  %last_underscore = alloca i1, align 1
  %saw_digit = alloca i1, align 1
  %valid = alloca i1, align 1
  %index = alloca i64, align 8
  %maximum = alloca i128, align 8
  %result = alloca i128, align 8
  %one = alloca i64, align 8
  %data1 = alloca i32, align 4
  store i32 %data, ptr %data1, align 4
  %start2 = alloca i64, align 8
  store i64 %start, ptr %start2, align 4
  %end3 = alloca i64, align 8
  store i64 %end, ptr %end3, align 4
  %radix4 = alloca i8, align 1
  store i8 %radix, ptr %radix4, align 1
  store i64 1, ptr %one, align 4
  store i128 0, ptr %result, align 4
  store i128 -170141183460469231731687303715884105728, ptr %maximum, align 4
  %start5 = load i64, ptr %start2, align 4
  store i64 %start5, ptr %index, align 4
  store i1 true, ptr %valid, align 1
  store i1 false, ptr %saw_digit, align 1
  store i1 false, ptr %last_underscore, align 1
  br label %while.cond.0

while.cond.0:                                     ; preds = %if.merge, %entry
  %index6 = load i64, ptr %index, align 4
  %end7 = load i64, ptr %end3, align 4
  %lt = icmp ult i64 %index6, %end7
  br i1 %lt, label %short_circuit.rhs, label %short_circuit.merge

while.body.1:                                     ; preds = %short_circuit.merge
  %index9 = load i64, ptr %index, align 4
  %int_extend = zext i64 %index9 to i128
  store i128 %int_extend, ptr %wide_index, align 4
  %wide_index10 = load i128, ptr %wide_index, align 4
  %int_trunc = trunc i128 %wide_index10 to i64
  store i64 %int_trunc, ptr %position, align 4
  store i8 0, ptr %byte, align 1
  %data11 = load i32, ptr %data1, align 4
  %position12 = load i64, ptr %position, align 4
  %offset_base = inttoptr i32 %data11 to ptr
  %offset = getelementptr inbounds i8, ptr %offset_base, i64 %position12
  %offset_address = ptrtoint ptr %offset to i32
  %load_base = inttoptr i32 %offset_address to ptr
  %load = load i8, ptr %load_base, align 1
  store i8 %load, ptr %assignment_value, align 1
  %assignment_value13 = load i8, ptr %assignment_value, align 1
  store i8 %assignment_value13, ptr %byte, align 1
  %byte14 = load i8, ptr %byte, align 1
  %call = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f756e64657273636f7265(i8 %byte14)
  store i1 %call, ptr %underscore, align 1
  %byte15 = load i8, ptr %byte, align 1
  %call16 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f64696769745f76616c7565(i8 %byte15)
  store i8 %call16, ptr %digit, align 1
  %byte17 = load i8, ptr %byte, align 1
  %radix18 = load i8, ptr %radix4, align 1
  %call19 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f64696769745f76616c6964(i8 %byte17, i8 %radix18)
  store i1 %call19, ptr %digit_ok, align 1
  %digit_ok20 = load i1, ptr %digit_ok, align 1
  br i1 %digit_ok20, label %if.then, label %if.else

while.exit.2:                                     ; preds = %short_circuit.merge
  %saw_digit76 = load i1, ptr %saw_digit, align 1
  %not = xor i1 %saw_digit76, true
  br i1 %not, label %if.then77, label %if.merge78

short_circuit.rhs:                                ; preds = %while.cond.0
  %valid8 = load i1, ptr %valid, align 1
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %while.cond.0
  %short_circuit = phi i1 [ false, %while.cond.0 ], [ %valid8, %short_circuit.rhs ]
  br i1 %short_circuit, label %while.body.1, label %while.exit.2

if.then:                                          ; preds = %while.body.1
  %digit21 = load i8, ptr %digit, align 1
  %int_extend22 = zext i8 %digit21 to i128
  store i128 %int_extend22, ptr %digit_wide, align 4
  %radix23 = load i8, ptr %radix4, align 1
  %int_extend24 = zext i8 %radix23 to i128
  store i128 %int_extend24, ptr %radix_wide, align 4
  %maximum25 = load i128, ptr %maximum, align 4
  %digit_wide26 = load i128, ptr %digit_wide, align 4
  %sub = sub i128 %maximum25, %digit_wide26
  %radix_wide27 = load i128, ptr %radix_wide, align 4
  %div = udiv i128 %sub, %radix_wide27
  store i128 %div, ptr %limit, align 4
  %result28 = load i128, ptr %result, align 4
  %limit29 = load i128, ptr %limit, align 4
  %gt = icmp ugt i128 %result28, %limit29
  br i1 %gt, label %if.then30, label %if.else31

if.else:                                          ; preds = %while.body.1
  %underscore44 = load i1, ptr %underscore, align 1
  br i1 %underscore44, label %short_circuit.rhs45, label %short_circuit.merge46

if.merge:                                         ; preds = %if.merge66, %if.merge32
  %index71 = load i64, ptr %index, align 4
  %one72 = load i64, ptr %one, align 4
  %add73 = add i64 %index71, %one72
  store i64 %add73, ptr %assignment_value74, align 4
  %assignment_value75 = load i64, ptr %assignment_value74, align 4
  store i64 %assignment_value75, ptr %index, align 4
  br label %while.cond.0

if.then30:                                        ; preds = %if.then
  store i1 false, ptr %assignment_value33, align 1
  %assignment_value34 = load i1, ptr %assignment_value33, align 1
  store i1 %assignment_value34, ptr %valid, align 1
  br label %if.merge32

if.else31:                                        ; preds = %if.then
  %result35 = load i128, ptr %result, align 4
  %radix_wide36 = load i128, ptr %radix_wide, align 4
  %mul = mul i128 %result35, %radix_wide36
  %digit_wide37 = load i128, ptr %digit_wide, align 4
  %add = add i128 %mul, %digit_wide37
  store i128 %add, ptr %assignment_value38, align 4
  %assignment_value39 = load i128, ptr %assignment_value38, align 4
  store i128 %assignment_value39, ptr %result, align 4
  br label %if.merge32

if.merge32:                                       ; preds = %if.else31, %if.then30
  store i1 false, ptr %assignment_value40, align 1
  %assignment_value41 = load i1, ptr %assignment_value40, align 1
  store i1 %assignment_value41, ptr %last_underscore, align 1
  store i1 true, ptr %assignment_value42, align 1
  %assignment_value43 = load i1, ptr %assignment_value42, align 1
  store i1 %assignment_value43, ptr %saw_digit, align 1
  br label %if.merge

short_circuit.rhs45:                              ; preds = %if.else
  %index47 = load i64, ptr %index, align 4
  %start48 = load i64, ptr %start2, align 4
  %eq = icmp eq i64 %index47, %start48
  br label %short_circuit.merge46

short_circuit.merge46:                            ; preds = %short_circuit.rhs45, %if.else
  %short_circuit49 = phi i1 [ false, %if.else ], [ %eq, %short_circuit.rhs45 ]
  br i1 %short_circuit49, label %if.then50, label %if.merge51

if.then50:                                        ; preds = %short_circuit.merge46
  store i1 false, ptr %assignment_value52, align 1
  %assignment_value53 = load i1, ptr %assignment_value52, align 1
  store i1 %assignment_value53, ptr %valid, align 1
  br label %if.merge51

if.merge51:                                       ; preds = %if.then50, %short_circuit.merge46
  %underscore54 = load i1, ptr %underscore, align 1
  br i1 %underscore54, label %short_circuit.rhs55, label %short_circuit.merge56

short_circuit.rhs55:                              ; preds = %if.merge51
  %last_underscore57 = load i1, ptr %last_underscore, align 1
  br label %short_circuit.merge56

short_circuit.merge56:                            ; preds = %short_circuit.rhs55, %if.merge51
  %short_circuit58 = phi i1 [ false, %if.merge51 ], [ %last_underscore57, %short_circuit.rhs55 ]
  br i1 %short_circuit58, label %if.then59, label %if.merge60

if.then59:                                        ; preds = %short_circuit.merge56
  store i1 false, ptr %assignment_value61, align 1
  %assignment_value62 = load i1, ptr %assignment_value61, align 1
  store i1 %assignment_value62, ptr %valid, align 1
  br label %if.merge60

if.merge60:                                       ; preds = %if.then59, %short_circuit.merge56
  %underscore63 = load i1, ptr %underscore, align 1
  br i1 %underscore63, label %if.then64, label %if.else65

if.then64:                                        ; preds = %if.merge60
  store i1 true, ptr %assignment_value67, align 1
  %assignment_value68 = load i1, ptr %assignment_value67, align 1
  store i1 %assignment_value68, ptr %last_underscore, align 1
  br label %if.merge66

if.else65:                                        ; preds = %if.merge60
  store i1 false, ptr %assignment_value69, align 1
  %assignment_value70 = load i1, ptr %assignment_value69, align 1
  store i1 %assignment_value70, ptr %valid, align 1
  br label %if.merge66

if.merge66:                                       ; preds = %if.else65, %if.then64
  br label %if.merge

if.then77:                                        ; preds = %while.exit.2
  store i1 false, ptr %assignment_value79, align 1
  %assignment_value80 = load i1, ptr %assignment_value79, align 1
  store i1 %assignment_value80, ptr %valid, align 1
  br label %if.merge78

if.merge78:                                       ; preds = %if.then77, %while.exit.2
  %last_underscore81 = load i1, ptr %last_underscore, align 1
  br i1 %last_underscore81, label %if.then82, label %if.merge83

if.then82:                                        ; preds = %if.merge78
  store i1 false, ptr %assignment_value84, align 1
  %assignment_value85 = load i1, ptr %assignment_value84, align 1
  store i1 %assignment_value85, ptr %valid, align 1
  br label %if.merge83

if.merge83:                                       ; preds = %if.then82, %if.merge78
  %result86 = load i128, ptr %result, align 4
  %valid87 = load i1, ptr %valid, align 1
  %output = insertvalue { i128, i1 } zeroinitializer, i128 %result86, 0
  %output88 = insertvalue { i128, i1 } %output, i1 %valid87, 1
  ret { i128, i1 } %output88
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f7538(i128 %value) {
entry:
  %limit = alloca i128, align 8
  %bound = alloca i8, align 1
  %value1 = alloca i128, align 8
  store i128 %value, ptr %value1, align 4
  store i8 -1, ptr %bound, align 1
  %bound2 = load i8, ptr %bound, align 1
  %int_extend = zext i8 %bound2 to i128
  store i128 %int_extend, ptr %limit, align 4
  %value3 = load i128, ptr %value1, align 4
  %limit4 = load i128, ptr %limit, align 4
  %le = icmp ule i128 %value3, %limit4
  ret i1 %le
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f753136(i128 %value) {
entry:
  %limit = alloca i128, align 8
  %bound = alloca i16, align 2
  %value1 = alloca i128, align 8
  store i128 %value, ptr %value1, align 4
  store i16 -1, ptr %bound, align 2
  %bound2 = load i16, ptr %bound, align 2
  %int_extend = zext i16 %bound2 to i128
  store i128 %int_extend, ptr %limit, align 4
  %value3 = load i128, ptr %value1, align 4
  %limit4 = load i128, ptr %limit, align 4
  %le = icmp ule i128 %value3, %limit4
  ret i1 %le
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f753332(i128 %value) {
entry:
  %limit = alloca i128, align 8
  %bound = alloca i32, align 4
  %value1 = alloca i128, align 8
  store i128 %value, ptr %value1, align 4
  store i32 -1, ptr %bound, align 4
  %bound2 = load i32, ptr %bound, align 4
  %int_extend = zext i32 %bound2 to i128
  store i128 %int_extend, ptr %limit, align 4
  %value3 = load i128, ptr %value1, align 4
  %limit4 = load i128, ptr %limit, align 4
  %le = icmp ule i128 %value3, %limit4
  ret i1 %le
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f753634(i128 %value) {
entry:
  %limit = alloca i128, align 8
  %bound = alloca i64, align 8
  %value1 = alloca i128, align 8
  store i128 %value, ptr %value1, align 4
  store i64 -1, ptr %bound, align 4
  %bound2 = load i64, ptr %bound, align 4
  %int_extend = zext i64 %bound2 to i128
  store i128 %int_extend, ptr %limit, align 4
  %value3 = load i128, ptr %value1, align 4
  %limit4 = load i128, ptr %limit, align 4
  %le = icmp ule i128 %value3, %limit4
  ret i1 %le
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f75313238(i128 %value) {
entry:
  %zero = alloca i128, align 8
  %value1 = alloca i128, align 8
  store i128 %value, ptr %value1, align 4
  store i128 0, ptr %zero, align 4
  %value2 = load i128, ptr %value1, align 4
  %zero3 = load i128, ptr %zero, align 4
  %ge = icmp uge i128 %value2, %zero3
  ret i1 %ge
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f6938(i128 %magnitude, i1 %negative) {
entry:
  %assignment_value19 = alloca i1, align 1
  %assignment_value = alloca i1, align 1
  %fits = alloca i1, align 1
  %negative_limit = alloca i128, align 8
  %positive_limit = alloca i128, align 8
  %negative_bound = alloca i8, align 1
  %positive_bound = alloca i8, align 1
  %magnitude1 = alloca i128, align 8
  store i128 %magnitude, ptr %magnitude1, align 4
  %negative2 = alloca i1, align 1
  store i1 %negative, ptr %negative2, align 1
  store i8 127, ptr %positive_bound, align 1
  store i8 -128, ptr %negative_bound, align 1
  %positive_bound3 = load i8, ptr %positive_bound, align 1
  %int_extend = zext i8 %positive_bound3 to i128
  store i128 %int_extend, ptr %positive_limit, align 4
  %negative_bound4 = load i8, ptr %negative_bound, align 1
  %int_extend5 = zext i8 %negative_bound4 to i128
  store i128 %int_extend5, ptr %negative_limit, align 4
  store i1 false, ptr %fits, align 1
  %negative6 = load i1, ptr %negative2, align 1
  %not = xor i1 %negative6, true
  br i1 %not, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %magnitude7 = load i128, ptr %magnitude1, align 4
  %positive_limit8 = load i128, ptr %positive_limit, align 4
  %le = icmp ule i128 %magnitude7, %positive_limit8
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %le, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.merge

if.then:                                          ; preds = %short_circuit.merge
  store i1 true, ptr %assignment_value, align 1
  %assignment_value9 = load i1, ptr %assignment_value, align 1
  store i1 %assignment_value9, ptr %fits, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %short_circuit.merge
  %negative10 = load i1, ptr %negative2, align 1
  br i1 %negative10, label %short_circuit.rhs11, label %short_circuit.merge12

short_circuit.rhs11:                              ; preds = %if.merge
  %magnitude13 = load i128, ptr %magnitude1, align 4
  %negative_limit14 = load i128, ptr %negative_limit, align 4
  %le15 = icmp ule i128 %magnitude13, %negative_limit14
  br label %short_circuit.merge12

short_circuit.merge12:                            ; preds = %short_circuit.rhs11, %if.merge
  %short_circuit16 = phi i1 [ false, %if.merge ], [ %le15, %short_circuit.rhs11 ]
  br i1 %short_circuit16, label %if.then17, label %if.merge18

if.then17:                                        ; preds = %short_circuit.merge12
  store i1 true, ptr %assignment_value19, align 1
  %assignment_value20 = load i1, ptr %assignment_value19, align 1
  store i1 %assignment_value20, ptr %fits, align 1
  br label %if.merge18

if.merge18:                                       ; preds = %if.then17, %short_circuit.merge12
  %fits21 = load i1, ptr %fits, align 1
  ret i1 %fits21
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f693136(i128 %magnitude, i1 %negative) {
entry:
  %assignment_value19 = alloca i1, align 1
  %assignment_value = alloca i1, align 1
  %fits = alloca i1, align 1
  %negative_limit = alloca i128, align 8
  %positive_limit = alloca i128, align 8
  %negative_bound = alloca i16, align 2
  %positive_bound = alloca i16, align 2
  %magnitude1 = alloca i128, align 8
  store i128 %magnitude, ptr %magnitude1, align 4
  %negative2 = alloca i1, align 1
  store i1 %negative, ptr %negative2, align 1
  store i16 32767, ptr %positive_bound, align 2
  store i16 -32768, ptr %negative_bound, align 2
  %positive_bound3 = load i16, ptr %positive_bound, align 2
  %int_extend = zext i16 %positive_bound3 to i128
  store i128 %int_extend, ptr %positive_limit, align 4
  %negative_bound4 = load i16, ptr %negative_bound, align 2
  %int_extend5 = zext i16 %negative_bound4 to i128
  store i128 %int_extend5, ptr %negative_limit, align 4
  store i1 false, ptr %fits, align 1
  %negative6 = load i1, ptr %negative2, align 1
  %not = xor i1 %negative6, true
  br i1 %not, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %magnitude7 = load i128, ptr %magnitude1, align 4
  %positive_limit8 = load i128, ptr %positive_limit, align 4
  %le = icmp ule i128 %magnitude7, %positive_limit8
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %le, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.merge

if.then:                                          ; preds = %short_circuit.merge
  store i1 true, ptr %assignment_value, align 1
  %assignment_value9 = load i1, ptr %assignment_value, align 1
  store i1 %assignment_value9, ptr %fits, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %short_circuit.merge
  %negative10 = load i1, ptr %negative2, align 1
  br i1 %negative10, label %short_circuit.rhs11, label %short_circuit.merge12

short_circuit.rhs11:                              ; preds = %if.merge
  %magnitude13 = load i128, ptr %magnitude1, align 4
  %negative_limit14 = load i128, ptr %negative_limit, align 4
  %le15 = icmp ule i128 %magnitude13, %negative_limit14
  br label %short_circuit.merge12

short_circuit.merge12:                            ; preds = %short_circuit.rhs11, %if.merge
  %short_circuit16 = phi i1 [ false, %if.merge ], [ %le15, %short_circuit.rhs11 ]
  br i1 %short_circuit16, label %if.then17, label %if.merge18

if.then17:                                        ; preds = %short_circuit.merge12
  store i1 true, ptr %assignment_value19, align 1
  %assignment_value20 = load i1, ptr %assignment_value19, align 1
  store i1 %assignment_value20, ptr %fits, align 1
  br label %if.merge18

if.merge18:                                       ; preds = %if.then17, %short_circuit.merge12
  %fits21 = load i1, ptr %fits, align 1
  ret i1 %fits21
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f693332(i128 %magnitude, i1 %negative) {
entry:
  %assignment_value19 = alloca i1, align 1
  %assignment_value = alloca i1, align 1
  %fits = alloca i1, align 1
  %negative_limit = alloca i128, align 8
  %positive_limit = alloca i128, align 8
  %negative_bound = alloca i32, align 4
  %positive_bound = alloca i32, align 4
  %magnitude1 = alloca i128, align 8
  store i128 %magnitude, ptr %magnitude1, align 4
  %negative2 = alloca i1, align 1
  store i1 %negative, ptr %negative2, align 1
  store i32 2147483647, ptr %positive_bound, align 4
  store i32 -2147483648, ptr %negative_bound, align 4
  %positive_bound3 = load i32, ptr %positive_bound, align 4
  %int_extend = zext i32 %positive_bound3 to i128
  store i128 %int_extend, ptr %positive_limit, align 4
  %negative_bound4 = load i32, ptr %negative_bound, align 4
  %int_extend5 = zext i32 %negative_bound4 to i128
  store i128 %int_extend5, ptr %negative_limit, align 4
  store i1 false, ptr %fits, align 1
  %negative6 = load i1, ptr %negative2, align 1
  %not = xor i1 %negative6, true
  br i1 %not, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %magnitude7 = load i128, ptr %magnitude1, align 4
  %positive_limit8 = load i128, ptr %positive_limit, align 4
  %le = icmp ule i128 %magnitude7, %positive_limit8
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %le, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.merge

if.then:                                          ; preds = %short_circuit.merge
  store i1 true, ptr %assignment_value, align 1
  %assignment_value9 = load i1, ptr %assignment_value, align 1
  store i1 %assignment_value9, ptr %fits, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %short_circuit.merge
  %negative10 = load i1, ptr %negative2, align 1
  br i1 %negative10, label %short_circuit.rhs11, label %short_circuit.merge12

short_circuit.rhs11:                              ; preds = %if.merge
  %magnitude13 = load i128, ptr %magnitude1, align 4
  %negative_limit14 = load i128, ptr %negative_limit, align 4
  %le15 = icmp ule i128 %magnitude13, %negative_limit14
  br label %short_circuit.merge12

short_circuit.merge12:                            ; preds = %short_circuit.rhs11, %if.merge
  %short_circuit16 = phi i1 [ false, %if.merge ], [ %le15, %short_circuit.rhs11 ]
  br i1 %short_circuit16, label %if.then17, label %if.merge18

if.then17:                                        ; preds = %short_circuit.merge12
  store i1 true, ptr %assignment_value19, align 1
  %assignment_value20 = load i1, ptr %assignment_value19, align 1
  store i1 %assignment_value20, ptr %fits, align 1
  br label %if.merge18

if.merge18:                                       ; preds = %if.then17, %short_circuit.merge12
  %fits21 = load i1, ptr %fits, align 1
  ret i1 %fits21
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f693634(i128 %magnitude, i1 %negative) {
entry:
  %assignment_value19 = alloca i1, align 1
  %assignment_value = alloca i1, align 1
  %fits = alloca i1, align 1
  %negative_limit = alloca i128, align 8
  %positive_limit = alloca i128, align 8
  %negative_bound = alloca i64, align 8
  %positive_bound = alloca i64, align 8
  %magnitude1 = alloca i128, align 8
  store i128 %magnitude, ptr %magnitude1, align 4
  %negative2 = alloca i1, align 1
  store i1 %negative, ptr %negative2, align 1
  store i64 9223372036854775807, ptr %positive_bound, align 4
  store i64 -9223372036854775808, ptr %negative_bound, align 4
  %positive_bound3 = load i64, ptr %positive_bound, align 4
  %int_extend = zext i64 %positive_bound3 to i128
  store i128 %int_extend, ptr %positive_limit, align 4
  %negative_bound4 = load i64, ptr %negative_bound, align 4
  %int_extend5 = zext i64 %negative_bound4 to i128
  store i128 %int_extend5, ptr %negative_limit, align 4
  store i1 false, ptr %fits, align 1
  %negative6 = load i1, ptr %negative2, align 1
  %not = xor i1 %negative6, true
  br i1 %not, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %magnitude7 = load i128, ptr %magnitude1, align 4
  %positive_limit8 = load i128, ptr %positive_limit, align 4
  %le = icmp ule i128 %magnitude7, %positive_limit8
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %le, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.merge

if.then:                                          ; preds = %short_circuit.merge
  store i1 true, ptr %assignment_value, align 1
  %assignment_value9 = load i1, ptr %assignment_value, align 1
  store i1 %assignment_value9, ptr %fits, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %short_circuit.merge
  %negative10 = load i1, ptr %negative2, align 1
  br i1 %negative10, label %short_circuit.rhs11, label %short_circuit.merge12

short_circuit.rhs11:                              ; preds = %if.merge
  %magnitude13 = load i128, ptr %magnitude1, align 4
  %negative_limit14 = load i128, ptr %negative_limit, align 4
  %le15 = icmp ule i128 %magnitude13, %negative_limit14
  br label %short_circuit.merge12

short_circuit.merge12:                            ; preds = %short_circuit.rhs11, %if.merge
  %short_circuit16 = phi i1 [ false, %if.merge ], [ %le15, %short_circuit.rhs11 ]
  br i1 %short_circuit16, label %if.then17, label %if.merge18

if.then17:                                        ; preds = %short_circuit.merge12
  store i1 true, ptr %assignment_value19, align 1
  %assignment_value20 = load i1, ptr %assignment_value19, align 1
  store i1 %assignment_value20, ptr %fits, align 1
  br label %if.merge18

if.merge18:                                       ; preds = %if.then17, %short_circuit.merge12
  %fits21 = load i1, ptr %fits, align 1
  ret i1 %fits21
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f69313238(i128 %magnitude, i1 %negative) {
entry:
  %assignment_value16 = alloca i1, align 1
  %assignment_value = alloca i1, align 1
  %fits = alloca i1, align 1
  %negative_limit = alloca i128, align 8
  %positive_limit = alloca i128, align 8
  %magnitude1 = alloca i128, align 8
  store i128 %magnitude, ptr %magnitude1, align 4
  %negative2 = alloca i1, align 1
  store i1 %negative, ptr %negative2, align 1
  store i128 170141183460469231731687303715884105727, ptr %positive_limit, align 4
  store i128 -170141183460469231731687303715884105728, ptr %negative_limit, align 4
  store i1 false, ptr %fits, align 1
  %negative3 = load i1, ptr %negative2, align 1
  %not = xor i1 %negative3, true
  br i1 %not, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %magnitude4 = load i128, ptr %magnitude1, align 4
  %positive_limit5 = load i128, ptr %positive_limit, align 4
  %le = icmp ule i128 %magnitude4, %positive_limit5
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %le, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.merge

if.then:                                          ; preds = %short_circuit.merge
  store i1 true, ptr %assignment_value, align 1
  %assignment_value6 = load i1, ptr %assignment_value, align 1
  store i1 %assignment_value6, ptr %fits, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %short_circuit.merge
  %negative7 = load i1, ptr %negative2, align 1
  br i1 %negative7, label %short_circuit.rhs8, label %short_circuit.merge9

short_circuit.rhs8:                               ; preds = %if.merge
  %magnitude10 = load i128, ptr %magnitude1, align 4
  %negative_limit11 = load i128, ptr %negative_limit, align 4
  %le12 = icmp ule i128 %magnitude10, %negative_limit11
  br label %short_circuit.merge9

short_circuit.merge9:                             ; preds = %short_circuit.rhs8, %if.merge
  %short_circuit13 = phi i1 [ false, %if.merge ], [ %le12, %short_circuit.rhs8 ]
  br i1 %short_circuit13, label %if.then14, label %if.merge15

if.then14:                                        ; preds = %short_circuit.merge9
  store i1 true, ptr %assignment_value16, align 1
  %assignment_value17 = load i1, ptr %assignment_value16, align 1
  store i1 %assignment_value17, ptr %fits, align 1
  br label %if.merge15

if.merge15:                                       ; preds = %if.then14, %short_circuit.merge9
  %fits18 = load i1, ptr %fits, align 1
  ret i1 %fits18
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f7538(ptr %text) {
entry:
  %assignment_value65 = alloca i32, align 4
  %assignment_value63 = alloca i8, align 1
  %fits = alloca i1, align 1
  %valid = alloca i1, align 1
  %magnitude = alloca i128, align 8
  %assignment_value42 = alloca i64, align 8
  %assignment_value36 = alloca i8, align 1
  %detected = alloca i8, align 1
  %assignment_value31 = alloca i8, align 1
  %second = alloca i8, align 1
  %position_one = alloca i64, align 8
  %wide_one = alloca i128, align 8
  %assignment_value12 = alloca i8, align 1
  %first = alloca i8, align 1
  %position_zero = alloca i64, align 8
  %wide_zero = alloca i128, align 8
  %assignment_value = alloca i32, align 4
  %result = alloca i32, align 4
  %out = alloca i8, align 1
  %start = alloca i64, align 8
  %radix = alloca i8, align 1
  %two = alloca i64, align 8
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store i64 2, ptr %two, align 4
  store i8 10, ptr %radix, align 1
  %zero2 = load i64, ptr %zero, align 4
  store i64 %zero2, ptr %start, align 4
  store i8 0, ptr %out, align 1
  store i32 0, ptr %result, align 4
  %data = getelementptr inbounds i8, ptr %text1, i8 0
  %place = load i32, ptr %data, align 4
  %eq = icmp eq i32 %place, 0
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %length = getelementptr inbounds i8, ptr %text1, i8 8
  %place3 = load i64, ptr %length, align 4
  %zero4 = load i64, ptr %zero, align 4
  %eq5 = icmp eq i64 %place3, %zero4
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %eq5, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  store i32 0, ptr %assignment_value, align 4
  %assignment_value6 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value6, ptr %result, align 4
  br label %if.merge

if.else:                                          ; preds = %short_circuit.merge
  %zero7 = load i64, ptr %zero, align 4
  %int_extend = zext i64 %zero7 to i128
  store i128 %int_extend, ptr %wide_zero, align 4
  %wide_zero8 = load i128, ptr %wide_zero, align 4
  %int_trunc = trunc i128 %wide_zero8 to i64
  store i64 %int_trunc, ptr %position_zero, align 4
  store i8 0, ptr %first, align 1
  %data9 = getelementptr inbounds i8, ptr %text1, i8 0
  %place10 = load i32, ptr %data9, align 4
  %position_zero11 = load i64, ptr %position_zero, align 4
  %offset_base = inttoptr i32 %place10 to ptr
  %offset = getelementptr inbounds i8, ptr %offset_base, i64 %position_zero11
  %offset_address = ptrtoint ptr %offset to i32
  %load_base = inttoptr i32 %offset_address to ptr
  %load = load i8, ptr %load_base, align 1
  store i8 %load, ptr %assignment_value12, align 1
  %assignment_value13 = load i8, ptr %assignment_value12, align 1
  store i8 %assignment_value13, ptr %first, align 1
  %length14 = getelementptr inbounds i8, ptr %text1, i8 8
  %place15 = load i64, ptr %length14, align 4
  %two16 = load i64, ptr %two, align 4
  %ge = icmp uge i64 %place15, %two16
  br i1 %ge, label %if.then17, label %if.merge18

if.merge:                                         ; preds = %if.merge60, %if.then
  %result67 = load i32, ptr %result, align 4
  %return_result_is_null = icmp eq i32 %result67, 0
  br i1 %return_result_is_null, label %return_result_null_0, label %return_result_copy_1

if.then17:                                        ; preds = %if.else
  %one19 = load i64, ptr %one, align 4
  %int_extend20 = zext i64 %one19 to i128
  store i128 %int_extend20, ptr %wide_one, align 4
  %wide_one21 = load i128, ptr %wide_one, align 4
  %int_trunc22 = trunc i128 %wide_one21 to i64
  store i64 %int_trunc22, ptr %position_one, align 4
  store i8 0, ptr %second, align 1
  %data23 = getelementptr inbounds i8, ptr %text1, i8 0
  %place24 = load i32, ptr %data23, align 4
  %position_one25 = load i64, ptr %position_one, align 4
  %offset_base26 = inttoptr i32 %place24 to ptr
  %offset27 = getelementptr inbounds i8, ptr %offset_base26, i64 %position_one25
  %offset_address28 = ptrtoint ptr %offset27 to i32
  %load_base29 = inttoptr i32 %offset_address28 to ptr
  %load30 = load i8, ptr %load_base29, align 1
  store i8 %load30, ptr %assignment_value31, align 1
  %assignment_value32 = load i8, ptr %assignment_value31, align 1
  store i8 %assignment_value32, ptr %second, align 1
  %first33 = load i8, ptr %first, align 1
  %second34 = load i8, ptr %second, align 1
  %call = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f72616469785f66726f6d5f707265666978(i8 %first33, i8 %second34)
  store i8 %call, ptr %detected, align 1
  %detected35 = load i8, ptr %detected, align 1
  store i8 %detected35, ptr %assignment_value36, align 1
  %assignment_value37 = load i8, ptr %assignment_value36, align 1
  store i8 %assignment_value37, ptr %radix, align 1
  %detected38 = load i8, ptr %detected, align 1
  %ne = icmp ne i8 %detected38, 10
  br i1 %ne, label %if.then39, label %if.merge40

if.merge18:                                       ; preds = %if.merge40, %if.else
  %data44 = getelementptr inbounds i8, ptr %text1, i8 0
  %place45 = load i32, ptr %data44, align 4
  %start46 = load i64, ptr %start, align 4
  %length47 = getelementptr inbounds i8, ptr %text1, i8 8
  %place48 = load i64, ptr %length47, align 4
  %radix49 = load i8, ptr %radix, align 1
  %call50 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f616363756d756c6174655f75313238(i32 %place45, i64 %start46, i64 %place48, i8 %radix49)
  %output = extractvalue { i128, i1 } %call50, 0
  store i128 %output, ptr %magnitude, align 4
  %output51 = extractvalue { i128, i1 } %call50, 1
  store i1 %output51, ptr %valid, align 1
  %magnitude52 = load i128, ptr %magnitude, align 4
  %call53 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f7538(i128 %magnitude52)
  store i1 %call53, ptr %fits, align 1
  %valid54 = load i1, ptr %valid, align 1
  br i1 %valid54, label %short_circuit.rhs55, label %short_circuit.merge56

if.then39:                                        ; preds = %if.then17
  %two41 = load i64, ptr %two, align 4
  store i64 %two41, ptr %assignment_value42, align 4
  %assignment_value43 = load i64, ptr %assignment_value42, align 4
  store i64 %assignment_value43, ptr %start, align 4
  br label %if.merge40

if.merge40:                                       ; preds = %if.then39, %if.then17
  br label %if.merge18

short_circuit.rhs55:                              ; preds = %if.merge18
  %fits57 = load i1, ptr %fits, align 1
  br label %short_circuit.merge56

short_circuit.merge56:                            ; preds = %short_circuit.rhs55, %if.merge18
  %short_circuit58 = phi i1 [ false, %if.merge18 ], [ %fits57, %short_circuit.rhs55 ]
  br i1 %short_circuit58, label %if.then59, label %if.merge60

if.then59:                                        ; preds = %short_circuit.merge56
  %magnitude61 = load i128, ptr %magnitude, align 4
  %int_trunc62 = trunc i128 %magnitude61 to i8
  store i8 %int_trunc62, ptr %assignment_value63, align 1
  %assignment_value64 = load i8, ptr %assignment_value63, align 1
  store i8 %assignment_value64, ptr %out, align 1
  %address = ptrtoint ptr %out to i32
  store i32 %address, ptr %assignment_value65, align 4
  %assignment_value66 = load i32, ptr %assignment_value65, align 4
  store i32 %assignment_value66, ptr %result, align 4
  br label %if.merge60

if.merge60:                                       ; preds = %if.then59, %short_circuit.merge56
  br label %if.merge

return_result_null_0:                             ; preds = %if.merge
  br label %return_result_merge_2

return_result_copy_1:                             ; preds = %if.merge
  %allocation = call ptr @__wosy_core_alloc(i64 1, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

return_result_merge_2:                            ; preds = %allocation_continue, %return_result_null_0
  %return_result = phi i32 [ 0, %return_result_null_0 ], [ %return_result_address, %allocation_continue ]
  ret i32 %return_result

allocation_panic:                                 ; preds = %return_result_copy_1
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %return_result_copy_1
  %return_result_source = inttoptr i32 %result67 to ptr
  %return_result_value = load i8, ptr %return_result_source, align 1
  store i8 %return_result_value, ptr %allocation, align 1
  %return_result_address = ptrtoint ptr %allocation to i32
  br label %return_result_merge_2
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f753136(ptr %text) {
entry:
  %assignment_value65 = alloca i32, align 4
  %assignment_value63 = alloca i16, align 2
  %fits = alloca i1, align 1
  %valid = alloca i1, align 1
  %magnitude = alloca i128, align 8
  %assignment_value42 = alloca i64, align 8
  %assignment_value36 = alloca i8, align 1
  %detected = alloca i8, align 1
  %assignment_value31 = alloca i8, align 1
  %second = alloca i8, align 1
  %position_one = alloca i64, align 8
  %wide_one = alloca i128, align 8
  %assignment_value12 = alloca i8, align 1
  %first = alloca i8, align 1
  %position_zero = alloca i64, align 8
  %wide_zero = alloca i128, align 8
  %assignment_value = alloca i32, align 4
  %result = alloca i32, align 4
  %out = alloca i16, align 2
  %start = alloca i64, align 8
  %radix = alloca i8, align 1
  %two = alloca i64, align 8
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store i64 2, ptr %two, align 4
  store i8 10, ptr %radix, align 1
  %zero2 = load i64, ptr %zero, align 4
  store i64 %zero2, ptr %start, align 4
  store i16 0, ptr %out, align 2
  store i32 0, ptr %result, align 4
  %data = getelementptr inbounds i8, ptr %text1, i8 0
  %place = load i32, ptr %data, align 4
  %eq = icmp eq i32 %place, 0
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %length = getelementptr inbounds i8, ptr %text1, i8 8
  %place3 = load i64, ptr %length, align 4
  %zero4 = load i64, ptr %zero, align 4
  %eq5 = icmp eq i64 %place3, %zero4
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %eq5, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  store i32 0, ptr %assignment_value, align 4
  %assignment_value6 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value6, ptr %result, align 4
  br label %if.merge

if.else:                                          ; preds = %short_circuit.merge
  %zero7 = load i64, ptr %zero, align 4
  %int_extend = zext i64 %zero7 to i128
  store i128 %int_extend, ptr %wide_zero, align 4
  %wide_zero8 = load i128, ptr %wide_zero, align 4
  %int_trunc = trunc i128 %wide_zero8 to i64
  store i64 %int_trunc, ptr %position_zero, align 4
  store i8 0, ptr %first, align 1
  %data9 = getelementptr inbounds i8, ptr %text1, i8 0
  %place10 = load i32, ptr %data9, align 4
  %position_zero11 = load i64, ptr %position_zero, align 4
  %offset_base = inttoptr i32 %place10 to ptr
  %offset = getelementptr inbounds i8, ptr %offset_base, i64 %position_zero11
  %offset_address = ptrtoint ptr %offset to i32
  %load_base = inttoptr i32 %offset_address to ptr
  %load = load i8, ptr %load_base, align 1
  store i8 %load, ptr %assignment_value12, align 1
  %assignment_value13 = load i8, ptr %assignment_value12, align 1
  store i8 %assignment_value13, ptr %first, align 1
  %length14 = getelementptr inbounds i8, ptr %text1, i8 8
  %place15 = load i64, ptr %length14, align 4
  %two16 = load i64, ptr %two, align 4
  %ge = icmp uge i64 %place15, %two16
  br i1 %ge, label %if.then17, label %if.merge18

if.merge:                                         ; preds = %if.merge60, %if.then
  %result67 = load i32, ptr %result, align 4
  %return_result_is_null = icmp eq i32 %result67, 0
  br i1 %return_result_is_null, label %return_result_null_0, label %return_result_copy_1

if.then17:                                        ; preds = %if.else
  %one19 = load i64, ptr %one, align 4
  %int_extend20 = zext i64 %one19 to i128
  store i128 %int_extend20, ptr %wide_one, align 4
  %wide_one21 = load i128, ptr %wide_one, align 4
  %int_trunc22 = trunc i128 %wide_one21 to i64
  store i64 %int_trunc22, ptr %position_one, align 4
  store i8 0, ptr %second, align 1
  %data23 = getelementptr inbounds i8, ptr %text1, i8 0
  %place24 = load i32, ptr %data23, align 4
  %position_one25 = load i64, ptr %position_one, align 4
  %offset_base26 = inttoptr i32 %place24 to ptr
  %offset27 = getelementptr inbounds i8, ptr %offset_base26, i64 %position_one25
  %offset_address28 = ptrtoint ptr %offset27 to i32
  %load_base29 = inttoptr i32 %offset_address28 to ptr
  %load30 = load i8, ptr %load_base29, align 1
  store i8 %load30, ptr %assignment_value31, align 1
  %assignment_value32 = load i8, ptr %assignment_value31, align 1
  store i8 %assignment_value32, ptr %second, align 1
  %first33 = load i8, ptr %first, align 1
  %second34 = load i8, ptr %second, align 1
  %call = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f72616469785f66726f6d5f707265666978(i8 %first33, i8 %second34)
  store i8 %call, ptr %detected, align 1
  %detected35 = load i8, ptr %detected, align 1
  store i8 %detected35, ptr %assignment_value36, align 1
  %assignment_value37 = load i8, ptr %assignment_value36, align 1
  store i8 %assignment_value37, ptr %radix, align 1
  %detected38 = load i8, ptr %detected, align 1
  %ne = icmp ne i8 %detected38, 10
  br i1 %ne, label %if.then39, label %if.merge40

if.merge18:                                       ; preds = %if.merge40, %if.else
  %data44 = getelementptr inbounds i8, ptr %text1, i8 0
  %place45 = load i32, ptr %data44, align 4
  %start46 = load i64, ptr %start, align 4
  %length47 = getelementptr inbounds i8, ptr %text1, i8 8
  %place48 = load i64, ptr %length47, align 4
  %radix49 = load i8, ptr %radix, align 1
  %call50 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f616363756d756c6174655f75313238(i32 %place45, i64 %start46, i64 %place48, i8 %radix49)
  %output = extractvalue { i128, i1 } %call50, 0
  store i128 %output, ptr %magnitude, align 4
  %output51 = extractvalue { i128, i1 } %call50, 1
  store i1 %output51, ptr %valid, align 1
  %magnitude52 = load i128, ptr %magnitude, align 4
  %call53 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f753136(i128 %magnitude52)
  store i1 %call53, ptr %fits, align 1
  %valid54 = load i1, ptr %valid, align 1
  br i1 %valid54, label %short_circuit.rhs55, label %short_circuit.merge56

if.then39:                                        ; preds = %if.then17
  %two41 = load i64, ptr %two, align 4
  store i64 %two41, ptr %assignment_value42, align 4
  %assignment_value43 = load i64, ptr %assignment_value42, align 4
  store i64 %assignment_value43, ptr %start, align 4
  br label %if.merge40

if.merge40:                                       ; preds = %if.then39, %if.then17
  br label %if.merge18

short_circuit.rhs55:                              ; preds = %if.merge18
  %fits57 = load i1, ptr %fits, align 1
  br label %short_circuit.merge56

short_circuit.merge56:                            ; preds = %short_circuit.rhs55, %if.merge18
  %short_circuit58 = phi i1 [ false, %if.merge18 ], [ %fits57, %short_circuit.rhs55 ]
  br i1 %short_circuit58, label %if.then59, label %if.merge60

if.then59:                                        ; preds = %short_circuit.merge56
  %magnitude61 = load i128, ptr %magnitude, align 4
  %int_trunc62 = trunc i128 %magnitude61 to i16
  store i16 %int_trunc62, ptr %assignment_value63, align 2
  %assignment_value64 = load i16, ptr %assignment_value63, align 2
  store i16 %assignment_value64, ptr %out, align 2
  %address = ptrtoint ptr %out to i32
  store i32 %address, ptr %assignment_value65, align 4
  %assignment_value66 = load i32, ptr %assignment_value65, align 4
  store i32 %assignment_value66, ptr %result, align 4
  br label %if.merge60

if.merge60:                                       ; preds = %if.then59, %short_circuit.merge56
  br label %if.merge

return_result_null_0:                             ; preds = %if.merge
  br label %return_result_merge_2

return_result_copy_1:                             ; preds = %if.merge
  %allocation = call ptr @__wosy_core_alloc(i64 2, i64 2)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

return_result_merge_2:                            ; preds = %allocation_continue, %return_result_null_0
  %return_result = phi i32 [ 0, %return_result_null_0 ], [ %return_result_address, %allocation_continue ]
  ret i32 %return_result

allocation_panic:                                 ; preds = %return_result_copy_1
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %return_result_copy_1
  %return_result_source = inttoptr i32 %result67 to ptr
  %return_result_value = load i16, ptr %return_result_source, align 2
  store i16 %return_result_value, ptr %allocation, align 2
  %return_result_address = ptrtoint ptr %allocation to i32
  br label %return_result_merge_2
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f753332(ptr %text) {
entry:
  %assignment_value65 = alloca i32, align 4
  %assignment_value63 = alloca i32, align 4
  %fits = alloca i1, align 1
  %valid = alloca i1, align 1
  %magnitude = alloca i128, align 8
  %assignment_value42 = alloca i64, align 8
  %assignment_value36 = alloca i8, align 1
  %detected = alloca i8, align 1
  %assignment_value31 = alloca i8, align 1
  %second = alloca i8, align 1
  %position_one = alloca i64, align 8
  %wide_one = alloca i128, align 8
  %assignment_value12 = alloca i8, align 1
  %first = alloca i8, align 1
  %position_zero = alloca i64, align 8
  %wide_zero = alloca i128, align 8
  %assignment_value = alloca i32, align 4
  %result = alloca i32, align 4
  %out = alloca i32, align 4
  %start = alloca i64, align 8
  %radix = alloca i8, align 1
  %two = alloca i64, align 8
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store i64 2, ptr %two, align 4
  store i8 10, ptr %radix, align 1
  %zero2 = load i64, ptr %zero, align 4
  store i64 %zero2, ptr %start, align 4
  store i32 0, ptr %out, align 4
  store i32 0, ptr %result, align 4
  %data = getelementptr inbounds i8, ptr %text1, i8 0
  %place = load i32, ptr %data, align 4
  %eq = icmp eq i32 %place, 0
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %length = getelementptr inbounds i8, ptr %text1, i8 8
  %place3 = load i64, ptr %length, align 4
  %zero4 = load i64, ptr %zero, align 4
  %eq5 = icmp eq i64 %place3, %zero4
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %eq5, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  store i32 0, ptr %assignment_value, align 4
  %assignment_value6 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value6, ptr %result, align 4
  br label %if.merge

if.else:                                          ; preds = %short_circuit.merge
  %zero7 = load i64, ptr %zero, align 4
  %int_extend = zext i64 %zero7 to i128
  store i128 %int_extend, ptr %wide_zero, align 4
  %wide_zero8 = load i128, ptr %wide_zero, align 4
  %int_trunc = trunc i128 %wide_zero8 to i64
  store i64 %int_trunc, ptr %position_zero, align 4
  store i8 0, ptr %first, align 1
  %data9 = getelementptr inbounds i8, ptr %text1, i8 0
  %place10 = load i32, ptr %data9, align 4
  %position_zero11 = load i64, ptr %position_zero, align 4
  %offset_base = inttoptr i32 %place10 to ptr
  %offset = getelementptr inbounds i8, ptr %offset_base, i64 %position_zero11
  %offset_address = ptrtoint ptr %offset to i32
  %load_base = inttoptr i32 %offset_address to ptr
  %load = load i8, ptr %load_base, align 1
  store i8 %load, ptr %assignment_value12, align 1
  %assignment_value13 = load i8, ptr %assignment_value12, align 1
  store i8 %assignment_value13, ptr %first, align 1
  %length14 = getelementptr inbounds i8, ptr %text1, i8 8
  %place15 = load i64, ptr %length14, align 4
  %two16 = load i64, ptr %two, align 4
  %ge = icmp uge i64 %place15, %two16
  br i1 %ge, label %if.then17, label %if.merge18

if.merge:                                         ; preds = %if.merge60, %if.then
  %result67 = load i32, ptr %result, align 4
  %return_result_is_null = icmp eq i32 %result67, 0
  br i1 %return_result_is_null, label %return_result_null_0, label %return_result_copy_1

if.then17:                                        ; preds = %if.else
  %one19 = load i64, ptr %one, align 4
  %int_extend20 = zext i64 %one19 to i128
  store i128 %int_extend20, ptr %wide_one, align 4
  %wide_one21 = load i128, ptr %wide_one, align 4
  %int_trunc22 = trunc i128 %wide_one21 to i64
  store i64 %int_trunc22, ptr %position_one, align 4
  store i8 0, ptr %second, align 1
  %data23 = getelementptr inbounds i8, ptr %text1, i8 0
  %place24 = load i32, ptr %data23, align 4
  %position_one25 = load i64, ptr %position_one, align 4
  %offset_base26 = inttoptr i32 %place24 to ptr
  %offset27 = getelementptr inbounds i8, ptr %offset_base26, i64 %position_one25
  %offset_address28 = ptrtoint ptr %offset27 to i32
  %load_base29 = inttoptr i32 %offset_address28 to ptr
  %load30 = load i8, ptr %load_base29, align 1
  store i8 %load30, ptr %assignment_value31, align 1
  %assignment_value32 = load i8, ptr %assignment_value31, align 1
  store i8 %assignment_value32, ptr %second, align 1
  %first33 = load i8, ptr %first, align 1
  %second34 = load i8, ptr %second, align 1
  %call = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f72616469785f66726f6d5f707265666978(i8 %first33, i8 %second34)
  store i8 %call, ptr %detected, align 1
  %detected35 = load i8, ptr %detected, align 1
  store i8 %detected35, ptr %assignment_value36, align 1
  %assignment_value37 = load i8, ptr %assignment_value36, align 1
  store i8 %assignment_value37, ptr %radix, align 1
  %detected38 = load i8, ptr %detected, align 1
  %ne = icmp ne i8 %detected38, 10
  br i1 %ne, label %if.then39, label %if.merge40

if.merge18:                                       ; preds = %if.merge40, %if.else
  %data44 = getelementptr inbounds i8, ptr %text1, i8 0
  %place45 = load i32, ptr %data44, align 4
  %start46 = load i64, ptr %start, align 4
  %length47 = getelementptr inbounds i8, ptr %text1, i8 8
  %place48 = load i64, ptr %length47, align 4
  %radix49 = load i8, ptr %radix, align 1
  %call50 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f616363756d756c6174655f75313238(i32 %place45, i64 %start46, i64 %place48, i8 %radix49)
  %output = extractvalue { i128, i1 } %call50, 0
  store i128 %output, ptr %magnitude, align 4
  %output51 = extractvalue { i128, i1 } %call50, 1
  store i1 %output51, ptr %valid, align 1
  %magnitude52 = load i128, ptr %magnitude, align 4
  %call53 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f753332(i128 %magnitude52)
  store i1 %call53, ptr %fits, align 1
  %valid54 = load i1, ptr %valid, align 1
  br i1 %valid54, label %short_circuit.rhs55, label %short_circuit.merge56

if.then39:                                        ; preds = %if.then17
  %two41 = load i64, ptr %two, align 4
  store i64 %two41, ptr %assignment_value42, align 4
  %assignment_value43 = load i64, ptr %assignment_value42, align 4
  store i64 %assignment_value43, ptr %start, align 4
  br label %if.merge40

if.merge40:                                       ; preds = %if.then39, %if.then17
  br label %if.merge18

short_circuit.rhs55:                              ; preds = %if.merge18
  %fits57 = load i1, ptr %fits, align 1
  br label %short_circuit.merge56

short_circuit.merge56:                            ; preds = %short_circuit.rhs55, %if.merge18
  %short_circuit58 = phi i1 [ false, %if.merge18 ], [ %fits57, %short_circuit.rhs55 ]
  br i1 %short_circuit58, label %if.then59, label %if.merge60

if.then59:                                        ; preds = %short_circuit.merge56
  %magnitude61 = load i128, ptr %magnitude, align 4
  %int_trunc62 = trunc i128 %magnitude61 to i32
  store i32 %int_trunc62, ptr %assignment_value63, align 4
  %assignment_value64 = load i32, ptr %assignment_value63, align 4
  store i32 %assignment_value64, ptr %out, align 4
  %address = ptrtoint ptr %out to i32
  store i32 %address, ptr %assignment_value65, align 4
  %assignment_value66 = load i32, ptr %assignment_value65, align 4
  store i32 %assignment_value66, ptr %result, align 4
  br label %if.merge60

if.merge60:                                       ; preds = %if.then59, %short_circuit.merge56
  br label %if.merge

return_result_null_0:                             ; preds = %if.merge
  br label %return_result_merge_2

return_result_copy_1:                             ; preds = %if.merge
  %allocation = call ptr @__wosy_core_alloc(i64 4, i64 4)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

return_result_merge_2:                            ; preds = %allocation_continue, %return_result_null_0
  %return_result = phi i32 [ 0, %return_result_null_0 ], [ %return_result_address, %allocation_continue ]
  ret i32 %return_result

allocation_panic:                                 ; preds = %return_result_copy_1
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %return_result_copy_1
  %return_result_source = inttoptr i32 %result67 to ptr
  %return_result_value = load i32, ptr %return_result_source, align 4
  store i32 %return_result_value, ptr %allocation, align 4
  %return_result_address = ptrtoint ptr %allocation to i32
  br label %return_result_merge_2
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f753634(ptr %text) {
entry:
  %assignment_value66 = alloca i32, align 4
  %assignment_value64 = alloca i64, align 8
  %fits = alloca i1, align 1
  %valid = alloca i1, align 1
  %magnitude = alloca i128, align 8
  %assignment_value43 = alloca i64, align 8
  %assignment_value37 = alloca i8, align 1
  %detected = alloca i8, align 1
  %assignment_value32 = alloca i8, align 1
  %second = alloca i8, align 1
  %position_one = alloca i64, align 8
  %wide_one = alloca i128, align 8
  %assignment_value13 = alloca i8, align 1
  %first = alloca i8, align 1
  %position_zero = alloca i64, align 8
  %wide_zero = alloca i128, align 8
  %assignment_value = alloca i32, align 4
  %result = alloca i32, align 4
  %out = alloca i64, align 8
  %start = alloca i64, align 8
  %radix = alloca i8, align 1
  %two = alloca i64, align 8
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store i64 2, ptr %two, align 4
  store i8 10, ptr %radix, align 1
  %zero2 = load i64, ptr %zero, align 4
  store i64 %zero2, ptr %start, align 4
  %zero3 = load i64, ptr %zero, align 4
  store i64 %zero3, ptr %out, align 4
  store i32 0, ptr %result, align 4
  %data = getelementptr inbounds i8, ptr %text1, i8 0
  %place = load i32, ptr %data, align 4
  %eq = icmp eq i32 %place, 0
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %length = getelementptr inbounds i8, ptr %text1, i8 8
  %place4 = load i64, ptr %length, align 4
  %zero5 = load i64, ptr %zero, align 4
  %eq6 = icmp eq i64 %place4, %zero5
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %eq6, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  store i32 0, ptr %assignment_value, align 4
  %assignment_value7 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value7, ptr %result, align 4
  br label %if.merge

if.else:                                          ; preds = %short_circuit.merge
  %zero8 = load i64, ptr %zero, align 4
  %int_extend = zext i64 %zero8 to i128
  store i128 %int_extend, ptr %wide_zero, align 4
  %wide_zero9 = load i128, ptr %wide_zero, align 4
  %int_trunc = trunc i128 %wide_zero9 to i64
  store i64 %int_trunc, ptr %position_zero, align 4
  store i8 0, ptr %first, align 1
  %data10 = getelementptr inbounds i8, ptr %text1, i8 0
  %place11 = load i32, ptr %data10, align 4
  %position_zero12 = load i64, ptr %position_zero, align 4
  %offset_base = inttoptr i32 %place11 to ptr
  %offset = getelementptr inbounds i8, ptr %offset_base, i64 %position_zero12
  %offset_address = ptrtoint ptr %offset to i32
  %load_base = inttoptr i32 %offset_address to ptr
  %load = load i8, ptr %load_base, align 1
  store i8 %load, ptr %assignment_value13, align 1
  %assignment_value14 = load i8, ptr %assignment_value13, align 1
  store i8 %assignment_value14, ptr %first, align 1
  %length15 = getelementptr inbounds i8, ptr %text1, i8 8
  %place16 = load i64, ptr %length15, align 4
  %two17 = load i64, ptr %two, align 4
  %ge = icmp uge i64 %place16, %two17
  br i1 %ge, label %if.then18, label %if.merge19

if.merge:                                         ; preds = %if.merge61, %if.then
  %result68 = load i32, ptr %result, align 4
  %return_result_is_null = icmp eq i32 %result68, 0
  br i1 %return_result_is_null, label %return_result_null_0, label %return_result_copy_1

if.then18:                                        ; preds = %if.else
  %one20 = load i64, ptr %one, align 4
  %int_extend21 = zext i64 %one20 to i128
  store i128 %int_extend21, ptr %wide_one, align 4
  %wide_one22 = load i128, ptr %wide_one, align 4
  %int_trunc23 = trunc i128 %wide_one22 to i64
  store i64 %int_trunc23, ptr %position_one, align 4
  store i8 0, ptr %second, align 1
  %data24 = getelementptr inbounds i8, ptr %text1, i8 0
  %place25 = load i32, ptr %data24, align 4
  %position_one26 = load i64, ptr %position_one, align 4
  %offset_base27 = inttoptr i32 %place25 to ptr
  %offset28 = getelementptr inbounds i8, ptr %offset_base27, i64 %position_one26
  %offset_address29 = ptrtoint ptr %offset28 to i32
  %load_base30 = inttoptr i32 %offset_address29 to ptr
  %load31 = load i8, ptr %load_base30, align 1
  store i8 %load31, ptr %assignment_value32, align 1
  %assignment_value33 = load i8, ptr %assignment_value32, align 1
  store i8 %assignment_value33, ptr %second, align 1
  %first34 = load i8, ptr %first, align 1
  %second35 = load i8, ptr %second, align 1
  %call = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f72616469785f66726f6d5f707265666978(i8 %first34, i8 %second35)
  store i8 %call, ptr %detected, align 1
  %detected36 = load i8, ptr %detected, align 1
  store i8 %detected36, ptr %assignment_value37, align 1
  %assignment_value38 = load i8, ptr %assignment_value37, align 1
  store i8 %assignment_value38, ptr %radix, align 1
  %detected39 = load i8, ptr %detected, align 1
  %ne = icmp ne i8 %detected39, 10
  br i1 %ne, label %if.then40, label %if.merge41

if.merge19:                                       ; preds = %if.merge41, %if.else
  %data45 = getelementptr inbounds i8, ptr %text1, i8 0
  %place46 = load i32, ptr %data45, align 4
  %start47 = load i64, ptr %start, align 4
  %length48 = getelementptr inbounds i8, ptr %text1, i8 8
  %place49 = load i64, ptr %length48, align 4
  %radix50 = load i8, ptr %radix, align 1
  %call51 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f616363756d756c6174655f75313238(i32 %place46, i64 %start47, i64 %place49, i8 %radix50)
  %output = extractvalue { i128, i1 } %call51, 0
  store i128 %output, ptr %magnitude, align 4
  %output52 = extractvalue { i128, i1 } %call51, 1
  store i1 %output52, ptr %valid, align 1
  %magnitude53 = load i128, ptr %magnitude, align 4
  %call54 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f753634(i128 %magnitude53)
  store i1 %call54, ptr %fits, align 1
  %valid55 = load i1, ptr %valid, align 1
  br i1 %valid55, label %short_circuit.rhs56, label %short_circuit.merge57

if.then40:                                        ; preds = %if.then18
  %two42 = load i64, ptr %two, align 4
  store i64 %two42, ptr %assignment_value43, align 4
  %assignment_value44 = load i64, ptr %assignment_value43, align 4
  store i64 %assignment_value44, ptr %start, align 4
  br label %if.merge41

if.merge41:                                       ; preds = %if.then40, %if.then18
  br label %if.merge19

short_circuit.rhs56:                              ; preds = %if.merge19
  %fits58 = load i1, ptr %fits, align 1
  br label %short_circuit.merge57

short_circuit.merge57:                            ; preds = %short_circuit.rhs56, %if.merge19
  %short_circuit59 = phi i1 [ false, %if.merge19 ], [ %fits58, %short_circuit.rhs56 ]
  br i1 %short_circuit59, label %if.then60, label %if.merge61

if.then60:                                        ; preds = %short_circuit.merge57
  %magnitude62 = load i128, ptr %magnitude, align 4
  %int_trunc63 = trunc i128 %magnitude62 to i64
  store i64 %int_trunc63, ptr %assignment_value64, align 4
  %assignment_value65 = load i64, ptr %assignment_value64, align 4
  store i64 %assignment_value65, ptr %out, align 4
  %address = ptrtoint ptr %out to i32
  store i32 %address, ptr %assignment_value66, align 4
  %assignment_value67 = load i32, ptr %assignment_value66, align 4
  store i32 %assignment_value67, ptr %result, align 4
  br label %if.merge61

if.merge61:                                       ; preds = %if.then60, %short_circuit.merge57
  br label %if.merge

return_result_null_0:                             ; preds = %if.merge
  br label %return_result_merge_2

return_result_copy_1:                             ; preds = %if.merge
  %allocation = call ptr @__wosy_core_alloc(i64 8, i64 8)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

return_result_merge_2:                            ; preds = %allocation_continue, %return_result_null_0
  %return_result = phi i32 [ 0, %return_result_null_0 ], [ %return_result_address, %allocation_continue ]
  ret i32 %return_result

allocation_panic:                                 ; preds = %return_result_copy_1
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %return_result_copy_1
  %return_result_source = inttoptr i32 %result68 to ptr
  %return_result_value = load i64, ptr %return_result_source, align 4
  store i64 %return_result_value, ptr %allocation, align 4
  %return_result_address = ptrtoint ptr %allocation to i32
  br label %return_result_merge_2
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f75313238(ptr %text) {
entry:
  %assignment_value64 = alloca i32, align 4
  %assignment_value62 = alloca i128, align 8
  %fits = alloca i1, align 1
  %valid = alloca i1, align 1
  %magnitude = alloca i128, align 8
  %assignment_value42 = alloca i64, align 8
  %assignment_value36 = alloca i8, align 1
  %detected = alloca i8, align 1
  %assignment_value31 = alloca i8, align 1
  %second = alloca i8, align 1
  %position_one = alloca i64, align 8
  %wide_one = alloca i128, align 8
  %assignment_value12 = alloca i8, align 1
  %first = alloca i8, align 1
  %position_zero = alloca i64, align 8
  %wide_zero = alloca i128, align 8
  %assignment_value = alloca i32, align 4
  %result = alloca i32, align 4
  %out = alloca i128, align 8
  %start = alloca i64, align 8
  %radix = alloca i8, align 1
  %two = alloca i64, align 8
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store i64 2, ptr %two, align 4
  store i8 10, ptr %radix, align 1
  %zero2 = load i64, ptr %zero, align 4
  store i64 %zero2, ptr %start, align 4
  store i128 0, ptr %out, align 4
  store i32 0, ptr %result, align 4
  %data = getelementptr inbounds i8, ptr %text1, i8 0
  %place = load i32, ptr %data, align 4
  %eq = icmp eq i32 %place, 0
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %length = getelementptr inbounds i8, ptr %text1, i8 8
  %place3 = load i64, ptr %length, align 4
  %zero4 = load i64, ptr %zero, align 4
  %eq5 = icmp eq i64 %place3, %zero4
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %eq5, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  store i32 0, ptr %assignment_value, align 4
  %assignment_value6 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value6, ptr %result, align 4
  br label %if.merge

if.else:                                          ; preds = %short_circuit.merge
  %zero7 = load i64, ptr %zero, align 4
  %int_extend = zext i64 %zero7 to i128
  store i128 %int_extend, ptr %wide_zero, align 4
  %wide_zero8 = load i128, ptr %wide_zero, align 4
  %int_trunc = trunc i128 %wide_zero8 to i64
  store i64 %int_trunc, ptr %position_zero, align 4
  store i8 0, ptr %first, align 1
  %data9 = getelementptr inbounds i8, ptr %text1, i8 0
  %place10 = load i32, ptr %data9, align 4
  %position_zero11 = load i64, ptr %position_zero, align 4
  %offset_base = inttoptr i32 %place10 to ptr
  %offset = getelementptr inbounds i8, ptr %offset_base, i64 %position_zero11
  %offset_address = ptrtoint ptr %offset to i32
  %load_base = inttoptr i32 %offset_address to ptr
  %load = load i8, ptr %load_base, align 1
  store i8 %load, ptr %assignment_value12, align 1
  %assignment_value13 = load i8, ptr %assignment_value12, align 1
  store i8 %assignment_value13, ptr %first, align 1
  %length14 = getelementptr inbounds i8, ptr %text1, i8 8
  %place15 = load i64, ptr %length14, align 4
  %two16 = load i64, ptr %two, align 4
  %ge = icmp uge i64 %place15, %two16
  br i1 %ge, label %if.then17, label %if.merge18

if.merge:                                         ; preds = %if.merge60, %if.then
  %result66 = load i32, ptr %result, align 4
  %return_result_is_null = icmp eq i32 %result66, 0
  br i1 %return_result_is_null, label %return_result_null_0, label %return_result_copy_1

if.then17:                                        ; preds = %if.else
  %one19 = load i64, ptr %one, align 4
  %int_extend20 = zext i64 %one19 to i128
  store i128 %int_extend20, ptr %wide_one, align 4
  %wide_one21 = load i128, ptr %wide_one, align 4
  %int_trunc22 = trunc i128 %wide_one21 to i64
  store i64 %int_trunc22, ptr %position_one, align 4
  store i8 0, ptr %second, align 1
  %data23 = getelementptr inbounds i8, ptr %text1, i8 0
  %place24 = load i32, ptr %data23, align 4
  %position_one25 = load i64, ptr %position_one, align 4
  %offset_base26 = inttoptr i32 %place24 to ptr
  %offset27 = getelementptr inbounds i8, ptr %offset_base26, i64 %position_one25
  %offset_address28 = ptrtoint ptr %offset27 to i32
  %load_base29 = inttoptr i32 %offset_address28 to ptr
  %load30 = load i8, ptr %load_base29, align 1
  store i8 %load30, ptr %assignment_value31, align 1
  %assignment_value32 = load i8, ptr %assignment_value31, align 1
  store i8 %assignment_value32, ptr %second, align 1
  %first33 = load i8, ptr %first, align 1
  %second34 = load i8, ptr %second, align 1
  %call = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f72616469785f66726f6d5f707265666978(i8 %first33, i8 %second34)
  store i8 %call, ptr %detected, align 1
  %detected35 = load i8, ptr %detected, align 1
  store i8 %detected35, ptr %assignment_value36, align 1
  %assignment_value37 = load i8, ptr %assignment_value36, align 1
  store i8 %assignment_value37, ptr %radix, align 1
  %detected38 = load i8, ptr %detected, align 1
  %ne = icmp ne i8 %detected38, 10
  br i1 %ne, label %if.then39, label %if.merge40

if.merge18:                                       ; preds = %if.merge40, %if.else
  %data44 = getelementptr inbounds i8, ptr %text1, i8 0
  %place45 = load i32, ptr %data44, align 4
  %start46 = load i64, ptr %start, align 4
  %length47 = getelementptr inbounds i8, ptr %text1, i8 8
  %place48 = load i64, ptr %length47, align 4
  %radix49 = load i8, ptr %radix, align 1
  %call50 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f616363756d756c6174655f75313238(i32 %place45, i64 %start46, i64 %place48, i8 %radix49)
  %output = extractvalue { i128, i1 } %call50, 0
  store i128 %output, ptr %magnitude, align 4
  %output51 = extractvalue { i128, i1 } %call50, 1
  store i1 %output51, ptr %valid, align 1
  %magnitude52 = load i128, ptr %magnitude, align 4
  %call53 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f75313238(i128 %magnitude52)
  store i1 %call53, ptr %fits, align 1
  %valid54 = load i1, ptr %valid, align 1
  br i1 %valid54, label %short_circuit.rhs55, label %short_circuit.merge56

if.then39:                                        ; preds = %if.then17
  %two41 = load i64, ptr %two, align 4
  store i64 %two41, ptr %assignment_value42, align 4
  %assignment_value43 = load i64, ptr %assignment_value42, align 4
  store i64 %assignment_value43, ptr %start, align 4
  br label %if.merge40

if.merge40:                                       ; preds = %if.then39, %if.then17
  br label %if.merge18

short_circuit.rhs55:                              ; preds = %if.merge18
  %fits57 = load i1, ptr %fits, align 1
  br label %short_circuit.merge56

short_circuit.merge56:                            ; preds = %short_circuit.rhs55, %if.merge18
  %short_circuit58 = phi i1 [ false, %if.merge18 ], [ %fits57, %short_circuit.rhs55 ]
  br i1 %short_circuit58, label %if.then59, label %if.merge60

if.then59:                                        ; preds = %short_circuit.merge56
  %magnitude61 = load i128, ptr %magnitude, align 4
  store i128 %magnitude61, ptr %assignment_value62, align 4
  %assignment_value63 = load i128, ptr %assignment_value62, align 4
  store i128 %assignment_value63, ptr %out, align 4
  %address = ptrtoint ptr %out to i32
  store i32 %address, ptr %assignment_value64, align 4
  %assignment_value65 = load i32, ptr %assignment_value64, align 4
  store i32 %assignment_value65, ptr %result, align 4
  br label %if.merge60

if.merge60:                                       ; preds = %if.then59, %short_circuit.merge56
  br label %if.merge

return_result_null_0:                             ; preds = %if.merge
  br label %return_result_merge_2

return_result_copy_1:                             ; preds = %if.merge
  %allocation = call ptr @__wosy_core_alloc(i64 16, i64 16)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

return_result_merge_2:                            ; preds = %allocation_continue, %return_result_null_0
  %return_result = phi i32 [ 0, %return_result_null_0 ], [ %return_result_address, %allocation_continue ]
  ret i32 %return_result

allocation_panic:                                 ; preds = %return_result_copy_1
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %return_result_copy_1
  %return_result_source = inttoptr i32 %result66 to ptr
  %return_result_value = load i128, ptr %return_result_source, align 4
  store i128 %return_result_value, ptr %allocation, align 4
  %return_result_address = ptrtoint ptr %allocation to i32
  br label %return_result_merge_2
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f6938(ptr %text) {
entry:
  %assignment_value108 = alloca i32, align 4
  %assignment_value106 = alloca i8, align 1
  %assignment_value102 = alloca i8, align 1
  %flipped = alloca i128, align 8
  %fits = alloca i1, align 1
  %valid = alloca i1, align 1
  %magnitude = alloca i128, align 8
  %assignment_value73 = alloca i64, align 8
  %assignment_value65 = alloca i8, align 1
  %detected = alloca i8, align 1
  %assignment_value59 = alloca i8, align 1
  %prefix_second = alloca i8, align 1
  %assignment_value49 = alloca i8, align 1
  %prefix_first = alloca i8, align 1
  %position_next = alloca i64, align 8
  %wide_next = alloca i128, align 8
  %position_start = alloca i64, align 8
  %wide_start = alloca i128, align 8
  %next = alloca i64, align 8
  %rest = alloca i64, align 8
  %assignment_value22 = alloca i64, align 8
  %assignment_value19 = alloca i1, align 1
  %assignment_value14 = alloca i8, align 1
  %first = alloca i8, align 1
  %position_zero = alloca i64, align 8
  %wide_zero = alloca i128, align 8
  %assignment_value = alloca i32, align 4
  %result = alloca i32, align 4
  %out = alloca i8, align 1
  %zero_wide = alloca i128, align 8
  %negative = alloca i1, align 1
  %start = alloca i64, align 8
  %radix = alloca i8, align 1
  %two = alloca i64, align 8
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store i64 2, ptr %two, align 4
  store i8 10, ptr %radix, align 1
  %zero2 = load i64, ptr %zero, align 4
  store i64 %zero2, ptr %start, align 4
  store i1 false, ptr %negative, align 1
  %zero3 = load i64, ptr %zero, align 4
  %int_extend = zext i64 %zero3 to i128
  store i128 %int_extend, ptr %zero_wide, align 4
  store i8 0, ptr %out, align 1
  store i32 0, ptr %result, align 4
  %data = getelementptr inbounds i8, ptr %text1, i8 0
  %place = load i32, ptr %data, align 4
  %eq = icmp eq i32 %place, 0
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %length = getelementptr inbounds i8, ptr %text1, i8 8
  %place4 = load i64, ptr %length, align 4
  %zero5 = load i64, ptr %zero, align 4
  %eq6 = icmp eq i64 %place4, %zero5
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %eq6, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  store i32 0, ptr %assignment_value, align 4
  %assignment_value7 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value7, ptr %result, align 4
  br label %if.merge

if.else:                                          ; preds = %short_circuit.merge
  %zero8 = load i64, ptr %zero, align 4
  %int_extend9 = zext i64 %zero8 to i128
  store i128 %int_extend9, ptr %wide_zero, align 4
  %wide_zero10 = load i128, ptr %wide_zero, align 4
  %int_trunc = trunc i128 %wide_zero10 to i64
  store i64 %int_trunc, ptr %position_zero, align 4
  store i8 0, ptr %first, align 1
  %data11 = getelementptr inbounds i8, ptr %text1, i8 0
  %place12 = load i32, ptr %data11, align 4
  %position_zero13 = load i64, ptr %position_zero, align 4
  %offset_base = inttoptr i32 %place12 to ptr
  %offset = getelementptr inbounds i8, ptr %offset_base, i64 %position_zero13
  %offset_address = ptrtoint ptr %offset to i32
  %load_base = inttoptr i32 %offset_address to ptr
  %load = load i8, ptr %load_base, align 1
  store i8 %load, ptr %assignment_value14, align 1
  %assignment_value15 = load i8, ptr %assignment_value14, align 1
  store i8 %assignment_value15, ptr %first, align 1
  %first16 = load i8, ptr %first, align 1
  %call = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f7369676e(i8 %first16)
  br i1 %call, label %if.then17, label %if.merge18

if.merge:                                         ; preds = %if.merge92, %if.then
  %result110 = load i32, ptr %result, align 4
  %return_result_is_null = icmp eq i32 %result110, 0
  br i1 %return_result_is_null, label %return_result_null_0, label %return_result_copy_1

if.then17:                                        ; preds = %if.else
  store i1 true, ptr %assignment_value19, align 1
  %assignment_value20 = load i1, ptr %assignment_value19, align 1
  store i1 %assignment_value20, ptr %negative, align 1
  %one21 = load i64, ptr %one, align 4
  store i64 %one21, ptr %assignment_value22, align 4
  %assignment_value23 = load i64, ptr %assignment_value22, align 4
  store i64 %assignment_value23, ptr %start, align 4
  br label %if.merge18

if.merge18:                                       ; preds = %if.then17, %if.else
  %length24 = getelementptr inbounds i8, ptr %text1, i8 8
  %place25 = load i64, ptr %length24, align 4
  %start26 = load i64, ptr %start, align 4
  %sub = sub i64 %place25, %start26
  store i64 %sub, ptr %rest, align 4
  %rest27 = load i64, ptr %rest, align 4
  %two28 = load i64, ptr %two, align 4
  %ge = icmp uge i64 %rest27, %two28
  br i1 %ge, label %if.then29, label %if.merge30

if.then29:                                        ; preds = %if.merge18
  %start31 = load i64, ptr %start, align 4
  %one32 = load i64, ptr %one, align 4
  %add = add i64 %start31, %one32
  store i64 %add, ptr %next, align 4
  %start33 = load i64, ptr %start, align 4
  %int_extend34 = zext i64 %start33 to i128
  store i128 %int_extend34, ptr %wide_start, align 4
  %wide_start35 = load i128, ptr %wide_start, align 4
  %int_trunc36 = trunc i128 %wide_start35 to i64
  store i64 %int_trunc36, ptr %position_start, align 4
  %next37 = load i64, ptr %next, align 4
  %int_extend38 = zext i64 %next37 to i128
  store i128 %int_extend38, ptr %wide_next, align 4
  %wide_next39 = load i128, ptr %wide_next, align 4
  %int_trunc40 = trunc i128 %wide_next39 to i64
  store i64 %int_trunc40, ptr %position_next, align 4
  store i8 0, ptr %prefix_first, align 1
  %data41 = getelementptr inbounds i8, ptr %text1, i8 0
  %place42 = load i32, ptr %data41, align 4
  %position_start43 = load i64, ptr %position_start, align 4
  %offset_base44 = inttoptr i32 %place42 to ptr
  %offset45 = getelementptr inbounds i8, ptr %offset_base44, i64 %position_start43
  %offset_address46 = ptrtoint ptr %offset45 to i32
  %load_base47 = inttoptr i32 %offset_address46 to ptr
  %load48 = load i8, ptr %load_base47, align 1
  store i8 %load48, ptr %assignment_value49, align 1
  %assignment_value50 = load i8, ptr %assignment_value49, align 1
  store i8 %assignment_value50, ptr %prefix_first, align 1
  store i8 0, ptr %prefix_second, align 1
  %data51 = getelementptr inbounds i8, ptr %text1, i8 0
  %place52 = load i32, ptr %data51, align 4
  %position_next53 = load i64, ptr %position_next, align 4
  %offset_base54 = inttoptr i32 %place52 to ptr
  %offset55 = getelementptr inbounds i8, ptr %offset_base54, i64 %position_next53
  %offset_address56 = ptrtoint ptr %offset55 to i32
  %load_base57 = inttoptr i32 %offset_address56 to ptr
  %load58 = load i8, ptr %load_base57, align 1
  store i8 %load58, ptr %assignment_value59, align 1
  %assignment_value60 = load i8, ptr %assignment_value59, align 1
  store i8 %assignment_value60, ptr %prefix_second, align 1
  %prefix_first61 = load i8, ptr %prefix_first, align 1
  %prefix_second62 = load i8, ptr %prefix_second, align 1
  %call63 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f72616469785f66726f6d5f707265666978(i8 %prefix_first61, i8 %prefix_second62)
  store i8 %call63, ptr %detected, align 1
  %detected64 = load i8, ptr %detected, align 1
  store i8 %detected64, ptr %assignment_value65, align 1
  %assignment_value66 = load i8, ptr %assignment_value65, align 1
  store i8 %assignment_value66, ptr %radix, align 1
  %detected67 = load i8, ptr %detected, align 1
  %ne = icmp ne i8 %detected67, 10
  br i1 %ne, label %if.then68, label %if.merge69

if.merge30:                                       ; preds = %if.merge69, %if.merge18
  %data75 = getelementptr inbounds i8, ptr %text1, i8 0
  %place76 = load i32, ptr %data75, align 4
  %start77 = load i64, ptr %start, align 4
  %length78 = getelementptr inbounds i8, ptr %text1, i8 8
  %place79 = load i64, ptr %length78, align 4
  %radix80 = load i8, ptr %radix, align 1
  %call81 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f616363756d756c6174655f693132385f6e6567(i32 %place76, i64 %start77, i64 %place79, i8 %radix80)
  %output = extractvalue { i128, i1 } %call81, 0
  store i128 %output, ptr %magnitude, align 4
  %output82 = extractvalue { i128, i1 } %call81, 1
  store i1 %output82, ptr %valid, align 1
  %magnitude83 = load i128, ptr %magnitude, align 4
  %negative84 = load i1, ptr %negative, align 1
  %call85 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f6938(i128 %magnitude83, i1 %negative84)
  store i1 %call85, ptr %fits, align 1
  %valid86 = load i1, ptr %valid, align 1
  br i1 %valid86, label %short_circuit.rhs87, label %short_circuit.merge88

if.then68:                                        ; preds = %if.then29
  %start70 = load i64, ptr %start, align 4
  %two71 = load i64, ptr %two, align 4
  %add72 = add i64 %start70, %two71
  store i64 %add72, ptr %assignment_value73, align 4
  %assignment_value74 = load i64, ptr %assignment_value73, align 4
  store i64 %assignment_value74, ptr %start, align 4
  br label %if.merge69

if.merge69:                                       ; preds = %if.then68, %if.then29
  br label %if.merge30

short_circuit.rhs87:                              ; preds = %if.merge30
  %fits89 = load i1, ptr %fits, align 1
  br label %short_circuit.merge88

short_circuit.merge88:                            ; preds = %short_circuit.rhs87, %if.merge30
  %short_circuit90 = phi i1 [ false, %if.merge30 ], [ %fits89, %short_circuit.rhs87 ]
  br i1 %short_circuit90, label %if.then91, label %if.merge92

if.then91:                                        ; preds = %short_circuit.merge88
  %negative93 = load i1, ptr %negative, align 1
  br i1 %negative93, label %if.then94, label %if.else95

if.merge92:                                       ; preds = %if.merge96, %short_circuit.merge88
  br label %if.merge

if.then94:                                        ; preds = %if.then91
  %zero_wide97 = load i128, ptr %zero_wide, align 4
  %magnitude98 = load i128, ptr %magnitude, align 4
  %sub99 = sub i128 %zero_wide97, %magnitude98
  store i128 %sub99, ptr %flipped, align 4
  %flipped100 = load i128, ptr %flipped, align 4
  %int_trunc101 = trunc i128 %flipped100 to i8
  store i8 %int_trunc101, ptr %assignment_value102, align 1
  %assignment_value103 = load i8, ptr %assignment_value102, align 1
  store i8 %assignment_value103, ptr %out, align 1
  br label %if.merge96

if.else95:                                        ; preds = %if.then91
  %magnitude104 = load i128, ptr %magnitude, align 4
  %int_trunc105 = trunc i128 %magnitude104 to i8
  store i8 %int_trunc105, ptr %assignment_value106, align 1
  %assignment_value107 = load i8, ptr %assignment_value106, align 1
  store i8 %assignment_value107, ptr %out, align 1
  br label %if.merge96

if.merge96:                                       ; preds = %if.else95, %if.then94
  %address = ptrtoint ptr %out to i32
  store i32 %address, ptr %assignment_value108, align 4
  %assignment_value109 = load i32, ptr %assignment_value108, align 4
  store i32 %assignment_value109, ptr %result, align 4
  br label %if.merge92

return_result_null_0:                             ; preds = %if.merge
  br label %return_result_merge_2

return_result_copy_1:                             ; preds = %if.merge
  %allocation = call ptr @__wosy_core_alloc(i64 1, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

return_result_merge_2:                            ; preds = %allocation_continue, %return_result_null_0
  %return_result = phi i32 [ 0, %return_result_null_0 ], [ %return_result_address, %allocation_continue ]
  ret i32 %return_result

allocation_panic:                                 ; preds = %return_result_copy_1
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %return_result_copy_1
  %return_result_source = inttoptr i32 %result110 to ptr
  %return_result_value = load i8, ptr %return_result_source, align 1
  store i8 %return_result_value, ptr %allocation, align 1
  %return_result_address = ptrtoint ptr %allocation to i32
  br label %return_result_merge_2
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f693136(ptr %text) {
entry:
  %assignment_value108 = alloca i32, align 4
  %assignment_value106 = alloca i16, align 2
  %assignment_value102 = alloca i16, align 2
  %flipped = alloca i128, align 8
  %fits = alloca i1, align 1
  %valid = alloca i1, align 1
  %magnitude = alloca i128, align 8
  %assignment_value73 = alloca i64, align 8
  %assignment_value65 = alloca i8, align 1
  %detected = alloca i8, align 1
  %assignment_value59 = alloca i8, align 1
  %prefix_second = alloca i8, align 1
  %assignment_value49 = alloca i8, align 1
  %prefix_first = alloca i8, align 1
  %position_next = alloca i64, align 8
  %wide_next = alloca i128, align 8
  %position_start = alloca i64, align 8
  %wide_start = alloca i128, align 8
  %next = alloca i64, align 8
  %rest = alloca i64, align 8
  %assignment_value22 = alloca i64, align 8
  %assignment_value19 = alloca i1, align 1
  %assignment_value14 = alloca i8, align 1
  %first = alloca i8, align 1
  %position_zero = alloca i64, align 8
  %wide_zero = alloca i128, align 8
  %assignment_value = alloca i32, align 4
  %result = alloca i32, align 4
  %out = alloca i16, align 2
  %zero_wide = alloca i128, align 8
  %negative = alloca i1, align 1
  %start = alloca i64, align 8
  %radix = alloca i8, align 1
  %two = alloca i64, align 8
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store i64 2, ptr %two, align 4
  store i8 10, ptr %radix, align 1
  %zero2 = load i64, ptr %zero, align 4
  store i64 %zero2, ptr %start, align 4
  store i1 false, ptr %negative, align 1
  %zero3 = load i64, ptr %zero, align 4
  %int_extend = zext i64 %zero3 to i128
  store i128 %int_extend, ptr %zero_wide, align 4
  store i16 0, ptr %out, align 2
  store i32 0, ptr %result, align 4
  %data = getelementptr inbounds i8, ptr %text1, i8 0
  %place = load i32, ptr %data, align 4
  %eq = icmp eq i32 %place, 0
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %length = getelementptr inbounds i8, ptr %text1, i8 8
  %place4 = load i64, ptr %length, align 4
  %zero5 = load i64, ptr %zero, align 4
  %eq6 = icmp eq i64 %place4, %zero5
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %eq6, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  store i32 0, ptr %assignment_value, align 4
  %assignment_value7 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value7, ptr %result, align 4
  br label %if.merge

if.else:                                          ; preds = %short_circuit.merge
  %zero8 = load i64, ptr %zero, align 4
  %int_extend9 = zext i64 %zero8 to i128
  store i128 %int_extend9, ptr %wide_zero, align 4
  %wide_zero10 = load i128, ptr %wide_zero, align 4
  %int_trunc = trunc i128 %wide_zero10 to i64
  store i64 %int_trunc, ptr %position_zero, align 4
  store i8 0, ptr %first, align 1
  %data11 = getelementptr inbounds i8, ptr %text1, i8 0
  %place12 = load i32, ptr %data11, align 4
  %position_zero13 = load i64, ptr %position_zero, align 4
  %offset_base = inttoptr i32 %place12 to ptr
  %offset = getelementptr inbounds i8, ptr %offset_base, i64 %position_zero13
  %offset_address = ptrtoint ptr %offset to i32
  %load_base = inttoptr i32 %offset_address to ptr
  %load = load i8, ptr %load_base, align 1
  store i8 %load, ptr %assignment_value14, align 1
  %assignment_value15 = load i8, ptr %assignment_value14, align 1
  store i8 %assignment_value15, ptr %first, align 1
  %first16 = load i8, ptr %first, align 1
  %call = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f7369676e(i8 %first16)
  br i1 %call, label %if.then17, label %if.merge18

if.merge:                                         ; preds = %if.merge92, %if.then
  %result110 = load i32, ptr %result, align 4
  %return_result_is_null = icmp eq i32 %result110, 0
  br i1 %return_result_is_null, label %return_result_null_0, label %return_result_copy_1

if.then17:                                        ; preds = %if.else
  store i1 true, ptr %assignment_value19, align 1
  %assignment_value20 = load i1, ptr %assignment_value19, align 1
  store i1 %assignment_value20, ptr %negative, align 1
  %one21 = load i64, ptr %one, align 4
  store i64 %one21, ptr %assignment_value22, align 4
  %assignment_value23 = load i64, ptr %assignment_value22, align 4
  store i64 %assignment_value23, ptr %start, align 4
  br label %if.merge18

if.merge18:                                       ; preds = %if.then17, %if.else
  %length24 = getelementptr inbounds i8, ptr %text1, i8 8
  %place25 = load i64, ptr %length24, align 4
  %start26 = load i64, ptr %start, align 4
  %sub = sub i64 %place25, %start26
  store i64 %sub, ptr %rest, align 4
  %rest27 = load i64, ptr %rest, align 4
  %two28 = load i64, ptr %two, align 4
  %ge = icmp uge i64 %rest27, %two28
  br i1 %ge, label %if.then29, label %if.merge30

if.then29:                                        ; preds = %if.merge18
  %start31 = load i64, ptr %start, align 4
  %one32 = load i64, ptr %one, align 4
  %add = add i64 %start31, %one32
  store i64 %add, ptr %next, align 4
  %start33 = load i64, ptr %start, align 4
  %int_extend34 = zext i64 %start33 to i128
  store i128 %int_extend34, ptr %wide_start, align 4
  %wide_start35 = load i128, ptr %wide_start, align 4
  %int_trunc36 = trunc i128 %wide_start35 to i64
  store i64 %int_trunc36, ptr %position_start, align 4
  %next37 = load i64, ptr %next, align 4
  %int_extend38 = zext i64 %next37 to i128
  store i128 %int_extend38, ptr %wide_next, align 4
  %wide_next39 = load i128, ptr %wide_next, align 4
  %int_trunc40 = trunc i128 %wide_next39 to i64
  store i64 %int_trunc40, ptr %position_next, align 4
  store i8 0, ptr %prefix_first, align 1
  %data41 = getelementptr inbounds i8, ptr %text1, i8 0
  %place42 = load i32, ptr %data41, align 4
  %position_start43 = load i64, ptr %position_start, align 4
  %offset_base44 = inttoptr i32 %place42 to ptr
  %offset45 = getelementptr inbounds i8, ptr %offset_base44, i64 %position_start43
  %offset_address46 = ptrtoint ptr %offset45 to i32
  %load_base47 = inttoptr i32 %offset_address46 to ptr
  %load48 = load i8, ptr %load_base47, align 1
  store i8 %load48, ptr %assignment_value49, align 1
  %assignment_value50 = load i8, ptr %assignment_value49, align 1
  store i8 %assignment_value50, ptr %prefix_first, align 1
  store i8 0, ptr %prefix_second, align 1
  %data51 = getelementptr inbounds i8, ptr %text1, i8 0
  %place52 = load i32, ptr %data51, align 4
  %position_next53 = load i64, ptr %position_next, align 4
  %offset_base54 = inttoptr i32 %place52 to ptr
  %offset55 = getelementptr inbounds i8, ptr %offset_base54, i64 %position_next53
  %offset_address56 = ptrtoint ptr %offset55 to i32
  %load_base57 = inttoptr i32 %offset_address56 to ptr
  %load58 = load i8, ptr %load_base57, align 1
  store i8 %load58, ptr %assignment_value59, align 1
  %assignment_value60 = load i8, ptr %assignment_value59, align 1
  store i8 %assignment_value60, ptr %prefix_second, align 1
  %prefix_first61 = load i8, ptr %prefix_first, align 1
  %prefix_second62 = load i8, ptr %prefix_second, align 1
  %call63 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f72616469785f66726f6d5f707265666978(i8 %prefix_first61, i8 %prefix_second62)
  store i8 %call63, ptr %detected, align 1
  %detected64 = load i8, ptr %detected, align 1
  store i8 %detected64, ptr %assignment_value65, align 1
  %assignment_value66 = load i8, ptr %assignment_value65, align 1
  store i8 %assignment_value66, ptr %radix, align 1
  %detected67 = load i8, ptr %detected, align 1
  %ne = icmp ne i8 %detected67, 10
  br i1 %ne, label %if.then68, label %if.merge69

if.merge30:                                       ; preds = %if.merge69, %if.merge18
  %data75 = getelementptr inbounds i8, ptr %text1, i8 0
  %place76 = load i32, ptr %data75, align 4
  %start77 = load i64, ptr %start, align 4
  %length78 = getelementptr inbounds i8, ptr %text1, i8 8
  %place79 = load i64, ptr %length78, align 4
  %radix80 = load i8, ptr %radix, align 1
  %call81 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f616363756d756c6174655f693132385f6e6567(i32 %place76, i64 %start77, i64 %place79, i8 %radix80)
  %output = extractvalue { i128, i1 } %call81, 0
  store i128 %output, ptr %magnitude, align 4
  %output82 = extractvalue { i128, i1 } %call81, 1
  store i1 %output82, ptr %valid, align 1
  %magnitude83 = load i128, ptr %magnitude, align 4
  %negative84 = load i1, ptr %negative, align 1
  %call85 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f693136(i128 %magnitude83, i1 %negative84)
  store i1 %call85, ptr %fits, align 1
  %valid86 = load i1, ptr %valid, align 1
  br i1 %valid86, label %short_circuit.rhs87, label %short_circuit.merge88

if.then68:                                        ; preds = %if.then29
  %start70 = load i64, ptr %start, align 4
  %two71 = load i64, ptr %two, align 4
  %add72 = add i64 %start70, %two71
  store i64 %add72, ptr %assignment_value73, align 4
  %assignment_value74 = load i64, ptr %assignment_value73, align 4
  store i64 %assignment_value74, ptr %start, align 4
  br label %if.merge69

if.merge69:                                       ; preds = %if.then68, %if.then29
  br label %if.merge30

short_circuit.rhs87:                              ; preds = %if.merge30
  %fits89 = load i1, ptr %fits, align 1
  br label %short_circuit.merge88

short_circuit.merge88:                            ; preds = %short_circuit.rhs87, %if.merge30
  %short_circuit90 = phi i1 [ false, %if.merge30 ], [ %fits89, %short_circuit.rhs87 ]
  br i1 %short_circuit90, label %if.then91, label %if.merge92

if.then91:                                        ; preds = %short_circuit.merge88
  %negative93 = load i1, ptr %negative, align 1
  br i1 %negative93, label %if.then94, label %if.else95

if.merge92:                                       ; preds = %if.merge96, %short_circuit.merge88
  br label %if.merge

if.then94:                                        ; preds = %if.then91
  %zero_wide97 = load i128, ptr %zero_wide, align 4
  %magnitude98 = load i128, ptr %magnitude, align 4
  %sub99 = sub i128 %zero_wide97, %magnitude98
  store i128 %sub99, ptr %flipped, align 4
  %flipped100 = load i128, ptr %flipped, align 4
  %int_trunc101 = trunc i128 %flipped100 to i16
  store i16 %int_trunc101, ptr %assignment_value102, align 2
  %assignment_value103 = load i16, ptr %assignment_value102, align 2
  store i16 %assignment_value103, ptr %out, align 2
  br label %if.merge96

if.else95:                                        ; preds = %if.then91
  %magnitude104 = load i128, ptr %magnitude, align 4
  %int_trunc105 = trunc i128 %magnitude104 to i16
  store i16 %int_trunc105, ptr %assignment_value106, align 2
  %assignment_value107 = load i16, ptr %assignment_value106, align 2
  store i16 %assignment_value107, ptr %out, align 2
  br label %if.merge96

if.merge96:                                       ; preds = %if.else95, %if.then94
  %address = ptrtoint ptr %out to i32
  store i32 %address, ptr %assignment_value108, align 4
  %assignment_value109 = load i32, ptr %assignment_value108, align 4
  store i32 %assignment_value109, ptr %result, align 4
  br label %if.merge92

return_result_null_0:                             ; preds = %if.merge
  br label %return_result_merge_2

return_result_copy_1:                             ; preds = %if.merge
  %allocation = call ptr @__wosy_core_alloc(i64 2, i64 2)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

return_result_merge_2:                            ; preds = %allocation_continue, %return_result_null_0
  %return_result = phi i32 [ 0, %return_result_null_0 ], [ %return_result_address, %allocation_continue ]
  ret i32 %return_result

allocation_panic:                                 ; preds = %return_result_copy_1
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %return_result_copy_1
  %return_result_source = inttoptr i32 %result110 to ptr
  %return_result_value = load i16, ptr %return_result_source, align 2
  store i16 %return_result_value, ptr %allocation, align 2
  %return_result_address = ptrtoint ptr %allocation to i32
  br label %return_result_merge_2
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f693332(ptr %text) {
entry:
  %assignment_value108 = alloca i32, align 4
  %assignment_value106 = alloca i32, align 4
  %assignment_value102 = alloca i32, align 4
  %flipped = alloca i128, align 8
  %fits = alloca i1, align 1
  %valid = alloca i1, align 1
  %magnitude = alloca i128, align 8
  %assignment_value73 = alloca i64, align 8
  %assignment_value65 = alloca i8, align 1
  %detected = alloca i8, align 1
  %assignment_value59 = alloca i8, align 1
  %prefix_second = alloca i8, align 1
  %assignment_value49 = alloca i8, align 1
  %prefix_first = alloca i8, align 1
  %position_next = alloca i64, align 8
  %wide_next = alloca i128, align 8
  %position_start = alloca i64, align 8
  %wide_start = alloca i128, align 8
  %next = alloca i64, align 8
  %rest = alloca i64, align 8
  %assignment_value22 = alloca i64, align 8
  %assignment_value19 = alloca i1, align 1
  %assignment_value14 = alloca i8, align 1
  %first = alloca i8, align 1
  %position_zero = alloca i64, align 8
  %wide_zero = alloca i128, align 8
  %assignment_value = alloca i32, align 4
  %result = alloca i32, align 4
  %out = alloca i32, align 4
  %zero_wide = alloca i128, align 8
  %negative = alloca i1, align 1
  %start = alloca i64, align 8
  %radix = alloca i8, align 1
  %two = alloca i64, align 8
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store i64 2, ptr %two, align 4
  store i8 10, ptr %radix, align 1
  %zero2 = load i64, ptr %zero, align 4
  store i64 %zero2, ptr %start, align 4
  store i1 false, ptr %negative, align 1
  %zero3 = load i64, ptr %zero, align 4
  %int_extend = zext i64 %zero3 to i128
  store i128 %int_extend, ptr %zero_wide, align 4
  store i32 0, ptr %out, align 4
  store i32 0, ptr %result, align 4
  %data = getelementptr inbounds i8, ptr %text1, i8 0
  %place = load i32, ptr %data, align 4
  %eq = icmp eq i32 %place, 0
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %length = getelementptr inbounds i8, ptr %text1, i8 8
  %place4 = load i64, ptr %length, align 4
  %zero5 = load i64, ptr %zero, align 4
  %eq6 = icmp eq i64 %place4, %zero5
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %eq6, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  store i32 0, ptr %assignment_value, align 4
  %assignment_value7 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value7, ptr %result, align 4
  br label %if.merge

if.else:                                          ; preds = %short_circuit.merge
  %zero8 = load i64, ptr %zero, align 4
  %int_extend9 = zext i64 %zero8 to i128
  store i128 %int_extend9, ptr %wide_zero, align 4
  %wide_zero10 = load i128, ptr %wide_zero, align 4
  %int_trunc = trunc i128 %wide_zero10 to i64
  store i64 %int_trunc, ptr %position_zero, align 4
  store i8 0, ptr %first, align 1
  %data11 = getelementptr inbounds i8, ptr %text1, i8 0
  %place12 = load i32, ptr %data11, align 4
  %position_zero13 = load i64, ptr %position_zero, align 4
  %offset_base = inttoptr i32 %place12 to ptr
  %offset = getelementptr inbounds i8, ptr %offset_base, i64 %position_zero13
  %offset_address = ptrtoint ptr %offset to i32
  %load_base = inttoptr i32 %offset_address to ptr
  %load = load i8, ptr %load_base, align 1
  store i8 %load, ptr %assignment_value14, align 1
  %assignment_value15 = load i8, ptr %assignment_value14, align 1
  store i8 %assignment_value15, ptr %first, align 1
  %first16 = load i8, ptr %first, align 1
  %call = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f7369676e(i8 %first16)
  br i1 %call, label %if.then17, label %if.merge18

if.merge:                                         ; preds = %if.merge92, %if.then
  %result110 = load i32, ptr %result, align 4
  %return_result_is_null = icmp eq i32 %result110, 0
  br i1 %return_result_is_null, label %return_result_null_0, label %return_result_copy_1

if.then17:                                        ; preds = %if.else
  store i1 true, ptr %assignment_value19, align 1
  %assignment_value20 = load i1, ptr %assignment_value19, align 1
  store i1 %assignment_value20, ptr %negative, align 1
  %one21 = load i64, ptr %one, align 4
  store i64 %one21, ptr %assignment_value22, align 4
  %assignment_value23 = load i64, ptr %assignment_value22, align 4
  store i64 %assignment_value23, ptr %start, align 4
  br label %if.merge18

if.merge18:                                       ; preds = %if.then17, %if.else
  %length24 = getelementptr inbounds i8, ptr %text1, i8 8
  %place25 = load i64, ptr %length24, align 4
  %start26 = load i64, ptr %start, align 4
  %sub = sub i64 %place25, %start26
  store i64 %sub, ptr %rest, align 4
  %rest27 = load i64, ptr %rest, align 4
  %two28 = load i64, ptr %two, align 4
  %ge = icmp uge i64 %rest27, %two28
  br i1 %ge, label %if.then29, label %if.merge30

if.then29:                                        ; preds = %if.merge18
  %start31 = load i64, ptr %start, align 4
  %one32 = load i64, ptr %one, align 4
  %add = add i64 %start31, %one32
  store i64 %add, ptr %next, align 4
  %start33 = load i64, ptr %start, align 4
  %int_extend34 = zext i64 %start33 to i128
  store i128 %int_extend34, ptr %wide_start, align 4
  %wide_start35 = load i128, ptr %wide_start, align 4
  %int_trunc36 = trunc i128 %wide_start35 to i64
  store i64 %int_trunc36, ptr %position_start, align 4
  %next37 = load i64, ptr %next, align 4
  %int_extend38 = zext i64 %next37 to i128
  store i128 %int_extend38, ptr %wide_next, align 4
  %wide_next39 = load i128, ptr %wide_next, align 4
  %int_trunc40 = trunc i128 %wide_next39 to i64
  store i64 %int_trunc40, ptr %position_next, align 4
  store i8 0, ptr %prefix_first, align 1
  %data41 = getelementptr inbounds i8, ptr %text1, i8 0
  %place42 = load i32, ptr %data41, align 4
  %position_start43 = load i64, ptr %position_start, align 4
  %offset_base44 = inttoptr i32 %place42 to ptr
  %offset45 = getelementptr inbounds i8, ptr %offset_base44, i64 %position_start43
  %offset_address46 = ptrtoint ptr %offset45 to i32
  %load_base47 = inttoptr i32 %offset_address46 to ptr
  %load48 = load i8, ptr %load_base47, align 1
  store i8 %load48, ptr %assignment_value49, align 1
  %assignment_value50 = load i8, ptr %assignment_value49, align 1
  store i8 %assignment_value50, ptr %prefix_first, align 1
  store i8 0, ptr %prefix_second, align 1
  %data51 = getelementptr inbounds i8, ptr %text1, i8 0
  %place52 = load i32, ptr %data51, align 4
  %position_next53 = load i64, ptr %position_next, align 4
  %offset_base54 = inttoptr i32 %place52 to ptr
  %offset55 = getelementptr inbounds i8, ptr %offset_base54, i64 %position_next53
  %offset_address56 = ptrtoint ptr %offset55 to i32
  %load_base57 = inttoptr i32 %offset_address56 to ptr
  %load58 = load i8, ptr %load_base57, align 1
  store i8 %load58, ptr %assignment_value59, align 1
  %assignment_value60 = load i8, ptr %assignment_value59, align 1
  store i8 %assignment_value60, ptr %prefix_second, align 1
  %prefix_first61 = load i8, ptr %prefix_first, align 1
  %prefix_second62 = load i8, ptr %prefix_second, align 1
  %call63 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f72616469785f66726f6d5f707265666978(i8 %prefix_first61, i8 %prefix_second62)
  store i8 %call63, ptr %detected, align 1
  %detected64 = load i8, ptr %detected, align 1
  store i8 %detected64, ptr %assignment_value65, align 1
  %assignment_value66 = load i8, ptr %assignment_value65, align 1
  store i8 %assignment_value66, ptr %radix, align 1
  %detected67 = load i8, ptr %detected, align 1
  %ne = icmp ne i8 %detected67, 10
  br i1 %ne, label %if.then68, label %if.merge69

if.merge30:                                       ; preds = %if.merge69, %if.merge18
  %data75 = getelementptr inbounds i8, ptr %text1, i8 0
  %place76 = load i32, ptr %data75, align 4
  %start77 = load i64, ptr %start, align 4
  %length78 = getelementptr inbounds i8, ptr %text1, i8 8
  %place79 = load i64, ptr %length78, align 4
  %radix80 = load i8, ptr %radix, align 1
  %call81 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f616363756d756c6174655f693132385f6e6567(i32 %place76, i64 %start77, i64 %place79, i8 %radix80)
  %output = extractvalue { i128, i1 } %call81, 0
  store i128 %output, ptr %magnitude, align 4
  %output82 = extractvalue { i128, i1 } %call81, 1
  store i1 %output82, ptr %valid, align 1
  %magnitude83 = load i128, ptr %magnitude, align 4
  %negative84 = load i1, ptr %negative, align 1
  %call85 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f693332(i128 %magnitude83, i1 %negative84)
  store i1 %call85, ptr %fits, align 1
  %valid86 = load i1, ptr %valid, align 1
  br i1 %valid86, label %short_circuit.rhs87, label %short_circuit.merge88

if.then68:                                        ; preds = %if.then29
  %start70 = load i64, ptr %start, align 4
  %two71 = load i64, ptr %two, align 4
  %add72 = add i64 %start70, %two71
  store i64 %add72, ptr %assignment_value73, align 4
  %assignment_value74 = load i64, ptr %assignment_value73, align 4
  store i64 %assignment_value74, ptr %start, align 4
  br label %if.merge69

if.merge69:                                       ; preds = %if.then68, %if.then29
  br label %if.merge30

short_circuit.rhs87:                              ; preds = %if.merge30
  %fits89 = load i1, ptr %fits, align 1
  br label %short_circuit.merge88

short_circuit.merge88:                            ; preds = %short_circuit.rhs87, %if.merge30
  %short_circuit90 = phi i1 [ false, %if.merge30 ], [ %fits89, %short_circuit.rhs87 ]
  br i1 %short_circuit90, label %if.then91, label %if.merge92

if.then91:                                        ; preds = %short_circuit.merge88
  %negative93 = load i1, ptr %negative, align 1
  br i1 %negative93, label %if.then94, label %if.else95

if.merge92:                                       ; preds = %if.merge96, %short_circuit.merge88
  br label %if.merge

if.then94:                                        ; preds = %if.then91
  %zero_wide97 = load i128, ptr %zero_wide, align 4
  %magnitude98 = load i128, ptr %magnitude, align 4
  %sub99 = sub i128 %zero_wide97, %magnitude98
  store i128 %sub99, ptr %flipped, align 4
  %flipped100 = load i128, ptr %flipped, align 4
  %int_trunc101 = trunc i128 %flipped100 to i32
  store i32 %int_trunc101, ptr %assignment_value102, align 4
  %assignment_value103 = load i32, ptr %assignment_value102, align 4
  store i32 %assignment_value103, ptr %out, align 4
  br label %if.merge96

if.else95:                                        ; preds = %if.then91
  %magnitude104 = load i128, ptr %magnitude, align 4
  %int_trunc105 = trunc i128 %magnitude104 to i32
  store i32 %int_trunc105, ptr %assignment_value106, align 4
  %assignment_value107 = load i32, ptr %assignment_value106, align 4
  store i32 %assignment_value107, ptr %out, align 4
  br label %if.merge96

if.merge96:                                       ; preds = %if.else95, %if.then94
  %address = ptrtoint ptr %out to i32
  store i32 %address, ptr %assignment_value108, align 4
  %assignment_value109 = load i32, ptr %assignment_value108, align 4
  store i32 %assignment_value109, ptr %result, align 4
  br label %if.merge92

return_result_null_0:                             ; preds = %if.merge
  br label %return_result_merge_2

return_result_copy_1:                             ; preds = %if.merge
  %allocation = call ptr @__wosy_core_alloc(i64 4, i64 4)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

return_result_merge_2:                            ; preds = %allocation_continue, %return_result_null_0
  %return_result = phi i32 [ 0, %return_result_null_0 ], [ %return_result_address, %allocation_continue ]
  ret i32 %return_result

allocation_panic:                                 ; preds = %return_result_copy_1
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %return_result_copy_1
  %return_result_source = inttoptr i32 %result110 to ptr
  %return_result_value = load i32, ptr %return_result_source, align 4
  store i32 %return_result_value, ptr %allocation, align 4
  %return_result_address = ptrtoint ptr %allocation to i32
  br label %return_result_merge_2
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f693634(ptr %text) {
entry:
  %assignment_value108 = alloca i32, align 4
  %assignment_value106 = alloca i64, align 8
  %assignment_value102 = alloca i64, align 8
  %flipped = alloca i128, align 8
  %fits = alloca i1, align 1
  %valid = alloca i1, align 1
  %magnitude = alloca i128, align 8
  %assignment_value73 = alloca i64, align 8
  %assignment_value65 = alloca i8, align 1
  %detected = alloca i8, align 1
  %assignment_value59 = alloca i8, align 1
  %prefix_second = alloca i8, align 1
  %assignment_value49 = alloca i8, align 1
  %prefix_first = alloca i8, align 1
  %position_next = alloca i64, align 8
  %wide_next = alloca i128, align 8
  %position_start = alloca i64, align 8
  %wide_start = alloca i128, align 8
  %next = alloca i64, align 8
  %rest = alloca i64, align 8
  %assignment_value22 = alloca i64, align 8
  %assignment_value19 = alloca i1, align 1
  %assignment_value14 = alloca i8, align 1
  %first = alloca i8, align 1
  %position_zero = alloca i64, align 8
  %wide_zero = alloca i128, align 8
  %assignment_value = alloca i32, align 4
  %result = alloca i32, align 4
  %out = alloca i64, align 8
  %zero_wide = alloca i128, align 8
  %negative = alloca i1, align 1
  %start = alloca i64, align 8
  %radix = alloca i8, align 1
  %two = alloca i64, align 8
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store i64 2, ptr %two, align 4
  store i8 10, ptr %radix, align 1
  %zero2 = load i64, ptr %zero, align 4
  store i64 %zero2, ptr %start, align 4
  store i1 false, ptr %negative, align 1
  %zero3 = load i64, ptr %zero, align 4
  %int_extend = zext i64 %zero3 to i128
  store i128 %int_extend, ptr %zero_wide, align 4
  store i64 0, ptr %out, align 4
  store i32 0, ptr %result, align 4
  %data = getelementptr inbounds i8, ptr %text1, i8 0
  %place = load i32, ptr %data, align 4
  %eq = icmp eq i32 %place, 0
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %length = getelementptr inbounds i8, ptr %text1, i8 8
  %place4 = load i64, ptr %length, align 4
  %zero5 = load i64, ptr %zero, align 4
  %eq6 = icmp eq i64 %place4, %zero5
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %eq6, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  store i32 0, ptr %assignment_value, align 4
  %assignment_value7 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value7, ptr %result, align 4
  br label %if.merge

if.else:                                          ; preds = %short_circuit.merge
  %zero8 = load i64, ptr %zero, align 4
  %int_extend9 = zext i64 %zero8 to i128
  store i128 %int_extend9, ptr %wide_zero, align 4
  %wide_zero10 = load i128, ptr %wide_zero, align 4
  %int_trunc = trunc i128 %wide_zero10 to i64
  store i64 %int_trunc, ptr %position_zero, align 4
  store i8 0, ptr %first, align 1
  %data11 = getelementptr inbounds i8, ptr %text1, i8 0
  %place12 = load i32, ptr %data11, align 4
  %position_zero13 = load i64, ptr %position_zero, align 4
  %offset_base = inttoptr i32 %place12 to ptr
  %offset = getelementptr inbounds i8, ptr %offset_base, i64 %position_zero13
  %offset_address = ptrtoint ptr %offset to i32
  %load_base = inttoptr i32 %offset_address to ptr
  %load = load i8, ptr %load_base, align 1
  store i8 %load, ptr %assignment_value14, align 1
  %assignment_value15 = load i8, ptr %assignment_value14, align 1
  store i8 %assignment_value15, ptr %first, align 1
  %first16 = load i8, ptr %first, align 1
  %call = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f7369676e(i8 %first16)
  br i1 %call, label %if.then17, label %if.merge18

if.merge:                                         ; preds = %if.merge92, %if.then
  %result110 = load i32, ptr %result, align 4
  %return_result_is_null = icmp eq i32 %result110, 0
  br i1 %return_result_is_null, label %return_result_null_0, label %return_result_copy_1

if.then17:                                        ; preds = %if.else
  store i1 true, ptr %assignment_value19, align 1
  %assignment_value20 = load i1, ptr %assignment_value19, align 1
  store i1 %assignment_value20, ptr %negative, align 1
  %one21 = load i64, ptr %one, align 4
  store i64 %one21, ptr %assignment_value22, align 4
  %assignment_value23 = load i64, ptr %assignment_value22, align 4
  store i64 %assignment_value23, ptr %start, align 4
  br label %if.merge18

if.merge18:                                       ; preds = %if.then17, %if.else
  %length24 = getelementptr inbounds i8, ptr %text1, i8 8
  %place25 = load i64, ptr %length24, align 4
  %start26 = load i64, ptr %start, align 4
  %sub = sub i64 %place25, %start26
  store i64 %sub, ptr %rest, align 4
  %rest27 = load i64, ptr %rest, align 4
  %two28 = load i64, ptr %two, align 4
  %ge = icmp uge i64 %rest27, %two28
  br i1 %ge, label %if.then29, label %if.merge30

if.then29:                                        ; preds = %if.merge18
  %start31 = load i64, ptr %start, align 4
  %one32 = load i64, ptr %one, align 4
  %add = add i64 %start31, %one32
  store i64 %add, ptr %next, align 4
  %start33 = load i64, ptr %start, align 4
  %int_extend34 = zext i64 %start33 to i128
  store i128 %int_extend34, ptr %wide_start, align 4
  %wide_start35 = load i128, ptr %wide_start, align 4
  %int_trunc36 = trunc i128 %wide_start35 to i64
  store i64 %int_trunc36, ptr %position_start, align 4
  %next37 = load i64, ptr %next, align 4
  %int_extend38 = zext i64 %next37 to i128
  store i128 %int_extend38, ptr %wide_next, align 4
  %wide_next39 = load i128, ptr %wide_next, align 4
  %int_trunc40 = trunc i128 %wide_next39 to i64
  store i64 %int_trunc40, ptr %position_next, align 4
  store i8 0, ptr %prefix_first, align 1
  %data41 = getelementptr inbounds i8, ptr %text1, i8 0
  %place42 = load i32, ptr %data41, align 4
  %position_start43 = load i64, ptr %position_start, align 4
  %offset_base44 = inttoptr i32 %place42 to ptr
  %offset45 = getelementptr inbounds i8, ptr %offset_base44, i64 %position_start43
  %offset_address46 = ptrtoint ptr %offset45 to i32
  %load_base47 = inttoptr i32 %offset_address46 to ptr
  %load48 = load i8, ptr %load_base47, align 1
  store i8 %load48, ptr %assignment_value49, align 1
  %assignment_value50 = load i8, ptr %assignment_value49, align 1
  store i8 %assignment_value50, ptr %prefix_first, align 1
  store i8 0, ptr %prefix_second, align 1
  %data51 = getelementptr inbounds i8, ptr %text1, i8 0
  %place52 = load i32, ptr %data51, align 4
  %position_next53 = load i64, ptr %position_next, align 4
  %offset_base54 = inttoptr i32 %place52 to ptr
  %offset55 = getelementptr inbounds i8, ptr %offset_base54, i64 %position_next53
  %offset_address56 = ptrtoint ptr %offset55 to i32
  %load_base57 = inttoptr i32 %offset_address56 to ptr
  %load58 = load i8, ptr %load_base57, align 1
  store i8 %load58, ptr %assignment_value59, align 1
  %assignment_value60 = load i8, ptr %assignment_value59, align 1
  store i8 %assignment_value60, ptr %prefix_second, align 1
  %prefix_first61 = load i8, ptr %prefix_first, align 1
  %prefix_second62 = load i8, ptr %prefix_second, align 1
  %call63 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f72616469785f66726f6d5f707265666978(i8 %prefix_first61, i8 %prefix_second62)
  store i8 %call63, ptr %detected, align 1
  %detected64 = load i8, ptr %detected, align 1
  store i8 %detected64, ptr %assignment_value65, align 1
  %assignment_value66 = load i8, ptr %assignment_value65, align 1
  store i8 %assignment_value66, ptr %radix, align 1
  %detected67 = load i8, ptr %detected, align 1
  %ne = icmp ne i8 %detected67, 10
  br i1 %ne, label %if.then68, label %if.merge69

if.merge30:                                       ; preds = %if.merge69, %if.merge18
  %data75 = getelementptr inbounds i8, ptr %text1, i8 0
  %place76 = load i32, ptr %data75, align 4
  %start77 = load i64, ptr %start, align 4
  %length78 = getelementptr inbounds i8, ptr %text1, i8 8
  %place79 = load i64, ptr %length78, align 4
  %radix80 = load i8, ptr %radix, align 1
  %call81 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f616363756d756c6174655f693132385f6e6567(i32 %place76, i64 %start77, i64 %place79, i8 %radix80)
  %output = extractvalue { i128, i1 } %call81, 0
  store i128 %output, ptr %magnitude, align 4
  %output82 = extractvalue { i128, i1 } %call81, 1
  store i1 %output82, ptr %valid, align 1
  %magnitude83 = load i128, ptr %magnitude, align 4
  %negative84 = load i1, ptr %negative, align 1
  %call85 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f693634(i128 %magnitude83, i1 %negative84)
  store i1 %call85, ptr %fits, align 1
  %valid86 = load i1, ptr %valid, align 1
  br i1 %valid86, label %short_circuit.rhs87, label %short_circuit.merge88

if.then68:                                        ; preds = %if.then29
  %start70 = load i64, ptr %start, align 4
  %two71 = load i64, ptr %two, align 4
  %add72 = add i64 %start70, %two71
  store i64 %add72, ptr %assignment_value73, align 4
  %assignment_value74 = load i64, ptr %assignment_value73, align 4
  store i64 %assignment_value74, ptr %start, align 4
  br label %if.merge69

if.merge69:                                       ; preds = %if.then68, %if.then29
  br label %if.merge30

short_circuit.rhs87:                              ; preds = %if.merge30
  %fits89 = load i1, ptr %fits, align 1
  br label %short_circuit.merge88

short_circuit.merge88:                            ; preds = %short_circuit.rhs87, %if.merge30
  %short_circuit90 = phi i1 [ false, %if.merge30 ], [ %fits89, %short_circuit.rhs87 ]
  br i1 %short_circuit90, label %if.then91, label %if.merge92

if.then91:                                        ; preds = %short_circuit.merge88
  %negative93 = load i1, ptr %negative, align 1
  br i1 %negative93, label %if.then94, label %if.else95

if.merge92:                                       ; preds = %if.merge96, %short_circuit.merge88
  br label %if.merge

if.then94:                                        ; preds = %if.then91
  %zero_wide97 = load i128, ptr %zero_wide, align 4
  %magnitude98 = load i128, ptr %magnitude, align 4
  %sub99 = sub i128 %zero_wide97, %magnitude98
  store i128 %sub99, ptr %flipped, align 4
  %flipped100 = load i128, ptr %flipped, align 4
  %int_trunc101 = trunc i128 %flipped100 to i64
  store i64 %int_trunc101, ptr %assignment_value102, align 4
  %assignment_value103 = load i64, ptr %assignment_value102, align 4
  store i64 %assignment_value103, ptr %out, align 4
  br label %if.merge96

if.else95:                                        ; preds = %if.then91
  %magnitude104 = load i128, ptr %magnitude, align 4
  %int_trunc105 = trunc i128 %magnitude104 to i64
  store i64 %int_trunc105, ptr %assignment_value106, align 4
  %assignment_value107 = load i64, ptr %assignment_value106, align 4
  store i64 %assignment_value107, ptr %out, align 4
  br label %if.merge96

if.merge96:                                       ; preds = %if.else95, %if.then94
  %address = ptrtoint ptr %out to i32
  store i32 %address, ptr %assignment_value108, align 4
  %assignment_value109 = load i32, ptr %assignment_value108, align 4
  store i32 %assignment_value109, ptr %result, align 4
  br label %if.merge92

return_result_null_0:                             ; preds = %if.merge
  br label %return_result_merge_2

return_result_copy_1:                             ; preds = %if.merge
  %allocation = call ptr @__wosy_core_alloc(i64 8, i64 8)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

return_result_merge_2:                            ; preds = %allocation_continue, %return_result_null_0
  %return_result = phi i32 [ 0, %return_result_null_0 ], [ %return_result_address, %allocation_continue ]
  ret i32 %return_result

allocation_panic:                                 ; preds = %return_result_copy_1
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %return_result_copy_1
  %return_result_source = inttoptr i32 %result110 to ptr
  %return_result_value = load i64, ptr %return_result_source, align 4
  store i64 %return_result_value, ptr %allocation, align 4
  %return_result_address = ptrtoint ptr %allocation to i32
  br label %return_result_merge_2
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69313238(ptr %text) {
entry:
  %assignment_value123 = alloca i32, align 4
  %assignment_value121 = alloca i128, align 8
  %assignment_value114 = alloca i128, align 8
  %neg_hi = alloca i128, align 8
  %hi_ext = alloca i128, align 8
  %lo_ext = alloca i128, align 8
  %hi64 = alloca i64, align 8
  %hi_wide = alloca i128, align 8
  %lo64 = alloca i64, align 8
  %fits = alloca i1, align 1
  %valid = alloca i1, align 1
  %magnitude = alloca i128, align 8
  %assignment_value73 = alloca i64, align 8
  %assignment_value65 = alloca i8, align 1
  %detected = alloca i8, align 1
  %assignment_value59 = alloca i8, align 1
  %prefix_second = alloca i8, align 1
  %assignment_value49 = alloca i8, align 1
  %prefix_first = alloca i8, align 1
  %position_next = alloca i64, align 8
  %wide_next = alloca i128, align 8
  %position_start = alloca i64, align 8
  %wide_start = alloca i128, align 8
  %next = alloca i64, align 8
  %rest = alloca i64, align 8
  %assignment_value22 = alloca i64, align 8
  %assignment_value19 = alloca i1, align 1
  %assignment_value14 = alloca i8, align 1
  %first = alloca i8, align 1
  %position_zero = alloca i64, align 8
  %wide_zero = alloca i128, align 8
  %assignment_value = alloca i32, align 4
  %result = alloca i32, align 4
  %out = alloca i128, align 8
  %zero_signed = alloca i128, align 8
  %two64_signed = alloca i128, align 8
  %two64 = alloca i128, align 8
  %negative = alloca i1, align 1
  %start = alloca i64, align 8
  %radix = alloca i8, align 1
  %two = alloca i64, align 8
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store i64 2, ptr %two, align 4
  store i8 10, ptr %radix, align 1
  %zero2 = load i64, ptr %zero, align 4
  store i64 %zero2, ptr %start, align 4
  store i1 false, ptr %negative, align 1
  store i128 18446744073709551616, ptr %two64, align 4
  store i128 18446744073709551616, ptr %two64_signed, align 4
  %zero3 = load i64, ptr %zero, align 4
  %int_extend = zext i64 %zero3 to i128
  store i128 %int_extend, ptr %zero_signed, align 4
  store i128 0, ptr %out, align 4
  store i32 0, ptr %result, align 4
  %data = getelementptr inbounds i8, ptr %text1, i8 0
  %place = load i32, ptr %data, align 4
  %eq = icmp eq i32 %place, 0
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %length = getelementptr inbounds i8, ptr %text1, i8 8
  %place4 = load i64, ptr %length, align 4
  %zero5 = load i64, ptr %zero, align 4
  %eq6 = icmp eq i64 %place4, %zero5
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %eq6, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  store i32 0, ptr %assignment_value, align 4
  %assignment_value7 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value7, ptr %result, align 4
  br label %if.merge

if.else:                                          ; preds = %short_circuit.merge
  %zero8 = load i64, ptr %zero, align 4
  %int_extend9 = zext i64 %zero8 to i128
  store i128 %int_extend9, ptr %wide_zero, align 4
  %wide_zero10 = load i128, ptr %wide_zero, align 4
  %int_trunc = trunc i128 %wide_zero10 to i64
  store i64 %int_trunc, ptr %position_zero, align 4
  store i8 0, ptr %first, align 1
  %data11 = getelementptr inbounds i8, ptr %text1, i8 0
  %place12 = load i32, ptr %data11, align 4
  %position_zero13 = load i64, ptr %position_zero, align 4
  %offset_base = inttoptr i32 %place12 to ptr
  %offset = getelementptr inbounds i8, ptr %offset_base, i64 %position_zero13
  %offset_address = ptrtoint ptr %offset to i32
  %load_base = inttoptr i32 %offset_address to ptr
  %load = load i8, ptr %load_base, align 1
  store i8 %load, ptr %assignment_value14, align 1
  %assignment_value15 = load i8, ptr %assignment_value14, align 1
  store i8 %assignment_value15, ptr %first, align 1
  %first16 = load i8, ptr %first, align 1
  %call = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f7369676e(i8 %first16)
  br i1 %call, label %if.then17, label %if.merge18

if.merge:                                         ; preds = %if.merge92, %if.then
  %result125 = load i32, ptr %result, align 4
  %return_result_is_null = icmp eq i32 %result125, 0
  br i1 %return_result_is_null, label %return_result_null_0, label %return_result_copy_1

if.then17:                                        ; preds = %if.else
  store i1 true, ptr %assignment_value19, align 1
  %assignment_value20 = load i1, ptr %assignment_value19, align 1
  store i1 %assignment_value20, ptr %negative, align 1
  %one21 = load i64, ptr %one, align 4
  store i64 %one21, ptr %assignment_value22, align 4
  %assignment_value23 = load i64, ptr %assignment_value22, align 4
  store i64 %assignment_value23, ptr %start, align 4
  br label %if.merge18

if.merge18:                                       ; preds = %if.then17, %if.else
  %length24 = getelementptr inbounds i8, ptr %text1, i8 8
  %place25 = load i64, ptr %length24, align 4
  %start26 = load i64, ptr %start, align 4
  %sub = sub i64 %place25, %start26
  store i64 %sub, ptr %rest, align 4
  %rest27 = load i64, ptr %rest, align 4
  %two28 = load i64, ptr %two, align 4
  %ge = icmp uge i64 %rest27, %two28
  br i1 %ge, label %if.then29, label %if.merge30

if.then29:                                        ; preds = %if.merge18
  %start31 = load i64, ptr %start, align 4
  %one32 = load i64, ptr %one, align 4
  %add = add i64 %start31, %one32
  store i64 %add, ptr %next, align 4
  %start33 = load i64, ptr %start, align 4
  %int_extend34 = zext i64 %start33 to i128
  store i128 %int_extend34, ptr %wide_start, align 4
  %wide_start35 = load i128, ptr %wide_start, align 4
  %int_trunc36 = trunc i128 %wide_start35 to i64
  store i64 %int_trunc36, ptr %position_start, align 4
  %next37 = load i64, ptr %next, align 4
  %int_extend38 = zext i64 %next37 to i128
  store i128 %int_extend38, ptr %wide_next, align 4
  %wide_next39 = load i128, ptr %wide_next, align 4
  %int_trunc40 = trunc i128 %wide_next39 to i64
  store i64 %int_trunc40, ptr %position_next, align 4
  store i8 0, ptr %prefix_first, align 1
  %data41 = getelementptr inbounds i8, ptr %text1, i8 0
  %place42 = load i32, ptr %data41, align 4
  %position_start43 = load i64, ptr %position_start, align 4
  %offset_base44 = inttoptr i32 %place42 to ptr
  %offset45 = getelementptr inbounds i8, ptr %offset_base44, i64 %position_start43
  %offset_address46 = ptrtoint ptr %offset45 to i32
  %load_base47 = inttoptr i32 %offset_address46 to ptr
  %load48 = load i8, ptr %load_base47, align 1
  store i8 %load48, ptr %assignment_value49, align 1
  %assignment_value50 = load i8, ptr %assignment_value49, align 1
  store i8 %assignment_value50, ptr %prefix_first, align 1
  store i8 0, ptr %prefix_second, align 1
  %data51 = getelementptr inbounds i8, ptr %text1, i8 0
  %place52 = load i32, ptr %data51, align 4
  %position_next53 = load i64, ptr %position_next, align 4
  %offset_base54 = inttoptr i32 %place52 to ptr
  %offset55 = getelementptr inbounds i8, ptr %offset_base54, i64 %position_next53
  %offset_address56 = ptrtoint ptr %offset55 to i32
  %load_base57 = inttoptr i32 %offset_address56 to ptr
  %load58 = load i8, ptr %load_base57, align 1
  store i8 %load58, ptr %assignment_value59, align 1
  %assignment_value60 = load i8, ptr %assignment_value59, align 1
  store i8 %assignment_value60, ptr %prefix_second, align 1
  %prefix_first61 = load i8, ptr %prefix_first, align 1
  %prefix_second62 = load i8, ptr %prefix_second, align 1
  %call63 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f72616469785f66726f6d5f707265666978(i8 %prefix_first61, i8 %prefix_second62)
  store i8 %call63, ptr %detected, align 1
  %detected64 = load i8, ptr %detected, align 1
  store i8 %detected64, ptr %assignment_value65, align 1
  %assignment_value66 = load i8, ptr %assignment_value65, align 1
  store i8 %assignment_value66, ptr %radix, align 1
  %detected67 = load i8, ptr %detected, align 1
  %ne = icmp ne i8 %detected67, 10
  br i1 %ne, label %if.then68, label %if.merge69

if.merge30:                                       ; preds = %if.merge69, %if.merge18
  %data75 = getelementptr inbounds i8, ptr %text1, i8 0
  %place76 = load i32, ptr %data75, align 4
  %start77 = load i64, ptr %start, align 4
  %length78 = getelementptr inbounds i8, ptr %text1, i8 8
  %place79 = load i64, ptr %length78, align 4
  %radix80 = load i8, ptr %radix, align 1
  %call81 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f616363756d756c6174655f693132385f6e6567(i32 %place76, i64 %start77, i64 %place79, i8 %radix80)
  %output = extractvalue { i128, i1 } %call81, 0
  store i128 %output, ptr %magnitude, align 4
  %output82 = extractvalue { i128, i1 } %call81, 1
  store i1 %output82, ptr %valid, align 1
  %magnitude83 = load i128, ptr %magnitude, align 4
  %negative84 = load i1, ptr %negative, align 1
  %call85 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f636865636b5f69313238(i128 %magnitude83, i1 %negative84)
  store i1 %call85, ptr %fits, align 1
  %valid86 = load i1, ptr %valid, align 1
  br i1 %valid86, label %short_circuit.rhs87, label %short_circuit.merge88

if.then68:                                        ; preds = %if.then29
  %start70 = load i64, ptr %start, align 4
  %two71 = load i64, ptr %two, align 4
  %add72 = add i64 %start70, %two71
  store i64 %add72, ptr %assignment_value73, align 4
  %assignment_value74 = load i64, ptr %assignment_value73, align 4
  store i64 %assignment_value74, ptr %start, align 4
  br label %if.merge69

if.merge69:                                       ; preds = %if.then68, %if.then29
  br label %if.merge30

short_circuit.rhs87:                              ; preds = %if.merge30
  %fits89 = load i1, ptr %fits, align 1
  br label %short_circuit.merge88

short_circuit.merge88:                            ; preds = %short_circuit.rhs87, %if.merge30
  %short_circuit90 = phi i1 [ false, %if.merge30 ], [ %fits89, %short_circuit.rhs87 ]
  br i1 %short_circuit90, label %if.then91, label %if.merge92

if.then91:                                        ; preds = %short_circuit.merge88
  %magnitude93 = load i128, ptr %magnitude, align 4
  %int_trunc94 = trunc i128 %magnitude93 to i64
  store i64 %int_trunc94, ptr %lo64, align 4
  %magnitude95 = load i128, ptr %magnitude, align 4
  %two6496 = load i128, ptr %two64, align 4
  %div = udiv i128 %magnitude95, %two6496
  store i128 %div, ptr %hi_wide, align 4
  %hi_wide97 = load i128, ptr %hi_wide, align 4
  %int_trunc98 = trunc i128 %hi_wide97 to i64
  store i64 %int_trunc98, ptr %hi64, align 4
  %lo6499 = load i64, ptr %lo64, align 4
  %int_extend100 = zext i64 %lo6499 to i128
  store i128 %int_extend100, ptr %lo_ext, align 4
  %hi64101 = load i64, ptr %hi64, align 4
  %int_extend102 = zext i64 %hi64101 to i128
  store i128 %int_extend102, ptr %hi_ext, align 4
  %negative103 = load i1, ptr %negative, align 1
  br i1 %negative103, label %if.then104, label %if.else105

if.merge92:                                       ; preds = %if.merge106, %short_circuit.merge88
  br label %if.merge

if.then104:                                       ; preds = %if.then91
  %zero_signed107 = load i128, ptr %zero_signed, align 4
  %hi_ext108 = load i128, ptr %hi_ext, align 4
  %sub109 = sub i128 %zero_signed107, %hi_ext108
  store i128 %sub109, ptr %neg_hi, align 4
  %neg_hi110 = load i128, ptr %neg_hi, align 4
  %two64_signed111 = load i128, ptr %two64_signed, align 4
  %mul = mul i128 %neg_hi110, %two64_signed111
  %lo_ext112 = load i128, ptr %lo_ext, align 4
  %sub113 = sub i128 %mul, %lo_ext112
  store i128 %sub113, ptr %assignment_value114, align 4
  %assignment_value115 = load i128, ptr %assignment_value114, align 4
  store i128 %assignment_value115, ptr %out, align 4
  br label %if.merge106

if.else105:                                       ; preds = %if.then91
  %hi_ext116 = load i128, ptr %hi_ext, align 4
  %two64_signed117 = load i128, ptr %two64_signed, align 4
  %mul118 = mul i128 %hi_ext116, %two64_signed117
  %lo_ext119 = load i128, ptr %lo_ext, align 4
  %add120 = add i128 %mul118, %lo_ext119
  store i128 %add120, ptr %assignment_value121, align 4
  %assignment_value122 = load i128, ptr %assignment_value121, align 4
  store i128 %assignment_value122, ptr %out, align 4
  br label %if.merge106

if.merge106:                                      ; preds = %if.else105, %if.then104
  %address = ptrtoint ptr %out to i32
  store i32 %address, ptr %assignment_value123, align 4
  %assignment_value124 = load i32, ptr %assignment_value123, align 4
  store i32 %assignment_value124, ptr %result, align 4
  br label %if.merge92

return_result_null_0:                             ; preds = %if.merge
  br label %return_result_merge_2

return_result_copy_1:                             ; preds = %if.merge
  %allocation = call ptr @__wosy_core_alloc(i64 16, i64 16)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

return_result_merge_2:                            ; preds = %allocation_continue, %return_result_null_0
  %return_result = phi i32 [ 0, %return_result_null_0 ], [ %return_result_address, %allocation_continue ]
  ret i32 %return_result

allocation_panic:                                 ; preds = %return_result_copy_1
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %return_result_copy_1
  %return_result_source = inttoptr i32 %result125 to ptr
  %return_result_value = load i128, ptr %return_result_source, align 4
  store i128 %return_result_value, ptr %allocation, align 4
  %return_result_address = ptrtoint ptr %allocation to i32
  br label %return_result_merge_2
}

define i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f666c6f61745f62797465(i32 %data, i64 %index) {
entry:
  %assignment_value = alloca i8, align 1
  %byte = alloca i8, align 1
  %position = alloca i64, align 8
  %wide = alloca i128, align 8
  %data1 = alloca i32, align 4
  store i32 %data, ptr %data1, align 4
  %index2 = alloca i64, align 8
  store i64 %index, ptr %index2, align 4
  %index3 = load i64, ptr %index2, align 4
  %int_extend = zext i64 %index3 to i128
  store i128 %int_extend, ptr %wide, align 4
  %wide4 = load i128, ptr %wide, align 4
  %int_trunc = trunc i128 %wide4 to i64
  store i64 %int_trunc, ptr %position, align 4
  store i8 0, ptr %byte, align 1
  %data5 = load i32, ptr %data1, align 4
  %position6 = load i64, ptr %position, align 4
  %offset_base = inttoptr i32 %data5 to ptr
  %offset = getelementptr inbounds i8, ptr %offset_base, i64 %position6
  %offset_address = ptrtoint ptr %offset to i32
  %load_base = inttoptr i32 %offset_address to ptr
  %load = load i8, ptr %load_base, align 1
  store i8 %load, ptr %assignment_value, align 1
  %assignment_value7 = load i8, ptr %assignment_value, align 1
  store i8 %assignment_value7, ptr %byte, align 1
  %byte8 = load i8, ptr %byte, align 1
  ret i8 %byte8
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f646f74(i8 %byte) {
entry:
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  %byte2 = load i8, ptr %byte1, align 1
  %eq = icmp eq i8 %byte2, 46
  ret i1 %eq
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f65(i8 %byte) {
entry:
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  %byte2 = load i8, ptr %byte1, align 1
  %eq = icmp eq i8 %byte2, 101
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %byte3 = load i8, ptr %byte1, align 1
  %eq4 = icmp eq i8 %byte3, 69
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %eq4, %short_circuit.rhs ]
  ret i1 %short_circuit
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f64696769743130(i8 %byte) {
entry:
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  %byte2 = load i8, ptr %byte1, align 1
  %ge = icmp uge i8 %byte2, 48
  br i1 %ge, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %byte3 = load i8, ptr %byte1, align 1
  %le = icmp ule i8 %byte3, 57
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %le, %short_circuit.rhs ]
  ret i1 %short_circuit
}

define { double, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f666c6f61745f663634(i32 %data, i64 %start, i64 %end) {
entry:
  %assignment_value480 = alloca i1, align 1
  %assignment_value470 = alloca i1, align 1
  %assignment_value459 = alloca i32, align 4
  %assignment_value454 = alloca double, align 8
  %assignment_value446 = alloca i32, align 4
  %assignment_value441 = alloca double, align 8
  %assignment_value427 = alloca i32, align 4
  %steps = alloca i32, align 4
  %assignment_value416 = alloca double, align 8
  %assignment_value407 = alloca i1, align 1
  %adjusted = alloca i32, align 4
  %assignment_value394 = alloca i32, align 4
  %exp_signed = alloca i32, align 4
  %assignment_value385 = alloca double, align 8
  %scaled = alloca double, align 8
  %assignment_value373 = alloca i1, align 1
  %assignment_value368 = alloca i1, align 1
  %assignment_value362 = alloca i1, align 1
  %assignment_value360 = alloca i64, align 8
  %assignment_value355 = alloca i1, align 1
  %assignment_value353 = alloca i1, align 1
  %assignment_value348 = alloca i1, align 1
  %assignment_value338 = alloca i64, align 8
  %assignment_value333 = alloca i1, align 1
  %assignment_value331 = alloca i1, align 1
  %assignment_value329 = alloca i32, align 4
  %piece = alloca i32, align 4
  %small = alloca i8, align 1
  %underscore309 = alloca i1, align 1
  %digit306 = alloca i1, align 1
  %byte303 = alloca i8, align 1
  %exp_done = alloca i1, align 1
  %exp_trail = alloca i1, align 1
  %exp_started = alloca i1, align 1
  %assignment_value286 = alloca i64, align 8
  %assignment_value277 = alloca i64, align 8
  %assignment_value272 = alloca i1, align 1
  %byte266 = alloca i8, align 1
  %assignment_value249 = alloca i64, align 8
  %assignment_value244 = alloca i1, align 1
  %byte239 = alloca i8, align 1
  %has_exp = alloca i1, align 1
  %assignment_value225 = alloca i1, align 1
  %assignment_value220 = alloca i1, align 1
  %assignment_value218 = alloca i64, align 8
  %assignment_value213 = alloca i1, align 1
  %assignment_value211 = alloca i1, align 1
  %assignment_value206 = alloca i1, align 1
  %assignment_value196 = alloca i64, align 8
  %assignment_value191 = alloca i1, align 1
  %assignment_value189 = alloca i1, align 1
  %assignment_value187 = alloca i32, align 4
  %assignment_value177 = alloca i32, align 4
  %assignment_value172 = alloca double, align 8
  %assignment_value162 = alloca double, align 8
  %digit_value156 = alloca double, align 8
  %wide_digit153 = alloca i64, align 8
  %small_frac = alloca i8, align 1
  %underscore143 = alloca i1, align 1
  %digit140 = alloca i1, align 1
  %byte137 = alloca i8, align 1
  %frac_done = alloca i1, align 1
  %frac_trail = alloca i1, align 1
  %frac_started = alloca i1, align 1
  %assignment_value113 = alloca i64, align 8
  %assignment_value108 = alloca i1, align 1
  %byte103 = alloca i8, align 1
  %has_dot = alloca i1, align 1
  %assignment_value89 = alloca i1, align 1
  %assignment_value83 = alloca i1, align 1
  %assignment_value78 = alloca i1, align 1
  %assignment_value76 = alloca i64, align 8
  %assignment_value71 = alloca i1, align 1
  %assignment_value69 = alloca i1, align 1
  %assignment_value64 = alloca i1, align 1
  %assignment_value54 = alloca i64, align 8
  %assignment_value49 = alloca i1, align 1
  %assignment_value47 = alloca i1, align 1
  %assignment_value45 = alloca i32, align 4
  %assignment_value40 = alloca double, align 8
  %assignment_value = alloca double, align 8
  %digit_value = alloca double, align 8
  %wide_digit = alloca i64, align 8
  %small_int = alloca i8, align 1
  %underscore = alloca i1, align 1
  %digit = alloca i1, align 1
  %byte = alloca i8, align 1
  %int_done = alloca i1, align 1
  %exp_negative = alloca i1, align 1
  %exp_value = alloca i32, align 4
  %frac_count = alloca i32, align 4
  %last_underscore = alloca i1, align 1
  %saw_int = alloca i1, align 1
  %valid = alloca i1, align 1
  %pos = alloca i64, align 8
  %extra_scale = alloca i32, align 4
  %mant = alloca double, align 8
  %neg_limit = alloca i32, align 4
  %limit = alloca i32, align 4
  %cap = alloca i32, align 4
  %ten_count = alloca i32, align 4
  %one_count = alloca i32, align 4
  %zero_count = alloca i32, align 4
  %zero_digit = alloca i8, align 1
  %f64_max = alloca double, align 8
  %big_limit = alloca double, align 8
  %ten_value = alloca double, align 8
  %zero_value = alloca double, align 8
  %one = alloca i64, align 8
  %data1 = alloca i32, align 4
  store i32 %data, ptr %data1, align 4
  %start2 = alloca i64, align 8
  store i64 %start, ptr %start2, align 4
  %end3 = alloca i64, align 8
  store i64 %end, ptr %end3, align 4
  store i64 1, ptr %one, align 4
  store double 0.000000e+00, ptr %zero_value, align 8
  store double 1.000000e+01, ptr %ten_value, align 8
  store double 1.000000e+300, ptr %big_limit, align 8
  store double 0x7FEFFFFFFFFFFFFF, ptr %f64_max, align 8
  store i8 48, ptr %zero_digit, align 1
  store i32 0, ptr %zero_count, align 4
  store i32 1, ptr %one_count, align 4
  store i32 10, ptr %ten_count, align 4
  store i32 1000000, ptr %cap, align 4
  store i32 1000, ptr %limit, align 4
  %zero_count4 = load i32, ptr %zero_count, align 4
  %limit5 = load i32, ptr %limit, align 4
  %sub = sub i32 %zero_count4, %limit5
  store i32 %sub, ptr %neg_limit, align 4
  %zero_value6 = load double, ptr %zero_value, align 8
  store double %zero_value6, ptr %mant, align 8
  %zero_count7 = load i32, ptr %zero_count, align 4
  store i32 %zero_count7, ptr %extra_scale, align 4
  %start8 = load i64, ptr %start2, align 4
  store i64 %start8, ptr %pos, align 4
  store i1 true, ptr %valid, align 1
  store i1 false, ptr %saw_int, align 1
  store i1 false, ptr %last_underscore, align 1
  %zero_count9 = load i32, ptr %zero_count, align 4
  store i32 %zero_count9, ptr %frac_count, align 4
  %zero_count10 = load i32, ptr %zero_count, align 4
  store i32 %zero_count10, ptr %exp_value, align 4
  store i1 false, ptr %exp_negative, align 1
  store i1 false, ptr %int_done, align 1
  br label %while.cond.0

while.cond.0:                                     ; preds = %if.merge, %entry
  %pos11 = load i64, ptr %pos, align 4
  %end12 = load i64, ptr %end3, align 4
  %lt = icmp ult i64 %pos11, %end12
  br i1 %lt, label %short_circuit.rhs, label %short_circuit.merge

while.body.1:                                     ; preds = %short_circuit.merge15
  %data18 = load i32, ptr %data1, align 4
  %pos19 = load i64, ptr %pos, align 4
  %call = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f666c6f61745f62797465(i32 %data18, i64 %pos19)
  store i8 %call, ptr %byte, align 1
  %byte20 = load i8, ptr %byte, align 1
  %call21 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f64696769743130(i8 %byte20)
  store i1 %call21, ptr %digit, align 1
  %byte22 = load i8, ptr %byte, align 1
  %call23 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f756e64657273636f7265(i8 %byte22)
  store i1 %call23, ptr %underscore, align 1
  %digit24 = load i1, ptr %digit, align 1
  br i1 %digit24, label %if.then, label %if.else

while.exit.2:                                     ; preds = %short_circuit.merge15
  %last_underscore80 = load i1, ptr %last_underscore, align 1
  br i1 %last_underscore80, label %if.then81, label %if.merge82

short_circuit.rhs:                                ; preds = %while.cond.0
  %valid13 = load i1, ptr %valid, align 1
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %while.cond.0
  %short_circuit = phi i1 [ false, %while.cond.0 ], [ %valid13, %short_circuit.rhs ]
  br i1 %short_circuit, label %short_circuit.rhs14, label %short_circuit.merge15

short_circuit.rhs14:                              ; preds = %short_circuit.merge
  %int_done16 = load i1, ptr %int_done, align 1
  %not = xor i1 %int_done16, true
  br label %short_circuit.merge15

short_circuit.merge15:                            ; preds = %short_circuit.rhs14, %short_circuit.merge
  %short_circuit17 = phi i1 [ false, %short_circuit.merge ], [ %not, %short_circuit.rhs14 ]
  br i1 %short_circuit17, label %while.body.1, label %while.exit.2

if.then:                                          ; preds = %while.body.1
  %byte25 = load i8, ptr %byte, align 1
  %zero_digit26 = load i8, ptr %zero_digit, align 1
  %sub27 = sub i8 %byte25, %zero_digit26
  store i8 %sub27, ptr %small_int, align 1
  %small_int28 = load i8, ptr %small_int, align 1
  %int_extend = zext i8 %small_int28 to i64
  store i64 %int_extend, ptr %wide_digit, align 4
  %wide_digit29 = load i64, ptr %wide_digit, align 4
  %uint_to_float = uitofp i64 %wide_digit29 to double
  store double %uint_to_float, ptr %digit_value, align 8
  %mant30 = load double, ptr %mant, align 8
  %ten_value31 = load double, ptr %ten_value, align 8
  %mul = fmul double %mant30, %ten_value31
  %digit_value32 = load double, ptr %digit_value, align 8
  %add = fadd double %mul, %digit_value32
  store double %add, ptr %assignment_value, align 8
  %assignment_value33 = load double, ptr %assignment_value, align 8
  store double %assignment_value33, ptr %mant, align 8
  %mant34 = load double, ptr %mant, align 8
  %big_limit35 = load double, ptr %big_limit, align 8
  %gt = fcmp ogt double %mant34, %big_limit35
  br i1 %gt, label %if.then36, label %if.merge37

if.else:                                          ; preds = %while.body.1
  %underscore56 = load i1, ptr %underscore, align 1
  br i1 %underscore56, label %if.then57, label %if.else58

if.merge:                                         ; preds = %if.merge59, %if.merge37
  br label %while.cond.0

if.then36:                                        ; preds = %if.then
  %mant38 = load double, ptr %mant, align 8
  %ten_value39 = load double, ptr %ten_value, align 8
  %div = fdiv double %mant38, %ten_value39
  store double %div, ptr %assignment_value40, align 8
  %assignment_value41 = load double, ptr %assignment_value40, align 8
  store double %assignment_value41, ptr %mant, align 8
  %extra_scale42 = load i32, ptr %extra_scale, align 4
  %one_count43 = load i32, ptr %one_count, align 4
  %add44 = add i32 %extra_scale42, %one_count43
  store i32 %add44, ptr %assignment_value45, align 4
  %assignment_value46 = load i32, ptr %assignment_value45, align 4
  store i32 %assignment_value46, ptr %extra_scale, align 4
  br label %if.merge37

if.merge37:                                       ; preds = %if.then36, %if.then
  store i1 false, ptr %assignment_value47, align 1
  %assignment_value48 = load i1, ptr %assignment_value47, align 1
  store i1 %assignment_value48, ptr %last_underscore, align 1
  store i1 true, ptr %assignment_value49, align 1
  %assignment_value50 = load i1, ptr %assignment_value49, align 1
  store i1 %assignment_value50, ptr %saw_int, align 1
  %pos51 = load i64, ptr %pos, align 4
  %one52 = load i64, ptr %one, align 4
  %add53 = add i64 %pos51, %one52
  store i64 %add53, ptr %assignment_value54, align 4
  %assignment_value55 = load i64, ptr %assignment_value54, align 4
  store i64 %assignment_value55, ptr %pos, align 4
  br label %if.merge

if.then57:                                        ; preds = %if.else
  %saw_int60 = load i1, ptr %saw_int, align 1
  %not61 = xor i1 %saw_int60, true
  br i1 %not61, label %if.then62, label %if.merge63

if.else58:                                        ; preds = %if.else
  store i1 true, ptr %assignment_value78, align 1
  %assignment_value79 = load i1, ptr %assignment_value78, align 1
  store i1 %assignment_value79, ptr %int_done, align 1
  br label %if.merge59

if.merge59:                                       ; preds = %if.else58, %if.merge68
  br label %if.merge

if.then62:                                        ; preds = %if.then57
  store i1 false, ptr %assignment_value64, align 1
  %assignment_value65 = load i1, ptr %assignment_value64, align 1
  store i1 %assignment_value65, ptr %valid, align 1
  br label %if.merge63

if.merge63:                                       ; preds = %if.then62, %if.then57
  %last_underscore66 = load i1, ptr %last_underscore, align 1
  br i1 %last_underscore66, label %if.then67, label %if.merge68

if.then67:                                        ; preds = %if.merge63
  store i1 false, ptr %assignment_value69, align 1
  %assignment_value70 = load i1, ptr %assignment_value69, align 1
  store i1 %assignment_value70, ptr %valid, align 1
  br label %if.merge68

if.merge68:                                       ; preds = %if.then67, %if.merge63
  store i1 true, ptr %assignment_value71, align 1
  %assignment_value72 = load i1, ptr %assignment_value71, align 1
  store i1 %assignment_value72, ptr %last_underscore, align 1
  %pos73 = load i64, ptr %pos, align 4
  %one74 = load i64, ptr %one, align 4
  %add75 = add i64 %pos73, %one74
  store i64 %add75, ptr %assignment_value76, align 4
  %assignment_value77 = load i64, ptr %assignment_value76, align 4
  store i64 %assignment_value77, ptr %pos, align 4
  br label %if.merge59

if.then81:                                        ; preds = %while.exit.2
  store i1 false, ptr %assignment_value83, align 1
  %assignment_value84 = load i1, ptr %assignment_value83, align 1
  store i1 %assignment_value84, ptr %valid, align 1
  br label %if.merge82

if.merge82:                                       ; preds = %if.then81, %while.exit.2
  %saw_int85 = load i1, ptr %saw_int, align 1
  %not86 = xor i1 %saw_int85, true
  br i1 %not86, label %if.then87, label %if.merge88

if.then87:                                        ; preds = %if.merge82
  store i1 false, ptr %assignment_value89, align 1
  %assignment_value90 = load i1, ptr %assignment_value89, align 1
  store i1 %assignment_value90, ptr %valid, align 1
  br label %if.merge88

if.merge88:                                       ; preds = %if.then87, %if.merge82
  store i1 false, ptr %has_dot, align 1
  %valid91 = load i1, ptr %valid, align 1
  br i1 %valid91, label %short_circuit.rhs92, label %short_circuit.merge93

short_circuit.rhs92:                              ; preds = %if.merge88
  %pos94 = load i64, ptr %pos, align 4
  %end95 = load i64, ptr %end3, align 4
  %lt96 = icmp ult i64 %pos94, %end95
  br label %short_circuit.merge93

short_circuit.merge93:                            ; preds = %short_circuit.rhs92, %if.merge88
  %short_circuit97 = phi i1 [ false, %if.merge88 ], [ %lt96, %short_circuit.rhs92 ]
  br i1 %short_circuit97, label %if.then98, label %if.merge99

if.then98:                                        ; preds = %short_circuit.merge93
  %data100 = load i32, ptr %data1, align 4
  %pos101 = load i64, ptr %pos, align 4
  %call102 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f666c6f61745f62797465(i32 %data100, i64 %pos101)
  store i8 %call102, ptr %byte103, align 1
  %byte104 = load i8, ptr %byte103, align 1
  %call105 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f646f74(i8 %byte104)
  br i1 %call105, label %if.then106, label %if.merge107

if.merge99:                                       ; preds = %if.merge107, %short_circuit.merge93
  %valid115 = load i1, ptr %valid, align 1
  br i1 %valid115, label %short_circuit.rhs116, label %short_circuit.merge117

if.then106:                                       ; preds = %if.then98
  store i1 true, ptr %assignment_value108, align 1
  %assignment_value109 = load i1, ptr %assignment_value108, align 1
  store i1 %assignment_value109, ptr %has_dot, align 1
  %pos110 = load i64, ptr %pos, align 4
  %one111 = load i64, ptr %one, align 4
  %add112 = add i64 %pos110, %one111
  store i64 %add112, ptr %assignment_value113, align 4
  %assignment_value114 = load i64, ptr %assignment_value113, align 4
  store i64 %assignment_value114, ptr %pos, align 4
  br label %if.merge107

if.merge107:                                      ; preds = %if.then106, %if.then98
  br label %if.merge99

short_circuit.rhs116:                             ; preds = %if.merge99
  %has_dot118 = load i1, ptr %has_dot, align 1
  br label %short_circuit.merge117

short_circuit.merge117:                           ; preds = %short_circuit.rhs116, %if.merge99
  %short_circuit119 = phi i1 [ false, %if.merge99 ], [ %has_dot118, %short_circuit.rhs116 ]
  br i1 %short_circuit119, label %if.then120, label %if.merge121

if.then120:                                       ; preds = %short_circuit.merge117
  store i1 false, ptr %frac_started, align 1
  store i1 false, ptr %frac_trail, align 1
  store i1 false, ptr %frac_done, align 1
  br label %while.cond.3

if.merge121:                                      ; preds = %if.merge224, %short_circuit.merge117
  store i1 false, ptr %has_exp, align 1
  %valid227 = load i1, ptr %valid, align 1
  br i1 %valid227, label %short_circuit.rhs228, label %short_circuit.merge229

while.cond.3:                                     ; preds = %if.merge147, %if.then120
  %pos122 = load i64, ptr %pos, align 4
  %end123 = load i64, ptr %end3, align 4
  %lt124 = icmp ult i64 %pos122, %end123
  br i1 %lt124, label %short_circuit.rhs125, label %short_circuit.merge126

while.body.4:                                     ; preds = %short_circuit.merge130
  %data134 = load i32, ptr %data1, align 4
  %pos135 = load i64, ptr %pos, align 4
  %call136 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f666c6f61745f62797465(i32 %data134, i64 %pos135)
  store i8 %call136, ptr %byte137, align 1
  %byte138 = load i8, ptr %byte137, align 1
  %call139 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f64696769743130(i8 %byte138)
  store i1 %call139, ptr %digit140, align 1
  %byte141 = load i8, ptr %byte137, align 1
  %call142 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f756e64657273636f7265(i8 %byte141)
  store i1 %call142, ptr %underscore143, align 1
  %digit144 = load i1, ptr %digit140, align 1
  br i1 %digit144, label %if.then145, label %if.else146

while.exit.5:                                     ; preds = %short_circuit.merge130
  %frac_trail222 = load i1, ptr %frac_trail, align 1
  br i1 %frac_trail222, label %if.then223, label %if.merge224

short_circuit.rhs125:                             ; preds = %while.cond.3
  %valid127 = load i1, ptr %valid, align 1
  br label %short_circuit.merge126

short_circuit.merge126:                           ; preds = %short_circuit.rhs125, %while.cond.3
  %short_circuit128 = phi i1 [ false, %while.cond.3 ], [ %valid127, %short_circuit.rhs125 ]
  br i1 %short_circuit128, label %short_circuit.rhs129, label %short_circuit.merge130

short_circuit.rhs129:                             ; preds = %short_circuit.merge126
  %frac_done131 = load i1, ptr %frac_done, align 1
  %not132 = xor i1 %frac_done131, true
  br label %short_circuit.merge130

short_circuit.merge130:                           ; preds = %short_circuit.rhs129, %short_circuit.merge126
  %short_circuit133 = phi i1 [ false, %short_circuit.merge126 ], [ %not132, %short_circuit.rhs129 ]
  br i1 %short_circuit133, label %while.body.4, label %while.exit.5

if.then145:                                       ; preds = %while.body.4
  %byte148 = load i8, ptr %byte137, align 1
  %zero_digit149 = load i8, ptr %zero_digit, align 1
  %sub150 = sub i8 %byte148, %zero_digit149
  store i8 %sub150, ptr %small_frac, align 1
  %small_frac151 = load i8, ptr %small_frac, align 1
  %int_extend152 = zext i8 %small_frac151 to i64
  store i64 %int_extend152, ptr %wide_digit153, align 4
  %wide_digit154 = load i64, ptr %wide_digit153, align 4
  %uint_to_float155 = uitofp i64 %wide_digit154 to double
  store double %uint_to_float155, ptr %digit_value156, align 8
  %mant157 = load double, ptr %mant, align 8
  %ten_value158 = load double, ptr %ten_value, align 8
  %mul159 = fmul double %mant157, %ten_value158
  %digit_value160 = load double, ptr %digit_value156, align 8
  %add161 = fadd double %mul159, %digit_value160
  store double %add161, ptr %assignment_value162, align 8
  %assignment_value163 = load double, ptr %assignment_value162, align 8
  store double %assignment_value163, ptr %mant, align 8
  %mant164 = load double, ptr %mant, align 8
  %big_limit165 = load double, ptr %big_limit, align 8
  %gt166 = fcmp ogt double %mant164, %big_limit165
  br i1 %gt166, label %if.then167, label %if.merge168

if.else146:                                       ; preds = %while.body.4
  %underscore198 = load i1, ptr %underscore143, align 1
  br i1 %underscore198, label %if.then199, label %if.else200

if.merge147:                                      ; preds = %if.merge201, %if.merge183
  br label %while.cond.3

if.then167:                                       ; preds = %if.then145
  %mant169 = load double, ptr %mant, align 8
  %ten_value170 = load double, ptr %ten_value, align 8
  %div171 = fdiv double %mant169, %ten_value170
  store double %div171, ptr %assignment_value172, align 8
  %assignment_value173 = load double, ptr %assignment_value172, align 8
  store double %assignment_value173, ptr %mant, align 8
  %extra_scale174 = load i32, ptr %extra_scale, align 4
  %one_count175 = load i32, ptr %one_count, align 4
  %add176 = add i32 %extra_scale174, %one_count175
  store i32 %add176, ptr %assignment_value177, align 4
  %assignment_value178 = load i32, ptr %assignment_value177, align 4
  store i32 %assignment_value178, ptr %extra_scale, align 4
  br label %if.merge168

if.merge168:                                      ; preds = %if.then167, %if.then145
  %frac_count179 = load i32, ptr %frac_count, align 4
  %cap180 = load i32, ptr %cap, align 4
  %lt181 = icmp slt i32 %frac_count179, %cap180
  br i1 %lt181, label %if.then182, label %if.merge183

if.then182:                                       ; preds = %if.merge168
  %frac_count184 = load i32, ptr %frac_count, align 4
  %one_count185 = load i32, ptr %one_count, align 4
  %add186 = add i32 %frac_count184, %one_count185
  store i32 %add186, ptr %assignment_value187, align 4
  %assignment_value188 = load i32, ptr %assignment_value187, align 4
  store i32 %assignment_value188, ptr %frac_count, align 4
  br label %if.merge183

if.merge183:                                      ; preds = %if.then182, %if.merge168
  store i1 true, ptr %assignment_value189, align 1
  %assignment_value190 = load i1, ptr %assignment_value189, align 1
  store i1 %assignment_value190, ptr %frac_started, align 1
  store i1 false, ptr %assignment_value191, align 1
  %assignment_value192 = load i1, ptr %assignment_value191, align 1
  store i1 %assignment_value192, ptr %frac_trail, align 1
  %pos193 = load i64, ptr %pos, align 4
  %one194 = load i64, ptr %one, align 4
  %add195 = add i64 %pos193, %one194
  store i64 %add195, ptr %assignment_value196, align 4
  %assignment_value197 = load i64, ptr %assignment_value196, align 4
  store i64 %assignment_value197, ptr %pos, align 4
  br label %if.merge147

if.then199:                                       ; preds = %if.else146
  %frac_started202 = load i1, ptr %frac_started, align 1
  %not203 = xor i1 %frac_started202, true
  br i1 %not203, label %if.then204, label %if.merge205

if.else200:                                       ; preds = %if.else146
  store i1 true, ptr %assignment_value220, align 1
  %assignment_value221 = load i1, ptr %assignment_value220, align 1
  store i1 %assignment_value221, ptr %frac_done, align 1
  br label %if.merge201

if.merge201:                                      ; preds = %if.else200, %if.merge210
  br label %if.merge147

if.then204:                                       ; preds = %if.then199
  store i1 false, ptr %assignment_value206, align 1
  %assignment_value207 = load i1, ptr %assignment_value206, align 1
  store i1 %assignment_value207, ptr %valid, align 1
  br label %if.merge205

if.merge205:                                      ; preds = %if.then204, %if.then199
  %frac_trail208 = load i1, ptr %frac_trail, align 1
  br i1 %frac_trail208, label %if.then209, label %if.merge210

if.then209:                                       ; preds = %if.merge205
  store i1 false, ptr %assignment_value211, align 1
  %assignment_value212 = load i1, ptr %assignment_value211, align 1
  store i1 %assignment_value212, ptr %valid, align 1
  br label %if.merge210

if.merge210:                                      ; preds = %if.then209, %if.merge205
  store i1 true, ptr %assignment_value213, align 1
  %assignment_value214 = load i1, ptr %assignment_value213, align 1
  store i1 %assignment_value214, ptr %frac_trail, align 1
  %pos215 = load i64, ptr %pos, align 4
  %one216 = load i64, ptr %one, align 4
  %add217 = add i64 %pos215, %one216
  store i64 %add217, ptr %assignment_value218, align 4
  %assignment_value219 = load i64, ptr %assignment_value218, align 4
  store i64 %assignment_value219, ptr %pos, align 4
  br label %if.merge201

if.then223:                                       ; preds = %while.exit.5
  store i1 false, ptr %assignment_value225, align 1
  %assignment_value226 = load i1, ptr %assignment_value225, align 1
  store i1 %assignment_value226, ptr %valid, align 1
  br label %if.merge224

if.merge224:                                      ; preds = %if.then223, %while.exit.5
  br label %if.merge121

short_circuit.rhs228:                             ; preds = %if.merge121
  %pos230 = load i64, ptr %pos, align 4
  %end231 = load i64, ptr %end3, align 4
  %lt232 = icmp ult i64 %pos230, %end231
  br label %short_circuit.merge229

short_circuit.merge229:                           ; preds = %short_circuit.rhs228, %if.merge121
  %short_circuit233 = phi i1 [ false, %if.merge121 ], [ %lt232, %short_circuit.rhs228 ]
  br i1 %short_circuit233, label %if.then234, label %if.merge235

if.then234:                                       ; preds = %short_circuit.merge229
  %data236 = load i32, ptr %data1, align 4
  %pos237 = load i64, ptr %pos, align 4
  %call238 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f666c6f61745f62797465(i32 %data236, i64 %pos237)
  store i8 %call238, ptr %byte239, align 1
  %byte240 = load i8, ptr %byte239, align 1
  %call241 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f65(i8 %byte240)
  br i1 %call241, label %if.then242, label %if.merge243

if.merge235:                                      ; preds = %if.merge243, %short_circuit.merge229
  %valid251 = load i1, ptr %valid, align 1
  br i1 %valid251, label %short_circuit.rhs252, label %short_circuit.merge253

if.then242:                                       ; preds = %if.then234
  store i1 true, ptr %assignment_value244, align 1
  %assignment_value245 = load i1, ptr %assignment_value244, align 1
  store i1 %assignment_value245, ptr %has_exp, align 1
  %pos246 = load i64, ptr %pos, align 4
  %one247 = load i64, ptr %one, align 4
  %add248 = add i64 %pos246, %one247
  store i64 %add248, ptr %assignment_value249, align 4
  %assignment_value250 = load i64, ptr %assignment_value249, align 4
  store i64 %assignment_value250, ptr %pos, align 4
  br label %if.merge243

if.merge243:                                      ; preds = %if.then242, %if.then234
  br label %if.merge235

short_circuit.rhs252:                             ; preds = %if.merge235
  %has_exp254 = load i1, ptr %has_exp, align 1
  br label %short_circuit.merge253

short_circuit.merge253:                           ; preds = %short_circuit.rhs252, %if.merge235
  %short_circuit255 = phi i1 [ false, %if.merge235 ], [ %has_exp254, %short_circuit.rhs252 ]
  br i1 %short_circuit255, label %if.then256, label %if.merge257

if.then256:                                       ; preds = %short_circuit.merge253
  %pos258 = load i64, ptr %pos, align 4
  %end259 = load i64, ptr %end3, align 4
  %lt260 = icmp ult i64 %pos258, %end259
  br i1 %lt260, label %if.then261, label %if.merge262

if.merge257:                                      ; preds = %if.merge372, %short_circuit.merge253
  %mant375 = load double, ptr %mant, align 8
  store double %mant375, ptr %scaled, align 8
  %valid376 = load i1, ptr %valid, align 1
  br i1 %valid376, label %if.then377, label %if.merge378

if.then261:                                       ; preds = %if.then256
  %data263 = load i32, ptr %data1, align 4
  %pos264 = load i64, ptr %pos, align 4
  %call265 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f666c6f61745f62797465(i32 %data263, i64 %pos264)
  store i8 %call265, ptr %byte266, align 1
  %byte267 = load i8, ptr %byte266, align 1
  %call268 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f7369676e(i8 %byte267)
  br i1 %call268, label %if.then269, label %if.else270

if.merge262:                                      ; preds = %if.merge271, %if.then256
  store i1 false, ptr %exp_started, align 1
  store i1 false, ptr %exp_trail, align 1
  store i1 false, ptr %exp_done, align 1
  br label %while.cond.6

if.then269:                                       ; preds = %if.then261
  store i1 true, ptr %assignment_value272, align 1
  %assignment_value273 = load i1, ptr %assignment_value272, align 1
  store i1 %assignment_value273, ptr %exp_negative, align 1
  %pos274 = load i64, ptr %pos, align 4
  %one275 = load i64, ptr %one, align 4
  %add276 = add i64 %pos274, %one275
  store i64 %add276, ptr %assignment_value277, align 4
  %assignment_value278 = load i64, ptr %assignment_value277, align 4
  store i64 %assignment_value278, ptr %pos, align 4
  br label %if.merge271

if.else270:                                       ; preds = %if.then261
  %byte279 = load i8, ptr %byte266, align 1
  %call280 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f706c7573(i8 %byte279)
  br i1 %call280, label %if.then281, label %if.merge282

if.merge271:                                      ; preds = %if.merge282, %if.then269
  br label %if.merge262

if.then281:                                       ; preds = %if.else270
  %pos283 = load i64, ptr %pos, align 4
  %one284 = load i64, ptr %one, align 4
  %add285 = add i64 %pos283, %one284
  store i64 %add285, ptr %assignment_value286, align 4
  %assignment_value287 = load i64, ptr %assignment_value286, align 4
  store i64 %assignment_value287, ptr %pos, align 4
  br label %if.merge282

if.merge282:                                      ; preds = %if.then281, %if.else270
  br label %if.merge271

while.cond.6:                                     ; preds = %if.merge313, %if.merge262
  %pos288 = load i64, ptr %pos, align 4
  %end289 = load i64, ptr %end3, align 4
  %lt290 = icmp ult i64 %pos288, %end289
  br i1 %lt290, label %short_circuit.rhs291, label %short_circuit.merge292

while.body.7:                                     ; preds = %short_circuit.merge296
  %data300 = load i32, ptr %data1, align 4
  %pos301 = load i64, ptr %pos, align 4
  %call302 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f666c6f61745f62797465(i32 %data300, i64 %pos301)
  store i8 %call302, ptr %byte303, align 1
  %byte304 = load i8, ptr %byte303, align 1
  %call305 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f64696769743130(i8 %byte304)
  store i1 %call305, ptr %digit306, align 1
  %byte307 = load i8, ptr %byte303, align 1
  %call308 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f756e64657273636f7265(i8 %byte307)
  store i1 %call308, ptr %underscore309, align 1
  %digit310 = load i1, ptr %digit306, align 1
  br i1 %digit310, label %if.then311, label %if.else312

while.exit.8:                                     ; preds = %short_circuit.merge296
  %exp_started364 = load i1, ptr %exp_started, align 1
  %not365 = xor i1 %exp_started364, true
  br i1 %not365, label %if.then366, label %if.merge367

short_circuit.rhs291:                             ; preds = %while.cond.6
  %valid293 = load i1, ptr %valid, align 1
  br label %short_circuit.merge292

short_circuit.merge292:                           ; preds = %short_circuit.rhs291, %while.cond.6
  %short_circuit294 = phi i1 [ false, %while.cond.6 ], [ %valid293, %short_circuit.rhs291 ]
  br i1 %short_circuit294, label %short_circuit.rhs295, label %short_circuit.merge296

short_circuit.rhs295:                             ; preds = %short_circuit.merge292
  %exp_done297 = load i1, ptr %exp_done, align 1
  %not298 = xor i1 %exp_done297, true
  br label %short_circuit.merge296

short_circuit.merge296:                           ; preds = %short_circuit.rhs295, %short_circuit.merge292
  %short_circuit299 = phi i1 [ false, %short_circuit.merge292 ], [ %not298, %short_circuit.rhs295 ]
  br i1 %short_circuit299, label %while.body.7, label %while.exit.8

if.then311:                                       ; preds = %while.body.7
  %byte314 = load i8, ptr %byte303, align 1
  %zero_digit315 = load i8, ptr %zero_digit, align 1
  %sub316 = sub i8 %byte314, %zero_digit315
  store i8 %sub316, ptr %small, align 1
  %small317 = load i8, ptr %small, align 1
  %int_extend318 = zext i8 %small317 to i32
  store i32 %int_extend318, ptr %piece, align 4
  %exp_value319 = load i32, ptr %exp_value, align 4
  %cap320 = load i32, ptr %cap, align 4
  %lt321 = icmp slt i32 %exp_value319, %cap320
  br i1 %lt321, label %if.then322, label %if.merge323

if.else312:                                       ; preds = %while.body.7
  %underscore340 = load i1, ptr %underscore309, align 1
  br i1 %underscore340, label %if.then341, label %if.else342

if.merge313:                                      ; preds = %if.merge343, %if.merge323
  br label %while.cond.6

if.then322:                                       ; preds = %if.then311
  %exp_value324 = load i32, ptr %exp_value, align 4
  %ten_count325 = load i32, ptr %ten_count, align 4
  %mul326 = mul i32 %exp_value324, %ten_count325
  %piece327 = load i32, ptr %piece, align 4
  %add328 = add i32 %mul326, %piece327
  store i32 %add328, ptr %assignment_value329, align 4
  %assignment_value330 = load i32, ptr %assignment_value329, align 4
  store i32 %assignment_value330, ptr %exp_value, align 4
  br label %if.merge323

if.merge323:                                      ; preds = %if.then322, %if.then311
  store i1 true, ptr %assignment_value331, align 1
  %assignment_value332 = load i1, ptr %assignment_value331, align 1
  store i1 %assignment_value332, ptr %exp_started, align 1
  store i1 false, ptr %assignment_value333, align 1
  %assignment_value334 = load i1, ptr %assignment_value333, align 1
  store i1 %assignment_value334, ptr %exp_trail, align 1
  %pos335 = load i64, ptr %pos, align 4
  %one336 = load i64, ptr %one, align 4
  %add337 = add i64 %pos335, %one336
  store i64 %add337, ptr %assignment_value338, align 4
  %assignment_value339 = load i64, ptr %assignment_value338, align 4
  store i64 %assignment_value339, ptr %pos, align 4
  br label %if.merge313

if.then341:                                       ; preds = %if.else312
  %exp_started344 = load i1, ptr %exp_started, align 1
  %not345 = xor i1 %exp_started344, true
  br i1 %not345, label %if.then346, label %if.merge347

if.else342:                                       ; preds = %if.else312
  store i1 true, ptr %assignment_value362, align 1
  %assignment_value363 = load i1, ptr %assignment_value362, align 1
  store i1 %assignment_value363, ptr %exp_done, align 1
  br label %if.merge343

if.merge343:                                      ; preds = %if.else342, %if.merge352
  br label %if.merge313

if.then346:                                       ; preds = %if.then341
  store i1 false, ptr %assignment_value348, align 1
  %assignment_value349 = load i1, ptr %assignment_value348, align 1
  store i1 %assignment_value349, ptr %valid, align 1
  br label %if.merge347

if.merge347:                                      ; preds = %if.then346, %if.then341
  %exp_trail350 = load i1, ptr %exp_trail, align 1
  br i1 %exp_trail350, label %if.then351, label %if.merge352

if.then351:                                       ; preds = %if.merge347
  store i1 false, ptr %assignment_value353, align 1
  %assignment_value354 = load i1, ptr %assignment_value353, align 1
  store i1 %assignment_value354, ptr %valid, align 1
  br label %if.merge352

if.merge352:                                      ; preds = %if.then351, %if.merge347
  store i1 true, ptr %assignment_value355, align 1
  %assignment_value356 = load i1, ptr %assignment_value355, align 1
  store i1 %assignment_value356, ptr %exp_trail, align 1
  %pos357 = load i64, ptr %pos, align 4
  %one358 = load i64, ptr %one, align 4
  %add359 = add i64 %pos357, %one358
  store i64 %add359, ptr %assignment_value360, align 4
  %assignment_value361 = load i64, ptr %assignment_value360, align 4
  store i64 %assignment_value361, ptr %pos, align 4
  br label %if.merge343

if.then366:                                       ; preds = %while.exit.8
  store i1 false, ptr %assignment_value368, align 1
  %assignment_value369 = load i1, ptr %assignment_value368, align 1
  store i1 %assignment_value369, ptr %valid, align 1
  br label %if.merge367

if.merge367:                                      ; preds = %if.then366, %while.exit.8
  %exp_trail370 = load i1, ptr %exp_trail, align 1
  br i1 %exp_trail370, label %if.then371, label %if.merge372

if.then371:                                       ; preds = %if.merge367
  store i1 false, ptr %assignment_value373, align 1
  %assignment_value374 = load i1, ptr %assignment_value373, align 1
  store i1 %assignment_value374, ptr %valid, align 1
  br label %if.merge372

if.merge372:                                      ; preds = %if.then371, %if.merge367
  br label %if.merge257

if.then377:                                       ; preds = %if.merge257
  %mant379 = load double, ptr %mant, align 8
  %zero_value380 = load double, ptr %zero_value, align 8
  %eq = fcmp oeq double %mant379, %zero_value380
  br i1 %eq, label %if.then381, label %if.else382

if.merge378:                                      ; preds = %if.merge383, %if.merge257
  %valid461 = load i1, ptr %valid, align 1
  br i1 %valid461, label %short_circuit.rhs462, label %short_circuit.merge463

if.then381:                                       ; preds = %if.then377
  %zero_value384 = load double, ptr %zero_value, align 8
  store double %zero_value384, ptr %assignment_value385, align 8
  %assignment_value386 = load double, ptr %assignment_value385, align 8
  store double %assignment_value386, ptr %scaled, align 8
  br label %if.merge383

if.else382:                                       ; preds = %if.then377
  %exp_value387 = load i32, ptr %exp_value, align 4
  store i32 %exp_value387, ptr %exp_signed, align 4
  %exp_negative388 = load i1, ptr %exp_negative, align 1
  br i1 %exp_negative388, label %if.then389, label %if.merge390

if.merge383:                                      ; preds = %if.merge406, %if.then381
  br label %if.merge378

if.then389:                                       ; preds = %if.else382
  %zero_count391 = load i32, ptr %zero_count, align 4
  %exp_value392 = load i32, ptr %exp_value, align 4
  %sub393 = sub i32 %zero_count391, %exp_value392
  store i32 %sub393, ptr %assignment_value394, align 4
  %assignment_value395 = load i32, ptr %assignment_value394, align 4
  store i32 %assignment_value395, ptr %exp_signed, align 4
  br label %if.merge390

if.merge390:                                      ; preds = %if.then389, %if.else382
  %exp_signed396 = load i32, ptr %exp_signed, align 4
  %frac_count397 = load i32, ptr %frac_count, align 4
  %sub398 = sub i32 %exp_signed396, %frac_count397
  %extra_scale399 = load i32, ptr %extra_scale, align 4
  %add400 = add i32 %sub398, %extra_scale399
  store i32 %add400, ptr %adjusted, align 4
  %adjusted401 = load i32, ptr %adjusted, align 4
  %limit402 = load i32, ptr %limit, align 4
  %gt403 = icmp sgt i32 %adjusted401, %limit402
  br i1 %gt403, label %if.then404, label %if.else405

if.then404:                                       ; preds = %if.merge390
  store i1 false, ptr %assignment_value407, align 1
  %assignment_value408 = load i1, ptr %assignment_value407, align 1
  store i1 %assignment_value408, ptr %valid, align 1
  br label %if.merge406

if.else405:                                       ; preds = %if.merge390
  %adjusted409 = load i32, ptr %adjusted, align 4
  %neg_limit410 = load i32, ptr %neg_limit, align 4
  %lt411 = icmp slt i32 %adjusted409, %neg_limit410
  br i1 %lt411, label %if.then412, label %if.else413

if.merge406:                                      ; preds = %if.merge414, %if.then404
  br label %if.merge383

if.then412:                                       ; preds = %if.else405
  %zero_value415 = load double, ptr %zero_value, align 8
  store double %zero_value415, ptr %assignment_value416, align 8
  %assignment_value417 = load double, ptr %assignment_value416, align 8
  store double %assignment_value417, ptr %scaled, align 8
  br label %if.merge414

if.else413:                                       ; preds = %if.else405
  %adjusted418 = load i32, ptr %adjusted, align 4
  store i32 %adjusted418, ptr %steps, align 4
  %steps419 = load i32, ptr %steps, align 4
  %zero_count420 = load i32, ptr %zero_count, align 4
  %lt421 = icmp slt i32 %steps419, %zero_count420
  br i1 %lt421, label %if.then422, label %if.merge423

if.merge414:                                      ; preds = %if.merge434, %if.then412
  br label %if.merge406

if.then422:                                       ; preds = %if.else413
  %zero_count424 = load i32, ptr %zero_count, align 4
  %steps425 = load i32, ptr %steps, align 4
  %sub426 = sub i32 %zero_count424, %steps425
  store i32 %sub426, ptr %assignment_value427, align 4
  %assignment_value428 = load i32, ptr %assignment_value427, align 4
  store i32 %assignment_value428, ptr %steps, align 4
  br label %if.merge423

if.merge423:                                      ; preds = %if.then422, %if.else413
  %adjusted429 = load i32, ptr %adjusted, align 4
  %zero_count430 = load i32, ptr %zero_count, align 4
  %gt431 = icmp sgt i32 %adjusted429, %zero_count430
  br i1 %gt431, label %if.then432, label %if.else433

if.then432:                                       ; preds = %if.merge423
  br label %while.cond.9

if.else433:                                       ; preds = %if.merge423
  br label %while.cond.12

if.merge434:                                      ; preds = %while.exit.14, %while.exit.11
  br label %if.merge414

while.cond.9:                                     ; preds = %while.body.10, %if.then432
  %steps435 = load i32, ptr %steps, align 4
  %zero_count436 = load i32, ptr %zero_count, align 4
  %gt437 = icmp sgt i32 %steps435, %zero_count436
  br i1 %gt437, label %while.body.10, label %while.exit.11

while.body.10:                                    ; preds = %while.cond.9
  %scaled438 = load double, ptr %scaled, align 8
  %ten_value439 = load double, ptr %ten_value, align 8
  %mul440 = fmul double %scaled438, %ten_value439
  store double %mul440, ptr %assignment_value441, align 8
  %assignment_value442 = load double, ptr %assignment_value441, align 8
  store double %assignment_value442, ptr %scaled, align 8
  %steps443 = load i32, ptr %steps, align 4
  %one_count444 = load i32, ptr %one_count, align 4
  %sub445 = sub i32 %steps443, %one_count444
  store i32 %sub445, ptr %assignment_value446, align 4
  %assignment_value447 = load i32, ptr %assignment_value446, align 4
  store i32 %assignment_value447, ptr %steps, align 4
  br label %while.cond.9

while.exit.11:                                    ; preds = %while.cond.9
  br label %if.merge434

while.cond.12:                                    ; preds = %while.body.13, %if.else433
  %steps448 = load i32, ptr %steps, align 4
  %zero_count449 = load i32, ptr %zero_count, align 4
  %gt450 = icmp sgt i32 %steps448, %zero_count449
  br i1 %gt450, label %while.body.13, label %while.exit.14

while.body.13:                                    ; preds = %while.cond.12
  %scaled451 = load double, ptr %scaled, align 8
  %ten_value452 = load double, ptr %ten_value, align 8
  %div453 = fdiv double %scaled451, %ten_value452
  store double %div453, ptr %assignment_value454, align 8
  %assignment_value455 = load double, ptr %assignment_value454, align 8
  store double %assignment_value455, ptr %scaled, align 8
  %steps456 = load i32, ptr %steps, align 4
  %one_count457 = load i32, ptr %one_count, align 4
  %sub458 = sub i32 %steps456, %one_count457
  store i32 %sub458, ptr %assignment_value459, align 4
  %assignment_value460 = load i32, ptr %assignment_value459, align 4
  store i32 %assignment_value460, ptr %steps, align 4
  br label %while.cond.12

while.exit.14:                                    ; preds = %while.cond.12
  br label %if.merge434

short_circuit.rhs462:                             ; preds = %if.merge378
  %scaled464 = load double, ptr %scaled, align 8
  %f64_max465 = load double, ptr %f64_max, align 8
  %gt466 = fcmp ogt double %scaled464, %f64_max465
  br label %short_circuit.merge463

short_circuit.merge463:                           ; preds = %short_circuit.rhs462, %if.merge378
  %short_circuit467 = phi i1 [ false, %if.merge378 ], [ %gt466, %short_circuit.rhs462 ]
  br i1 %short_circuit467, label %if.then468, label %if.merge469

if.then468:                                       ; preds = %short_circuit.merge463
  store i1 false, ptr %assignment_value470, align 1
  %assignment_value471 = load i1, ptr %assignment_value470, align 1
  store i1 %assignment_value471, ptr %valid, align 1
  br label %if.merge469

if.merge469:                                      ; preds = %if.then468, %short_circuit.merge463
  %valid472 = load i1, ptr %valid, align 1
  br i1 %valid472, label %short_circuit.rhs473, label %short_circuit.merge474

short_circuit.rhs473:                             ; preds = %if.merge469
  %pos475 = load i64, ptr %pos, align 4
  %end476 = load i64, ptr %end3, align 4
  %ne = icmp ne i64 %pos475, %end476
  br label %short_circuit.merge474

short_circuit.merge474:                           ; preds = %short_circuit.rhs473, %if.merge469
  %short_circuit477 = phi i1 [ false, %if.merge469 ], [ %ne, %short_circuit.rhs473 ]
  br i1 %short_circuit477, label %if.then478, label %if.merge479

if.then478:                                       ; preds = %short_circuit.merge474
  store i1 false, ptr %assignment_value480, align 1
  %assignment_value481 = load i1, ptr %assignment_value480, align 1
  store i1 %assignment_value481, ptr %valid, align 1
  br label %if.merge479

if.merge479:                                      ; preds = %if.then478, %short_circuit.merge474
  %scaled482 = load double, ptr %scaled, align 8
  %valid483 = load i1, ptr %valid, align 1
  %output = insertvalue { double, i1 } zeroinitializer, double %scaled482, 0
  %output484 = insertvalue { double, i1 } %output, i1 %valid483, 1
  ret { double, i1 } %output484
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f663332(ptr %text) {
entry:
  %assignment_value54 = alloca i32, align 4
  %out = alloca float, align 4
  %assignment_value51 = alloca float, align 4
  %neg_one = alloca float, align 4
  %one_single = alloca float, align 4
  %zero_single = alloca float, align 4
  %value = alloca float, align 4
  %fits = alloca i1, align 1
  %overflowed = alloca i1, align 1
  %back = alloca double, align 8
  %narrowed = alloca float, align 4
  %valid = alloca i1, align 1
  %magnitude = alloca double, align 8
  %assignment_value19 = alloca i64, align 8
  %assignment_value16 = alloca i1, align 1
  %negative = alloca i1, align 1
  %pos = alloca i64, align 8
  %rejected = alloca i1, align 1
  %first = alloca i8, align 1
  %assignment_value = alloca i32, align 4
  %result = alloca i32, align 4
  %f64_max = alloca double, align 8
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store double 0x7FEFFFFFFFFFFFFF, ptr %f64_max, align 8
  store i32 0, ptr %result, align 4
  %data = getelementptr inbounds i8, ptr %text1, i8 0
  %place = load i32, ptr %data, align 4
  %eq = icmp eq i32 %place, 0
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %length = getelementptr inbounds i8, ptr %text1, i8 8
  %place2 = load i64, ptr %length, align 4
  %zero3 = load i64, ptr %zero, align 4
  %eq4 = icmp eq i64 %place2, %zero3
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %eq4, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  store i32 0, ptr %assignment_value, align 4
  %assignment_value5 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value5, ptr %result, align 4
  br label %if.merge

if.else:                                          ; preds = %short_circuit.merge
  %data6 = getelementptr inbounds i8, ptr %text1, i8 0
  %place7 = load i32, ptr %data6, align 4
  %zero8 = load i64, ptr %zero, align 4
  %call = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f666c6f61745f62797465(i32 %place7, i64 %zero8)
  store i8 %call, ptr %first, align 1
  %first9 = load i8, ptr %first, align 1
  %call10 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f706c7573(i8 %first9)
  store i1 %call10, ptr %rejected, align 1
  %zero11 = load i64, ptr %zero, align 4
  store i64 %zero11, ptr %pos, align 4
  store i1 false, ptr %negative, align 1
  %first12 = load i8, ptr %first, align 1
  %call13 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f7369676e(i8 %first12)
  br i1 %call13, label %if.then14, label %if.merge15

if.merge:                                         ; preds = %if.merge23, %if.then
  %result56 = load i32, ptr %result, align 4
  %return_result_is_null = icmp eq i32 %result56, 0
  br i1 %return_result_is_null, label %return_result_null_0, label %return_result_copy_1

if.then14:                                        ; preds = %if.else
  store i1 true, ptr %assignment_value16, align 1
  %assignment_value17 = load i1, ptr %assignment_value16, align 1
  store i1 %assignment_value17, ptr %negative, align 1
  %one18 = load i64, ptr %one, align 4
  store i64 %one18, ptr %assignment_value19, align 4
  %assignment_value20 = load i64, ptr %assignment_value19, align 4
  store i64 %assignment_value20, ptr %pos, align 4
  br label %if.merge15

if.merge15:                                       ; preds = %if.then14, %if.else
  %rejected21 = load i1, ptr %rejected, align 1
  %not = xor i1 %rejected21, true
  br i1 %not, label %if.then22, label %if.merge23

if.then22:                                        ; preds = %if.merge15
  %data24 = getelementptr inbounds i8, ptr %text1, i8 0
  %place25 = load i32, ptr %data24, align 4
  %pos26 = load i64, ptr %pos, align 4
  %length27 = getelementptr inbounds i8, ptr %text1, i8 8
  %place28 = load i64, ptr %length27, align 4
  %call29 = call { double, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f666c6f61745f663634(i32 %place25, i64 %pos26, i64 %place28)
  %output = extractvalue { double, i1 } %call29, 0
  store double %output, ptr %magnitude, align 8
  %output30 = extractvalue { double, i1 } %call29, 1
  store i1 %output30, ptr %valid, align 1
  %valid31 = load i1, ptr %valid, align 1
  br i1 %valid31, label %if.then32, label %if.merge33

if.merge23:                                       ; preds = %if.merge33, %if.merge15
  br label %if.merge

if.then32:                                        ; preds = %if.then22
  %magnitude34 = load double, ptr %magnitude, align 8
  %float_trunc = fptrunc double %magnitude34 to float
  store float %float_trunc, ptr %narrowed, align 4
  %narrowed35 = load float, ptr %narrowed, align 4
  %float_extend = fpext float %narrowed35 to double
  store double %float_extend, ptr %back, align 8
  %back36 = load double, ptr %back, align 8
  %f64_max37 = load double, ptr %f64_max, align 8
  %gt = fcmp ogt double %back36, %f64_max37
  store i1 %gt, ptr %overflowed, align 1
  %overflowed38 = load i1, ptr %overflowed, align 1
  %not39 = xor i1 %overflowed38, true
  store i1 %not39, ptr %fits, align 1
  %fits40 = load i1, ptr %fits, align 1
  br i1 %fits40, label %if.then41, label %if.merge42

if.merge33:                                       ; preds = %if.merge42, %if.then22
  br label %if.merge23

if.then41:                                        ; preds = %if.then32
  %narrowed43 = load float, ptr %narrowed, align 4
  store float %narrowed43, ptr %value, align 4
  %negative44 = load i1, ptr %negative, align 1
  br i1 %negative44, label %if.then45, label %if.merge46

if.merge42:                                       ; preds = %if.merge46, %if.then32
  br label %if.merge33

if.then45:                                        ; preds = %if.then41
  store float 0.000000e+00, ptr %zero_single, align 4
  store float 1.000000e+00, ptr %one_single, align 4
  %zero_single47 = load float, ptr %zero_single, align 4
  %one_single48 = load float, ptr %one_single, align 4
  %sub = fsub float %zero_single47, %one_single48
  store float %sub, ptr %neg_one, align 4
  %narrowed49 = load float, ptr %narrowed, align 4
  %neg_one50 = load float, ptr %neg_one, align 4
  %mul = fmul float %narrowed49, %neg_one50
  store float %mul, ptr %assignment_value51, align 4
  %assignment_value52 = load float, ptr %assignment_value51, align 4
  store float %assignment_value52, ptr %value, align 4
  br label %if.merge46

if.merge46:                                       ; preds = %if.then45, %if.then41
  %value53 = load float, ptr %value, align 4
  store float %value53, ptr %out, align 4
  %address = ptrtoint ptr %out to i32
  store i32 %address, ptr %assignment_value54, align 4
  %assignment_value55 = load i32, ptr %assignment_value54, align 4
  store i32 %assignment_value55, ptr %result, align 4
  br label %if.merge42

return_result_null_0:                             ; preds = %if.merge
  br label %return_result_merge_2

return_result_copy_1:                             ; preds = %if.merge
  %allocation = call ptr @__wosy_core_alloc(i64 4, i64 4)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

return_result_merge_2:                            ; preds = %allocation_continue, %return_result_null_0
  %return_result = phi i32 [ 0, %return_result_null_0 ], [ %return_result_address, %allocation_continue ]
  ret i32 %return_result

allocation_panic:                                 ; preds = %return_result_copy_1
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %return_result_copy_1
  %return_result_source = inttoptr i32 %result56 to ptr
  %return_result_value = load float, ptr %return_result_source, align 4
  store float %return_result_value, ptr %allocation, align 4
  %return_result_address = ptrtoint ptr %allocation to i32
  br label %return_result_merge_2
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f663634(ptr %text) {
entry:
  %assignment_value45 = alloca i32, align 4
  %out = alloca double, align 8
  %assignment_value42 = alloca double, align 8
  %neg_one = alloca double, align 8
  %value = alloca double, align 8
  %valid = alloca i1, align 1
  %magnitude = alloca double, align 8
  %assignment_value19 = alloca i64, align 8
  %assignment_value16 = alloca i1, align 1
  %negative = alloca i1, align 1
  %pos = alloca i64, align 8
  %rejected = alloca i1, align 1
  %first = alloca i8, align 1
  %assignment_value = alloca i32, align 4
  %result = alloca i32, align 4
  %one_value = alloca double, align 8
  %zero_value = alloca double, align 8
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store double 0.000000e+00, ptr %zero_value, align 8
  store double 1.000000e+00, ptr %one_value, align 8
  store i32 0, ptr %result, align 4
  %data = getelementptr inbounds i8, ptr %text1, i8 0
  %place = load i32, ptr %data, align 4
  %eq = icmp eq i32 %place, 0
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %length = getelementptr inbounds i8, ptr %text1, i8 8
  %place2 = load i64, ptr %length, align 4
  %zero3 = load i64, ptr %zero, align 4
  %eq4 = icmp eq i64 %place2, %zero3
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %eq4, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  store i32 0, ptr %assignment_value, align 4
  %assignment_value5 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value5, ptr %result, align 4
  br label %if.merge

if.else:                                          ; preds = %short_circuit.merge
  %data6 = getelementptr inbounds i8, ptr %text1, i8 0
  %place7 = load i32, ptr %data6, align 4
  %zero8 = load i64, ptr %zero, align 4
  %call = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f666c6f61745f62797465(i32 %place7, i64 %zero8)
  store i8 %call, ptr %first, align 1
  %first9 = load i8, ptr %first, align 1
  %call10 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f706c7573(i8 %first9)
  store i1 %call10, ptr %rejected, align 1
  %zero11 = load i64, ptr %zero, align 4
  store i64 %zero11, ptr %pos, align 4
  store i1 false, ptr %negative, align 1
  %first12 = load i8, ptr %first, align 1
  %call13 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69735f7369676e(i8 %first12)
  br i1 %call13, label %if.then14, label %if.merge15

if.merge:                                         ; preds = %if.merge23, %if.then
  %result47 = load i32, ptr %result, align 4
  %return_result_is_null = icmp eq i32 %result47, 0
  br i1 %return_result_is_null, label %return_result_null_0, label %return_result_copy_1

if.then14:                                        ; preds = %if.else
  store i1 true, ptr %assignment_value16, align 1
  %assignment_value17 = load i1, ptr %assignment_value16, align 1
  store i1 %assignment_value17, ptr %negative, align 1
  %one18 = load i64, ptr %one, align 4
  store i64 %one18, ptr %assignment_value19, align 4
  %assignment_value20 = load i64, ptr %assignment_value19, align 4
  store i64 %assignment_value20, ptr %pos, align 4
  br label %if.merge15

if.merge15:                                       ; preds = %if.then14, %if.else
  %rejected21 = load i1, ptr %rejected, align 1
  %not = xor i1 %rejected21, true
  br i1 %not, label %if.then22, label %if.merge23

if.then22:                                        ; preds = %if.merge15
  %data24 = getelementptr inbounds i8, ptr %text1, i8 0
  %place25 = load i32, ptr %data24, align 4
  %pos26 = load i64, ptr %pos, align 4
  %length27 = getelementptr inbounds i8, ptr %text1, i8 8
  %place28 = load i64, ptr %length27, align 4
  %call29 = call { double, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f666c6f61745f663634(i32 %place25, i64 %pos26, i64 %place28)
  %output = extractvalue { double, i1 } %call29, 0
  store double %output, ptr %magnitude, align 8
  %output30 = extractvalue { double, i1 } %call29, 1
  store i1 %output30, ptr %valid, align 1
  %valid31 = load i1, ptr %valid, align 1
  br i1 %valid31, label %if.then32, label %if.merge33

if.merge23:                                       ; preds = %if.merge33, %if.merge15
  br label %if.merge

if.then32:                                        ; preds = %if.then22
  %magnitude34 = load double, ptr %magnitude, align 8
  store double %magnitude34, ptr %value, align 8
  %negative35 = load i1, ptr %negative, align 1
  br i1 %negative35, label %if.then36, label %if.merge37

if.merge33:                                       ; preds = %if.merge37, %if.then22
  br label %if.merge23

if.then36:                                        ; preds = %if.then32
  %zero_value38 = load double, ptr %zero_value, align 8
  %one_value39 = load double, ptr %one_value, align 8
  %sub = fsub double %zero_value38, %one_value39
  store double %sub, ptr %neg_one, align 8
  %magnitude40 = load double, ptr %magnitude, align 8
  %neg_one41 = load double, ptr %neg_one, align 8
  %mul = fmul double %magnitude40, %neg_one41
  store double %mul, ptr %assignment_value42, align 8
  %assignment_value43 = load double, ptr %assignment_value42, align 8
  store double %assignment_value43, ptr %value, align 8
  br label %if.merge37

if.merge37:                                       ; preds = %if.then36, %if.then32
  %value44 = load double, ptr %value, align 8
  store double %value44, ptr %out, align 8
  %address = ptrtoint ptr %out to i32
  store i32 %address, ptr %assignment_value45, align 4
  %assignment_value46 = load i32, ptr %assignment_value45, align 4
  store i32 %assignment_value46, ptr %result, align 4
  br label %if.merge33

return_result_null_0:                             ; preds = %if.merge
  br label %return_result_merge_2

return_result_copy_1:                             ; preds = %if.merge
  %allocation = call ptr @__wosy_core_alloc(i64 8, i64 8)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

return_result_merge_2:                            ; preds = %allocation_continue, %return_result_null_0
  %return_result = phi i32 [ 0, %return_result_null_0 ], [ %return_result_address, %allocation_continue ]
  ret i32 %return_result

allocation_panic:                                 ; preds = %return_result_copy_1
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %return_result_copy_1
  %return_result_source = inttoptr i32 %result47 to ptr
  %return_result_value = load double, ptr %return_result_source, align 8
  store double %return_result_value, ptr %allocation, align 8
  %return_result_address = ptrtoint ptr %allocation to i32
  br label %return_result_merge_2
}

define ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f626f6f6c(i1 %value) {
entry:
  %text = alloca [16 x i8], align 1
  %struct_literal = alloca [16 x i8], align 1
  %assignment_value31 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value28 = alloca i64, align 8
  %assignment_value25 = alloca i8, align 1
  %assignment_value22 = alloca i8, align 1
  %assignment_value19 = alloca i8, align 1
  %assignment_value16 = alloca i8, align 1
  %length = alloca i64, align 8
  %assignment_value12 = alloca i8, align 1
  %assignment_value9 = alloca i8, align 1
  %assignment_value6 = alloca i8, align 1
  %assignment_value3 = alloca i8, align 1
  %assignment_value = alloca i8, align 1
  %value1 = alloca i1, align 1
  store i1 %value, ptr %value1, align 1
  %allocation = call ptr @__wosy_core_alloc(i64 5, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  store i8 102, ptr %assignment_value, align 1
  %assignment_value2 = load i8, ptr %assignment_value, align 1
  %array_element = getelementptr inbounds i8, ptr %allocation, i64 0
  store i8 %assignment_value2, ptr %array_element, align 1
  store i8 97, ptr %assignment_value3, align 1
  %assignment_value4 = load i8, ptr %assignment_value3, align 1
  %array_element5 = getelementptr inbounds i8, ptr %allocation, i64 1
  store i8 %assignment_value4, ptr %array_element5, align 1
  store i8 108, ptr %assignment_value6, align 1
  %assignment_value7 = load i8, ptr %assignment_value6, align 1
  %array_element8 = getelementptr inbounds i8, ptr %allocation, i64 2
  store i8 %assignment_value7, ptr %array_element8, align 1
  store i8 115, ptr %assignment_value9, align 1
  %assignment_value10 = load i8, ptr %assignment_value9, align 1
  %array_element11 = getelementptr inbounds i8, ptr %allocation, i64 3
  store i8 %assignment_value10, ptr %array_element11, align 1
  store i8 101, ptr %assignment_value12, align 1
  %assignment_value13 = load i8, ptr %assignment_value12, align 1
  %array_element14 = getelementptr inbounds i8, ptr %allocation, i64 4
  store i8 %assignment_value13, ptr %array_element14, align 1
  store i64 5, ptr %length, align 4
  %value15 = load i1, ptr %value1, align 1
  br i1 %value15, label %if.then, label %if.merge

if.then:                                          ; preds = %allocation_continue
  store i8 116, ptr %assignment_value16, align 1
  %assignment_value17 = load i8, ptr %assignment_value16, align 1
  %array_element18 = getelementptr inbounds i8, ptr %allocation, i64 0
  store i8 %assignment_value17, ptr %array_element18, align 1
  store i8 114, ptr %assignment_value19, align 1
  %assignment_value20 = load i8, ptr %assignment_value19, align 1
  %array_element21 = getelementptr inbounds i8, ptr %allocation, i64 1
  store i8 %assignment_value20, ptr %array_element21, align 1
  store i8 117, ptr %assignment_value22, align 1
  %assignment_value23 = load i8, ptr %assignment_value22, align 1
  %array_element24 = getelementptr inbounds i8, ptr %allocation, i64 2
  store i8 %assignment_value23, ptr %array_element24, align 1
  store i8 101, ptr %assignment_value25, align 1
  %assignment_value26 = load i8, ptr %assignment_value25, align 1
  %array_element27 = getelementptr inbounds i8, ptr %allocation, i64 3
  store i8 %assignment_value26, ptr %array_element27, align 1
  store i64 4, ptr %assignment_value28, align 4
  %assignment_value29 = load i64, ptr %assignment_value28, align 4
  store i64 %assignment_value29, ptr %length, align 4
  br label %if.merge

if.merge:                                         ; preds = %if.then, %allocation_continue
  store i32 0, ptr %data, align 4
  %array_element30 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element30 to i32
  store i32 %address, ptr %assignment_value31, align 4
  %assignment_value32 = load i32, ptr %assignment_value31, align 4
  store i32 %assignment_value32, ptr %data, align 4
  %data33 = load i32, ptr %data, align 4
  %length34 = load i64, ptr %length, align 4
  %data35 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data33, ptr %data35, align 4
  %length36 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length34, ptr %length36, align 4
  %struct_value = load [16 x i8], ptr %struct_literal, align 1
  store [16 x i8] %struct_value, ptr %text, align 1
  ret ptr %text
}

define ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f63686172(i32 %value) {
entry:
  %text = alloca [16 x i8], align 1
  %struct_literal = alloca [16 x i8], align 1
  %assignment_value137 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value132 = alloca i8, align 1
  %assignment_value122 = alloca i8, align 1
  %assignment_value110 = alloca i8, align 1
  %assignment_value99 = alloca i8, align 1
  %assignment_value84 = alloca i8, align 1
  %assignment_value74 = alloca i8, align 1
  %assignment_value63 = alloca i8, align 1
  %assignment_value48 = alloca i8, align 1
  %assignment_value40 = alloca i8, align 1
  %assignment_value29 = alloca i8, align 1
  %assignment_value22 = alloca i64, align 8
  %assignment_value14 = alloca i64, align 8
  %assignment_value = alloca i64, align 8
  %length = alloca i64, align 8
  %shift_eighteen = alloca i32, align 4
  %shift_twelve = alloca i32, align 4
  %shift_six = alloca i32, align 4
  %payload_mask = alloca i32, align 4
  %four_byte_prefix = alloca i32, align 4
  %three_byte_prefix = alloca i32, align 4
  %two_byte_prefix = alloca i32, align 4
  %continuation_prefix = alloca i32, align 4
  %three_byte_limit = alloca i32, align 4
  %two_byte_limit = alloca i32, align 4
  %ascii_limit = alloca i32, align 4
  %four = alloca i64, align 8
  %three = alloca i64, align 8
  %two = alloca i64, align 8
  %one = alloca i64, align 8
  %scalar = alloca i32, align 4
  %value1 = alloca i32, align 4
  store i32 %value, ptr %value1, align 4
  %value2 = load i32, ptr %value1, align 4
  store i32 %value2, ptr %scalar, align 4
  %allocation = call ptr @__wosy_core_alloc(i64 4, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  store i64 1, ptr %one, align 4
  store i64 2, ptr %two, align 4
  store i64 3, ptr %three, align 4
  store i64 4, ptr %four, align 4
  store i32 128, ptr %ascii_limit, align 4
  store i32 2048, ptr %two_byte_limit, align 4
  store i32 65536, ptr %three_byte_limit, align 4
  store i32 128, ptr %continuation_prefix, align 4
  store i32 192, ptr %two_byte_prefix, align 4
  store i32 224, ptr %three_byte_prefix, align 4
  store i32 240, ptr %four_byte_prefix, align 4
  store i32 63, ptr %payload_mask, align 4
  store i32 6, ptr %shift_six, align 4
  store i32 12, ptr %shift_twelve, align 4
  store i32 18, ptr %shift_eighteen, align 4
  %one3 = load i64, ptr %one, align 4
  store i64 %one3, ptr %length, align 4
  %scalar4 = load i32, ptr %scalar, align 4
  %ascii_limit5 = load i32, ptr %ascii_limit, align 4
  %ge = icmp uge i32 %scalar4, %ascii_limit5
  br i1 %ge, label %if.then, label %if.merge

if.then:                                          ; preds = %allocation_continue
  %two6 = load i64, ptr %two, align 4
  store i64 %two6, ptr %assignment_value, align 4
  %assignment_value7 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value7, ptr %length, align 4
  br label %if.merge

if.merge:                                         ; preds = %if.then, %allocation_continue
  %scalar8 = load i32, ptr %scalar, align 4
  %two_byte_limit9 = load i32, ptr %two_byte_limit, align 4
  %ge10 = icmp uge i32 %scalar8, %two_byte_limit9
  br i1 %ge10, label %if.then11, label %if.merge12

if.then11:                                        ; preds = %if.merge
  %three13 = load i64, ptr %three, align 4
  store i64 %three13, ptr %assignment_value14, align 4
  %assignment_value15 = load i64, ptr %assignment_value14, align 4
  store i64 %assignment_value15, ptr %length, align 4
  br label %if.merge12

if.merge12:                                       ; preds = %if.then11, %if.merge
  %scalar16 = load i32, ptr %scalar, align 4
  %three_byte_limit17 = load i32, ptr %three_byte_limit, align 4
  %ge18 = icmp uge i32 %scalar16, %three_byte_limit17
  br i1 %ge18, label %if.then19, label %if.merge20

if.then19:                                        ; preds = %if.merge12
  %four21 = load i64, ptr %four, align 4
  store i64 %four21, ptr %assignment_value22, align 4
  %assignment_value23 = load i64, ptr %assignment_value22, align 4
  store i64 %assignment_value23, ptr %length, align 4
  br label %if.merge20

if.merge20:                                       ; preds = %if.then19, %if.merge12
  %length24 = load i64, ptr %length, align 4
  %one25 = load i64, ptr %one, align 4
  %eq = icmp eq i64 %length24, %one25
  br i1 %eq, label %if.then26, label %if.merge27

if.then26:                                        ; preds = %if.merge20
  %scalar28 = load i32, ptr %scalar, align 4
  %int_trunc = trunc i32 %scalar28 to i8
  store i8 %int_trunc, ptr %assignment_value29, align 1
  %assignment_value30 = load i8, ptr %assignment_value29, align 1
  %array_element = getelementptr inbounds i8, ptr %allocation, i64 0
  store i8 %assignment_value30, ptr %array_element, align 1
  br label %if.merge27

if.merge27:                                       ; preds = %if.then26, %if.merge20
  %length31 = load i64, ptr %length, align 4
  %two32 = load i64, ptr %two, align 4
  %eq33 = icmp eq i64 %length31, %two32
  br i1 %eq33, label %if.then34, label %if.merge35

if.then34:                                        ; preds = %if.merge27
  %two_byte_prefix36 = load i32, ptr %two_byte_prefix, align 4
  %scalar37 = load i32, ptr %scalar, align 4
  %shift_six38 = load i32, ptr %shift_six, align 4
  %shr = lshr i32 %scalar37, %shift_six38
  %or = or i32 %two_byte_prefix36, %shr
  %int_trunc39 = trunc i32 %or to i8
  store i8 %int_trunc39, ptr %assignment_value40, align 1
  %assignment_value41 = load i8, ptr %assignment_value40, align 1
  %array_element42 = getelementptr inbounds i8, ptr %allocation, i64 0
  store i8 %assignment_value41, ptr %array_element42, align 1
  %continuation_prefix43 = load i32, ptr %continuation_prefix, align 4
  %scalar44 = load i32, ptr %scalar, align 4
  %payload_mask45 = load i32, ptr %payload_mask, align 4
  %and = and i32 %scalar44, %payload_mask45
  %or46 = or i32 %continuation_prefix43, %and
  %int_trunc47 = trunc i32 %or46 to i8
  store i8 %int_trunc47, ptr %assignment_value48, align 1
  %assignment_value49 = load i8, ptr %assignment_value48, align 1
  %one50 = load i64, ptr %one, align 4
  %array_element51 = getelementptr inbounds i8, ptr %allocation, i64 %one50
  store i8 %assignment_value49, ptr %array_element51, align 1
  br label %if.merge35

if.merge35:                                       ; preds = %if.then34, %if.merge27
  %length52 = load i64, ptr %length, align 4
  %three53 = load i64, ptr %three, align 4
  %eq54 = icmp eq i64 %length52, %three53
  br i1 %eq54, label %if.then55, label %if.merge56

if.then55:                                        ; preds = %if.merge35
  %three_byte_prefix57 = load i32, ptr %three_byte_prefix, align 4
  %scalar58 = load i32, ptr %scalar, align 4
  %shift_twelve59 = load i32, ptr %shift_twelve, align 4
  %shr60 = lshr i32 %scalar58, %shift_twelve59
  %or61 = or i32 %three_byte_prefix57, %shr60
  %int_trunc62 = trunc i32 %or61 to i8
  store i8 %int_trunc62, ptr %assignment_value63, align 1
  %assignment_value64 = load i8, ptr %assignment_value63, align 1
  %array_element65 = getelementptr inbounds i8, ptr %allocation, i64 0
  store i8 %assignment_value64, ptr %array_element65, align 1
  %continuation_prefix66 = load i32, ptr %continuation_prefix, align 4
  %scalar67 = load i32, ptr %scalar, align 4
  %shift_six68 = load i32, ptr %shift_six, align 4
  %shr69 = lshr i32 %scalar67, %shift_six68
  %payload_mask70 = load i32, ptr %payload_mask, align 4
  %and71 = and i32 %shr69, %payload_mask70
  %or72 = or i32 %continuation_prefix66, %and71
  %int_trunc73 = trunc i32 %or72 to i8
  store i8 %int_trunc73, ptr %assignment_value74, align 1
  %assignment_value75 = load i8, ptr %assignment_value74, align 1
  %one76 = load i64, ptr %one, align 4
  %array_element77 = getelementptr inbounds i8, ptr %allocation, i64 %one76
  store i8 %assignment_value75, ptr %array_element77, align 1
  %continuation_prefix78 = load i32, ptr %continuation_prefix, align 4
  %scalar79 = load i32, ptr %scalar, align 4
  %payload_mask80 = load i32, ptr %payload_mask, align 4
  %and81 = and i32 %scalar79, %payload_mask80
  %or82 = or i32 %continuation_prefix78, %and81
  %int_trunc83 = trunc i32 %or82 to i8
  store i8 %int_trunc83, ptr %assignment_value84, align 1
  %assignment_value85 = load i8, ptr %assignment_value84, align 1
  %two86 = load i64, ptr %two, align 4
  %array_element87 = getelementptr inbounds i8, ptr %allocation, i64 %two86
  store i8 %assignment_value85, ptr %array_element87, align 1
  br label %if.merge56

if.merge56:                                       ; preds = %if.then55, %if.merge35
  %length88 = load i64, ptr %length, align 4
  %four89 = load i64, ptr %four, align 4
  %eq90 = icmp eq i64 %length88, %four89
  br i1 %eq90, label %if.then91, label %if.merge92

if.then91:                                        ; preds = %if.merge56
  %four_byte_prefix93 = load i32, ptr %four_byte_prefix, align 4
  %scalar94 = load i32, ptr %scalar, align 4
  %shift_eighteen95 = load i32, ptr %shift_eighteen, align 4
  %shr96 = lshr i32 %scalar94, %shift_eighteen95
  %or97 = or i32 %four_byte_prefix93, %shr96
  %int_trunc98 = trunc i32 %or97 to i8
  store i8 %int_trunc98, ptr %assignment_value99, align 1
  %assignment_value100 = load i8, ptr %assignment_value99, align 1
  %array_element101 = getelementptr inbounds i8, ptr %allocation, i64 0
  store i8 %assignment_value100, ptr %array_element101, align 1
  %continuation_prefix102 = load i32, ptr %continuation_prefix, align 4
  %scalar103 = load i32, ptr %scalar, align 4
  %shift_twelve104 = load i32, ptr %shift_twelve, align 4
  %shr105 = lshr i32 %scalar103, %shift_twelve104
  %payload_mask106 = load i32, ptr %payload_mask, align 4
  %and107 = and i32 %shr105, %payload_mask106
  %or108 = or i32 %continuation_prefix102, %and107
  %int_trunc109 = trunc i32 %or108 to i8
  store i8 %int_trunc109, ptr %assignment_value110, align 1
  %assignment_value111 = load i8, ptr %assignment_value110, align 1
  %one112 = load i64, ptr %one, align 4
  %array_element113 = getelementptr inbounds i8, ptr %allocation, i64 %one112
  store i8 %assignment_value111, ptr %array_element113, align 1
  %continuation_prefix114 = load i32, ptr %continuation_prefix, align 4
  %scalar115 = load i32, ptr %scalar, align 4
  %shift_six116 = load i32, ptr %shift_six, align 4
  %shr117 = lshr i32 %scalar115, %shift_six116
  %payload_mask118 = load i32, ptr %payload_mask, align 4
  %and119 = and i32 %shr117, %payload_mask118
  %or120 = or i32 %continuation_prefix114, %and119
  %int_trunc121 = trunc i32 %or120 to i8
  store i8 %int_trunc121, ptr %assignment_value122, align 1
  %assignment_value123 = load i8, ptr %assignment_value122, align 1
  %two124 = load i64, ptr %two, align 4
  %array_element125 = getelementptr inbounds i8, ptr %allocation, i64 %two124
  store i8 %assignment_value123, ptr %array_element125, align 1
  %continuation_prefix126 = load i32, ptr %continuation_prefix, align 4
  %scalar127 = load i32, ptr %scalar, align 4
  %payload_mask128 = load i32, ptr %payload_mask, align 4
  %and129 = and i32 %scalar127, %payload_mask128
  %or130 = or i32 %continuation_prefix126, %and129
  %int_trunc131 = trunc i32 %or130 to i8
  store i8 %int_trunc131, ptr %assignment_value132, align 1
  %assignment_value133 = load i8, ptr %assignment_value132, align 1
  %three134 = load i64, ptr %three, align 4
  %array_element135 = getelementptr inbounds i8, ptr %allocation, i64 %three134
  store i8 %assignment_value133, ptr %array_element135, align 1
  br label %if.merge92

if.merge92:                                       ; preds = %if.then91, %if.merge56
  store i32 0, ptr %data, align 4
  %array_element136 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element136 to i32
  store i32 %address, ptr %assignment_value137, align 4
  %assignment_value138 = load i32, ptr %assignment_value137, align 4
  store i32 %assignment_value138, ptr %data, align 4
  %data139 = load i32, ptr %data, align 4
  %length140 = load i64, ptr %length, align 4
  %data141 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data139, ptr %data141, align 4
  %length142 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length140, ptr %length142, align 4
  %struct_value = load [16 x i8], ptr %struct_literal, align 1
  store [16 x i8] %struct_value, ptr %text, align 1
  ret ptr %text
}

define ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f6938(i8 %value) {
entry:
  %text = alloca [16 x i8], align 1
  %struct_literal = alloca [16 x i8], align 1
  %assignment_value82 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value79 = alloca i64, align 8
  %assignment_value74 = alloca i64, align 8
  %assignment_value67 = alloca i8, align 1
  %source = alloca i64, align 8
  %target = alloca i64, align 8
  %length = alloca i64, align 8
  %assignment_value53 = alloca i8, align 1
  %assignment_value50 = alloca i64, align 8
  %assignment_value42 = alloca i8, align 1
  %assignment_value36 = alloca i8, align 1
  %assignment_value32 = alloca i64, align 8
  %digit = alloca i8, align 1
  %wide_digit = alloca i16, align 2
  %assignment_value25 = alloca i8, align 1
  %digit_signed = alloca i8, align 1
  %assignment_value12 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %negative = alloca i1, align 1
  %remaining = alloca i8, align 1
  %index = alloca i64, align 8
  %minus = alloca i8, align 1
  %zero_digit = alloca i8, align 1
  %ten = alloca i8, align 1
  %zero = alloca i8, align 1
  %zero_count = alloca i64, align 8
  %one = alloca i64, align 8
  %capacity = alloca i64, align 8
  %value1 = alloca i8, align 1
  store i8 %value, ptr %value1, align 1
  %allocation = call ptr @__wosy_core_alloc(i64 4, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  store i64 4, ptr %capacity, align 4
  store i64 1, ptr %one, align 4
  store i64 0, ptr %zero_count, align 4
  store i8 0, ptr %zero, align 1
  store i8 10, ptr %ten, align 1
  store i8 48, ptr %zero_digit, align 1
  store i8 45, ptr %minus, align 1
  %capacity2 = load i64, ptr %capacity, align 4
  store i64 %capacity2, ptr %index, align 4
  %value3 = load i8, ptr %value1, align 1
  store i8 %value3, ptr %remaining, align 1
  %remaining4 = load i8, ptr %remaining, align 1
  %zero5 = load i8, ptr %zero, align 1
  %lt = icmp slt i8 %remaining4, %zero5
  store i1 %lt, ptr %negative, align 1
  %remaining6 = load i8, ptr %remaining, align 1
  %zero7 = load i8, ptr %zero, align 1
  %eq = icmp eq i8 %remaining6, %zero7
  br i1 %eq, label %if.then, label %if.merge

if.then:                                          ; preds = %allocation_continue
  %index8 = load i64, ptr %index, align 4
  %one9 = load i64, ptr %one, align 4
  %sub = sub i64 %index8, %one9
  store i64 %sub, ptr %assignment_value, align 4
  %assignment_value10 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value10, ptr %index, align 4
  %zero_digit11 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit11, ptr %assignment_value12, align 1
  %assignment_value13 = load i8, ptr %assignment_value12, align 1
  %index14 = load i64, ptr %index, align 4
  %array_element = getelementptr inbounds i8, ptr %allocation, i64 %index14
  store i8 %assignment_value13, ptr %array_element, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %allocation_continue
  br label %while.cond.0

while.cond.0:                                     ; preds = %if.merge21, %if.merge
  %remaining15 = load i8, ptr %remaining, align 1
  %zero16 = load i8, ptr %zero, align 1
  %ne = icmp ne i8 %remaining15, %zero16
  br i1 %ne, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %remaining17 = load i8, ptr %remaining, align 1
  %ten18 = load i8, ptr %ten, align 1
  %rem = srem i8 %remaining17, %ten18
  store i8 %rem, ptr %digit_signed, align 1
  %negative19 = load i1, ptr %negative, align 1
  br i1 %negative19, label %if.then20, label %if.merge21

while.exit.2:                                     ; preds = %while.cond.0
  %negative44 = load i1, ptr %negative, align 1
  br i1 %negative44, label %if.then45, label %if.merge46

if.then20:                                        ; preds = %while.body.1
  %zero22 = load i8, ptr %zero, align 1
  %digit_signed23 = load i8, ptr %digit_signed, align 1
  %sub24 = sub i8 %zero22, %digit_signed23
  store i8 %sub24, ptr %assignment_value25, align 1
  %assignment_value26 = load i8, ptr %assignment_value25, align 1
  store i8 %assignment_value26, ptr %digit_signed, align 1
  br label %if.merge21

if.merge21:                                       ; preds = %if.then20, %while.body.1
  %digit_signed27 = load i8, ptr %digit_signed, align 1
  %int_extend = sext i8 %digit_signed27 to i16
  store i16 %int_extend, ptr %wide_digit, align 2
  %wide_digit28 = load i16, ptr %wide_digit, align 2
  %int_trunc = trunc i16 %wide_digit28 to i8
  store i8 %int_trunc, ptr %digit, align 1
  %index29 = load i64, ptr %index, align 4
  %one30 = load i64, ptr %one, align 4
  %sub31 = sub i64 %index29, %one30
  store i64 %sub31, ptr %assignment_value32, align 4
  %assignment_value33 = load i64, ptr %assignment_value32, align 4
  store i64 %assignment_value33, ptr %index, align 4
  %zero_digit34 = load i8, ptr %zero_digit, align 1
  %digit35 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit34, %digit35
  store i8 %add, ptr %assignment_value36, align 1
  %assignment_value37 = load i8, ptr %assignment_value36, align 1
  %index38 = load i64, ptr %index, align 4
  %array_element39 = getelementptr inbounds i8, ptr %allocation, i64 %index38
  store i8 %assignment_value37, ptr %array_element39, align 1
  %remaining40 = load i8, ptr %remaining, align 1
  %ten41 = load i8, ptr %ten, align 1
  %div = sdiv i8 %remaining40, %ten41
  store i8 %div, ptr %assignment_value42, align 1
  %assignment_value43 = load i8, ptr %assignment_value42, align 1
  store i8 %assignment_value43, ptr %remaining, align 1
  br label %while.cond.0

if.then45:                                        ; preds = %while.exit.2
  %index47 = load i64, ptr %index, align 4
  %one48 = load i64, ptr %one, align 4
  %sub49 = sub i64 %index47, %one48
  store i64 %sub49, ptr %assignment_value50, align 4
  %assignment_value51 = load i64, ptr %assignment_value50, align 4
  store i64 %assignment_value51, ptr %index, align 4
  %minus52 = load i8, ptr %minus, align 1
  store i8 %minus52, ptr %assignment_value53, align 1
  %assignment_value54 = load i8, ptr %assignment_value53, align 1
  %index55 = load i64, ptr %index, align 4
  %array_element56 = getelementptr inbounds i8, ptr %allocation, i64 %index55
  store i8 %assignment_value54, ptr %array_element56, align 1
  br label %if.merge46

if.merge46:                                       ; preds = %if.then45, %while.exit.2
  %capacity57 = load i64, ptr %capacity, align 4
  %index58 = load i64, ptr %index, align 4
  %sub59 = sub i64 %capacity57, %index58
  store i64 %sub59, ptr %length, align 4
  %zero_count60 = load i64, ptr %zero_count, align 4
  store i64 %zero_count60, ptr %target, align 4
  %index61 = load i64, ptr %index, align 4
  store i64 %index61, ptr %source, align 4
  br label %while.cond.3

while.cond.3:                                     ; preds = %while.body.4, %if.merge46
  %source62 = load i64, ptr %source, align 4
  %capacity63 = load i64, ptr %capacity, align 4
  %lt64 = icmp ult i64 %source62, %capacity63
  br i1 %lt64, label %while.body.4, label %while.exit.5

while.body.4:                                     ; preds = %while.cond.3
  %source65 = load i64, ptr %source, align 4
  %array_element66 = getelementptr inbounds i8, ptr %allocation, i64 %source65
  %place = load i8, ptr %array_element66, align 1
  store i8 %place, ptr %assignment_value67, align 1
  %assignment_value68 = load i8, ptr %assignment_value67, align 1
  %target69 = load i64, ptr %target, align 4
  %array_element70 = getelementptr inbounds i8, ptr %allocation, i64 %target69
  store i8 %assignment_value68, ptr %array_element70, align 1
  %target71 = load i64, ptr %target, align 4
  %one72 = load i64, ptr %one, align 4
  %add73 = add i64 %target71, %one72
  store i64 %add73, ptr %assignment_value74, align 4
  %assignment_value75 = load i64, ptr %assignment_value74, align 4
  store i64 %assignment_value75, ptr %target, align 4
  %source76 = load i64, ptr %source, align 4
  %one77 = load i64, ptr %one, align 4
  %add78 = add i64 %source76, %one77
  store i64 %add78, ptr %assignment_value79, align 4
  %assignment_value80 = load i64, ptr %assignment_value79, align 4
  store i64 %assignment_value80, ptr %source, align 4
  br label %while.cond.3

while.exit.5:                                     ; preds = %while.cond.3
  store i32 0, ptr %data, align 4
  %array_element81 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element81 to i32
  store i32 %address, ptr %assignment_value82, align 4
  %assignment_value83 = load i32, ptr %assignment_value82, align 4
  store i32 %assignment_value83, ptr %data, align 4
  %data84 = load i32, ptr %data, align 4
  %length85 = load i64, ptr %length, align 4
  %data86 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data84, ptr %data86, align 4
  %length87 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length85, ptr %length87, align 4
  %struct_value = load [16 x i8], ptr %struct_literal, align 1
  store [16 x i8] %struct_value, ptr %text, align 1
  ret ptr %text
}

define ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f693136(i16 %value) {
entry:
  %text = alloca [16 x i8], align 1
  %struct_literal = alloca [16 x i8], align 1
  %assignment_value81 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value78 = alloca i64, align 8
  %assignment_value73 = alloca i64, align 8
  %assignment_value66 = alloca i8, align 1
  %source = alloca i64, align 8
  %target = alloca i64, align 8
  %length = alloca i64, align 8
  %assignment_value52 = alloca i8, align 1
  %assignment_value49 = alloca i64, align 8
  %assignment_value41 = alloca i16, align 2
  %assignment_value35 = alloca i8, align 1
  %assignment_value31 = alloca i64, align 8
  %digit = alloca i8, align 1
  %assignment_value25 = alloca i16, align 2
  %digit_signed = alloca i16, align 2
  %assignment_value12 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %negative = alloca i1, align 1
  %remaining = alloca i16, align 2
  %index = alloca i64, align 8
  %minus = alloca i8, align 1
  %zero_digit = alloca i8, align 1
  %ten = alloca i16, align 2
  %zero = alloca i16, align 2
  %zero_count = alloca i64, align 8
  %one = alloca i64, align 8
  %capacity = alloca i64, align 8
  %value1 = alloca i16, align 2
  store i16 %value, ptr %value1, align 2
  %allocation = call ptr @__wosy_core_alloc(i64 6, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  store i64 6, ptr %capacity, align 4
  store i64 1, ptr %one, align 4
  store i64 0, ptr %zero_count, align 4
  store i16 0, ptr %zero, align 2
  store i16 10, ptr %ten, align 2
  store i8 48, ptr %zero_digit, align 1
  store i8 45, ptr %minus, align 1
  %capacity2 = load i64, ptr %capacity, align 4
  store i64 %capacity2, ptr %index, align 4
  %value3 = load i16, ptr %value1, align 2
  store i16 %value3, ptr %remaining, align 2
  %remaining4 = load i16, ptr %remaining, align 2
  %zero5 = load i16, ptr %zero, align 2
  %lt = icmp slt i16 %remaining4, %zero5
  store i1 %lt, ptr %negative, align 1
  %remaining6 = load i16, ptr %remaining, align 2
  %zero7 = load i16, ptr %zero, align 2
  %eq = icmp eq i16 %remaining6, %zero7
  br i1 %eq, label %if.then, label %if.merge

if.then:                                          ; preds = %allocation_continue
  %index8 = load i64, ptr %index, align 4
  %one9 = load i64, ptr %one, align 4
  %sub = sub i64 %index8, %one9
  store i64 %sub, ptr %assignment_value, align 4
  %assignment_value10 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value10, ptr %index, align 4
  %zero_digit11 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit11, ptr %assignment_value12, align 1
  %assignment_value13 = load i8, ptr %assignment_value12, align 1
  %index14 = load i64, ptr %index, align 4
  %array_element = getelementptr inbounds i8, ptr %allocation, i64 %index14
  store i8 %assignment_value13, ptr %array_element, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %allocation_continue
  br label %while.cond.0

while.cond.0:                                     ; preds = %if.merge21, %if.merge
  %remaining15 = load i16, ptr %remaining, align 2
  %zero16 = load i16, ptr %zero, align 2
  %ne = icmp ne i16 %remaining15, %zero16
  br i1 %ne, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %remaining17 = load i16, ptr %remaining, align 2
  %ten18 = load i16, ptr %ten, align 2
  %rem = srem i16 %remaining17, %ten18
  store i16 %rem, ptr %digit_signed, align 2
  %negative19 = load i1, ptr %negative, align 1
  br i1 %negative19, label %if.then20, label %if.merge21

while.exit.2:                                     ; preds = %while.cond.0
  %negative43 = load i1, ptr %negative, align 1
  br i1 %negative43, label %if.then44, label %if.merge45

if.then20:                                        ; preds = %while.body.1
  %zero22 = load i16, ptr %zero, align 2
  %digit_signed23 = load i16, ptr %digit_signed, align 2
  %sub24 = sub i16 %zero22, %digit_signed23
  store i16 %sub24, ptr %assignment_value25, align 2
  %assignment_value26 = load i16, ptr %assignment_value25, align 2
  store i16 %assignment_value26, ptr %digit_signed, align 2
  br label %if.merge21

if.merge21:                                       ; preds = %if.then20, %while.body.1
  %digit_signed27 = load i16, ptr %digit_signed, align 2
  %int_trunc = trunc i16 %digit_signed27 to i8
  store i8 %int_trunc, ptr %digit, align 1
  %index28 = load i64, ptr %index, align 4
  %one29 = load i64, ptr %one, align 4
  %sub30 = sub i64 %index28, %one29
  store i64 %sub30, ptr %assignment_value31, align 4
  %assignment_value32 = load i64, ptr %assignment_value31, align 4
  store i64 %assignment_value32, ptr %index, align 4
  %zero_digit33 = load i8, ptr %zero_digit, align 1
  %digit34 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit33, %digit34
  store i8 %add, ptr %assignment_value35, align 1
  %assignment_value36 = load i8, ptr %assignment_value35, align 1
  %index37 = load i64, ptr %index, align 4
  %array_element38 = getelementptr inbounds i8, ptr %allocation, i64 %index37
  store i8 %assignment_value36, ptr %array_element38, align 1
  %remaining39 = load i16, ptr %remaining, align 2
  %ten40 = load i16, ptr %ten, align 2
  %div = sdiv i16 %remaining39, %ten40
  store i16 %div, ptr %assignment_value41, align 2
  %assignment_value42 = load i16, ptr %assignment_value41, align 2
  store i16 %assignment_value42, ptr %remaining, align 2
  br label %while.cond.0

if.then44:                                        ; preds = %while.exit.2
  %index46 = load i64, ptr %index, align 4
  %one47 = load i64, ptr %one, align 4
  %sub48 = sub i64 %index46, %one47
  store i64 %sub48, ptr %assignment_value49, align 4
  %assignment_value50 = load i64, ptr %assignment_value49, align 4
  store i64 %assignment_value50, ptr %index, align 4
  %minus51 = load i8, ptr %minus, align 1
  store i8 %minus51, ptr %assignment_value52, align 1
  %assignment_value53 = load i8, ptr %assignment_value52, align 1
  %index54 = load i64, ptr %index, align 4
  %array_element55 = getelementptr inbounds i8, ptr %allocation, i64 %index54
  store i8 %assignment_value53, ptr %array_element55, align 1
  br label %if.merge45

if.merge45:                                       ; preds = %if.then44, %while.exit.2
  %capacity56 = load i64, ptr %capacity, align 4
  %index57 = load i64, ptr %index, align 4
  %sub58 = sub i64 %capacity56, %index57
  store i64 %sub58, ptr %length, align 4
  %zero_count59 = load i64, ptr %zero_count, align 4
  store i64 %zero_count59, ptr %target, align 4
  %index60 = load i64, ptr %index, align 4
  store i64 %index60, ptr %source, align 4
  br label %while.cond.3

while.cond.3:                                     ; preds = %while.body.4, %if.merge45
  %source61 = load i64, ptr %source, align 4
  %capacity62 = load i64, ptr %capacity, align 4
  %lt63 = icmp ult i64 %source61, %capacity62
  br i1 %lt63, label %while.body.4, label %while.exit.5

while.body.4:                                     ; preds = %while.cond.3
  %source64 = load i64, ptr %source, align 4
  %array_element65 = getelementptr inbounds i8, ptr %allocation, i64 %source64
  %place = load i8, ptr %array_element65, align 1
  store i8 %place, ptr %assignment_value66, align 1
  %assignment_value67 = load i8, ptr %assignment_value66, align 1
  %target68 = load i64, ptr %target, align 4
  %array_element69 = getelementptr inbounds i8, ptr %allocation, i64 %target68
  store i8 %assignment_value67, ptr %array_element69, align 1
  %target70 = load i64, ptr %target, align 4
  %one71 = load i64, ptr %one, align 4
  %add72 = add i64 %target70, %one71
  store i64 %add72, ptr %assignment_value73, align 4
  %assignment_value74 = load i64, ptr %assignment_value73, align 4
  store i64 %assignment_value74, ptr %target, align 4
  %source75 = load i64, ptr %source, align 4
  %one76 = load i64, ptr %one, align 4
  %add77 = add i64 %source75, %one76
  store i64 %add77, ptr %assignment_value78, align 4
  %assignment_value79 = load i64, ptr %assignment_value78, align 4
  store i64 %assignment_value79, ptr %source, align 4
  br label %while.cond.3

while.exit.5:                                     ; preds = %while.cond.3
  store i32 0, ptr %data, align 4
  %array_element80 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element80 to i32
  store i32 %address, ptr %assignment_value81, align 4
  %assignment_value82 = load i32, ptr %assignment_value81, align 4
  store i32 %assignment_value82, ptr %data, align 4
  %data83 = load i32, ptr %data, align 4
  %length84 = load i64, ptr %length, align 4
  %data85 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data83, ptr %data85, align 4
  %length86 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length84, ptr %length86, align 4
  %struct_value = load [16 x i8], ptr %struct_literal, align 1
  store [16 x i8] %struct_value, ptr %text, align 1
  ret ptr %text
}

define ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f693332(i32 %value) {
entry:
  %text = alloca [16 x i8], align 1
  %struct_literal = alloca [16 x i8], align 1
  %assignment_value81 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value78 = alloca i64, align 8
  %assignment_value73 = alloca i64, align 8
  %assignment_value66 = alloca i8, align 1
  %source = alloca i64, align 8
  %target = alloca i64, align 8
  %length = alloca i64, align 8
  %assignment_value52 = alloca i8, align 1
  %assignment_value49 = alloca i64, align 8
  %assignment_value41 = alloca i32, align 4
  %assignment_value35 = alloca i8, align 1
  %assignment_value31 = alloca i64, align 8
  %digit = alloca i8, align 1
  %assignment_value25 = alloca i32, align 4
  %digit_signed = alloca i32, align 4
  %assignment_value12 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %negative = alloca i1, align 1
  %remaining = alloca i32, align 4
  %index = alloca i64, align 8
  %minus = alloca i8, align 1
  %zero_digit = alloca i8, align 1
  %ten = alloca i32, align 4
  %zero = alloca i32, align 4
  %zero_count = alloca i64, align 8
  %one = alloca i64, align 8
  %capacity = alloca i64, align 8
  %value1 = alloca i32, align 4
  store i32 %value, ptr %value1, align 4
  %allocation = call ptr @__wosy_core_alloc(i64 11, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  store i64 11, ptr %capacity, align 4
  store i64 1, ptr %one, align 4
  store i64 0, ptr %zero_count, align 4
  store i32 0, ptr %zero, align 4
  store i32 10, ptr %ten, align 4
  store i8 48, ptr %zero_digit, align 1
  store i8 45, ptr %minus, align 1
  %capacity2 = load i64, ptr %capacity, align 4
  store i64 %capacity2, ptr %index, align 4
  %value3 = load i32, ptr %value1, align 4
  store i32 %value3, ptr %remaining, align 4
  %remaining4 = load i32, ptr %remaining, align 4
  %zero5 = load i32, ptr %zero, align 4
  %lt = icmp slt i32 %remaining4, %zero5
  store i1 %lt, ptr %negative, align 1
  %remaining6 = load i32, ptr %remaining, align 4
  %zero7 = load i32, ptr %zero, align 4
  %eq = icmp eq i32 %remaining6, %zero7
  br i1 %eq, label %if.then, label %if.merge

if.then:                                          ; preds = %allocation_continue
  %index8 = load i64, ptr %index, align 4
  %one9 = load i64, ptr %one, align 4
  %sub = sub i64 %index8, %one9
  store i64 %sub, ptr %assignment_value, align 4
  %assignment_value10 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value10, ptr %index, align 4
  %zero_digit11 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit11, ptr %assignment_value12, align 1
  %assignment_value13 = load i8, ptr %assignment_value12, align 1
  %index14 = load i64, ptr %index, align 4
  %array_element = getelementptr inbounds i8, ptr %allocation, i64 %index14
  store i8 %assignment_value13, ptr %array_element, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %allocation_continue
  br label %while.cond.0

while.cond.0:                                     ; preds = %if.merge21, %if.merge
  %remaining15 = load i32, ptr %remaining, align 4
  %zero16 = load i32, ptr %zero, align 4
  %ne = icmp ne i32 %remaining15, %zero16
  br i1 %ne, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %remaining17 = load i32, ptr %remaining, align 4
  %ten18 = load i32, ptr %ten, align 4
  %rem = srem i32 %remaining17, %ten18
  store i32 %rem, ptr %digit_signed, align 4
  %negative19 = load i1, ptr %negative, align 1
  br i1 %negative19, label %if.then20, label %if.merge21

while.exit.2:                                     ; preds = %while.cond.0
  %negative43 = load i1, ptr %negative, align 1
  br i1 %negative43, label %if.then44, label %if.merge45

if.then20:                                        ; preds = %while.body.1
  %zero22 = load i32, ptr %zero, align 4
  %digit_signed23 = load i32, ptr %digit_signed, align 4
  %sub24 = sub i32 %zero22, %digit_signed23
  store i32 %sub24, ptr %assignment_value25, align 4
  %assignment_value26 = load i32, ptr %assignment_value25, align 4
  store i32 %assignment_value26, ptr %digit_signed, align 4
  br label %if.merge21

if.merge21:                                       ; preds = %if.then20, %while.body.1
  %digit_signed27 = load i32, ptr %digit_signed, align 4
  %int_trunc = trunc i32 %digit_signed27 to i8
  store i8 %int_trunc, ptr %digit, align 1
  %index28 = load i64, ptr %index, align 4
  %one29 = load i64, ptr %one, align 4
  %sub30 = sub i64 %index28, %one29
  store i64 %sub30, ptr %assignment_value31, align 4
  %assignment_value32 = load i64, ptr %assignment_value31, align 4
  store i64 %assignment_value32, ptr %index, align 4
  %zero_digit33 = load i8, ptr %zero_digit, align 1
  %digit34 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit33, %digit34
  store i8 %add, ptr %assignment_value35, align 1
  %assignment_value36 = load i8, ptr %assignment_value35, align 1
  %index37 = load i64, ptr %index, align 4
  %array_element38 = getelementptr inbounds i8, ptr %allocation, i64 %index37
  store i8 %assignment_value36, ptr %array_element38, align 1
  %remaining39 = load i32, ptr %remaining, align 4
  %ten40 = load i32, ptr %ten, align 4
  %div = sdiv i32 %remaining39, %ten40
  store i32 %div, ptr %assignment_value41, align 4
  %assignment_value42 = load i32, ptr %assignment_value41, align 4
  store i32 %assignment_value42, ptr %remaining, align 4
  br label %while.cond.0

if.then44:                                        ; preds = %while.exit.2
  %index46 = load i64, ptr %index, align 4
  %one47 = load i64, ptr %one, align 4
  %sub48 = sub i64 %index46, %one47
  store i64 %sub48, ptr %assignment_value49, align 4
  %assignment_value50 = load i64, ptr %assignment_value49, align 4
  store i64 %assignment_value50, ptr %index, align 4
  %minus51 = load i8, ptr %minus, align 1
  store i8 %minus51, ptr %assignment_value52, align 1
  %assignment_value53 = load i8, ptr %assignment_value52, align 1
  %index54 = load i64, ptr %index, align 4
  %array_element55 = getelementptr inbounds i8, ptr %allocation, i64 %index54
  store i8 %assignment_value53, ptr %array_element55, align 1
  br label %if.merge45

if.merge45:                                       ; preds = %if.then44, %while.exit.2
  %capacity56 = load i64, ptr %capacity, align 4
  %index57 = load i64, ptr %index, align 4
  %sub58 = sub i64 %capacity56, %index57
  store i64 %sub58, ptr %length, align 4
  %zero_count59 = load i64, ptr %zero_count, align 4
  store i64 %zero_count59, ptr %target, align 4
  %index60 = load i64, ptr %index, align 4
  store i64 %index60, ptr %source, align 4
  br label %while.cond.3

while.cond.3:                                     ; preds = %while.body.4, %if.merge45
  %source61 = load i64, ptr %source, align 4
  %capacity62 = load i64, ptr %capacity, align 4
  %lt63 = icmp ult i64 %source61, %capacity62
  br i1 %lt63, label %while.body.4, label %while.exit.5

while.body.4:                                     ; preds = %while.cond.3
  %source64 = load i64, ptr %source, align 4
  %array_element65 = getelementptr inbounds i8, ptr %allocation, i64 %source64
  %place = load i8, ptr %array_element65, align 1
  store i8 %place, ptr %assignment_value66, align 1
  %assignment_value67 = load i8, ptr %assignment_value66, align 1
  %target68 = load i64, ptr %target, align 4
  %array_element69 = getelementptr inbounds i8, ptr %allocation, i64 %target68
  store i8 %assignment_value67, ptr %array_element69, align 1
  %target70 = load i64, ptr %target, align 4
  %one71 = load i64, ptr %one, align 4
  %add72 = add i64 %target70, %one71
  store i64 %add72, ptr %assignment_value73, align 4
  %assignment_value74 = load i64, ptr %assignment_value73, align 4
  store i64 %assignment_value74, ptr %target, align 4
  %source75 = load i64, ptr %source, align 4
  %one76 = load i64, ptr %one, align 4
  %add77 = add i64 %source75, %one76
  store i64 %add77, ptr %assignment_value78, align 4
  %assignment_value79 = load i64, ptr %assignment_value78, align 4
  store i64 %assignment_value79, ptr %source, align 4
  br label %while.cond.3

while.exit.5:                                     ; preds = %while.cond.3
  store i32 0, ptr %data, align 4
  %array_element80 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element80 to i32
  store i32 %address, ptr %assignment_value81, align 4
  %assignment_value82 = load i32, ptr %assignment_value81, align 4
  store i32 %assignment_value82, ptr %data, align 4
  %data83 = load i32, ptr %data, align 4
  %length84 = load i64, ptr %length, align 4
  %data85 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data83, ptr %data85, align 4
  %length86 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length84, ptr %length86, align 4
  %struct_value = load [16 x i8], ptr %struct_literal, align 1
  store [16 x i8] %struct_value, ptr %text, align 1
  ret ptr %text
}

define ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f693634(i64 %value) {
entry:
  %text = alloca [16 x i8], align 1
  %struct_literal = alloca [16 x i8], align 1
  %assignment_value81 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value78 = alloca i64, align 8
  %assignment_value73 = alloca i64, align 8
  %assignment_value66 = alloca i8, align 1
  %source = alloca i64, align 8
  %target = alloca i64, align 8
  %length = alloca i64, align 8
  %assignment_value52 = alloca i8, align 1
  %assignment_value49 = alloca i64, align 8
  %assignment_value41 = alloca i64, align 8
  %assignment_value35 = alloca i8, align 1
  %assignment_value31 = alloca i64, align 8
  %digit = alloca i8, align 1
  %assignment_value25 = alloca i64, align 8
  %digit_signed = alloca i64, align 8
  %assignment_value12 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %negative = alloca i1, align 1
  %remaining = alloca i64, align 8
  %index = alloca i64, align 8
  %minus = alloca i8, align 1
  %zero_digit = alloca i8, align 1
  %ten = alloca i64, align 8
  %zero = alloca i64, align 8
  %zero_count = alloca i64, align 8
  %one = alloca i64, align 8
  %capacity = alloca i64, align 8
  %value1 = alloca i64, align 8
  store i64 %value, ptr %value1, align 4
  %allocation = call ptr @__wosy_core_alloc(i64 20, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  store i64 20, ptr %capacity, align 4
  store i64 1, ptr %one, align 4
  store i64 0, ptr %zero_count, align 4
  store i64 0, ptr %zero, align 4
  store i64 10, ptr %ten, align 4
  store i8 48, ptr %zero_digit, align 1
  store i8 45, ptr %minus, align 1
  %capacity2 = load i64, ptr %capacity, align 4
  store i64 %capacity2, ptr %index, align 4
  %value3 = load i64, ptr %value1, align 4
  store i64 %value3, ptr %remaining, align 4
  %remaining4 = load i64, ptr %remaining, align 4
  %zero5 = load i64, ptr %zero, align 4
  %lt = icmp slt i64 %remaining4, %zero5
  store i1 %lt, ptr %negative, align 1
  %remaining6 = load i64, ptr %remaining, align 4
  %zero7 = load i64, ptr %zero, align 4
  %eq = icmp eq i64 %remaining6, %zero7
  br i1 %eq, label %if.then, label %if.merge

if.then:                                          ; preds = %allocation_continue
  %index8 = load i64, ptr %index, align 4
  %one9 = load i64, ptr %one, align 4
  %sub = sub i64 %index8, %one9
  store i64 %sub, ptr %assignment_value, align 4
  %assignment_value10 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value10, ptr %index, align 4
  %zero_digit11 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit11, ptr %assignment_value12, align 1
  %assignment_value13 = load i8, ptr %assignment_value12, align 1
  %index14 = load i64, ptr %index, align 4
  %array_element = getelementptr inbounds i8, ptr %allocation, i64 %index14
  store i8 %assignment_value13, ptr %array_element, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %allocation_continue
  br label %while.cond.0

while.cond.0:                                     ; preds = %if.merge21, %if.merge
  %remaining15 = load i64, ptr %remaining, align 4
  %zero16 = load i64, ptr %zero, align 4
  %ne = icmp ne i64 %remaining15, %zero16
  br i1 %ne, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %remaining17 = load i64, ptr %remaining, align 4
  %ten18 = load i64, ptr %ten, align 4
  %rem = srem i64 %remaining17, %ten18
  store i64 %rem, ptr %digit_signed, align 4
  %negative19 = load i1, ptr %negative, align 1
  br i1 %negative19, label %if.then20, label %if.merge21

while.exit.2:                                     ; preds = %while.cond.0
  %negative43 = load i1, ptr %negative, align 1
  br i1 %negative43, label %if.then44, label %if.merge45

if.then20:                                        ; preds = %while.body.1
  %zero22 = load i64, ptr %zero, align 4
  %digit_signed23 = load i64, ptr %digit_signed, align 4
  %sub24 = sub i64 %zero22, %digit_signed23
  store i64 %sub24, ptr %assignment_value25, align 4
  %assignment_value26 = load i64, ptr %assignment_value25, align 4
  store i64 %assignment_value26, ptr %digit_signed, align 4
  br label %if.merge21

if.merge21:                                       ; preds = %if.then20, %while.body.1
  %digit_signed27 = load i64, ptr %digit_signed, align 4
  %int_trunc = trunc i64 %digit_signed27 to i8
  store i8 %int_trunc, ptr %digit, align 1
  %index28 = load i64, ptr %index, align 4
  %one29 = load i64, ptr %one, align 4
  %sub30 = sub i64 %index28, %one29
  store i64 %sub30, ptr %assignment_value31, align 4
  %assignment_value32 = load i64, ptr %assignment_value31, align 4
  store i64 %assignment_value32, ptr %index, align 4
  %zero_digit33 = load i8, ptr %zero_digit, align 1
  %digit34 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit33, %digit34
  store i8 %add, ptr %assignment_value35, align 1
  %assignment_value36 = load i8, ptr %assignment_value35, align 1
  %index37 = load i64, ptr %index, align 4
  %array_element38 = getelementptr inbounds i8, ptr %allocation, i64 %index37
  store i8 %assignment_value36, ptr %array_element38, align 1
  %remaining39 = load i64, ptr %remaining, align 4
  %ten40 = load i64, ptr %ten, align 4
  %div = sdiv i64 %remaining39, %ten40
  store i64 %div, ptr %assignment_value41, align 4
  %assignment_value42 = load i64, ptr %assignment_value41, align 4
  store i64 %assignment_value42, ptr %remaining, align 4
  br label %while.cond.0

if.then44:                                        ; preds = %while.exit.2
  %index46 = load i64, ptr %index, align 4
  %one47 = load i64, ptr %one, align 4
  %sub48 = sub i64 %index46, %one47
  store i64 %sub48, ptr %assignment_value49, align 4
  %assignment_value50 = load i64, ptr %assignment_value49, align 4
  store i64 %assignment_value50, ptr %index, align 4
  %minus51 = load i8, ptr %minus, align 1
  store i8 %minus51, ptr %assignment_value52, align 1
  %assignment_value53 = load i8, ptr %assignment_value52, align 1
  %index54 = load i64, ptr %index, align 4
  %array_element55 = getelementptr inbounds i8, ptr %allocation, i64 %index54
  store i8 %assignment_value53, ptr %array_element55, align 1
  br label %if.merge45

if.merge45:                                       ; preds = %if.then44, %while.exit.2
  %capacity56 = load i64, ptr %capacity, align 4
  %index57 = load i64, ptr %index, align 4
  %sub58 = sub i64 %capacity56, %index57
  store i64 %sub58, ptr %length, align 4
  %zero_count59 = load i64, ptr %zero_count, align 4
  store i64 %zero_count59, ptr %target, align 4
  %index60 = load i64, ptr %index, align 4
  store i64 %index60, ptr %source, align 4
  br label %while.cond.3

while.cond.3:                                     ; preds = %while.body.4, %if.merge45
  %source61 = load i64, ptr %source, align 4
  %capacity62 = load i64, ptr %capacity, align 4
  %lt63 = icmp ult i64 %source61, %capacity62
  br i1 %lt63, label %while.body.4, label %while.exit.5

while.body.4:                                     ; preds = %while.cond.3
  %source64 = load i64, ptr %source, align 4
  %array_element65 = getelementptr inbounds i8, ptr %allocation, i64 %source64
  %place = load i8, ptr %array_element65, align 1
  store i8 %place, ptr %assignment_value66, align 1
  %assignment_value67 = load i8, ptr %assignment_value66, align 1
  %target68 = load i64, ptr %target, align 4
  %array_element69 = getelementptr inbounds i8, ptr %allocation, i64 %target68
  store i8 %assignment_value67, ptr %array_element69, align 1
  %target70 = load i64, ptr %target, align 4
  %one71 = load i64, ptr %one, align 4
  %add72 = add i64 %target70, %one71
  store i64 %add72, ptr %assignment_value73, align 4
  %assignment_value74 = load i64, ptr %assignment_value73, align 4
  store i64 %assignment_value74, ptr %target, align 4
  %source75 = load i64, ptr %source, align 4
  %one76 = load i64, ptr %one, align 4
  %add77 = add i64 %source75, %one76
  store i64 %add77, ptr %assignment_value78, align 4
  %assignment_value79 = load i64, ptr %assignment_value78, align 4
  store i64 %assignment_value79, ptr %source, align 4
  br label %while.cond.3

while.exit.5:                                     ; preds = %while.cond.3
  store i32 0, ptr %data, align 4
  %array_element80 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element80 to i32
  store i32 %address, ptr %assignment_value81, align 4
  %assignment_value82 = load i32, ptr %assignment_value81, align 4
  store i32 %assignment_value82, ptr %data, align 4
  %data83 = load i32, ptr %data, align 4
  %length84 = load i64, ptr %length, align 4
  %data85 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data83, ptr %data85, align 4
  %length86 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length84, ptr %length86, align 4
  %struct_value = load [16 x i8], ptr %struct_literal, align 1
  store [16 x i8] %struct_value, ptr %text, align 1
  ret ptr %text
}

define ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f69313238(i128 %value) {
entry:
  %text = alloca [16 x i8], align 1
  %struct_literal = alloca [16 x i8], align 1
  %assignment_value109 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value102 = alloca i8, align 1
  %assignment_value95 = alloca i64, align 8
  %assignment_value90 = alloca i1, align 1
  %assignment_value88 = alloca i64, align 8
  %assignment_value81 = alloca i8, align 1
  %assignment_value67 = alloca i8, align 1
  %assignment_value62 = alloca i128, align 8
  %digit = alloca i8, align 1
  %assignment_value54 = alloca i64, align 8
  %assignment_value49 = alloca i128, align 8
  %eighth = alloca i128, align 8
  %fourth = alloca i128, align 8
  %twice = alloca i128, align 8
  %step = alloca i64, align 8
  %scale = alloca i128, align 8
  %steps = alloca i64, align 8
  %position = alloca i64, align 8
  %started = alloca i1, align 1
  %assignment_value23 = alloca i128, align 8
  %assignment_value18 = alloca i128, align 8
  %assignment_value10 = alloca i64, align 8
  %assignment_value = alloca i8, align 1
  %negative = alloca i1, align 1
  %remaining = alloca i128, align 8
  %emit = alloca i64, align 8
  %minimum = alloca i128, align 8
  %one_wide = alloca i128, align 8
  %zero = alloca i128, align 8
  %minus = alloca i8, align 1
  %one_digit = alloca i8, align 1
  %zero_digit = alloca i8, align 1
  %last = alloca i64, align 8
  %zero_count = alloca i64, align 8
  %one = alloca i64, align 8
  %digits = alloca i64, align 8
  %value1 = alloca i128, align 8
  store i128 %value, ptr %value1, align 4
  %allocation = call ptr @__wosy_core_alloc(i64 40, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  store i64 39, ptr %digits, align 4
  store i64 1, ptr %one, align 4
  store i64 0, ptr %zero_count, align 4
  store i64 38, ptr %last, align 4
  store i8 48, ptr %zero_digit, align 1
  store i8 1, ptr %one_digit, align 1
  store i8 45, ptr %minus, align 1
  store i128 0, ptr %zero, align 4
  store i128 1, ptr %one_wide, align 4
  store i128 -170141183460469231731687303715884105728, ptr %minimum, align 4
  %zero_count2 = load i64, ptr %zero_count, align 4
  store i64 %zero_count2, ptr %emit, align 4
  %value3 = load i128, ptr %value1, align 4
  store i128 %value3, ptr %remaining, align 4
  %value4 = load i128, ptr %value1, align 4
  %zero5 = load i128, ptr %zero, align 4
  %lt = icmp slt i128 %value4, %zero5
  store i1 %lt, ptr %negative, align 1
  %negative6 = load i1, ptr %negative, align 1
  br i1 %negative6, label %if.then, label %if.merge

if.then:                                          ; preds = %allocation_continue
  %minus7 = load i8, ptr %minus, align 1
  store i8 %minus7, ptr %assignment_value, align 1
  %assignment_value8 = load i8, ptr %assignment_value, align 1
  %array_element = getelementptr inbounds i8, ptr %allocation, i64 0
  store i8 %assignment_value8, ptr %array_element, align 1
  %one9 = load i64, ptr %one, align 4
  store i64 %one9, ptr %assignment_value10, align 4
  %assignment_value11 = load i64, ptr %assignment_value10, align 4
  store i64 %assignment_value11, ptr %emit, align 4
  %value12 = load i128, ptr %value1, align 4
  %minimum13 = load i128, ptr %minimum, align 4
  %eq = icmp eq i128 %value12, %minimum13
  br i1 %eq, label %if.then14, label %if.else

if.merge:                                         ; preds = %if.merge15, %allocation_continue
  store i1 false, ptr %started, align 1
  %zero_count25 = load i64, ptr %zero_count, align 4
  store i64 %zero_count25, ptr %position, align 4
  br label %while.cond.0

if.then14:                                        ; preds = %if.then
  %zero16 = load i128, ptr %zero, align 4
  %value17 = load i128, ptr %value1, align 4
  %add = add i128 %value17, 1
  %sub = sub i128 %zero16, %add
  store i128 %sub, ptr %assignment_value18, align 4
  %assignment_value19 = load i128, ptr %assignment_value18, align 4
  store i128 %assignment_value19, ptr %remaining, align 4
  br label %if.merge15

if.else:                                          ; preds = %if.then
  %zero20 = load i128, ptr %zero, align 4
  %value21 = load i128, ptr %value1, align 4
  %sub22 = sub i128 %zero20, %value21
  store i128 %sub22, ptr %assignment_value23, align 4
  %assignment_value24 = load i128, ptr %assignment_value23, align 4
  store i128 %assignment_value24, ptr %remaining, align 4
  br label %if.merge15

if.merge15:                                       ; preds = %if.else, %if.then14
  br label %if.merge

while.cond.0:                                     ; preds = %if.merge79, %if.merge
  %position26 = load i64, ptr %position, align 4
  %digits27 = load i64, ptr %digits, align 4
  %lt28 = icmp ult i64 %position26, %digits27
  br i1 %lt28, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %last29 = load i64, ptr %last, align 4
  %position30 = load i64, ptr %position, align 4
  %sub31 = sub i64 %last29, %position30
  store i64 %sub31, ptr %steps, align 4
  %one_wide32 = load i128, ptr %one_wide, align 4
  store i128 %one_wide32, ptr %scale, align 4
  %zero_count33 = load i64, ptr %zero_count, align 4
  store i64 %zero_count33, ptr %step, align 4
  br label %while.cond.3

while.exit.2:                                     ; preds = %while.cond.0
  %value97 = load i128, ptr %value1, align 4
  %minimum98 = load i128, ptr %minimum, align 4
  %eq99 = icmp eq i128 %value97, %minimum98
  br i1 %eq99, label %if.then100, label %if.merge101

while.cond.3:                                     ; preds = %while.body.4, %while.body.1
  %step34 = load i64, ptr %step, align 4
  %steps35 = load i64, ptr %steps, align 4
  %lt36 = icmp ult i64 %step34, %steps35
  br i1 %lt36, label %while.body.4, label %while.exit.5

while.body.4:                                     ; preds = %while.cond.3
  %scale37 = load i128, ptr %scale, align 4
  %scale38 = load i128, ptr %scale, align 4
  %add39 = add i128 %scale37, %scale38
  store i128 %add39, ptr %twice, align 4
  %twice40 = load i128, ptr %twice, align 4
  %twice41 = load i128, ptr %twice, align 4
  %add42 = add i128 %twice40, %twice41
  store i128 %add42, ptr %fourth, align 4
  %fourth43 = load i128, ptr %fourth, align 4
  %fourth44 = load i128, ptr %fourth, align 4
  %add45 = add i128 %fourth43, %fourth44
  store i128 %add45, ptr %eighth, align 4
  %eighth46 = load i128, ptr %eighth, align 4
  %twice47 = load i128, ptr %twice, align 4
  %add48 = add i128 %eighth46, %twice47
  store i128 %add48, ptr %assignment_value49, align 4
  %assignment_value50 = load i128, ptr %assignment_value49, align 4
  store i128 %assignment_value50, ptr %scale, align 4
  %step51 = load i64, ptr %step, align 4
  %one52 = load i64, ptr %one, align 4
  %add53 = add i64 %step51, %one52
  store i64 %add53, ptr %assignment_value54, align 4
  %assignment_value55 = load i64, ptr %assignment_value54, align 4
  store i64 %assignment_value55, ptr %step, align 4
  br label %while.cond.3

while.exit.5:                                     ; preds = %while.cond.3
  %zero_digit56 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit56, ptr %digit, align 1
  br label %while.cond.6

while.cond.6:                                     ; preds = %while.body.7, %while.exit.5
  %remaining57 = load i128, ptr %remaining, align 4
  %scale58 = load i128, ptr %scale, align 4
  %ge = icmp sge i128 %remaining57, %scale58
  br i1 %ge, label %while.body.7, label %while.exit.8

while.body.7:                                     ; preds = %while.cond.6
  %remaining59 = load i128, ptr %remaining, align 4
  %scale60 = load i128, ptr %scale, align 4
  %sub61 = sub i128 %remaining59, %scale60
  store i128 %sub61, ptr %assignment_value62, align 4
  %assignment_value63 = load i128, ptr %assignment_value62, align 4
  store i128 %assignment_value63, ptr %remaining, align 4
  %digit64 = load i8, ptr %digit, align 1
  %one_digit65 = load i8, ptr %one_digit, align 1
  %add66 = add i8 %digit64, %one_digit65
  store i8 %add66, ptr %assignment_value67, align 1
  %assignment_value68 = load i8, ptr %assignment_value67, align 1
  store i8 %assignment_value68, ptr %digit, align 1
  br label %while.cond.6

while.exit.8:                                     ; preds = %while.cond.6
  %started69 = load i1, ptr %started, align 1
  br i1 %started69, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %while.exit.8
  %digit70 = load i8, ptr %digit, align 1
  %zero_digit71 = load i8, ptr %zero_digit, align 1
  %ne = icmp ne i8 %digit70, %zero_digit71
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %while.exit.8
  %short_circuit = phi i1 [ true, %while.exit.8 ], [ %ne, %short_circuit.rhs ]
  br i1 %short_circuit, label %short_circuit.merge73, label %short_circuit.rhs72

short_circuit.rhs72:                              ; preds = %short_circuit.merge
  %position74 = load i64, ptr %position, align 4
  %last75 = load i64, ptr %last, align 4
  %eq76 = icmp eq i64 %position74, %last75
  br label %short_circuit.merge73

short_circuit.merge73:                            ; preds = %short_circuit.rhs72, %short_circuit.merge
  %short_circuit77 = phi i1 [ true, %short_circuit.merge ], [ %eq76, %short_circuit.rhs72 ]
  br i1 %short_circuit77, label %if.then78, label %if.merge79

if.then78:                                        ; preds = %short_circuit.merge73
  %digit80 = load i8, ptr %digit, align 1
  store i8 %digit80, ptr %assignment_value81, align 1
  %assignment_value82 = load i8, ptr %assignment_value81, align 1
  %emit83 = load i64, ptr %emit, align 4
  %array_element84 = getelementptr inbounds i8, ptr %allocation, i64 %emit83
  store i8 %assignment_value82, ptr %array_element84, align 1
  %emit85 = load i64, ptr %emit, align 4
  %one86 = load i64, ptr %one, align 4
  %add87 = add i64 %emit85, %one86
  store i64 %add87, ptr %assignment_value88, align 4
  %assignment_value89 = load i64, ptr %assignment_value88, align 4
  store i64 %assignment_value89, ptr %emit, align 4
  store i1 true, ptr %assignment_value90, align 1
  %assignment_value91 = load i1, ptr %assignment_value90, align 1
  store i1 %assignment_value91, ptr %started, align 1
  br label %if.merge79

if.merge79:                                       ; preds = %if.then78, %short_circuit.merge73
  %position92 = load i64, ptr %position, align 4
  %one93 = load i64, ptr %one, align 4
  %add94 = add i64 %position92, %one93
  store i64 %add94, ptr %assignment_value95, align 4
  %assignment_value96 = load i64, ptr %assignment_value95, align 4
  store i64 %assignment_value96, ptr %position, align 4
  br label %while.cond.0

if.then100:                                       ; preds = %while.exit.2
  store i8 56, ptr %assignment_value102, align 1
  %assignment_value103 = load i8, ptr %assignment_value102, align 1
  %emit104 = load i64, ptr %emit, align 4
  %one105 = load i64, ptr %one, align 4
  %sub106 = sub i64 %emit104, %one105
  %array_element107 = getelementptr inbounds i8, ptr %allocation, i64 %sub106
  store i8 %assignment_value103, ptr %array_element107, align 1
  br label %if.merge101

if.merge101:                                      ; preds = %if.then100, %while.exit.2
  store i32 0, ptr %data, align 4
  %array_element108 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element108 to i32
  store i32 %address, ptr %assignment_value109, align 4
  %assignment_value110 = load i32, ptr %assignment_value109, align 4
  store i32 %assignment_value110, ptr %data, align 4
  %data111 = load i32, ptr %data, align 4
  %emit112 = load i64, ptr %emit, align 4
  %data113 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data111, ptr %data113, align 4
  %length = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %emit112, ptr %length, align 4
  %struct_value = load [16 x i8], ptr %struct_literal, align 1
  store [16 x i8] %struct_value, ptr %text, align 1
  ret ptr %text
}

define ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f7538(i8 %value) {
entry:
  %text = alloca [16 x i8], align 1
  %struct_literal = alloca [16 x i8], align 1
  %assignment_value56 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value53 = alloca i64, align 8
  %assignment_value48 = alloca i64, align 8
  %assignment_value41 = alloca i8, align 1
  %source = alloca i64, align 8
  %target = alloca i64, align 8
  %length = alloca i64, align 8
  %assignment_value30 = alloca i8, align 1
  %assignment_value24 = alloca i8, align 1
  %assignment_value20 = alloca i64, align 8
  %digit = alloca i8, align 1
  %assignment_value10 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %remaining = alloca i8, align 1
  %index = alloca i64, align 8
  %zero_value = alloca i8, align 1
  %zero_digit = alloca i8, align 1
  %ten = alloca i8, align 1
  %zero = alloca i64, align 8
  %one = alloca i64, align 8
  %capacity = alloca i64, align 8
  %value1 = alloca i8, align 1
  store i8 %value, ptr %value1, align 1
  %allocation = call ptr @__wosy_core_alloc(i64 3, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  store i64 3, ptr %capacity, align 4
  store i64 1, ptr %one, align 4
  store i64 0, ptr %zero, align 4
  store i8 10, ptr %ten, align 1
  store i8 48, ptr %zero_digit, align 1
  store i8 0, ptr %zero_value, align 1
  %capacity2 = load i64, ptr %capacity, align 4
  store i64 %capacity2, ptr %index, align 4
  %value3 = load i8, ptr %value1, align 1
  store i8 %value3, ptr %remaining, align 1
  %remaining4 = load i8, ptr %remaining, align 1
  %zero_value5 = load i8, ptr %zero_value, align 1
  %eq = icmp eq i8 %remaining4, %zero_value5
  br i1 %eq, label %if.then, label %if.merge

if.then:                                          ; preds = %allocation_continue
  %index6 = load i64, ptr %index, align 4
  %one7 = load i64, ptr %one, align 4
  %sub = sub i64 %index6, %one7
  store i64 %sub, ptr %assignment_value, align 4
  %assignment_value8 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value8, ptr %index, align 4
  %zero_digit9 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit9, ptr %assignment_value10, align 1
  %assignment_value11 = load i8, ptr %assignment_value10, align 1
  %index12 = load i64, ptr %index, align 4
  %array_element = getelementptr inbounds i8, ptr %allocation, i64 %index12
  store i8 %assignment_value11, ptr %array_element, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %allocation_continue
  br label %while.cond.0

while.cond.0:                                     ; preds = %while.body.1, %if.merge
  %remaining13 = load i8, ptr %remaining, align 1
  %zero_value14 = load i8, ptr %zero_value, align 1
  %ne = icmp ne i8 %remaining13, %zero_value14
  br i1 %ne, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %remaining15 = load i8, ptr %remaining, align 1
  %ten16 = load i8, ptr %ten, align 1
  %rem = urem i8 %remaining15, %ten16
  store i8 %rem, ptr %digit, align 1
  %index17 = load i64, ptr %index, align 4
  %one18 = load i64, ptr %one, align 4
  %sub19 = sub i64 %index17, %one18
  store i64 %sub19, ptr %assignment_value20, align 4
  %assignment_value21 = load i64, ptr %assignment_value20, align 4
  store i64 %assignment_value21, ptr %index, align 4
  %zero_digit22 = load i8, ptr %zero_digit, align 1
  %digit23 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit22, %digit23
  store i8 %add, ptr %assignment_value24, align 1
  %assignment_value25 = load i8, ptr %assignment_value24, align 1
  %index26 = load i64, ptr %index, align 4
  %array_element27 = getelementptr inbounds i8, ptr %allocation, i64 %index26
  store i8 %assignment_value25, ptr %array_element27, align 1
  %remaining28 = load i8, ptr %remaining, align 1
  %ten29 = load i8, ptr %ten, align 1
  %div = udiv i8 %remaining28, %ten29
  store i8 %div, ptr %assignment_value30, align 1
  %assignment_value31 = load i8, ptr %assignment_value30, align 1
  store i8 %assignment_value31, ptr %remaining, align 1
  br label %while.cond.0

while.exit.2:                                     ; preds = %while.cond.0
  %capacity32 = load i64, ptr %capacity, align 4
  %index33 = load i64, ptr %index, align 4
  %sub34 = sub i64 %capacity32, %index33
  store i64 %sub34, ptr %length, align 4
  %zero35 = load i64, ptr %zero, align 4
  store i64 %zero35, ptr %target, align 4
  %index36 = load i64, ptr %index, align 4
  store i64 %index36, ptr %source, align 4
  br label %while.cond.3

while.cond.3:                                     ; preds = %while.body.4, %while.exit.2
  %source37 = load i64, ptr %source, align 4
  %capacity38 = load i64, ptr %capacity, align 4
  %lt = icmp ult i64 %source37, %capacity38
  br i1 %lt, label %while.body.4, label %while.exit.5

while.body.4:                                     ; preds = %while.cond.3
  %source39 = load i64, ptr %source, align 4
  %array_element40 = getelementptr inbounds i8, ptr %allocation, i64 %source39
  %place = load i8, ptr %array_element40, align 1
  store i8 %place, ptr %assignment_value41, align 1
  %assignment_value42 = load i8, ptr %assignment_value41, align 1
  %target43 = load i64, ptr %target, align 4
  %array_element44 = getelementptr inbounds i8, ptr %allocation, i64 %target43
  store i8 %assignment_value42, ptr %array_element44, align 1
  %target45 = load i64, ptr %target, align 4
  %one46 = load i64, ptr %one, align 4
  %add47 = add i64 %target45, %one46
  store i64 %add47, ptr %assignment_value48, align 4
  %assignment_value49 = load i64, ptr %assignment_value48, align 4
  store i64 %assignment_value49, ptr %target, align 4
  %source50 = load i64, ptr %source, align 4
  %one51 = load i64, ptr %one, align 4
  %add52 = add i64 %source50, %one51
  store i64 %add52, ptr %assignment_value53, align 4
  %assignment_value54 = load i64, ptr %assignment_value53, align 4
  store i64 %assignment_value54, ptr %source, align 4
  br label %while.cond.3

while.exit.5:                                     ; preds = %while.cond.3
  store i32 0, ptr %data, align 4
  %array_element55 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element55 to i32
  store i32 %address, ptr %assignment_value56, align 4
  %assignment_value57 = load i32, ptr %assignment_value56, align 4
  store i32 %assignment_value57, ptr %data, align 4
  %data58 = load i32, ptr %data, align 4
  %length59 = load i64, ptr %length, align 4
  %data60 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data58, ptr %data60, align 4
  %length61 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length59, ptr %length61, align 4
  %struct_value = load [16 x i8], ptr %struct_literal, align 1
  store [16 x i8] %struct_value, ptr %text, align 1
  ret ptr %text
}

define ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f753136(i16 %value) {
entry:
  %text = alloca [16 x i8], align 1
  %struct_literal = alloca [16 x i8], align 1
  %assignment_value57 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value54 = alloca i64, align 8
  %assignment_value49 = alloca i64, align 8
  %assignment_value42 = alloca i8, align 1
  %source = alloca i64, align 8
  %target = alloca i64, align 8
  %length = alloca i64, align 8
  %assignment_value31 = alloca i16, align 2
  %assignment_value25 = alloca i8, align 1
  %assignment_value21 = alloca i64, align 8
  %digit = alloca i8, align 1
  %digit_wide = alloca i16, align 2
  %assignment_value10 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %remaining = alloca i16, align 2
  %index = alloca i64, align 8
  %zero_value = alloca i16, align 2
  %zero_digit = alloca i8, align 1
  %ten = alloca i16, align 2
  %zero = alloca i64, align 8
  %one = alloca i64, align 8
  %capacity = alloca i64, align 8
  %value1 = alloca i16, align 2
  store i16 %value, ptr %value1, align 2
  %allocation = call ptr @__wosy_core_alloc(i64 5, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  store i64 5, ptr %capacity, align 4
  store i64 1, ptr %one, align 4
  store i64 0, ptr %zero, align 4
  store i16 10, ptr %ten, align 2
  store i8 48, ptr %zero_digit, align 1
  store i16 0, ptr %zero_value, align 2
  %capacity2 = load i64, ptr %capacity, align 4
  store i64 %capacity2, ptr %index, align 4
  %value3 = load i16, ptr %value1, align 2
  store i16 %value3, ptr %remaining, align 2
  %remaining4 = load i16, ptr %remaining, align 2
  %zero_value5 = load i16, ptr %zero_value, align 2
  %eq = icmp eq i16 %remaining4, %zero_value5
  br i1 %eq, label %if.then, label %if.merge

if.then:                                          ; preds = %allocation_continue
  %index6 = load i64, ptr %index, align 4
  %one7 = load i64, ptr %one, align 4
  %sub = sub i64 %index6, %one7
  store i64 %sub, ptr %assignment_value, align 4
  %assignment_value8 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value8, ptr %index, align 4
  %zero_digit9 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit9, ptr %assignment_value10, align 1
  %assignment_value11 = load i8, ptr %assignment_value10, align 1
  %index12 = load i64, ptr %index, align 4
  %array_element = getelementptr inbounds i8, ptr %allocation, i64 %index12
  store i8 %assignment_value11, ptr %array_element, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %allocation_continue
  br label %while.cond.0

while.cond.0:                                     ; preds = %while.body.1, %if.merge
  %remaining13 = load i16, ptr %remaining, align 2
  %zero_value14 = load i16, ptr %zero_value, align 2
  %ne = icmp ne i16 %remaining13, %zero_value14
  br i1 %ne, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %remaining15 = load i16, ptr %remaining, align 2
  %ten16 = load i16, ptr %ten, align 2
  %rem = urem i16 %remaining15, %ten16
  store i16 %rem, ptr %digit_wide, align 2
  %digit_wide17 = load i16, ptr %digit_wide, align 2
  %int_trunc = trunc i16 %digit_wide17 to i8
  store i8 %int_trunc, ptr %digit, align 1
  %index18 = load i64, ptr %index, align 4
  %one19 = load i64, ptr %one, align 4
  %sub20 = sub i64 %index18, %one19
  store i64 %sub20, ptr %assignment_value21, align 4
  %assignment_value22 = load i64, ptr %assignment_value21, align 4
  store i64 %assignment_value22, ptr %index, align 4
  %zero_digit23 = load i8, ptr %zero_digit, align 1
  %digit24 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit23, %digit24
  store i8 %add, ptr %assignment_value25, align 1
  %assignment_value26 = load i8, ptr %assignment_value25, align 1
  %index27 = load i64, ptr %index, align 4
  %array_element28 = getelementptr inbounds i8, ptr %allocation, i64 %index27
  store i8 %assignment_value26, ptr %array_element28, align 1
  %remaining29 = load i16, ptr %remaining, align 2
  %ten30 = load i16, ptr %ten, align 2
  %div = udiv i16 %remaining29, %ten30
  store i16 %div, ptr %assignment_value31, align 2
  %assignment_value32 = load i16, ptr %assignment_value31, align 2
  store i16 %assignment_value32, ptr %remaining, align 2
  br label %while.cond.0

while.exit.2:                                     ; preds = %while.cond.0
  %capacity33 = load i64, ptr %capacity, align 4
  %index34 = load i64, ptr %index, align 4
  %sub35 = sub i64 %capacity33, %index34
  store i64 %sub35, ptr %length, align 4
  %zero36 = load i64, ptr %zero, align 4
  store i64 %zero36, ptr %target, align 4
  %index37 = load i64, ptr %index, align 4
  store i64 %index37, ptr %source, align 4
  br label %while.cond.3

while.cond.3:                                     ; preds = %while.body.4, %while.exit.2
  %source38 = load i64, ptr %source, align 4
  %capacity39 = load i64, ptr %capacity, align 4
  %lt = icmp ult i64 %source38, %capacity39
  br i1 %lt, label %while.body.4, label %while.exit.5

while.body.4:                                     ; preds = %while.cond.3
  %source40 = load i64, ptr %source, align 4
  %array_element41 = getelementptr inbounds i8, ptr %allocation, i64 %source40
  %place = load i8, ptr %array_element41, align 1
  store i8 %place, ptr %assignment_value42, align 1
  %assignment_value43 = load i8, ptr %assignment_value42, align 1
  %target44 = load i64, ptr %target, align 4
  %array_element45 = getelementptr inbounds i8, ptr %allocation, i64 %target44
  store i8 %assignment_value43, ptr %array_element45, align 1
  %target46 = load i64, ptr %target, align 4
  %one47 = load i64, ptr %one, align 4
  %add48 = add i64 %target46, %one47
  store i64 %add48, ptr %assignment_value49, align 4
  %assignment_value50 = load i64, ptr %assignment_value49, align 4
  store i64 %assignment_value50, ptr %target, align 4
  %source51 = load i64, ptr %source, align 4
  %one52 = load i64, ptr %one, align 4
  %add53 = add i64 %source51, %one52
  store i64 %add53, ptr %assignment_value54, align 4
  %assignment_value55 = load i64, ptr %assignment_value54, align 4
  store i64 %assignment_value55, ptr %source, align 4
  br label %while.cond.3

while.exit.5:                                     ; preds = %while.cond.3
  store i32 0, ptr %data, align 4
  %array_element56 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element56 to i32
  store i32 %address, ptr %assignment_value57, align 4
  %assignment_value58 = load i32, ptr %assignment_value57, align 4
  store i32 %assignment_value58, ptr %data, align 4
  %data59 = load i32, ptr %data, align 4
  %length60 = load i64, ptr %length, align 4
  %data61 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data59, ptr %data61, align 4
  %length62 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length60, ptr %length62, align 4
  %struct_value = load [16 x i8], ptr %struct_literal, align 1
  store [16 x i8] %struct_value, ptr %text, align 1
  ret ptr %text
}

define ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f753332(i32 %value) {
entry:
  %text = alloca [16 x i8], align 1
  %struct_literal = alloca [16 x i8], align 1
  %assignment_value57 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value54 = alloca i64, align 8
  %assignment_value49 = alloca i64, align 8
  %assignment_value42 = alloca i8, align 1
  %source = alloca i64, align 8
  %target = alloca i64, align 8
  %length = alloca i64, align 8
  %assignment_value31 = alloca i32, align 4
  %assignment_value25 = alloca i8, align 1
  %assignment_value21 = alloca i64, align 8
  %digit = alloca i8, align 1
  %digit_wide = alloca i32, align 4
  %assignment_value10 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %remaining = alloca i32, align 4
  %index = alloca i64, align 8
  %zero_value = alloca i32, align 4
  %zero_digit = alloca i8, align 1
  %ten = alloca i32, align 4
  %zero = alloca i64, align 8
  %one = alloca i64, align 8
  %capacity = alloca i64, align 8
  %value1 = alloca i32, align 4
  store i32 %value, ptr %value1, align 4
  %allocation = call ptr @__wosy_core_alloc(i64 10, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  store i64 10, ptr %capacity, align 4
  store i64 1, ptr %one, align 4
  store i64 0, ptr %zero, align 4
  store i32 10, ptr %ten, align 4
  store i8 48, ptr %zero_digit, align 1
  store i32 0, ptr %zero_value, align 4
  %capacity2 = load i64, ptr %capacity, align 4
  store i64 %capacity2, ptr %index, align 4
  %value3 = load i32, ptr %value1, align 4
  store i32 %value3, ptr %remaining, align 4
  %remaining4 = load i32, ptr %remaining, align 4
  %zero_value5 = load i32, ptr %zero_value, align 4
  %eq = icmp eq i32 %remaining4, %zero_value5
  br i1 %eq, label %if.then, label %if.merge

if.then:                                          ; preds = %allocation_continue
  %index6 = load i64, ptr %index, align 4
  %one7 = load i64, ptr %one, align 4
  %sub = sub i64 %index6, %one7
  store i64 %sub, ptr %assignment_value, align 4
  %assignment_value8 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value8, ptr %index, align 4
  %zero_digit9 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit9, ptr %assignment_value10, align 1
  %assignment_value11 = load i8, ptr %assignment_value10, align 1
  %index12 = load i64, ptr %index, align 4
  %array_element = getelementptr inbounds i8, ptr %allocation, i64 %index12
  store i8 %assignment_value11, ptr %array_element, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %allocation_continue
  br label %while.cond.0

while.cond.0:                                     ; preds = %while.body.1, %if.merge
  %remaining13 = load i32, ptr %remaining, align 4
  %zero_value14 = load i32, ptr %zero_value, align 4
  %ne = icmp ne i32 %remaining13, %zero_value14
  br i1 %ne, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %remaining15 = load i32, ptr %remaining, align 4
  %ten16 = load i32, ptr %ten, align 4
  %rem = urem i32 %remaining15, %ten16
  store i32 %rem, ptr %digit_wide, align 4
  %digit_wide17 = load i32, ptr %digit_wide, align 4
  %int_trunc = trunc i32 %digit_wide17 to i8
  store i8 %int_trunc, ptr %digit, align 1
  %index18 = load i64, ptr %index, align 4
  %one19 = load i64, ptr %one, align 4
  %sub20 = sub i64 %index18, %one19
  store i64 %sub20, ptr %assignment_value21, align 4
  %assignment_value22 = load i64, ptr %assignment_value21, align 4
  store i64 %assignment_value22, ptr %index, align 4
  %zero_digit23 = load i8, ptr %zero_digit, align 1
  %digit24 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit23, %digit24
  store i8 %add, ptr %assignment_value25, align 1
  %assignment_value26 = load i8, ptr %assignment_value25, align 1
  %index27 = load i64, ptr %index, align 4
  %array_element28 = getelementptr inbounds i8, ptr %allocation, i64 %index27
  store i8 %assignment_value26, ptr %array_element28, align 1
  %remaining29 = load i32, ptr %remaining, align 4
  %ten30 = load i32, ptr %ten, align 4
  %div = udiv i32 %remaining29, %ten30
  store i32 %div, ptr %assignment_value31, align 4
  %assignment_value32 = load i32, ptr %assignment_value31, align 4
  store i32 %assignment_value32, ptr %remaining, align 4
  br label %while.cond.0

while.exit.2:                                     ; preds = %while.cond.0
  %capacity33 = load i64, ptr %capacity, align 4
  %index34 = load i64, ptr %index, align 4
  %sub35 = sub i64 %capacity33, %index34
  store i64 %sub35, ptr %length, align 4
  %zero36 = load i64, ptr %zero, align 4
  store i64 %zero36, ptr %target, align 4
  %index37 = load i64, ptr %index, align 4
  store i64 %index37, ptr %source, align 4
  br label %while.cond.3

while.cond.3:                                     ; preds = %while.body.4, %while.exit.2
  %source38 = load i64, ptr %source, align 4
  %capacity39 = load i64, ptr %capacity, align 4
  %lt = icmp ult i64 %source38, %capacity39
  br i1 %lt, label %while.body.4, label %while.exit.5

while.body.4:                                     ; preds = %while.cond.3
  %source40 = load i64, ptr %source, align 4
  %array_element41 = getelementptr inbounds i8, ptr %allocation, i64 %source40
  %place = load i8, ptr %array_element41, align 1
  store i8 %place, ptr %assignment_value42, align 1
  %assignment_value43 = load i8, ptr %assignment_value42, align 1
  %target44 = load i64, ptr %target, align 4
  %array_element45 = getelementptr inbounds i8, ptr %allocation, i64 %target44
  store i8 %assignment_value43, ptr %array_element45, align 1
  %target46 = load i64, ptr %target, align 4
  %one47 = load i64, ptr %one, align 4
  %add48 = add i64 %target46, %one47
  store i64 %add48, ptr %assignment_value49, align 4
  %assignment_value50 = load i64, ptr %assignment_value49, align 4
  store i64 %assignment_value50, ptr %target, align 4
  %source51 = load i64, ptr %source, align 4
  %one52 = load i64, ptr %one, align 4
  %add53 = add i64 %source51, %one52
  store i64 %add53, ptr %assignment_value54, align 4
  %assignment_value55 = load i64, ptr %assignment_value54, align 4
  store i64 %assignment_value55, ptr %source, align 4
  br label %while.cond.3

while.exit.5:                                     ; preds = %while.cond.3
  store i32 0, ptr %data, align 4
  %array_element56 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element56 to i32
  store i32 %address, ptr %assignment_value57, align 4
  %assignment_value58 = load i32, ptr %assignment_value57, align 4
  store i32 %assignment_value58, ptr %data, align 4
  %data59 = load i32, ptr %data, align 4
  %length60 = load i64, ptr %length, align 4
  %data61 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data59, ptr %data61, align 4
  %length62 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length60, ptr %length62, align 4
  %struct_value = load [16 x i8], ptr %struct_literal, align 1
  store [16 x i8] %struct_value, ptr %text, align 1
  ret ptr %text
}

define ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f753634(i64 %value) {
entry:
  %text = alloca [16 x i8], align 1
  %struct_literal = alloca [16 x i8], align 1
  %assignment_value57 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value54 = alloca i64, align 8
  %assignment_value49 = alloca i64, align 8
  %assignment_value42 = alloca i8, align 1
  %source = alloca i64, align 8
  %target = alloca i64, align 8
  %length = alloca i64, align 8
  %assignment_value31 = alloca i64, align 8
  %assignment_value25 = alloca i8, align 1
  %assignment_value21 = alloca i64, align 8
  %digit = alloca i8, align 1
  %digit_wide = alloca i64, align 8
  %assignment_value10 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %remaining = alloca i64, align 8
  %index = alloca i64, align 8
  %zero_value = alloca i64, align 8
  %zero_digit = alloca i8, align 1
  %ten = alloca i64, align 8
  %zero = alloca i64, align 8
  %one = alloca i64, align 8
  %capacity = alloca i64, align 8
  %value1 = alloca i64, align 8
  store i64 %value, ptr %value1, align 4
  %allocation = call ptr @__wosy_core_alloc(i64 20, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  store i64 20, ptr %capacity, align 4
  store i64 1, ptr %one, align 4
  store i64 0, ptr %zero, align 4
  store i64 10, ptr %ten, align 4
  store i8 48, ptr %zero_digit, align 1
  store i64 0, ptr %zero_value, align 4
  %capacity2 = load i64, ptr %capacity, align 4
  store i64 %capacity2, ptr %index, align 4
  %value3 = load i64, ptr %value1, align 4
  store i64 %value3, ptr %remaining, align 4
  %remaining4 = load i64, ptr %remaining, align 4
  %zero_value5 = load i64, ptr %zero_value, align 4
  %eq = icmp eq i64 %remaining4, %zero_value5
  br i1 %eq, label %if.then, label %if.merge

if.then:                                          ; preds = %allocation_continue
  %index6 = load i64, ptr %index, align 4
  %one7 = load i64, ptr %one, align 4
  %sub = sub i64 %index6, %one7
  store i64 %sub, ptr %assignment_value, align 4
  %assignment_value8 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value8, ptr %index, align 4
  %zero_digit9 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit9, ptr %assignment_value10, align 1
  %assignment_value11 = load i8, ptr %assignment_value10, align 1
  %index12 = load i64, ptr %index, align 4
  %array_element = getelementptr inbounds i8, ptr %allocation, i64 %index12
  store i8 %assignment_value11, ptr %array_element, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %allocation_continue
  br label %while.cond.0

while.cond.0:                                     ; preds = %while.body.1, %if.merge
  %remaining13 = load i64, ptr %remaining, align 4
  %zero_value14 = load i64, ptr %zero_value, align 4
  %ne = icmp ne i64 %remaining13, %zero_value14
  br i1 %ne, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %remaining15 = load i64, ptr %remaining, align 4
  %ten16 = load i64, ptr %ten, align 4
  %rem = urem i64 %remaining15, %ten16
  store i64 %rem, ptr %digit_wide, align 4
  %digit_wide17 = load i64, ptr %digit_wide, align 4
  %int_trunc = trunc i64 %digit_wide17 to i8
  store i8 %int_trunc, ptr %digit, align 1
  %index18 = load i64, ptr %index, align 4
  %one19 = load i64, ptr %one, align 4
  %sub20 = sub i64 %index18, %one19
  store i64 %sub20, ptr %assignment_value21, align 4
  %assignment_value22 = load i64, ptr %assignment_value21, align 4
  store i64 %assignment_value22, ptr %index, align 4
  %zero_digit23 = load i8, ptr %zero_digit, align 1
  %digit24 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit23, %digit24
  store i8 %add, ptr %assignment_value25, align 1
  %assignment_value26 = load i8, ptr %assignment_value25, align 1
  %index27 = load i64, ptr %index, align 4
  %array_element28 = getelementptr inbounds i8, ptr %allocation, i64 %index27
  store i8 %assignment_value26, ptr %array_element28, align 1
  %remaining29 = load i64, ptr %remaining, align 4
  %ten30 = load i64, ptr %ten, align 4
  %div = udiv i64 %remaining29, %ten30
  store i64 %div, ptr %assignment_value31, align 4
  %assignment_value32 = load i64, ptr %assignment_value31, align 4
  store i64 %assignment_value32, ptr %remaining, align 4
  br label %while.cond.0

while.exit.2:                                     ; preds = %while.cond.0
  %capacity33 = load i64, ptr %capacity, align 4
  %index34 = load i64, ptr %index, align 4
  %sub35 = sub i64 %capacity33, %index34
  store i64 %sub35, ptr %length, align 4
  %zero36 = load i64, ptr %zero, align 4
  store i64 %zero36, ptr %target, align 4
  %index37 = load i64, ptr %index, align 4
  store i64 %index37, ptr %source, align 4
  br label %while.cond.3

while.cond.3:                                     ; preds = %while.body.4, %while.exit.2
  %source38 = load i64, ptr %source, align 4
  %capacity39 = load i64, ptr %capacity, align 4
  %lt = icmp ult i64 %source38, %capacity39
  br i1 %lt, label %while.body.4, label %while.exit.5

while.body.4:                                     ; preds = %while.cond.3
  %source40 = load i64, ptr %source, align 4
  %array_element41 = getelementptr inbounds i8, ptr %allocation, i64 %source40
  %place = load i8, ptr %array_element41, align 1
  store i8 %place, ptr %assignment_value42, align 1
  %assignment_value43 = load i8, ptr %assignment_value42, align 1
  %target44 = load i64, ptr %target, align 4
  %array_element45 = getelementptr inbounds i8, ptr %allocation, i64 %target44
  store i8 %assignment_value43, ptr %array_element45, align 1
  %target46 = load i64, ptr %target, align 4
  %one47 = load i64, ptr %one, align 4
  %add48 = add i64 %target46, %one47
  store i64 %add48, ptr %assignment_value49, align 4
  %assignment_value50 = load i64, ptr %assignment_value49, align 4
  store i64 %assignment_value50, ptr %target, align 4
  %source51 = load i64, ptr %source, align 4
  %one52 = load i64, ptr %one, align 4
  %add53 = add i64 %source51, %one52
  store i64 %add53, ptr %assignment_value54, align 4
  %assignment_value55 = load i64, ptr %assignment_value54, align 4
  store i64 %assignment_value55, ptr %source, align 4
  br label %while.cond.3

while.exit.5:                                     ; preds = %while.cond.3
  store i32 0, ptr %data, align 4
  %array_element56 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element56 to i32
  store i32 %address, ptr %assignment_value57, align 4
  %assignment_value58 = load i32, ptr %assignment_value57, align 4
  store i32 %assignment_value58, ptr %data, align 4
  %data59 = load i32, ptr %data, align 4
  %length60 = load i64, ptr %length, align 4
  %data61 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data59, ptr %data61, align 4
  %length62 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length60, ptr %length62, align 4
  %struct_value = load [16 x i8], ptr %struct_literal, align 1
  store [16 x i8] %struct_value, ptr %text, align 1
  ret ptr %text
}

define ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f75313238(i128 %value) {
entry:
  %text = alloca [16 x i8], align 1
  %struct_literal = alloca [16 x i8], align 1
  %assignment_value69 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value66 = alloca i64, align 8
  %assignment_value61 = alloca i1, align 1
  %assignment_value59 = alloca i64, align 8
  %assignment_value53 = alloca i8, align 1
  %assignment_value42 = alloca i8, align 1
  %assignment_value37 = alloca i128, align 8
  %digit = alloca i8, align 1
  %assignment_value29 = alloca i64, align 8
  %assignment_value = alloca i128, align 8
  %eighth = alloca i128, align 8
  %fourth = alloca i128, align 8
  %twice = alloca i128, align 8
  %step = alloca i64, align 8
  %scale = alloca i128, align 8
  %steps = alloca i64, align 8
  %position = alloca i64, align 8
  %started = alloca i1, align 1
  %remaining = alloca i128, align 8
  %emit = alloca i64, align 8
  %one_wide = alloca i128, align 8
  %one_digit = alloca i8, align 1
  %zero_digit = alloca i8, align 1
  %last = alloca i64, align 8
  %zero = alloca i64, align 8
  %one = alloca i64, align 8
  %capacity = alloca i64, align 8
  %value1 = alloca i128, align 8
  store i128 %value, ptr %value1, align 4
  %allocation = call ptr @__wosy_core_alloc(i64 39, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  store i64 39, ptr %capacity, align 4
  store i64 1, ptr %one, align 4
  store i64 0, ptr %zero, align 4
  store i64 38, ptr %last, align 4
  store i8 48, ptr %zero_digit, align 1
  store i8 1, ptr %one_digit, align 1
  store i128 1, ptr %one_wide, align 4
  %zero2 = load i64, ptr %zero, align 4
  store i64 %zero2, ptr %emit, align 4
  %value3 = load i128, ptr %value1, align 4
  store i128 %value3, ptr %remaining, align 4
  store i1 false, ptr %started, align 1
  %zero4 = load i64, ptr %zero, align 4
  store i64 %zero4, ptr %position, align 4
  br label %while.cond.0

while.cond.0:                                     ; preds = %if.merge, %allocation_continue
  %position5 = load i64, ptr %position, align 4
  %capacity6 = load i64, ptr %capacity, align 4
  %lt = icmp ult i64 %position5, %capacity6
  br i1 %lt, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %last7 = load i64, ptr %last, align 4
  %position8 = load i64, ptr %position, align 4
  %sub = sub i64 %last7, %position8
  store i64 %sub, ptr %steps, align 4
  %one_wide9 = load i128, ptr %one_wide, align 4
  store i128 %one_wide9, ptr %scale, align 4
  %zero10 = load i64, ptr %zero, align 4
  store i64 %zero10, ptr %step, align 4
  br label %while.cond.3

while.exit.2:                                     ; preds = %while.cond.0
  store i32 0, ptr %data, align 4
  %array_element68 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element68 to i32
  store i32 %address, ptr %assignment_value69, align 4
  %assignment_value70 = load i32, ptr %assignment_value69, align 4
  store i32 %assignment_value70, ptr %data, align 4
  %data71 = load i32, ptr %data, align 4
  %emit72 = load i64, ptr %emit, align 4
  %data73 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data71, ptr %data73, align 4
  %length = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %emit72, ptr %length, align 4
  %struct_value = load [16 x i8], ptr %struct_literal, align 1
  store [16 x i8] %struct_value, ptr %text, align 1
  ret ptr %text

while.cond.3:                                     ; preds = %while.body.4, %while.body.1
  %step11 = load i64, ptr %step, align 4
  %steps12 = load i64, ptr %steps, align 4
  %lt13 = icmp ult i64 %step11, %steps12
  br i1 %lt13, label %while.body.4, label %while.exit.5

while.body.4:                                     ; preds = %while.cond.3
  %scale14 = load i128, ptr %scale, align 4
  %scale15 = load i128, ptr %scale, align 4
  %add = add i128 %scale14, %scale15
  store i128 %add, ptr %twice, align 4
  %twice16 = load i128, ptr %twice, align 4
  %twice17 = load i128, ptr %twice, align 4
  %add18 = add i128 %twice16, %twice17
  store i128 %add18, ptr %fourth, align 4
  %fourth19 = load i128, ptr %fourth, align 4
  %fourth20 = load i128, ptr %fourth, align 4
  %add21 = add i128 %fourth19, %fourth20
  store i128 %add21, ptr %eighth, align 4
  %eighth22 = load i128, ptr %eighth, align 4
  %twice23 = load i128, ptr %twice, align 4
  %add24 = add i128 %eighth22, %twice23
  store i128 %add24, ptr %assignment_value, align 4
  %assignment_value25 = load i128, ptr %assignment_value, align 4
  store i128 %assignment_value25, ptr %scale, align 4
  %step26 = load i64, ptr %step, align 4
  %one27 = load i64, ptr %one, align 4
  %add28 = add i64 %step26, %one27
  store i64 %add28, ptr %assignment_value29, align 4
  %assignment_value30 = load i64, ptr %assignment_value29, align 4
  store i64 %assignment_value30, ptr %step, align 4
  br label %while.cond.3

while.exit.5:                                     ; preds = %while.cond.3
  %zero_digit31 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit31, ptr %digit, align 1
  br label %while.cond.6

while.cond.6:                                     ; preds = %while.body.7, %while.exit.5
  %remaining32 = load i128, ptr %remaining, align 4
  %scale33 = load i128, ptr %scale, align 4
  %ge = icmp uge i128 %remaining32, %scale33
  br i1 %ge, label %while.body.7, label %while.exit.8

while.body.7:                                     ; preds = %while.cond.6
  %remaining34 = load i128, ptr %remaining, align 4
  %scale35 = load i128, ptr %scale, align 4
  %sub36 = sub i128 %remaining34, %scale35
  store i128 %sub36, ptr %assignment_value37, align 4
  %assignment_value38 = load i128, ptr %assignment_value37, align 4
  store i128 %assignment_value38, ptr %remaining, align 4
  %digit39 = load i8, ptr %digit, align 1
  %one_digit40 = load i8, ptr %one_digit, align 1
  %add41 = add i8 %digit39, %one_digit40
  store i8 %add41, ptr %assignment_value42, align 1
  %assignment_value43 = load i8, ptr %assignment_value42, align 1
  store i8 %assignment_value43, ptr %digit, align 1
  br label %while.cond.6

while.exit.8:                                     ; preds = %while.cond.6
  %started44 = load i1, ptr %started, align 1
  br i1 %started44, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %while.exit.8
  %digit45 = load i8, ptr %digit, align 1
  %zero_digit46 = load i8, ptr %zero_digit, align 1
  %ne = icmp ne i8 %digit45, %zero_digit46
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %while.exit.8
  %short_circuit = phi i1 [ true, %while.exit.8 ], [ %ne, %short_circuit.rhs ]
  br i1 %short_circuit, label %short_circuit.merge48, label %short_circuit.rhs47

short_circuit.rhs47:                              ; preds = %short_circuit.merge
  %position49 = load i64, ptr %position, align 4
  %last50 = load i64, ptr %last, align 4
  %eq = icmp eq i64 %position49, %last50
  br label %short_circuit.merge48

short_circuit.merge48:                            ; preds = %short_circuit.rhs47, %short_circuit.merge
  %short_circuit51 = phi i1 [ true, %short_circuit.merge ], [ %eq, %short_circuit.rhs47 ]
  br i1 %short_circuit51, label %if.then, label %if.merge

if.then:                                          ; preds = %short_circuit.merge48
  %digit52 = load i8, ptr %digit, align 1
  store i8 %digit52, ptr %assignment_value53, align 1
  %assignment_value54 = load i8, ptr %assignment_value53, align 1
  %emit55 = load i64, ptr %emit, align 4
  %array_element = getelementptr inbounds i8, ptr %allocation, i64 %emit55
  store i8 %assignment_value54, ptr %array_element, align 1
  %emit56 = load i64, ptr %emit, align 4
  %one57 = load i64, ptr %one, align 4
  %add58 = add i64 %emit56, %one57
  store i64 %add58, ptr %assignment_value59, align 4
  %assignment_value60 = load i64, ptr %assignment_value59, align 4
  store i64 %assignment_value60, ptr %emit, align 4
  store i1 true, ptr %assignment_value61, align 1
  %assignment_value62 = load i1, ptr %assignment_value61, align 1
  store i1 %assignment_value62, ptr %started, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %short_circuit.merge48
  %position63 = load i64, ptr %position, align 4
  %one64 = load i64, ptr %one, align 4
  %add65 = add i64 %position63, %one64
  store i64 %add65, ptr %assignment_value66, align 4
  %assignment_value67 = load i64, ptr %assignment_value66, align 4
  store i64 %assignment_value67, ptr %position, align 4
  br label %while.cond.0
}

define ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f663332(float %value) {
entry:
  %text = alloca [16 x i8], align 1
  %struct_literal = alloca [16 x i8], align 1
  %assignment_value342 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value339 = alloca i64, align 8
  %assignment_value332 = alloca i8, align 1
  %one_digit_out = alloca i8, align 1
  %assignment_value325 = alloca i64, align 8
  %assignment_value318 = alloca i8, align 1
  %ten_digit = alloca i8, align 1
  %assignment_value308 = alloca i64, align 8
  %assignment_value301 = alloca i8, align 1
  %hundred_digit = alloca i8, align 1
  %assignment_value289 = alloca i1, align 1
  %assignment_value282 = alloca i1, align 1
  %show_tens = alloca i1, align 1
  %ones = alloca i32, align 4
  %tens = alloca i32, align 4
  %after_hundreds = alloca i32, align 4
  %hundreds = alloca i32, align 4
  %ten_scale = alloca i32, align 4
  %hundred_scale = alloca i32, align 4
  %assignment_value260 = alloca i32, align 4
  %assignment_value255 = alloca i64, align 8
  %assignment_value248 = alloca i8, align 1
  %remaining_exponent = alloca i32, align 4
  %assignment_value239 = alloca i64, align 8
  %assignment_value232 = alloca i8, align 1
  %assignment_value229 = alloca i32, align 4
  %assignment_value222 = alloca i8, align 1
  %leading = alloca i8, align 1
  %assignment_value211 = alloca i1, align 1
  %assignment_value207 = alloca i8, align 1
  %assignment_value199 = alloca i1, align 1
  %assignment_value195 = alloca i8, align 1
  %assignment_value189 = alloca i64, align 8
  %assignment_value182 = alloca i8, align 1
  %carry_done = alloca i1, align 1
  %carry_stop = alloca i1, align 1
  %carry_back = alloca i1, align 1
  %assignment_value152 = alloca i64, align 8
  %at_start = alloca i1, align 1
  %is_nine = alloca i1, align 1
  %is_point = alloca i1, align 1
  %current = alloca i8, align 1
  %rounding = alloca i1, align 1
  %round_position = alloca i64, align 8
  %assignment_value124 = alloca i64, align 8
  %assignment_value119 = alloca double, align 8
  %assignment_value114 = alloca double, align 8
  %whole = alloca double, align 8
  %assignment_value108 = alloca i64, align 8
  %assignment_value101 = alloca i8, align 1
  %assignment_value93 = alloca i64, align 8
  %assignment_value86 = alloca i8, align 1
  %digit = alloca i8, align 1
  %digit_value = alloca i32, align 4
  %produced = alloca i64, align 8
  %digit_start = alloca i64, align 8
  %assignment_value74 = alloca i32, align 4
  %assignment_value69 = alloca double, align 8
  %assignment_value62 = alloca i32, align 4
  %assignment_value57 = alloca double, align 8
  %exponent = alloca i32, align 4
  %scaled = alloca double, align 8
  %assignment_value48 = alloca i64, align 8
  %assignment_value41 = alloca i8, align 1
  %assignment_value38 = alloca i64, align 8
  %assignment_value32 = alloca i8, align 1
  %probe = alloca double, align 8
  %assignment_value18 = alloca i64, align 8
  %assignment_value15 = alloca i8, align 1
  %emit = alloca i64, align 8
  %assignment_value = alloca double, align 8
  %magnitude = alloca double, align 8
  %negative = alloca i1, align 1
  %wide = alloca double, align 8
  %one_count = alloca i32, align 4
  %zero_count = alloca i32, align 4
  %exponent_mark = alloca i8, align 1
  %point = alloca i8, align 1
  %minus = alloca i8, align 1
  %one_char = alloca i8, align 1
  %nine_digit = alloca i8, align 1
  %one_digit = alloca i8, align 1
  %zero_digit = alloca i8, align 1
  %five_value = alloca double, align 8
  %ten_value = alloca double, align 8
  %one_value = alloca double, align 8
  %zero_value = alloca double, align 8
  %nine = alloca i64, align 8
  %zero = alloca i64, align 8
  %one = alloca i64, align 8
  %value1 = alloca float, align 4
  store float %value, ptr %value1, align 4
  %allocation = call ptr @__wosy_core_alloc(i64 15, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  store i64 1, ptr %one, align 4
  store i64 0, ptr %zero, align 4
  store i64 9, ptr %nine, align 4
  store double 0.000000e+00, ptr %zero_value, align 8
  store double 1.000000e+00, ptr %one_value, align 8
  store double 1.000000e+01, ptr %ten_value, align 8
  store double 5.000000e+00, ptr %five_value, align 8
  store i8 48, ptr %zero_digit, align 1
  store i8 1, ptr %one_digit, align 1
  store i8 57, ptr %nine_digit, align 1
  store i8 49, ptr %one_char, align 1
  store i8 45, ptr %minus, align 1
  store i8 46, ptr %point, align 1
  store i8 101, ptr %exponent_mark, align 1
  store i32 0, ptr %zero_count, align 4
  store i32 1, ptr %one_count, align 4
  %value2 = load float, ptr %value1, align 4
  %float_extend = fpext float %value2 to double
  store double %float_extend, ptr %wide, align 8
  %wide3 = load double, ptr %wide, align 8
  %zero_value4 = load double, ptr %zero_value, align 8
  %lt = fcmp olt double %wide3, %zero_value4
  store i1 %lt, ptr %negative, align 1
  %wide5 = load double, ptr %wide, align 8
  store double %wide5, ptr %magnitude, align 8
  %negative6 = load i1, ptr %negative, align 1
  br i1 %negative6, label %if.then, label %if.merge

if.then:                                          ; preds = %allocation_continue
  %zero_value7 = load double, ptr %zero_value, align 8
  %wide8 = load double, ptr %wide, align 8
  %sub = fsub double %zero_value7, %wide8
  store double %sub, ptr %assignment_value, align 8
  %assignment_value9 = load double, ptr %assignment_value, align 8
  store double %assignment_value9, ptr %magnitude, align 8
  br label %if.merge

if.merge:                                         ; preds = %if.then, %allocation_continue
  %zero10 = load i64, ptr %zero, align 4
  store i64 %zero10, ptr %emit, align 4
  %negative11 = load i1, ptr %negative, align 1
  br i1 %negative11, label %if.then12, label %if.merge13

if.then12:                                        ; preds = %if.merge
  %minus14 = load i8, ptr %minus, align 1
  store i8 %minus14, ptr %assignment_value15, align 1
  %assignment_value16 = load i8, ptr %assignment_value15, align 1
  %array_element = getelementptr inbounds i8, ptr %allocation, i64 0
  store i8 %assignment_value16, ptr %array_element, align 1
  %one17 = load i64, ptr %one, align 4
  store i64 %one17, ptr %assignment_value18, align 4
  %assignment_value19 = load i64, ptr %assignment_value18, align 4
  store i64 %assignment_value19, ptr %emit, align 4
  br label %if.merge13

if.merge13:                                       ; preds = %if.then12, %if.merge
  %magnitude20 = load double, ptr %magnitude, align 8
  %zero_value21 = load double, ptr %zero_value, align 8
  %eq = fcmp oeq double %magnitude20, %zero_value21
  br i1 %eq, label %if.then22, label %if.else

if.then22:                                        ; preds = %if.merge13
  %one_value24 = load double, ptr %one_value, align 8
  %magnitude25 = load double, ptr %magnitude, align 8
  %div = fdiv double %one_value24, %magnitude25
  store double %div, ptr %probe, align 8
  %probe26 = load double, ptr %probe, align 8
  %zero_value27 = load double, ptr %zero_value, align 8
  %lt28 = fcmp olt double %probe26, %zero_value27
  br i1 %lt28, label %if.then29, label %if.merge30

if.else:                                          ; preds = %if.merge13
  %magnitude50 = load double, ptr %magnitude, align 8
  store double %magnitude50, ptr %scaled, align 8
  %zero_count51 = load i32, ptr %zero_count, align 4
  store i32 %zero_count51, ptr %exponent, align 4
  br label %while.cond.0

if.merge23:                                       ; preds = %if.merge312, %if.merge30
  store i32 0, ptr %data, align 4
  %array_element341 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element341 to i32
  store i32 %address, ptr %assignment_value342, align 4
  %assignment_value343 = load i32, ptr %assignment_value342, align 4
  store i32 %assignment_value343, ptr %data, align 4
  %data344 = load i32, ptr %data, align 4
  %emit345 = load i64, ptr %emit, align 4
  %data346 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data344, ptr %data346, align 4
  %length = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %emit345, ptr %length, align 4
  %struct_value = load [16 x i8], ptr %struct_literal, align 1
  store [16 x i8] %struct_value, ptr %text, align 1
  ret ptr %text

if.then29:                                        ; preds = %if.then22
  %minus31 = load i8, ptr %minus, align 1
  store i8 %minus31, ptr %assignment_value32, align 1
  %assignment_value33 = load i8, ptr %assignment_value32, align 1
  %emit34 = load i64, ptr %emit, align 4
  %array_element35 = getelementptr inbounds i8, ptr %allocation, i64 %emit34
  store i8 %assignment_value33, ptr %array_element35, align 1
  %emit36 = load i64, ptr %emit, align 4
  %one37 = load i64, ptr %one, align 4
  %add = add i64 %emit36, %one37
  store i64 %add, ptr %assignment_value38, align 4
  %assignment_value39 = load i64, ptr %assignment_value38, align 4
  store i64 %assignment_value39, ptr %emit, align 4
  br label %if.merge30

if.merge30:                                       ; preds = %if.then29, %if.then22
  %zero_digit40 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit40, ptr %assignment_value41, align 1
  %assignment_value42 = load i8, ptr %assignment_value41, align 1
  %emit43 = load i64, ptr %emit, align 4
  %array_element44 = getelementptr inbounds i8, ptr %allocation, i64 %emit43
  store i8 %assignment_value42, ptr %array_element44, align 1
  %emit45 = load i64, ptr %emit, align 4
  %one46 = load i64, ptr %one, align 4
  %add47 = add i64 %emit45, %one46
  store i64 %add47, ptr %assignment_value48, align 4
  %assignment_value49 = load i64, ptr %assignment_value48, align 4
  store i64 %assignment_value49, ptr %emit, align 4
  br label %if.merge23

while.cond.0:                                     ; preds = %while.body.1, %if.else
  %scaled52 = load double, ptr %scaled, align 8
  %ten_value53 = load double, ptr %ten_value, align 8
  %ge = fcmp oge double %scaled52, %ten_value53
  br i1 %ge, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %scaled54 = load double, ptr %scaled, align 8
  %ten_value55 = load double, ptr %ten_value, align 8
  %div56 = fdiv double %scaled54, %ten_value55
  store double %div56, ptr %assignment_value57, align 8
  %assignment_value58 = load double, ptr %assignment_value57, align 8
  store double %assignment_value58, ptr %scaled, align 8
  %exponent59 = load i32, ptr %exponent, align 4
  %one_count60 = load i32, ptr %one_count, align 4
  %add61 = add i32 %exponent59, %one_count60
  store i32 %add61, ptr %assignment_value62, align 4
  %assignment_value63 = load i32, ptr %assignment_value62, align 4
  store i32 %assignment_value63, ptr %exponent, align 4
  br label %while.cond.0

while.exit.2:                                     ; preds = %while.cond.0
  br label %while.cond.3

while.cond.3:                                     ; preds = %while.body.4, %while.exit.2
  %scaled64 = load double, ptr %scaled, align 8
  %one_value65 = load double, ptr %one_value, align 8
  %lt66 = fcmp olt double %scaled64, %one_value65
  br i1 %lt66, label %while.body.4, label %while.exit.5

while.body.4:                                     ; preds = %while.cond.3
  %scaled67 = load double, ptr %scaled, align 8
  %ten_value68 = load double, ptr %ten_value, align 8
  %mul = fmul double %scaled67, %ten_value68
  store double %mul, ptr %assignment_value69, align 8
  %assignment_value70 = load double, ptr %assignment_value69, align 8
  store double %assignment_value70, ptr %scaled, align 8
  %exponent71 = load i32, ptr %exponent, align 4
  %one_count72 = load i32, ptr %one_count, align 4
  %sub73 = sub i32 %exponent71, %one_count72
  store i32 %sub73, ptr %assignment_value74, align 4
  %assignment_value75 = load i32, ptr %assignment_value74, align 4
  store i32 %assignment_value75, ptr %exponent, align 4
  br label %while.cond.3

while.exit.5:                                     ; preds = %while.cond.3
  %emit76 = load i64, ptr %emit, align 4
  store i64 %emit76, ptr %digit_start, align 4
  %zero77 = load i64, ptr %zero, align 4
  store i64 %zero77, ptr %produced, align 4
  br label %while.cond.6

while.cond.6:                                     ; preds = %if.merge99, %while.exit.5
  %produced78 = load i64, ptr %produced, align 4
  %nine79 = load i64, ptr %nine, align 4
  %lt80 = icmp ult i64 %produced78, %nine79
  br i1 %lt80, label %while.body.7, label %while.exit.8

while.body.7:                                     ; preds = %while.cond.6
  %scaled81 = load double, ptr %scaled, align 8
  %float_to_sint_trunc = fptosi double %scaled81 to i32
  store i32 %float_to_sint_trunc, ptr %digit_value, align 4
  %digit_value82 = load i32, ptr %digit_value, align 4
  %int_trunc = trunc i32 %digit_value82 to i8
  store i8 %int_trunc, ptr %digit, align 1
  %zero_digit83 = load i8, ptr %zero_digit, align 1
  %digit84 = load i8, ptr %digit, align 1
  %add85 = add i8 %zero_digit83, %digit84
  store i8 %add85, ptr %assignment_value86, align 1
  %assignment_value87 = load i8, ptr %assignment_value86, align 1
  %emit88 = load i64, ptr %emit, align 4
  %array_element89 = getelementptr inbounds i8, ptr %allocation, i64 %emit88
  store i8 %assignment_value87, ptr %array_element89, align 1
  %emit90 = load i64, ptr %emit, align 4
  %one91 = load i64, ptr %one, align 4
  %add92 = add i64 %emit90, %one91
  store i64 %add92, ptr %assignment_value93, align 4
  %assignment_value94 = load i64, ptr %assignment_value93, align 4
  store i64 %assignment_value94, ptr %emit, align 4
  %produced95 = load i64, ptr %produced, align 4
  %zero96 = load i64, ptr %zero, align 4
  %eq97 = icmp eq i64 %produced95, %zero96
  br i1 %eq97, label %if.then98, label %if.merge99

while.exit.8:                                     ; preds = %while.cond.6
  %scaled126 = load double, ptr %scaled, align 8
  %five_value127 = load double, ptr %five_value, align 8
  %ge128 = fcmp oge double %scaled126, %five_value127
  br i1 %ge128, label %if.then129, label %if.merge130

if.then98:                                        ; preds = %while.body.7
  %point100 = load i8, ptr %point, align 1
  store i8 %point100, ptr %assignment_value101, align 1
  %assignment_value102 = load i8, ptr %assignment_value101, align 1
  %emit103 = load i64, ptr %emit, align 4
  %array_element104 = getelementptr inbounds i8, ptr %allocation, i64 %emit103
  store i8 %assignment_value102, ptr %array_element104, align 1
  %emit105 = load i64, ptr %emit, align 4
  %one106 = load i64, ptr %one, align 4
  %add107 = add i64 %emit105, %one106
  store i64 %add107, ptr %assignment_value108, align 4
  %assignment_value109 = load i64, ptr %assignment_value108, align 4
  store i64 %assignment_value109, ptr %emit, align 4
  br label %if.merge99

if.merge99:                                       ; preds = %if.then98, %while.body.7
  %digit_value110 = load i32, ptr %digit_value, align 4
  %sint_to_float = sitofp i32 %digit_value110 to double
  store double %sint_to_float, ptr %whole, align 8
  %scaled111 = load double, ptr %scaled, align 8
  %whole112 = load double, ptr %whole, align 8
  %sub113 = fsub double %scaled111, %whole112
  store double %sub113, ptr %assignment_value114, align 8
  %assignment_value115 = load double, ptr %assignment_value114, align 8
  store double %assignment_value115, ptr %scaled, align 8
  %scaled116 = load double, ptr %scaled, align 8
  %ten_value117 = load double, ptr %ten_value, align 8
  %mul118 = fmul double %scaled116, %ten_value117
  store double %mul118, ptr %assignment_value119, align 8
  %assignment_value120 = load double, ptr %assignment_value119, align 8
  store double %assignment_value120, ptr %scaled, align 8
  %produced121 = load i64, ptr %produced, align 4
  %one122 = load i64, ptr %one, align 4
  %add123 = add i64 %produced121, %one122
  store i64 %add123, ptr %assignment_value124, align 4
  %assignment_value125 = load i64, ptr %assignment_value124, align 4
  store i64 %assignment_value125, ptr %produced, align 4
  br label %while.cond.6

if.then129:                                       ; preds = %while.exit.8
  %emit131 = load i64, ptr %emit, align 4
  %one132 = load i64, ptr %one, align 4
  %sub133 = sub i64 %emit131, %one132
  store i64 %sub133, ptr %round_position, align 4
  store i1 true, ptr %rounding, align 1
  br label %while.cond.9

if.merge130:                                      ; preds = %if.merge220, %while.exit.8
  %exponent_mark231 = load i8, ptr %exponent_mark, align 1
  store i8 %exponent_mark231, ptr %assignment_value232, align 1
  %assignment_value233 = load i8, ptr %assignment_value232, align 1
  %emit234 = load i64, ptr %emit, align 4
  %array_element235 = getelementptr inbounds i8, ptr %allocation, i64 %emit234
  store i8 %assignment_value233, ptr %array_element235, align 1
  %emit236 = load i64, ptr %emit, align 4
  %one237 = load i64, ptr %one, align 4
  %add238 = add i64 %emit236, %one237
  store i64 %add238, ptr %assignment_value239, align 4
  %assignment_value240 = load i64, ptr %assignment_value239, align 4
  store i64 %assignment_value240, ptr %emit, align 4
  %exponent241 = load i32, ptr %exponent, align 4
  store i32 %exponent241, ptr %remaining_exponent, align 4
  %remaining_exponent242 = load i32, ptr %remaining_exponent, align 4
  %zero_count243 = load i32, ptr %zero_count, align 4
  %lt244 = icmp slt i32 %remaining_exponent242, %zero_count243
  br i1 %lt244, label %if.then245, label %if.merge246

while.cond.9:                                     ; preds = %if.merge203, %if.then129
  %rounding134 = load i1, ptr %rounding, align 1
  br i1 %rounding134, label %while.body.10, label %while.exit.11

while.body.10:                                    ; preds = %while.cond.9
  %round_position135 = load i64, ptr %round_position, align 4
  %array_element136 = getelementptr inbounds i8, ptr %allocation, i64 %round_position135
  %place = load i8, ptr %array_element136, align 1
  store i8 %place, ptr %current, align 1
  %current137 = load i8, ptr %current, align 1
  %point138 = load i8, ptr %point, align 1
  %eq139 = icmp eq i8 %current137, %point138
  store i1 %eq139, ptr %is_point, align 1
  %current140 = load i8, ptr %current, align 1
  %nine_digit141 = load i8, ptr %nine_digit, align 1
  %eq142 = icmp eq i8 %current140, %nine_digit141
  store i1 %eq142, ptr %is_nine, align 1
  %round_position143 = load i64, ptr %round_position, align 4
  %digit_start144 = load i64, ptr %digit_start, align 4
  %eq145 = icmp eq i64 %round_position143, %digit_start144
  store i1 %eq145, ptr %at_start, align 1
  %is_point146 = load i1, ptr %is_point, align 1
  br i1 %is_point146, label %if.then147, label %if.merge148

while.exit.11:                                    ; preds = %while.cond.9
  %digit_start213 = load i64, ptr %digit_start, align 4
  %array_element214 = getelementptr inbounds i8, ptr %allocation, i64 %digit_start213
  %place215 = load i8, ptr %array_element214, align 1
  store i8 %place215, ptr %leading, align 1
  %leading216 = load i8, ptr %leading, align 1
  %zero_digit217 = load i8, ptr %zero_digit, align 1
  %eq218 = icmp eq i8 %leading216, %zero_digit217
  br i1 %eq218, label %if.then219, label %if.merge220

if.then147:                                       ; preds = %while.body.10
  %round_position149 = load i64, ptr %round_position, align 4
  %one150 = load i64, ptr %one, align 4
  %sub151 = sub i64 %round_position149, %one150
  store i64 %sub151, ptr %assignment_value152, align 4
  %assignment_value153 = load i64, ptr %assignment_value152, align 4
  store i64 %assignment_value153, ptr %round_position, align 4
  br label %if.merge148

if.merge148:                                      ; preds = %if.then147, %while.body.10
  %is_point154 = load i1, ptr %is_point, align 1
  %not = xor i1 %is_point154, true
  br i1 %not, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %if.merge148
  %is_nine155 = load i1, ptr %is_nine, align 1
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %if.merge148
  %short_circuit = phi i1 [ false, %if.merge148 ], [ %is_nine155, %short_circuit.rhs ]
  br i1 %short_circuit, label %short_circuit.rhs156, label %short_circuit.merge157

short_circuit.rhs156:                             ; preds = %short_circuit.merge
  %at_start158 = load i1, ptr %at_start, align 1
  %not159 = xor i1 %at_start158, true
  br label %short_circuit.merge157

short_circuit.merge157:                           ; preds = %short_circuit.rhs156, %short_circuit.merge
  %short_circuit160 = phi i1 [ false, %short_circuit.merge ], [ %not159, %short_circuit.rhs156 ]
  store i1 %short_circuit160, ptr %carry_back, align 1
  %is_point161 = load i1, ptr %is_point, align 1
  %not162 = xor i1 %is_point161, true
  br i1 %not162, label %short_circuit.rhs163, label %short_circuit.merge164

short_circuit.rhs163:                             ; preds = %short_circuit.merge157
  %is_nine165 = load i1, ptr %is_nine, align 1
  br label %short_circuit.merge164

short_circuit.merge164:                           ; preds = %short_circuit.rhs163, %short_circuit.merge157
  %short_circuit166 = phi i1 [ false, %short_circuit.merge157 ], [ %is_nine165, %short_circuit.rhs163 ]
  br i1 %short_circuit166, label %short_circuit.rhs167, label %short_circuit.merge168

short_circuit.rhs167:                             ; preds = %short_circuit.merge164
  %at_start169 = load i1, ptr %at_start, align 1
  br label %short_circuit.merge168

short_circuit.merge168:                           ; preds = %short_circuit.rhs167, %short_circuit.merge164
  %short_circuit170 = phi i1 [ false, %short_circuit.merge164 ], [ %at_start169, %short_circuit.rhs167 ]
  store i1 %short_circuit170, ptr %carry_stop, align 1
  %is_point171 = load i1, ptr %is_point, align 1
  %not172 = xor i1 %is_point171, true
  br i1 %not172, label %short_circuit.rhs173, label %short_circuit.merge174

short_circuit.rhs173:                             ; preds = %short_circuit.merge168
  %is_nine175 = load i1, ptr %is_nine, align 1
  %not176 = xor i1 %is_nine175, true
  br label %short_circuit.merge174

short_circuit.merge174:                           ; preds = %short_circuit.rhs173, %short_circuit.merge168
  %short_circuit177 = phi i1 [ false, %short_circuit.merge168 ], [ %not176, %short_circuit.rhs173 ]
  store i1 %short_circuit177, ptr %carry_done, align 1
  %carry_back178 = load i1, ptr %carry_back, align 1
  br i1 %carry_back178, label %if.then179, label %if.merge180

if.then179:                                       ; preds = %short_circuit.merge174
  %zero_digit181 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit181, ptr %assignment_value182, align 1
  %assignment_value183 = load i8, ptr %assignment_value182, align 1
  %round_position184 = load i64, ptr %round_position, align 4
  %array_element185 = getelementptr inbounds i8, ptr %allocation, i64 %round_position184
  store i8 %assignment_value183, ptr %array_element185, align 1
  %round_position186 = load i64, ptr %round_position, align 4
  %one187 = load i64, ptr %one, align 4
  %sub188 = sub i64 %round_position186, %one187
  store i64 %sub188, ptr %assignment_value189, align 4
  %assignment_value190 = load i64, ptr %assignment_value189, align 4
  store i64 %assignment_value190, ptr %round_position, align 4
  br label %if.merge180

if.merge180:                                      ; preds = %if.then179, %short_circuit.merge174
  %carry_stop191 = load i1, ptr %carry_stop, align 1
  br i1 %carry_stop191, label %if.then192, label %if.merge193

if.then192:                                       ; preds = %if.merge180
  %zero_digit194 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit194, ptr %assignment_value195, align 1
  %assignment_value196 = load i8, ptr %assignment_value195, align 1
  %round_position197 = load i64, ptr %round_position, align 4
  %array_element198 = getelementptr inbounds i8, ptr %allocation, i64 %round_position197
  store i8 %assignment_value196, ptr %array_element198, align 1
  store i1 false, ptr %assignment_value199, align 1
  %assignment_value200 = load i1, ptr %assignment_value199, align 1
  store i1 %assignment_value200, ptr %rounding, align 1
  br label %if.merge193

if.merge193:                                      ; preds = %if.then192, %if.merge180
  %carry_done201 = load i1, ptr %carry_done, align 1
  br i1 %carry_done201, label %if.then202, label %if.merge203

if.then202:                                       ; preds = %if.merge193
  %current204 = load i8, ptr %current, align 1
  %one_digit205 = load i8, ptr %one_digit, align 1
  %add206 = add i8 %current204, %one_digit205
  store i8 %add206, ptr %assignment_value207, align 1
  %assignment_value208 = load i8, ptr %assignment_value207, align 1
  %round_position209 = load i64, ptr %round_position, align 4
  %array_element210 = getelementptr inbounds i8, ptr %allocation, i64 %round_position209
  store i8 %assignment_value208, ptr %array_element210, align 1
  store i1 false, ptr %assignment_value211, align 1
  %assignment_value212 = load i1, ptr %assignment_value211, align 1
  store i1 %assignment_value212, ptr %rounding, align 1
  br label %if.merge203

if.merge203:                                      ; preds = %if.then202, %if.merge193
  br label %while.cond.9

if.then219:                                       ; preds = %while.exit.11
  %one_char221 = load i8, ptr %one_char, align 1
  store i8 %one_char221, ptr %assignment_value222, align 1
  %assignment_value223 = load i8, ptr %assignment_value222, align 1
  %digit_start224 = load i64, ptr %digit_start, align 4
  %array_element225 = getelementptr inbounds i8, ptr %allocation, i64 %digit_start224
  store i8 %assignment_value223, ptr %array_element225, align 1
  %exponent226 = load i32, ptr %exponent, align 4
  %one_count227 = load i32, ptr %one_count, align 4
  %add228 = add i32 %exponent226, %one_count227
  store i32 %add228, ptr %assignment_value229, align 4
  %assignment_value230 = load i32, ptr %assignment_value229, align 4
  store i32 %assignment_value230, ptr %exponent, align 4
  br label %if.merge220

if.merge220:                                      ; preds = %if.then219, %while.exit.11
  br label %if.merge130

if.then245:                                       ; preds = %if.merge130
  %minus247 = load i8, ptr %minus, align 1
  store i8 %minus247, ptr %assignment_value248, align 1
  %assignment_value249 = load i8, ptr %assignment_value248, align 1
  %emit250 = load i64, ptr %emit, align 4
  %array_element251 = getelementptr inbounds i8, ptr %allocation, i64 %emit250
  store i8 %assignment_value249, ptr %array_element251, align 1
  %emit252 = load i64, ptr %emit, align 4
  %one253 = load i64, ptr %one, align 4
  %add254 = add i64 %emit252, %one253
  store i64 %add254, ptr %assignment_value255, align 4
  %assignment_value256 = load i64, ptr %assignment_value255, align 4
  store i64 %assignment_value256, ptr %emit, align 4
  %zero_count257 = load i32, ptr %zero_count, align 4
  %remaining_exponent258 = load i32, ptr %remaining_exponent, align 4
  %sub259 = sub i32 %zero_count257, %remaining_exponent258
  store i32 %sub259, ptr %assignment_value260, align 4
  %assignment_value261 = load i32, ptr %assignment_value260, align 4
  store i32 %assignment_value261, ptr %remaining_exponent, align 4
  br label %if.merge246

if.merge246:                                      ; preds = %if.then245, %if.merge130
  store i32 100, ptr %hundred_scale, align 4
  store i32 10, ptr %ten_scale, align 4
  %remaining_exponent262 = load i32, ptr %remaining_exponent, align 4
  %hundred_scale263 = load i32, ptr %hundred_scale, align 4
  %div264 = sdiv i32 %remaining_exponent262, %hundred_scale263
  store i32 %div264, ptr %hundreds, align 4
  %remaining_exponent265 = load i32, ptr %remaining_exponent, align 4
  %hundreds266 = load i32, ptr %hundreds, align 4
  %hundred_scale267 = load i32, ptr %hundred_scale, align 4
  %mul268 = mul i32 %hundreds266, %hundred_scale267
  %sub269 = sub i32 %remaining_exponent265, %mul268
  store i32 %sub269, ptr %after_hundreds, align 4
  %after_hundreds270 = load i32, ptr %after_hundreds, align 4
  %ten_scale271 = load i32, ptr %ten_scale, align 4
  %div272 = sdiv i32 %after_hundreds270, %ten_scale271
  store i32 %div272, ptr %tens, align 4
  %after_hundreds273 = load i32, ptr %after_hundreds, align 4
  %tens274 = load i32, ptr %tens, align 4
  %ten_scale275 = load i32, ptr %ten_scale, align 4
  %mul276 = mul i32 %tens274, %ten_scale275
  %sub277 = sub i32 %after_hundreds273, %mul276
  store i32 %sub277, ptr %ones, align 4
  store i1 false, ptr %show_tens, align 1
  %hundreds278 = load i32, ptr %hundreds, align 4
  %zero_count279 = load i32, ptr %zero_count, align 4
  %ne = icmp ne i32 %hundreds278, %zero_count279
  br i1 %ne, label %if.then280, label %if.merge281

if.then280:                                       ; preds = %if.merge246
  store i1 true, ptr %assignment_value282, align 1
  %assignment_value283 = load i1, ptr %assignment_value282, align 1
  store i1 %assignment_value283, ptr %show_tens, align 1
  br label %if.merge281

if.merge281:                                      ; preds = %if.then280, %if.merge246
  %tens284 = load i32, ptr %tens, align 4
  %zero_count285 = load i32, ptr %zero_count, align 4
  %ne286 = icmp ne i32 %tens284, %zero_count285
  br i1 %ne286, label %if.then287, label %if.merge288

if.then287:                                       ; preds = %if.merge281
  store i1 true, ptr %assignment_value289, align 1
  %assignment_value290 = load i1, ptr %assignment_value289, align 1
  store i1 %assignment_value290, ptr %show_tens, align 1
  br label %if.merge288

if.merge288:                                      ; preds = %if.then287, %if.merge281
  %hundreds291 = load i32, ptr %hundreds, align 4
  %zero_count292 = load i32, ptr %zero_count, align 4
  %ne293 = icmp ne i32 %hundreds291, %zero_count292
  br i1 %ne293, label %if.then294, label %if.merge295

if.then294:                                       ; preds = %if.merge288
  %hundreds296 = load i32, ptr %hundreds, align 4
  %int_trunc297 = trunc i32 %hundreds296 to i8
  store i8 %int_trunc297, ptr %hundred_digit, align 1
  %zero_digit298 = load i8, ptr %zero_digit, align 1
  %hundred_digit299 = load i8, ptr %hundred_digit, align 1
  %add300 = add i8 %zero_digit298, %hundred_digit299
  store i8 %add300, ptr %assignment_value301, align 1
  %assignment_value302 = load i8, ptr %assignment_value301, align 1
  %emit303 = load i64, ptr %emit, align 4
  %array_element304 = getelementptr inbounds i8, ptr %allocation, i64 %emit303
  store i8 %assignment_value302, ptr %array_element304, align 1
  %emit305 = load i64, ptr %emit, align 4
  %one306 = load i64, ptr %one, align 4
  %add307 = add i64 %emit305, %one306
  store i64 %add307, ptr %assignment_value308, align 4
  %assignment_value309 = load i64, ptr %assignment_value308, align 4
  store i64 %assignment_value309, ptr %emit, align 4
  br label %if.merge295

if.merge295:                                      ; preds = %if.then294, %if.merge288
  %show_tens310 = load i1, ptr %show_tens, align 1
  br i1 %show_tens310, label %if.then311, label %if.merge312

if.then311:                                       ; preds = %if.merge295
  %tens313 = load i32, ptr %tens, align 4
  %int_trunc314 = trunc i32 %tens313 to i8
  store i8 %int_trunc314, ptr %ten_digit, align 1
  %zero_digit315 = load i8, ptr %zero_digit, align 1
  %ten_digit316 = load i8, ptr %ten_digit, align 1
  %add317 = add i8 %zero_digit315, %ten_digit316
  store i8 %add317, ptr %assignment_value318, align 1
  %assignment_value319 = load i8, ptr %assignment_value318, align 1
  %emit320 = load i64, ptr %emit, align 4
  %array_element321 = getelementptr inbounds i8, ptr %allocation, i64 %emit320
  store i8 %assignment_value319, ptr %array_element321, align 1
  %emit322 = load i64, ptr %emit, align 4
  %one323 = load i64, ptr %one, align 4
  %add324 = add i64 %emit322, %one323
  store i64 %add324, ptr %assignment_value325, align 4
  %assignment_value326 = load i64, ptr %assignment_value325, align 4
  store i64 %assignment_value326, ptr %emit, align 4
  br label %if.merge312

if.merge312:                                      ; preds = %if.then311, %if.merge295
  %ones327 = load i32, ptr %ones, align 4
  %int_trunc328 = trunc i32 %ones327 to i8
  store i8 %int_trunc328, ptr %one_digit_out, align 1
  %zero_digit329 = load i8, ptr %zero_digit, align 1
  %one_digit_out330 = load i8, ptr %one_digit_out, align 1
  %add331 = add i8 %zero_digit329, %one_digit_out330
  store i8 %add331, ptr %assignment_value332, align 1
  %assignment_value333 = load i8, ptr %assignment_value332, align 1
  %emit334 = load i64, ptr %emit, align 4
  %array_element335 = getelementptr inbounds i8, ptr %allocation, i64 %emit334
  store i8 %assignment_value333, ptr %array_element335, align 1
  %emit336 = load i64, ptr %emit, align 4
  %one337 = load i64, ptr %one, align 4
  %add338 = add i64 %emit336, %one337
  store i64 %add338, ptr %assignment_value339, align 4
  %assignment_value340 = load i64, ptr %assignment_value339, align 4
  store i64 %assignment_value340, ptr %emit, align 4
  br label %if.merge23
}

define ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f663634(double %value) {
entry:
  %text = alloca [16 x i8], align 1
  %struct_literal = alloca [16 x i8], align 1
  %assignment_value341 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value338 = alloca i64, align 8
  %assignment_value331 = alloca i8, align 1
  %one_digit_out = alloca i8, align 1
  %assignment_value324 = alloca i64, align 8
  %assignment_value317 = alloca i8, align 1
  %ten_digit = alloca i8, align 1
  %assignment_value307 = alloca i64, align 8
  %assignment_value300 = alloca i8, align 1
  %hundred_digit = alloca i8, align 1
  %assignment_value288 = alloca i1, align 1
  %assignment_value281 = alloca i1, align 1
  %show_tens = alloca i1, align 1
  %ones = alloca i32, align 4
  %tens = alloca i32, align 4
  %after_hundreds = alloca i32, align 4
  %hundreds = alloca i32, align 4
  %ten_scale = alloca i32, align 4
  %hundred_scale = alloca i32, align 4
  %assignment_value259 = alloca i32, align 4
  %assignment_value254 = alloca i64, align 8
  %assignment_value247 = alloca i8, align 1
  %remaining_exponent = alloca i32, align 4
  %assignment_value238 = alloca i64, align 8
  %assignment_value231 = alloca i8, align 1
  %assignment_value228 = alloca i32, align 4
  %assignment_value221 = alloca i8, align 1
  %leading = alloca i8, align 1
  %assignment_value210 = alloca i1, align 1
  %assignment_value206 = alloca i8, align 1
  %assignment_value198 = alloca i1, align 1
  %assignment_value194 = alloca i8, align 1
  %assignment_value188 = alloca i64, align 8
  %assignment_value181 = alloca i8, align 1
  %carry_done = alloca i1, align 1
  %carry_stop = alloca i1, align 1
  %carry_back = alloca i1, align 1
  %assignment_value151 = alloca i64, align 8
  %at_start = alloca i1, align 1
  %is_nine = alloca i1, align 1
  %is_point = alloca i1, align 1
  %current = alloca i8, align 1
  %rounding = alloca i1, align 1
  %round_position = alloca i64, align 8
  %assignment_value123 = alloca i64, align 8
  %assignment_value118 = alloca double, align 8
  %assignment_value113 = alloca double, align 8
  %whole = alloca double, align 8
  %assignment_value107 = alloca i64, align 8
  %assignment_value100 = alloca i8, align 1
  %assignment_value92 = alloca i64, align 8
  %assignment_value85 = alloca i8, align 1
  %digit = alloca i8, align 1
  %digit_value = alloca i32, align 4
  %produced = alloca i64, align 8
  %digit_start = alloca i64, align 8
  %assignment_value73 = alloca i32, align 4
  %assignment_value68 = alloca double, align 8
  %assignment_value61 = alloca i32, align 4
  %assignment_value56 = alloca double, align 8
  %exponent = alloca i32, align 4
  %scaled = alloca double, align 8
  %assignment_value47 = alloca i64, align 8
  %assignment_value40 = alloca i8, align 1
  %assignment_value37 = alloca i64, align 8
  %assignment_value31 = alloca i8, align 1
  %probe = alloca double, align 8
  %assignment_value17 = alloca i64, align 8
  %assignment_value14 = alloca i8, align 1
  %emit = alloca i64, align 8
  %assignment_value = alloca double, align 8
  %magnitude = alloca double, align 8
  %negative = alloca i1, align 1
  %one_count = alloca i32, align 4
  %zero_count = alloca i32, align 4
  %exponent_mark = alloca i8, align 1
  %point = alloca i8, align 1
  %minus = alloca i8, align 1
  %one_char = alloca i8, align 1
  %nine_digit = alloca i8, align 1
  %one_digit = alloca i8, align 1
  %zero_digit = alloca i8, align 1
  %five_value = alloca double, align 8
  %ten_value = alloca double, align 8
  %one_value = alloca double, align 8
  %zero_value = alloca double, align 8
  %seventeen = alloca i64, align 8
  %zero = alloca i64, align 8
  %one = alloca i64, align 8
  %value1 = alloca double, align 8
  store double %value, ptr %value1, align 8
  %allocation = call ptr @__wosy_core_alloc(i64 24, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  store i64 1, ptr %one, align 4
  store i64 0, ptr %zero, align 4
  store i64 17, ptr %seventeen, align 4
  store double 0.000000e+00, ptr %zero_value, align 8
  store double 1.000000e+00, ptr %one_value, align 8
  store double 1.000000e+01, ptr %ten_value, align 8
  store double 5.000000e+00, ptr %five_value, align 8
  store i8 48, ptr %zero_digit, align 1
  store i8 1, ptr %one_digit, align 1
  store i8 57, ptr %nine_digit, align 1
  store i8 49, ptr %one_char, align 1
  store i8 45, ptr %minus, align 1
  store i8 46, ptr %point, align 1
  store i8 101, ptr %exponent_mark, align 1
  store i32 0, ptr %zero_count, align 4
  store i32 1, ptr %one_count, align 4
  %value2 = load double, ptr %value1, align 8
  %zero_value3 = load double, ptr %zero_value, align 8
  %lt = fcmp olt double %value2, %zero_value3
  store i1 %lt, ptr %negative, align 1
  %value4 = load double, ptr %value1, align 8
  store double %value4, ptr %magnitude, align 8
  %negative5 = load i1, ptr %negative, align 1
  br i1 %negative5, label %if.then, label %if.merge

if.then:                                          ; preds = %allocation_continue
  %zero_value6 = load double, ptr %zero_value, align 8
  %value7 = load double, ptr %value1, align 8
  %sub = fsub double %zero_value6, %value7
  store double %sub, ptr %assignment_value, align 8
  %assignment_value8 = load double, ptr %assignment_value, align 8
  store double %assignment_value8, ptr %magnitude, align 8
  br label %if.merge

if.merge:                                         ; preds = %if.then, %allocation_continue
  %zero9 = load i64, ptr %zero, align 4
  store i64 %zero9, ptr %emit, align 4
  %negative10 = load i1, ptr %negative, align 1
  br i1 %negative10, label %if.then11, label %if.merge12

if.then11:                                        ; preds = %if.merge
  %minus13 = load i8, ptr %minus, align 1
  store i8 %minus13, ptr %assignment_value14, align 1
  %assignment_value15 = load i8, ptr %assignment_value14, align 1
  %array_element = getelementptr inbounds i8, ptr %allocation, i64 0
  store i8 %assignment_value15, ptr %array_element, align 1
  %one16 = load i64, ptr %one, align 4
  store i64 %one16, ptr %assignment_value17, align 4
  %assignment_value18 = load i64, ptr %assignment_value17, align 4
  store i64 %assignment_value18, ptr %emit, align 4
  br label %if.merge12

if.merge12:                                       ; preds = %if.then11, %if.merge
  %magnitude19 = load double, ptr %magnitude, align 8
  %zero_value20 = load double, ptr %zero_value, align 8
  %eq = fcmp oeq double %magnitude19, %zero_value20
  br i1 %eq, label %if.then21, label %if.else

if.then21:                                        ; preds = %if.merge12
  %one_value23 = load double, ptr %one_value, align 8
  %magnitude24 = load double, ptr %magnitude, align 8
  %div = fdiv double %one_value23, %magnitude24
  store double %div, ptr %probe, align 8
  %probe25 = load double, ptr %probe, align 8
  %zero_value26 = load double, ptr %zero_value, align 8
  %lt27 = fcmp olt double %probe25, %zero_value26
  br i1 %lt27, label %if.then28, label %if.merge29

if.else:                                          ; preds = %if.merge12
  %magnitude49 = load double, ptr %magnitude, align 8
  store double %magnitude49, ptr %scaled, align 8
  %zero_count50 = load i32, ptr %zero_count, align 4
  store i32 %zero_count50, ptr %exponent, align 4
  br label %while.cond.0

if.merge22:                                       ; preds = %if.merge311, %if.merge29
  store i32 0, ptr %data, align 4
  %array_element340 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element340 to i32
  store i32 %address, ptr %assignment_value341, align 4
  %assignment_value342 = load i32, ptr %assignment_value341, align 4
  store i32 %assignment_value342, ptr %data, align 4
  %data343 = load i32, ptr %data, align 4
  %emit344 = load i64, ptr %emit, align 4
  %data345 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data343, ptr %data345, align 4
  %length = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %emit344, ptr %length, align 4
  %struct_value = load [16 x i8], ptr %struct_literal, align 1
  store [16 x i8] %struct_value, ptr %text, align 1
  ret ptr %text

if.then28:                                        ; preds = %if.then21
  %minus30 = load i8, ptr %minus, align 1
  store i8 %minus30, ptr %assignment_value31, align 1
  %assignment_value32 = load i8, ptr %assignment_value31, align 1
  %emit33 = load i64, ptr %emit, align 4
  %array_element34 = getelementptr inbounds i8, ptr %allocation, i64 %emit33
  store i8 %assignment_value32, ptr %array_element34, align 1
  %emit35 = load i64, ptr %emit, align 4
  %one36 = load i64, ptr %one, align 4
  %add = add i64 %emit35, %one36
  store i64 %add, ptr %assignment_value37, align 4
  %assignment_value38 = load i64, ptr %assignment_value37, align 4
  store i64 %assignment_value38, ptr %emit, align 4
  br label %if.merge29

if.merge29:                                       ; preds = %if.then28, %if.then21
  %zero_digit39 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit39, ptr %assignment_value40, align 1
  %assignment_value41 = load i8, ptr %assignment_value40, align 1
  %emit42 = load i64, ptr %emit, align 4
  %array_element43 = getelementptr inbounds i8, ptr %allocation, i64 %emit42
  store i8 %assignment_value41, ptr %array_element43, align 1
  %emit44 = load i64, ptr %emit, align 4
  %one45 = load i64, ptr %one, align 4
  %add46 = add i64 %emit44, %one45
  store i64 %add46, ptr %assignment_value47, align 4
  %assignment_value48 = load i64, ptr %assignment_value47, align 4
  store i64 %assignment_value48, ptr %emit, align 4
  br label %if.merge22

while.cond.0:                                     ; preds = %while.body.1, %if.else
  %scaled51 = load double, ptr %scaled, align 8
  %ten_value52 = load double, ptr %ten_value, align 8
  %ge = fcmp oge double %scaled51, %ten_value52
  br i1 %ge, label %while.body.1, label %while.exit.2

while.body.1:                                     ; preds = %while.cond.0
  %scaled53 = load double, ptr %scaled, align 8
  %ten_value54 = load double, ptr %ten_value, align 8
  %div55 = fdiv double %scaled53, %ten_value54
  store double %div55, ptr %assignment_value56, align 8
  %assignment_value57 = load double, ptr %assignment_value56, align 8
  store double %assignment_value57, ptr %scaled, align 8
  %exponent58 = load i32, ptr %exponent, align 4
  %one_count59 = load i32, ptr %one_count, align 4
  %add60 = add i32 %exponent58, %one_count59
  store i32 %add60, ptr %assignment_value61, align 4
  %assignment_value62 = load i32, ptr %assignment_value61, align 4
  store i32 %assignment_value62, ptr %exponent, align 4
  br label %while.cond.0

while.exit.2:                                     ; preds = %while.cond.0
  br label %while.cond.3

while.cond.3:                                     ; preds = %while.body.4, %while.exit.2
  %scaled63 = load double, ptr %scaled, align 8
  %one_value64 = load double, ptr %one_value, align 8
  %lt65 = fcmp olt double %scaled63, %one_value64
  br i1 %lt65, label %while.body.4, label %while.exit.5

while.body.4:                                     ; preds = %while.cond.3
  %scaled66 = load double, ptr %scaled, align 8
  %ten_value67 = load double, ptr %ten_value, align 8
  %mul = fmul double %scaled66, %ten_value67
  store double %mul, ptr %assignment_value68, align 8
  %assignment_value69 = load double, ptr %assignment_value68, align 8
  store double %assignment_value69, ptr %scaled, align 8
  %exponent70 = load i32, ptr %exponent, align 4
  %one_count71 = load i32, ptr %one_count, align 4
  %sub72 = sub i32 %exponent70, %one_count71
  store i32 %sub72, ptr %assignment_value73, align 4
  %assignment_value74 = load i32, ptr %assignment_value73, align 4
  store i32 %assignment_value74, ptr %exponent, align 4
  br label %while.cond.3

while.exit.5:                                     ; preds = %while.cond.3
  %emit75 = load i64, ptr %emit, align 4
  store i64 %emit75, ptr %digit_start, align 4
  %zero76 = load i64, ptr %zero, align 4
  store i64 %zero76, ptr %produced, align 4
  br label %while.cond.6

while.cond.6:                                     ; preds = %if.merge98, %while.exit.5
  %produced77 = load i64, ptr %produced, align 4
  %seventeen78 = load i64, ptr %seventeen, align 4
  %lt79 = icmp ult i64 %produced77, %seventeen78
  br i1 %lt79, label %while.body.7, label %while.exit.8

while.body.7:                                     ; preds = %while.cond.6
  %scaled80 = load double, ptr %scaled, align 8
  %float_to_sint_trunc = fptosi double %scaled80 to i32
  store i32 %float_to_sint_trunc, ptr %digit_value, align 4
  %digit_value81 = load i32, ptr %digit_value, align 4
  %int_trunc = trunc i32 %digit_value81 to i8
  store i8 %int_trunc, ptr %digit, align 1
  %zero_digit82 = load i8, ptr %zero_digit, align 1
  %digit83 = load i8, ptr %digit, align 1
  %add84 = add i8 %zero_digit82, %digit83
  store i8 %add84, ptr %assignment_value85, align 1
  %assignment_value86 = load i8, ptr %assignment_value85, align 1
  %emit87 = load i64, ptr %emit, align 4
  %array_element88 = getelementptr inbounds i8, ptr %allocation, i64 %emit87
  store i8 %assignment_value86, ptr %array_element88, align 1
  %emit89 = load i64, ptr %emit, align 4
  %one90 = load i64, ptr %one, align 4
  %add91 = add i64 %emit89, %one90
  store i64 %add91, ptr %assignment_value92, align 4
  %assignment_value93 = load i64, ptr %assignment_value92, align 4
  store i64 %assignment_value93, ptr %emit, align 4
  %produced94 = load i64, ptr %produced, align 4
  %zero95 = load i64, ptr %zero, align 4
  %eq96 = icmp eq i64 %produced94, %zero95
  br i1 %eq96, label %if.then97, label %if.merge98

while.exit.8:                                     ; preds = %while.cond.6
  %scaled125 = load double, ptr %scaled, align 8
  %five_value126 = load double, ptr %five_value, align 8
  %ge127 = fcmp oge double %scaled125, %five_value126
  br i1 %ge127, label %if.then128, label %if.merge129

if.then97:                                        ; preds = %while.body.7
  %point99 = load i8, ptr %point, align 1
  store i8 %point99, ptr %assignment_value100, align 1
  %assignment_value101 = load i8, ptr %assignment_value100, align 1
  %emit102 = load i64, ptr %emit, align 4
  %array_element103 = getelementptr inbounds i8, ptr %allocation, i64 %emit102
  store i8 %assignment_value101, ptr %array_element103, align 1
  %emit104 = load i64, ptr %emit, align 4
  %one105 = load i64, ptr %one, align 4
  %add106 = add i64 %emit104, %one105
  store i64 %add106, ptr %assignment_value107, align 4
  %assignment_value108 = load i64, ptr %assignment_value107, align 4
  store i64 %assignment_value108, ptr %emit, align 4
  br label %if.merge98

if.merge98:                                       ; preds = %if.then97, %while.body.7
  %digit_value109 = load i32, ptr %digit_value, align 4
  %sint_to_float = sitofp i32 %digit_value109 to double
  store double %sint_to_float, ptr %whole, align 8
  %scaled110 = load double, ptr %scaled, align 8
  %whole111 = load double, ptr %whole, align 8
  %sub112 = fsub double %scaled110, %whole111
  store double %sub112, ptr %assignment_value113, align 8
  %assignment_value114 = load double, ptr %assignment_value113, align 8
  store double %assignment_value114, ptr %scaled, align 8
  %scaled115 = load double, ptr %scaled, align 8
  %ten_value116 = load double, ptr %ten_value, align 8
  %mul117 = fmul double %scaled115, %ten_value116
  store double %mul117, ptr %assignment_value118, align 8
  %assignment_value119 = load double, ptr %assignment_value118, align 8
  store double %assignment_value119, ptr %scaled, align 8
  %produced120 = load i64, ptr %produced, align 4
  %one121 = load i64, ptr %one, align 4
  %add122 = add i64 %produced120, %one121
  store i64 %add122, ptr %assignment_value123, align 4
  %assignment_value124 = load i64, ptr %assignment_value123, align 4
  store i64 %assignment_value124, ptr %produced, align 4
  br label %while.cond.6

if.then128:                                       ; preds = %while.exit.8
  %emit130 = load i64, ptr %emit, align 4
  %one131 = load i64, ptr %one, align 4
  %sub132 = sub i64 %emit130, %one131
  store i64 %sub132, ptr %round_position, align 4
  store i1 true, ptr %rounding, align 1
  br label %while.cond.9

if.merge129:                                      ; preds = %if.merge219, %while.exit.8
  %exponent_mark230 = load i8, ptr %exponent_mark, align 1
  store i8 %exponent_mark230, ptr %assignment_value231, align 1
  %assignment_value232 = load i8, ptr %assignment_value231, align 1
  %emit233 = load i64, ptr %emit, align 4
  %array_element234 = getelementptr inbounds i8, ptr %allocation, i64 %emit233
  store i8 %assignment_value232, ptr %array_element234, align 1
  %emit235 = load i64, ptr %emit, align 4
  %one236 = load i64, ptr %one, align 4
  %add237 = add i64 %emit235, %one236
  store i64 %add237, ptr %assignment_value238, align 4
  %assignment_value239 = load i64, ptr %assignment_value238, align 4
  store i64 %assignment_value239, ptr %emit, align 4
  %exponent240 = load i32, ptr %exponent, align 4
  store i32 %exponent240, ptr %remaining_exponent, align 4
  %remaining_exponent241 = load i32, ptr %remaining_exponent, align 4
  %zero_count242 = load i32, ptr %zero_count, align 4
  %lt243 = icmp slt i32 %remaining_exponent241, %zero_count242
  br i1 %lt243, label %if.then244, label %if.merge245

while.cond.9:                                     ; preds = %if.merge202, %if.then128
  %rounding133 = load i1, ptr %rounding, align 1
  br i1 %rounding133, label %while.body.10, label %while.exit.11

while.body.10:                                    ; preds = %while.cond.9
  %round_position134 = load i64, ptr %round_position, align 4
  %array_element135 = getelementptr inbounds i8, ptr %allocation, i64 %round_position134
  %place = load i8, ptr %array_element135, align 1
  store i8 %place, ptr %current, align 1
  %current136 = load i8, ptr %current, align 1
  %point137 = load i8, ptr %point, align 1
  %eq138 = icmp eq i8 %current136, %point137
  store i1 %eq138, ptr %is_point, align 1
  %current139 = load i8, ptr %current, align 1
  %nine_digit140 = load i8, ptr %nine_digit, align 1
  %eq141 = icmp eq i8 %current139, %nine_digit140
  store i1 %eq141, ptr %is_nine, align 1
  %round_position142 = load i64, ptr %round_position, align 4
  %digit_start143 = load i64, ptr %digit_start, align 4
  %eq144 = icmp eq i64 %round_position142, %digit_start143
  store i1 %eq144, ptr %at_start, align 1
  %is_point145 = load i1, ptr %is_point, align 1
  br i1 %is_point145, label %if.then146, label %if.merge147

while.exit.11:                                    ; preds = %while.cond.9
  %digit_start212 = load i64, ptr %digit_start, align 4
  %array_element213 = getelementptr inbounds i8, ptr %allocation, i64 %digit_start212
  %place214 = load i8, ptr %array_element213, align 1
  store i8 %place214, ptr %leading, align 1
  %leading215 = load i8, ptr %leading, align 1
  %zero_digit216 = load i8, ptr %zero_digit, align 1
  %eq217 = icmp eq i8 %leading215, %zero_digit216
  br i1 %eq217, label %if.then218, label %if.merge219

if.then146:                                       ; preds = %while.body.10
  %round_position148 = load i64, ptr %round_position, align 4
  %one149 = load i64, ptr %one, align 4
  %sub150 = sub i64 %round_position148, %one149
  store i64 %sub150, ptr %assignment_value151, align 4
  %assignment_value152 = load i64, ptr %assignment_value151, align 4
  store i64 %assignment_value152, ptr %round_position, align 4
  br label %if.merge147

if.merge147:                                      ; preds = %if.then146, %while.body.10
  %is_point153 = load i1, ptr %is_point, align 1
  %not = xor i1 %is_point153, true
  br i1 %not, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %if.merge147
  %is_nine154 = load i1, ptr %is_nine, align 1
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %if.merge147
  %short_circuit = phi i1 [ false, %if.merge147 ], [ %is_nine154, %short_circuit.rhs ]
  br i1 %short_circuit, label %short_circuit.rhs155, label %short_circuit.merge156

short_circuit.rhs155:                             ; preds = %short_circuit.merge
  %at_start157 = load i1, ptr %at_start, align 1
  %not158 = xor i1 %at_start157, true
  br label %short_circuit.merge156

short_circuit.merge156:                           ; preds = %short_circuit.rhs155, %short_circuit.merge
  %short_circuit159 = phi i1 [ false, %short_circuit.merge ], [ %not158, %short_circuit.rhs155 ]
  store i1 %short_circuit159, ptr %carry_back, align 1
  %is_point160 = load i1, ptr %is_point, align 1
  %not161 = xor i1 %is_point160, true
  br i1 %not161, label %short_circuit.rhs162, label %short_circuit.merge163

short_circuit.rhs162:                             ; preds = %short_circuit.merge156
  %is_nine164 = load i1, ptr %is_nine, align 1
  br label %short_circuit.merge163

short_circuit.merge163:                           ; preds = %short_circuit.rhs162, %short_circuit.merge156
  %short_circuit165 = phi i1 [ false, %short_circuit.merge156 ], [ %is_nine164, %short_circuit.rhs162 ]
  br i1 %short_circuit165, label %short_circuit.rhs166, label %short_circuit.merge167

short_circuit.rhs166:                             ; preds = %short_circuit.merge163
  %at_start168 = load i1, ptr %at_start, align 1
  br label %short_circuit.merge167

short_circuit.merge167:                           ; preds = %short_circuit.rhs166, %short_circuit.merge163
  %short_circuit169 = phi i1 [ false, %short_circuit.merge163 ], [ %at_start168, %short_circuit.rhs166 ]
  store i1 %short_circuit169, ptr %carry_stop, align 1
  %is_point170 = load i1, ptr %is_point, align 1
  %not171 = xor i1 %is_point170, true
  br i1 %not171, label %short_circuit.rhs172, label %short_circuit.merge173

short_circuit.rhs172:                             ; preds = %short_circuit.merge167
  %is_nine174 = load i1, ptr %is_nine, align 1
  %not175 = xor i1 %is_nine174, true
  br label %short_circuit.merge173

short_circuit.merge173:                           ; preds = %short_circuit.rhs172, %short_circuit.merge167
  %short_circuit176 = phi i1 [ false, %short_circuit.merge167 ], [ %not175, %short_circuit.rhs172 ]
  store i1 %short_circuit176, ptr %carry_done, align 1
  %carry_back177 = load i1, ptr %carry_back, align 1
  br i1 %carry_back177, label %if.then178, label %if.merge179

if.then178:                                       ; preds = %short_circuit.merge173
  %zero_digit180 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit180, ptr %assignment_value181, align 1
  %assignment_value182 = load i8, ptr %assignment_value181, align 1
  %round_position183 = load i64, ptr %round_position, align 4
  %array_element184 = getelementptr inbounds i8, ptr %allocation, i64 %round_position183
  store i8 %assignment_value182, ptr %array_element184, align 1
  %round_position185 = load i64, ptr %round_position, align 4
  %one186 = load i64, ptr %one, align 4
  %sub187 = sub i64 %round_position185, %one186
  store i64 %sub187, ptr %assignment_value188, align 4
  %assignment_value189 = load i64, ptr %assignment_value188, align 4
  store i64 %assignment_value189, ptr %round_position, align 4
  br label %if.merge179

if.merge179:                                      ; preds = %if.then178, %short_circuit.merge173
  %carry_stop190 = load i1, ptr %carry_stop, align 1
  br i1 %carry_stop190, label %if.then191, label %if.merge192

if.then191:                                       ; preds = %if.merge179
  %zero_digit193 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit193, ptr %assignment_value194, align 1
  %assignment_value195 = load i8, ptr %assignment_value194, align 1
  %round_position196 = load i64, ptr %round_position, align 4
  %array_element197 = getelementptr inbounds i8, ptr %allocation, i64 %round_position196
  store i8 %assignment_value195, ptr %array_element197, align 1
  store i1 false, ptr %assignment_value198, align 1
  %assignment_value199 = load i1, ptr %assignment_value198, align 1
  store i1 %assignment_value199, ptr %rounding, align 1
  br label %if.merge192

if.merge192:                                      ; preds = %if.then191, %if.merge179
  %carry_done200 = load i1, ptr %carry_done, align 1
  br i1 %carry_done200, label %if.then201, label %if.merge202

if.then201:                                       ; preds = %if.merge192
  %current203 = load i8, ptr %current, align 1
  %one_digit204 = load i8, ptr %one_digit, align 1
  %add205 = add i8 %current203, %one_digit204
  store i8 %add205, ptr %assignment_value206, align 1
  %assignment_value207 = load i8, ptr %assignment_value206, align 1
  %round_position208 = load i64, ptr %round_position, align 4
  %array_element209 = getelementptr inbounds i8, ptr %allocation, i64 %round_position208
  store i8 %assignment_value207, ptr %array_element209, align 1
  store i1 false, ptr %assignment_value210, align 1
  %assignment_value211 = load i1, ptr %assignment_value210, align 1
  store i1 %assignment_value211, ptr %rounding, align 1
  br label %if.merge202

if.merge202:                                      ; preds = %if.then201, %if.merge192
  br label %while.cond.9

if.then218:                                       ; preds = %while.exit.11
  %one_char220 = load i8, ptr %one_char, align 1
  store i8 %one_char220, ptr %assignment_value221, align 1
  %assignment_value222 = load i8, ptr %assignment_value221, align 1
  %digit_start223 = load i64, ptr %digit_start, align 4
  %array_element224 = getelementptr inbounds i8, ptr %allocation, i64 %digit_start223
  store i8 %assignment_value222, ptr %array_element224, align 1
  %exponent225 = load i32, ptr %exponent, align 4
  %one_count226 = load i32, ptr %one_count, align 4
  %add227 = add i32 %exponent225, %one_count226
  store i32 %add227, ptr %assignment_value228, align 4
  %assignment_value229 = load i32, ptr %assignment_value228, align 4
  store i32 %assignment_value229, ptr %exponent, align 4
  br label %if.merge219

if.merge219:                                      ; preds = %if.then218, %while.exit.11
  br label %if.merge129

if.then244:                                       ; preds = %if.merge129
  %minus246 = load i8, ptr %minus, align 1
  store i8 %minus246, ptr %assignment_value247, align 1
  %assignment_value248 = load i8, ptr %assignment_value247, align 1
  %emit249 = load i64, ptr %emit, align 4
  %array_element250 = getelementptr inbounds i8, ptr %allocation, i64 %emit249
  store i8 %assignment_value248, ptr %array_element250, align 1
  %emit251 = load i64, ptr %emit, align 4
  %one252 = load i64, ptr %one, align 4
  %add253 = add i64 %emit251, %one252
  store i64 %add253, ptr %assignment_value254, align 4
  %assignment_value255 = load i64, ptr %assignment_value254, align 4
  store i64 %assignment_value255, ptr %emit, align 4
  %zero_count256 = load i32, ptr %zero_count, align 4
  %remaining_exponent257 = load i32, ptr %remaining_exponent, align 4
  %sub258 = sub i32 %zero_count256, %remaining_exponent257
  store i32 %sub258, ptr %assignment_value259, align 4
  %assignment_value260 = load i32, ptr %assignment_value259, align 4
  store i32 %assignment_value260, ptr %remaining_exponent, align 4
  br label %if.merge245

if.merge245:                                      ; preds = %if.then244, %if.merge129
  store i32 100, ptr %hundred_scale, align 4
  store i32 10, ptr %ten_scale, align 4
  %remaining_exponent261 = load i32, ptr %remaining_exponent, align 4
  %hundred_scale262 = load i32, ptr %hundred_scale, align 4
  %div263 = sdiv i32 %remaining_exponent261, %hundred_scale262
  store i32 %div263, ptr %hundreds, align 4
  %remaining_exponent264 = load i32, ptr %remaining_exponent, align 4
  %hundreds265 = load i32, ptr %hundreds, align 4
  %hundred_scale266 = load i32, ptr %hundred_scale, align 4
  %mul267 = mul i32 %hundreds265, %hundred_scale266
  %sub268 = sub i32 %remaining_exponent264, %mul267
  store i32 %sub268, ptr %after_hundreds, align 4
  %after_hundreds269 = load i32, ptr %after_hundreds, align 4
  %ten_scale270 = load i32, ptr %ten_scale, align 4
  %div271 = sdiv i32 %after_hundreds269, %ten_scale270
  store i32 %div271, ptr %tens, align 4
  %after_hundreds272 = load i32, ptr %after_hundreds, align 4
  %tens273 = load i32, ptr %tens, align 4
  %ten_scale274 = load i32, ptr %ten_scale, align 4
  %mul275 = mul i32 %tens273, %ten_scale274
  %sub276 = sub i32 %after_hundreds272, %mul275
  store i32 %sub276, ptr %ones, align 4
  store i1 false, ptr %show_tens, align 1
  %hundreds277 = load i32, ptr %hundreds, align 4
  %zero_count278 = load i32, ptr %zero_count, align 4
  %ne = icmp ne i32 %hundreds277, %zero_count278
  br i1 %ne, label %if.then279, label %if.merge280

if.then279:                                       ; preds = %if.merge245
  store i1 true, ptr %assignment_value281, align 1
  %assignment_value282 = load i1, ptr %assignment_value281, align 1
  store i1 %assignment_value282, ptr %show_tens, align 1
  br label %if.merge280

if.merge280:                                      ; preds = %if.then279, %if.merge245
  %tens283 = load i32, ptr %tens, align 4
  %zero_count284 = load i32, ptr %zero_count, align 4
  %ne285 = icmp ne i32 %tens283, %zero_count284
  br i1 %ne285, label %if.then286, label %if.merge287

if.then286:                                       ; preds = %if.merge280
  store i1 true, ptr %assignment_value288, align 1
  %assignment_value289 = load i1, ptr %assignment_value288, align 1
  store i1 %assignment_value289, ptr %show_tens, align 1
  br label %if.merge287

if.merge287:                                      ; preds = %if.then286, %if.merge280
  %hundreds290 = load i32, ptr %hundreds, align 4
  %zero_count291 = load i32, ptr %zero_count, align 4
  %ne292 = icmp ne i32 %hundreds290, %zero_count291
  br i1 %ne292, label %if.then293, label %if.merge294

if.then293:                                       ; preds = %if.merge287
  %hundreds295 = load i32, ptr %hundreds, align 4
  %int_trunc296 = trunc i32 %hundreds295 to i8
  store i8 %int_trunc296, ptr %hundred_digit, align 1
  %zero_digit297 = load i8, ptr %zero_digit, align 1
  %hundred_digit298 = load i8, ptr %hundred_digit, align 1
  %add299 = add i8 %zero_digit297, %hundred_digit298
  store i8 %add299, ptr %assignment_value300, align 1
  %assignment_value301 = load i8, ptr %assignment_value300, align 1
  %emit302 = load i64, ptr %emit, align 4
  %array_element303 = getelementptr inbounds i8, ptr %allocation, i64 %emit302
  store i8 %assignment_value301, ptr %array_element303, align 1
  %emit304 = load i64, ptr %emit, align 4
  %one305 = load i64, ptr %one, align 4
  %add306 = add i64 %emit304, %one305
  store i64 %add306, ptr %assignment_value307, align 4
  %assignment_value308 = load i64, ptr %assignment_value307, align 4
  store i64 %assignment_value308, ptr %emit, align 4
  br label %if.merge294

if.merge294:                                      ; preds = %if.then293, %if.merge287
  %show_tens309 = load i1, ptr %show_tens, align 1
  br i1 %show_tens309, label %if.then310, label %if.merge311

if.then310:                                       ; preds = %if.merge294
  %tens312 = load i32, ptr %tens, align 4
  %int_trunc313 = trunc i32 %tens312 to i8
  store i8 %int_trunc313, ptr %ten_digit, align 1
  %zero_digit314 = load i8, ptr %zero_digit, align 1
  %ten_digit315 = load i8, ptr %ten_digit, align 1
  %add316 = add i8 %zero_digit314, %ten_digit315
  store i8 %add316, ptr %assignment_value317, align 1
  %assignment_value318 = load i8, ptr %assignment_value317, align 1
  %emit319 = load i64, ptr %emit, align 4
  %array_element320 = getelementptr inbounds i8, ptr %allocation, i64 %emit319
  store i8 %assignment_value318, ptr %array_element320, align 1
  %emit321 = load i64, ptr %emit, align 4
  %one322 = load i64, ptr %one, align 4
  %add323 = add i64 %emit321, %one322
  store i64 %add323, ptr %assignment_value324, align 4
  %assignment_value325 = load i64, ptr %assignment_value324, align 4
  store i64 %assignment_value325, ptr %emit, align 4
  br label %if.merge311

if.merge311:                                      ; preds = %if.then310, %if.merge294
  %ones326 = load i32, ptr %ones, align 4
  %int_trunc327 = trunc i32 %ones326 to i8
  store i8 %int_trunc327, ptr %one_digit_out, align 1
  %zero_digit328 = load i8, ptr %zero_digit, align 1
  %one_digit_out329 = load i8, ptr %one_digit_out, align 1
  %add330 = add i8 %zero_digit328, %one_digit_out329
  store i8 %add330, ptr %assignment_value331, align 1
  %assignment_value332 = load i8, ptr %assignment_value331, align 1
  %emit333 = load i64, ptr %emit, align 4
  %array_element334 = getelementptr inbounds i8, ptr %allocation, i64 %emit333
  store i8 %assignment_value332, ptr %array_element334, align 1
  %emit335 = load i64, ptr %emit, align 4
  %one336 = load i64, ptr %one, align 4
  %add337 = add i64 %emit335, %one336
  store i64 %add337, ptr %assignment_value338, align 4
  %assignment_value339 = load i64, ptr %assignment_value338, align 4
  store i64 %assignment_value339, ptr %emit, align 4
  br label %if.merge22
}





define { i64, i1 } @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f66645f726561645f6f6e6365(i32 %destination, i64 %capacity) {
entry:
  %assignment_value15 = alloca i1, align 1
  %assignment_value13 = alloca i64, align 8
  %assignment_value11 = alloca i1, align 1
  %assignment_value = alloca i64, align 8
  %complete = alloca i1, align 1
  %reported = alloca i64, align 8
  %_result = alloca i32, align 4
  %_byte_count = alloca [4 x i8], align 1
  %struct_literal5 = alloca [4 x i8], align 1
  %_iovec = alloca [8 x i8], align 1
  %struct_literal = alloca [8 x i8], align 1
  %destination1 = alloca i32, align 4
  store i32 %destination, ptr %destination1, align 4
  %capacity2 = alloca i64, align 8
  store i64 %capacity, ptr %capacity2, align 4
  %destination3 = load i32, ptr %destination1, align 4
  %capacity4 = load i64, ptr %capacity2, align 4
  %int_trunc = trunc i64 %capacity4 to i32
  %data = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %destination3, ptr %data, align 4
  %length = getelementptr inbounds i8, ptr %struct_literal, i8 4
  store i32 %int_trunc, ptr %length, align 4
  %struct_value = load [8 x i8], ptr %struct_literal, align 1
  store [8 x i8] %struct_value, ptr %_iovec, align 1
  %value = getelementptr inbounds i8, ptr %struct_literal5, i8 0
  store i32 0, ptr %value, align 4
  %struct_value6 = load [4 x i8], ptr %struct_literal5, align 1
  store [4 x i8] %struct_value6, ptr %_byte_count, align 1
  %address = ptrtoint ptr %_iovec to i32
  %address7 = ptrtoint ptr %_byte_count to i32
  %call = call i32 @wosy_extern__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f77617369__776173695f736e617073686f745f7072657669657731__5f66645f72656164(i32 0, i32 %address, i32 1, i32 %address7)
  store i32 %call, ptr %_result, align 4
  store i64 0, ptr %reported, align 4
  store i1 false, ptr %complete, align 1
  %_result8 = load i32, ptr %_result, align 4
  %eq = icmp eq i32 %_result8, 0
  br i1 %eq, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  %value9 = getelementptr inbounds i8, ptr %_byte_count, i8 0
  %place = load i32, ptr %value9, align 4
  %int_extend = zext i32 %place to i64
  store i64 %int_extend, ptr %assignment_value, align 4
  %assignment_value10 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value10, ptr %reported, align 4
  store i1 true, ptr %assignment_value11, align 1
  %assignment_value12 = load i1, ptr %assignment_value11, align 1
  store i1 %assignment_value12, ptr %complete, align 1
  br label %if.merge

if.else:                                          ; preds = %entry
  store i64 0, ptr %assignment_value13, align 4
  %assignment_value14 = load i64, ptr %assignment_value13, align 4
  store i64 %assignment_value14, ptr %reported, align 4
  store i1 false, ptr %assignment_value15, align 1
  %assignment_value16 = load i1, ptr %assignment_value15, align 1
  store i1 %assignment_value16, ptr %complete, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.then
  %reported17 = load i64, ptr %reported, align 4
  %complete18 = load i1, ptr %complete, align 1
  %output = insertvalue { i64, i1 } zeroinitializer, i64 %reported17, 0
  %output19 = insertvalue { i64, i1 } %output, i1 %complete18, 1
  ret { i64, i1 } %output19
}

define i32 @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__66645f77726974655f6f6e6365(i32 %descriptor, i32 %iovec_address, i32 %byte_count_address) {
entry:
  %result = alloca i32, align 4
  %descriptor1 = alloca i32, align 4
  store i32 %descriptor, ptr %descriptor1, align 4
  %iovec_address2 = alloca i32, align 4
  store i32 %iovec_address, ptr %iovec_address2, align 4
  %byte_count_address3 = alloca i32, align 4
  store i32 %byte_count_address, ptr %byte_count_address3, align 4
  %descriptor4 = load i32, ptr %descriptor1, align 4
  %iovec_address5 = load i32, ptr %iovec_address2, align 4
  %byte_count_address6 = load i32, ptr %byte_count_address3, align 4
  %call = call i32 @wosy_extern__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f77617369__776173695f736e617073686f745f7072657669657731__66645f7772697465(i32 %descriptor4, i32 %iovec_address5, i32 1, i32 %byte_count_address6)
  store i32 %call, ptr %result, align 4
  %result7 = load i32, ptr %result, align 4
  ret i32 %result7
}

define { i64, i1 } @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__726561645f696e746f(i32 %destination, i64 %capacity) {
entry:
  %assignment_value26 = alloca i1, align 1
  %assignment_value24 = alloca i64, align 8
  %assignment_value21 = alloca i1, align 1
  %assignment_value18 = alloca i64, align 8
  %assignment_value9 = alloca i1, align 1
  %assignment_value = alloca i64, align 8
  %complete = alloca i1, align 1
  %reported = alloca i64, align 8
  %maximum = alloca i64, align 8
  %maximum_u32 = alloca i32, align 4
  %zero = alloca i64, align 8
  %destination1 = alloca i32, align 4
  store i32 %destination, ptr %destination1, align 4
  %capacity2 = alloca i64, align 8
  store i64 %capacity, ptr %capacity2, align 4
  store i64 0, ptr %zero, align 4
  store i32 -1, ptr %maximum_u32, align 4
  %maximum_u323 = load i32, ptr %maximum_u32, align 4
  %int_extend = zext i32 %maximum_u323 to i64
  store i64 %int_extend, ptr %maximum, align 4
  %zero4 = load i64, ptr %zero, align 4
  store i64 %zero4, ptr %reported, align 4
  store i1 false, ptr %complete, align 1
  %capacity5 = load i64, ptr %capacity2, align 4
  %zero6 = load i64, ptr %zero, align 4
  %eq = icmp eq i64 %capacity5, %zero6
  br i1 %eq, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  %zero7 = load i64, ptr %zero, align 4
  store i64 %zero7, ptr %assignment_value, align 4
  %assignment_value8 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value8, ptr %reported, align 4
  store i1 true, ptr %assignment_value9, align 1
  %assignment_value10 = load i1, ptr %assignment_value9, align 1
  store i1 %assignment_value10, ptr %complete, align 1
  br label %if.merge

if.else:                                          ; preds = %entry
  %capacity11 = load i64, ptr %capacity2, align 4
  %maximum12 = load i64, ptr %maximum, align 4
  %le = icmp ule i64 %capacity11, %maximum12
  br i1 %le, label %if.then13, label %if.else14

if.merge:                                         ; preds = %if.merge15, %if.then
  %reported28 = load i64, ptr %reported, align 4
  %complete29 = load i1, ptr %complete, align 1
  %output30 = insertvalue { i64, i1 } zeroinitializer, i64 %reported28, 0
  %output31 = insertvalue { i64, i1 } %output30, i1 %complete29, 1
  ret { i64, i1 } %output31

if.then13:                                        ; preds = %if.else
  %destination16 = load i32, ptr %destination1, align 4
  %capacity17 = load i64, ptr %capacity2, align 4
  %call = call { i64, i1 } @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f66645f726561645f6f6e6365(i32 %destination16, i64 %capacity17)
  %output = extractvalue { i64, i1 } %call, 0
  store i64 %output, ptr %assignment_value18, align 4
  %assignment_value19 = load i64, ptr %assignment_value18, align 4
  %output20 = extractvalue { i64, i1 } %call, 1
  store i1 %output20, ptr %assignment_value21, align 1
  %assignment_value22 = load i1, ptr %assignment_value21, align 1
  store i64 %assignment_value19, ptr %reported, align 4
  store i1 %assignment_value22, ptr %complete, align 1
  br label %if.merge15

if.else14:                                        ; preds = %if.else
  %zero23 = load i64, ptr %zero, align 4
  store i64 %zero23, ptr %assignment_value24, align 4
  %assignment_value25 = load i64, ptr %assignment_value24, align 4
  store i64 %assignment_value25, ptr %reported, align 4
  store i1 false, ptr %assignment_value26, align 1
  %assignment_value27 = load i1, ptr %assignment_value26, align 1
  store i1 %assignment_value27, ptr %complete, align 1
  br label %if.merge15

if.merge15:                                       ; preds = %if.else14, %if.then13
  br label %if.merge
}

define void @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__65786974(i32 %code) {
entry:
  %slot = alloca i32, align 4
  %code1 = alloca i32, align 4
  store i32 %code, ptr %code1, align 4
  %code2 = load i32, ptr %code1, align 4
  %int_extend = zext i32 %code2 to i64
  %int_trunc = trunc i64 %int_extend to i32
  store i32 %int_trunc, ptr %slot, align 4
  %slot3 = load i32, ptr %slot, align 4
  call void @wosy_extern__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f77617369__776173695f736e617073686f745f7072657669657731__5f70726f635f65786974(i32 %slot3)
  ret void
}

define void @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__736c6565705f6e73(i64 %nanoseconds) {
entry:
  %fired = alloca [4 x i8], align 1
  %struct_literal21 = alloca [4 x i8], align 1
  %event = alloca [32 x i8], align 1
  %struct_literal3 = alloca [32 x i8], align 1
  %sub = alloca [48 x i8], align 1
  %struct_literal = alloca [48 x i8], align 1
  %nanoseconds1 = alloca i64, align 8
  store i64 %nanoseconds, ptr %nanoseconds1, align 4
  %nanoseconds2 = load i64, ptr %nanoseconds1, align 4
  %userdata = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i64 0, ptr %userdata, align 4
  %clock_tag = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i8 0, ptr %clock_tag, align 1
  %_pad_1 = getelementptr inbounds i8, ptr %struct_literal, i8 9
  store i8 0, ptr %_pad_1, align 1
  %_pad_2 = getelementptr inbounds i8, ptr %struct_literal, i8 10
  store i8 0, ptr %_pad_2, align 1
  %_pad_3 = getelementptr inbounds i8, ptr %struct_literal, i8 11
  store i8 0, ptr %_pad_3, align 1
  %_pad_4 = getelementptr inbounds i8, ptr %struct_literal, i8 12
  store i8 0, ptr %_pad_4, align 1
  %_pad_5 = getelementptr inbounds i8, ptr %struct_literal, i8 13
  store i8 0, ptr %_pad_5, align 1
  %_pad_6 = getelementptr inbounds i8, ptr %struct_literal, i8 14
  store i8 0, ptr %_pad_6, align 1
  %_pad_7 = getelementptr inbounds i8, ptr %struct_literal, i8 15
  store i8 0, ptr %_pad_7, align 1
  %clock_id = getelementptr inbounds i8, ptr %struct_literal, i8 16
  store i32 1, ptr %clock_id, align 4
  %_pad_8 = getelementptr inbounds i8, ptr %struct_literal, i8 20
  store i8 0, ptr %_pad_8, align 1
  %_pad_9 = getelementptr inbounds i8, ptr %struct_literal, i8 21
  store i8 0, ptr %_pad_9, align 1
  %_pad_10 = getelementptr inbounds i8, ptr %struct_literal, i8 22
  store i8 0, ptr %_pad_10, align 1
  %_pad_11 = getelementptr inbounds i8, ptr %struct_literal, i8 23
  store i8 0, ptr %_pad_11, align 1
  %timeout = getelementptr inbounds i8, ptr %struct_literal, i8 24
  store i64 %nanoseconds2, ptr %timeout, align 4
  %precision = getelementptr inbounds i8, ptr %struct_literal, i8 32
  store i64 0, ptr %precision, align 4
  %flags = getelementptr inbounds i8, ptr %struct_literal, i8 40
  store i16 0, ptr %flags, align 2
  %_pad_12 = getelementptr inbounds i8, ptr %struct_literal, i8 42
  store i8 0, ptr %_pad_12, align 1
  %_pad_13 = getelementptr inbounds i8, ptr %struct_literal, i8 43
  store i8 0, ptr %_pad_13, align 1
  %_pad_14 = getelementptr inbounds i8, ptr %struct_literal, i8 44
  store i8 0, ptr %_pad_14, align 1
  %_pad_15 = getelementptr inbounds i8, ptr %struct_literal, i8 45
  store i8 0, ptr %_pad_15, align 1
  %_pad_16 = getelementptr inbounds i8, ptr %struct_literal, i8 46
  store i8 0, ptr %_pad_16, align 1
  %_pad_17 = getelementptr inbounds i8, ptr %struct_literal, i8 47
  store i8 0, ptr %_pad_17, align 1
  %struct_value = load [48 x i8], ptr %struct_literal, align 1
  store [48 x i8] %struct_value, ptr %sub, align 1
  %userdata4 = getelementptr inbounds i8, ptr %struct_literal3, i8 0
  store i64 0, ptr %userdata4, align 4
  %error = getelementptr inbounds i8, ptr %struct_literal3, i8 8
  store i16 0, ptr %error, align 2
  %event_type = getelementptr inbounds i8, ptr %struct_literal3, i8 10
  store i8 0, ptr %event_type, align 1
  %_pad_18 = getelementptr inbounds i8, ptr %struct_literal3, i8 11
  store i8 0, ptr %_pad_18, align 1
  %_pad_29 = getelementptr inbounds i8, ptr %struct_literal3, i8 12
  store i8 0, ptr %_pad_29, align 1
  %_pad_310 = getelementptr inbounds i8, ptr %struct_literal3, i8 13
  store i8 0, ptr %_pad_310, align 1
  %_pad_411 = getelementptr inbounds i8, ptr %struct_literal3, i8 14
  store i8 0, ptr %_pad_411, align 1
  %_pad_512 = getelementptr inbounds i8, ptr %struct_literal3, i8 15
  store i8 0, ptr %_pad_512, align 1
  %nbytes = getelementptr inbounds i8, ptr %struct_literal3, i8 16
  store i64 0, ptr %nbytes, align 4
  %flags13 = getelementptr inbounds i8, ptr %struct_literal3, i8 24
  store i16 0, ptr %flags13, align 2
  %_pad_614 = getelementptr inbounds i8, ptr %struct_literal3, i8 26
  store i8 0, ptr %_pad_614, align 1
  %_pad_715 = getelementptr inbounds i8, ptr %struct_literal3, i8 27
  store i8 0, ptr %_pad_715, align 1
  %_pad_816 = getelementptr inbounds i8, ptr %struct_literal3, i8 28
  store i8 0, ptr %_pad_816, align 1
  %_pad_917 = getelementptr inbounds i8, ptr %struct_literal3, i8 29
  store i8 0, ptr %_pad_917, align 1
  %_pad_1018 = getelementptr inbounds i8, ptr %struct_literal3, i8 30
  store i8 0, ptr %_pad_1018, align 1
  %_pad_1119 = getelementptr inbounds i8, ptr %struct_literal3, i8 31
  store i8 0, ptr %_pad_1119, align 1
  %struct_value20 = load [32 x i8], ptr %struct_literal3, align 1
  store [32 x i8] %struct_value20, ptr %event, align 1
  %value = getelementptr inbounds i8, ptr %struct_literal21, i8 0
  store i32 0, ptr %value, align 4
  %struct_value22 = load [4 x i8], ptr %struct_literal21, align 1
  store [4 x i8] %struct_value22, ptr %fired, align 1
  %address = ptrtoint ptr %sub to i32
  %address23 = ptrtoint ptr %event to i32
  %address24 = ptrtoint ptr %fired to i32
  %call = call i32 @wosy_extern__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f77617369__776173695f736e617073686f745f7072657669657731__5f706f6c6c5f6f6e656f6666(i32 %address, i32 %address23, i32 1, i32 %address24)
  ret void
}

define ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333334333333363336333233333335333333313333333633333339333333353333333333363336333333343333333433333331333333373336333233333338333633363333333133333333333333323333333533333332333333353333333933363332333633353336333633363332333633353333333633363334333633353333333333363333333333343333333133363334333633353333333733333331333333363333333633363335333333313333333933333333333333393333333933333334333633333333333233333338333333333336333633333332333333383333333733333338333333373333333433333330333633343333333633333331356635663336333633363636333733323336363433363331333733345f5f3330__(i1 %value) {
entry:
  %value1 = alloca i1, align 1
  store i1 %value, ptr %value1, align 1
  %value2 = load i1, ptr %value1, align 1
  %call = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f626f6f6c(i1 %value2)
  ret ptr %call
}

define ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333334333333363336333233333335333333313333333633333339333333353333333333363336333333343333333433333331333333373336333233333338333633363333333133333333333333323333333533333332333333353333333933363332333633353336333633363332333633353333333633363334333633353333333333363333333333343333333133363334333633353333333733333331333333363333333633363335333333313333333933333333333333393333333933333334333633333333333233333338333333333336333633333332333333383333333733333338333333373333333433333330333633343333333633333331356635663336333633363636333733323336363433363331333733345f5f3331__(i32 %value) {
entry:
  %value1 = alloca i32, align 4
  store i32 %value, ptr %value1, align 4
  %value2 = load i32, ptr %value1, align 4
  %call = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f63686172(i32 %value2)
  ret ptr %call
}

define ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333334333333363336333233333335333333313333333633333339333333353333333333363336333333343333333433333331333333373336333233333338333633363333333133333333333333323333333533333332333333353333333933363332333633353336333633363332333633353333333633363334333633353333333333363333333333343333333133363334333633353333333733333331333333363333333633363335333333313333333933333333333333393333333933333334333633333333333233333338333333333336333633333332333333383333333733333338333333373333333433333330333633343333333633333331356635663336333633363636333733323336363433363331333733345f5f33313330__(i64 %value) {
entry:
  %value1 = alloca i64, align 8
  store i64 %value, ptr %value1, align 4
  %value2 = load i64, ptr %value1, align 4
  %call = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f753634(i64 %value2)
  ret ptr %call
}

define ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333334333333363336333233333335333333313333333633333339333333353333333333363336333333343333333433333331333333373336333233333338333633363333333133333333333333323333333533333332333333353333333933363332333633353336333633363332333633353333333633363334333633353333333333363333333333343333333133363334333633353333333733333331333333363333333633363335333333313333333933333333333333393333333933333334333633333333333233333338333333333336333633333332333333383333333733333338333333373333333433333330333633343333333633333331356635663336333633363636333733323336363433363331333733345f5f33313331__(i128 %value) {
entry:
  %value1 = alloca i128, align 8
  store i128 %value, ptr %value1, align 4
  %value2 = load i128, ptr %value1, align 4
  %call = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f75313238(i128 %value2)
  ret ptr %call
}

define ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333334333333363336333233333335333333313333333633333339333333353333333333363336333333343333333433333331333333373336333233333338333633363333333133333333333333323333333533333332333333353333333933363332333633353336333633363332333633353333333633363334333633353333333333363333333333343333333133363334333633353333333733333331333333363333333633363335333333313333333933333333333333393333333933333334333633333333333233333338333333333336333633333332333333383333333733333338333333373333333433333330333633343333333633333331356635663336333633363636333733323336363433363331333733345f5f33313332__(float %value) {
entry:
  %value1 = alloca float, align 4
  store float %value, ptr %value1, align 4
  %value2 = load float, ptr %value1, align 4
  %call = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f663332(float %value2)
  ret ptr %call
}

define ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333334333333363336333233333335333333313333333633333339333333353333333333363336333333343333333433333331333333373336333233333338333633363333333133333333333333323333333533333332333333353333333933363332333633353336333633363332333633353333333633363334333633353333333333363333333333343333333133363334333633353333333733333331333333363333333633363335333333313333333933333333333333393333333933333334333633333333333233333338333333333336333633333332333333383333333733333338333333373333333433333330333633343333333633333331356635663336333633363636333733323336363433363331333733345f5f33313333__(double %value) {
entry:
  %value1 = alloca double, align 8
  store double %value, ptr %value1, align 8
  %value2 = load double, ptr %value1, align 8
  %call = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f663634(double %value2)
  ret ptr %call
}

define ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333334333333363336333233333335333333313333333633333339333333353333333333363336333333343333333433333331333333373336333233333338333633363333333133333333333333323333333533333332333333353333333933363332333633353336333633363332333633353333333633363334333633353333333333363333333333343333333133363334333633353333333733333331333333363333333633363335333333313333333933333333333333393333333933333334333633333333333233333338333333333336333633333332333333383333333733333338333333373333333433333330333633343333333633333331356635663336333633363636333733323336363433363331333733345f5f3332__(i8 %value) {
entry:
  %value1 = alloca i8, align 1
  store i8 %value, ptr %value1, align 1
  %value2 = load i8, ptr %value1, align 1
  %call = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f6938(i8 %value2)
  ret ptr %call
}

define ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333334333333363336333233333335333333313333333633333339333333353333333333363336333333343333333433333331333333373336333233333338333633363333333133333333333333323333333533333332333333353333333933363332333633353336333633363332333633353333333633363334333633353333333333363333333333343333333133363334333633353333333733333331333333363333333633363335333333313333333933333333333333393333333933333334333633333333333233333338333333333336333633333332333333383333333733333338333333373333333433333330333633343333333633333331356635663336333633363636333733323336363433363331333733345f5f3333__(i16 %value) {
entry:
  %value1 = alloca i16, align 2
  store i16 %value, ptr %value1, align 2
  %value2 = load i16, ptr %value1, align 2
  %call = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f693136(i16 %value2)
  ret ptr %call
}

define ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333334333333363336333233333335333333313333333633333339333333353333333333363336333333343333333433333331333333373336333233333338333633363333333133333333333333323333333533333332333333353333333933363332333633353336333633363332333633353333333633363334333633353333333333363333333333343333333133363334333633353333333733333331333333363333333633363335333333313333333933333333333333393333333933333334333633333333333233333338333333333336333633333332333333383333333733333338333333373333333433333330333633343333333633333331356635663336333633363636333733323336363433363331333733345f5f3334__(i32 %value) {
entry:
  %value1 = alloca i32, align 4
  store i32 %value, ptr %value1, align 4
  %value2 = load i32, ptr %value1, align 4
  %call = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f693332(i32 %value2)
  ret ptr %call
}

define ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333334333333363336333233333335333333313333333633333339333333353333333333363336333333343333333433333331333333373336333233333338333633363333333133333333333333323333333533333332333333353333333933363332333633353336333633363332333633353333333633363334333633353333333333363333333333343333333133363334333633353333333733333331333333363333333633363335333333313333333933333333333333393333333933333334333633333333333233333338333333333336333633333332333333383333333733333338333333373333333433333330333633343333333633333331356635663336333633363636333733323336363433363331333733345f5f3335__(i64 %value) {
entry:
  %value1 = alloca i64, align 8
  store i64 %value, ptr %value1, align 4
  %value2 = load i64, ptr %value1, align 4
  %call = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f693634(i64 %value2)
  ret ptr %call
}

define ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333334333333363336333233333335333333313333333633333339333333353333333333363336333333343333333433333331333333373336333233333338333633363333333133333333333333323333333533333332333333353333333933363332333633353336333633363332333633353333333633363334333633353333333333363333333333343333333133363334333633353333333733333331333333363333333633363335333333313333333933333333333333393333333933333334333633333333333233333338333333333336333633333332333333383333333733333338333333373333333433333330333633343333333633333331356635663336333633363636333733323336363433363331333733345f5f3336__(i128 %value) {
entry:
  %value1 = alloca i128, align 8
  store i128 %value, ptr %value1, align 4
  %value2 = load i128, ptr %value1, align 4
  %call = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f69313238(i128 %value2)
  ret ptr %call
}

define ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333334333333363336333233333335333333313333333633333339333333353333333333363336333333343333333433333331333333373336333233333338333633363333333133333333333333323333333533333332333333353333333933363332333633353336333633363332333633353333333633363334333633353333333333363333333333343333333133363334333633353333333733333331333333363333333633363335333333313333333933333333333333393333333933333334333633333333333233333338333333333336333633333332333333383333333733333338333333373333333433333330333633343333333633333331356635663336333633363636333733323336363433363331333733345f5f3337__(i8 %value) {
entry:
  %value1 = alloca i8, align 1
  store i8 %value, ptr %value1, align 1
  %value2 = load i8, ptr %value1, align 1
  %call = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f7538(i8 %value2)
  ret ptr %call
}

define ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333334333333363336333233333335333333313333333633333339333333353333333333363336333333343333333433333331333333373336333233333338333633363333333133333333333333323333333533333332333333353333333933363332333633353336333633363332333633353333333633363334333633353333333333363333333333343333333133363334333633353333333733333331333333363333333633363335333333313333333933333333333333393333333933333334333633333333333233333338333333333336333633333332333333383333333733333338333333373333333433333330333633343333333633333331356635663336333633363636333733323336363433363331333733345f5f3338__(i16 %value) {
entry:
  %value1 = alloca i16, align 2
  store i16 %value, ptr %value1, align 2
  %value2 = load i16, ptr %value1, align 2
  %call = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f753136(i16 %value2)
  ret ptr %call
}

define ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333334333333363336333233333335333333313333333633333339333333353333333333363336333333343333333433333331333333373336333233333338333633363333333133333333333333323333333533333332333333353333333933363332333633353336333633363332333633353333333633363334333633353333333333363333333333343333333133363334333633353333333733333331333333363333333633363335333333313333333933333333333333393333333933333334333633333333333233333338333333333336333633333332333333383333333733333338333333373333333433333330333633343333333633333331356635663336333633363636333733323336363433363331333733345f5f3339__(i32 %value) {
entry:
  %value1 = alloca i32, align 4
  store i32 %value, ptr %value1, align 4
  %value2 = load i32, ptr %value1, align 4
  %call = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f666f726d61745f753332(i32 %value2)
  ret ptr %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333343333333633363332333333353333333133333336333333393333333533333333333633363333333433333334333333313333333733363332333333383336333633333331333333333333333233333335333333323333333533333339333633323336333533363336333633323336333533333336333633343336333533333333333633333333333433333331333633343336333533333337333333313333333633333336333633353333333133333339333333333333333933333339333333343336333333333332333333383333333333363336333333323333333833333337333333383333333733333334333333303336333433333336333333313566356633373330333633313337333233373333333633355f5f3330__(ptr %text) {
entry:
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f7538(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333343333333633363332333333353333333133333336333333393333333533333333333633363333333433333334333333313333333733363332333333383336333633333331333333333333333233333335333333323333333533333339333633323336333533363336333633323336333533333336333633343336333533333333333633333333333433333331333633343336333533333337333333313333333633333336333633353333333133333339333333333333333933333339333333343336333333333332333333383333333333363336333333323333333833333337333333383333333733333334333333303336333433333336333333313566356633373330333633313337333233373333333633355f5f3331__(ptr %text) {
entry:
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f753136(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333343333333633363332333333353333333133333336333333393333333533333333333633363333333433333334333333313333333733363332333333383336333633333331333333333333333233333335333333323333333533333339333633323336333533363336333633323336333533333336333633343336333533333333333633333333333433333331333633343336333533333337333333313333333633333336333633353333333133333339333333333333333933333339333333343336333333333332333333383333333333363336333333323333333833333337333333383333333733333334333333303336333433333336333333313566356633373330333633313337333233373333333633355f5f33313330__(ptr %text) {
entry:
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f663332(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333343333333633363332333333353333333133333336333333393333333533333333333633363333333433333334333333313333333733363332333333383336333633333331333333333333333233333335333333323333333533333339333633323336333533363336333633323336333533333336333633343336333533333333333633333333333433333331333633343336333533333337333333313333333633333336333633353333333133333339333333333333333933333339333333343336333333333332333333383333333333363336333333323333333833333337333333383333333733333334333333303336333433333336333333313566356633373330333633313337333233373333333633355f5f33313331__(ptr %text) {
entry:
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f663634(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333343333333633363332333333353333333133333336333333393333333533333333333633363333333433333334333333313333333733363332333333383336333633333331333333333333333233333335333333323333333533333339333633323336333533363336333633323336333533333336333633343336333533333333333633333333333433333331333633343336333533333337333333313333333633333336333633353333333133333339333333333333333933333339333333343336333333333332333333383333333333363336333333323333333833333337333333383333333733333334333333303336333433333336333333313566356633373330333633313337333233373333333633355f5f3332__(ptr %text) {
entry:
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f753332(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333343333333633363332333333353333333133333336333333393333333533333333333633363333333433333334333333313333333733363332333333383336333633333331333333333333333233333335333333323333333533333339333633323336333533363336333633323336333533333336333633343336333533333333333633333333333433333331333633343336333533333337333333313333333633333336333633353333333133333339333333333333333933333339333333343336333333333332333333383333333333363336333333323333333833333337333333383333333733333334333333303336333433333336333333313566356633373330333633313337333233373333333633355f5f3333__(ptr %text) {
entry:
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f753634(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333343333333633363332333333353333333133333336333333393333333533333333333633363333333433333334333333313333333733363332333333383336333633333331333333333333333233333335333333323333333533333339333633323336333533363336333633323336333533333336333633343336333533333333333633333333333433333331333633343336333533333337333333313333333633333336333633353333333133333339333333333333333933333339333333343336333333333332333333383333333333363336333333323333333833333337333333383333333733333334333333303336333433333336333333313566356633373330333633313337333233373333333633355f5f3334__(ptr %text) {
entry:
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f75313238(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333343333333633363332333333353333333133333336333333393333333533333333333633363333333433333334333333313333333733363332333333383336333633333331333333333333333233333335333333323333333533333339333633323336333533363336333633323336333533333336333633343336333533333333333633333333333433333331333633343336333533333337333333313333333633333336333633353333333133333339333333333333333933333339333333343336333333333332333333383333333333363336333333323333333833333337333333383333333733333334333333303336333433333336333333313566356633373330333633313337333233373333333633355f5f3335__(ptr %text) {
entry:
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f6938(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333343333333633363332333333353333333133333336333333393333333533333333333633363333333433333334333333313333333733363332333333383336333633333331333333333333333233333335333333323333333533333339333633323336333533363336333633323336333533333336333633343336333533333333333633333333333433333331333633343336333533333337333333313333333633333336333633353333333133333339333333333333333933333339333333343336333333333332333333383333333333363336333333323333333833333337333333383333333733333334333333303336333433333336333333313566356633373330333633313337333233373333333633355f5f3336__(ptr %text) {
entry:
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f693136(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333343333333633363332333333353333333133333336333333393333333533333333333633363333333433333334333333313333333733363332333333383336333633333331333333333333333233333335333333323333333533333339333633323336333533363336333633323336333533333336333633343336333533333333333633333333333433333331333633343336333533333337333333313333333633333336333633353333333133333339333333333333333933333339333333343336333333333332333333383333333333363336333333323333333833333337333333383333333733333334333333303336333433333336333333313566356633373330333633313337333233373333333633355f5f3337__(ptr %text) {
entry:
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f693332(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333343333333633363332333333353333333133333336333333393333333533333333333633363333333433333334333333313333333733363332333333383336333633333331333333333333333233333335333333323333333533333339333633323336333533363336333633323336333533333336333633343336333533333333333633333333333433333331333633343336333533333337333333313333333633333336333633353333333133333339333333333333333933333339333333343336333333333332333333383333333333363336333333323333333833333337333333383333333733333334333333303336333433333336333333313566356633373330333633313337333233373333333633355f5f3338__(ptr %text) {
entry:
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f693634(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333343333333633363332333333353333333133333336333333393333333533333333333633363333333433333334333333313333333733363332333333383336333633333331333333333333333233333335333333323333333533333339333633323336333533363336333633323336333533333336333633343336333533333333333633333333333433333331333633343336333533333337333333313333333633333336333633353333333133333339333333333333333933333339333333343336333333333332333333383333333333363336333333323333333833333337333333383333333733333334333333303336333433333336333333313566356633373330333633313337333233373333333633355f5f3339__(ptr %text) {
entry:
  %text1 = alloca [16 x i8], align 1
  %struct_value = load [16 x i8], ptr %text, align 1
  store [16 x i8] %struct_value, ptr %text1, align 1
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__34366235313639353366343431376238663133323532353962656662653664653363343164653731363665313933393934633238336632383738373430643631__5f70617273655f69313238(ptr %text1)
  ret i32 %call
}

define i32 @main() {
entry:
  call void @wosy_fn__706f696e7465722d636173742d726177__7372632f6d61696e2e77__66666637306432326135346365616161303661656666326632666637636634316635633333636234663761653938643433353161363033373136343463386663__72756e()
  ret i32 0
}

declare ptr @__wosy_core_alloc(i64, i64)

declare void @__wosy_core_free(ptr)

declare void @__wosy_core_system_panic()

declare ptr @__wosy_core_alloc.1(i64, i64)

declare void @__wosy_core_free.2(ptr)

declare void @__wosy_core_system_panic.3()

declare ptr @__wosy_core_alloc.4(i64, i64)

declare void @__wosy_core_free.5(ptr)

declare void @__wosy_core_system_panic.6()

declare ptr @__wosy_core_alloc.7(i64, i64)

declare void @__wosy_core_free.8(ptr)

declare void @__wosy_core_system_panic.9()

declare ptr @__wosy_core_alloc.10(i64, i64)

declare void @__wosy_core_free.11(ptr)

declare void @__wosy_core_system_panic.12()

declare ptr @__wosy_core_alloc.13(i64, i64)

declare void @__wosy_core_free.14(ptr)

declare void @__wosy_core_system_panic.15()

declare ptr @__wosy_core_alloc.16(i64, i64)

declare void @__wosy_core_free.17(ptr)

declare void @__wosy_core_system_panic.18()

declare ptr @__wosy_core_alloc.19(i64, i64)

declare void @__wosy_core_free.20(ptr)

declare void @__wosy_core_system_panic.21()

declare ptr @__wosy_core_alloc.22(i64, i64)

declare void @__wosy_core_free.23(ptr)

declare void @__wosy_core_system_panic.24()

declare ptr @__wosy_core_alloc.25(i64, i64)

declare void @__wosy_core_free.26(ptr)

declare void @__wosy_core_system_panic.27()

declare ptr @__wosy_core_alloc.28(i64, i64)

declare void @__wosy_core_free.29(ptr)

declare void @__wosy_core_system_panic.30()

declare ptr @__wosy_core_alloc.31(i64, i64)

declare void @__wosy_core_free.32(ptr)

declare void @__wosy_core_system_panic.33()

declare ptr @__wosy_core_alloc.34(i64, i64)

declare void @__wosy_core_free.35(ptr)

declare void @__wosy_core_system_panic.36()

declare ptr @__wosy_core_alloc.37(i64, i64)

declare void @__wosy_core_free.38(ptr)

declare void @__wosy_core_system_panic.39()

declare ptr @__wosy_core_alloc.40(i64, i64)

declare void @__wosy_core_free.41(ptr)

declare void @__wosy_core_system_panic.42()

declare ptr @__wosy_core_alloc.43(i64, i64)

declare void @__wosy_core_free.44(ptr)

declare void @__wosy_core_system_panic.45()

declare ptr @__wosy_core_alloc.46(i64, i64)

declare void @__wosy_core_free.47(ptr)

declare void @__wosy_core_system_panic.48()

declare ptr @__wosy_core_alloc.49(i64, i64)

declare void @__wosy_core_free.50(ptr)

declare void @__wosy_core_system_panic.51()

declare ptr @__wosy_core_alloc.52(i64, i64)

declare void @__wosy_core_free.53(ptr)

declare void @__wosy_core_system_panic.54()

declare ptr @__wosy_core_alloc.55(i64, i64)

declare void @__wosy_core_free.56(ptr)

declare void @__wosy_core_system_panic.57()

declare ptr @__wosy_core_alloc.58(i64, i64)

declare void @__wosy_core_free.59(ptr)

declare void @__wosy_core_system_panic.60()

declare ptr @__wosy_core_alloc.61(i64, i64)

declare void @__wosy_core_free.62(ptr)

declare void @__wosy_core_system_panic.63()

declare ptr @__wosy_core_alloc.64(i64, i64)

declare void @__wosy_core_free.65(ptr)

declare void @__wosy_core_system_panic.66()

declare ptr @__wosy_core_alloc.67(i64, i64)

declare void @__wosy_core_free.68(ptr)

declare void @__wosy_core_system_panic.69()

declare ptr @__wosy_core_alloc.70(i64, i64)

declare void @__wosy_core_free.71(ptr)

declare void @__wosy_core_system_panic.72()

declare ptr @__wosy_core_alloc.73(i64, i64)

declare void @__wosy_core_free.74(ptr)

declare void @__wosy_core_system_panic.75()

declare ptr @__wosy_core_alloc.76(i64, i64)

declare void @__wosy_core_free.77(ptr)

declare void @__wosy_core_system_panic.78()

declare ptr @__wosy_core_alloc.79(i64, i64)

declare void @__wosy_core_free.80(ptr)

declare void @__wosy_core_system_panic.81()

declare ptr @__wosy_core_alloc.82(i64, i64)

declare void @__wosy_core_free.83(ptr)

declare void @__wosy_core_system_panic.84()

declare ptr @__wosy_core_alloc.85(i64, i64)

declare void @__wosy_core_free.86(ptr)

declare void @__wosy_core_system_panic.87()

declare ptr @__wosy_core_alloc.88(i64, i64)

declare void @__wosy_core_free.89(ptr)

declare void @__wosy_core_system_panic.90()