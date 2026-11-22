; ModuleID = 'src/main.w'
source_filename = "src/main.w"

declare i32 @wosy_extern__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f77617369__776173695f736e617073686f745f7072657669657731__5f66645f72656164(i32, i32, i32, i32) #2
declare i32 @wosy_extern__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f77617369__776173695f736e617073686f745f7072657669657731__66645f7772697465(i32, i32, i32, i32) #3
declare i32 @wosy_extern__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f77617369__776173695f736e617073686f745f7072657669657731__5f706f6c6c5f6f6e656f6666(i32, i32, i32, i32) #4
declare void @wosy_extern__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f77617369__776173695f736e617073686f745f7072657669657731__5f70726f635f65786974(i32) #5

attributes #2 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="fd_read" }
attributes #3 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="fd_write" }
attributes #4 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="poll_oneoff" }
attributes #5 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="proc_exit" }

@wosy_utf8_literal_0 = private constant [1 x i8] c"\0A"
@wosy_utf8_literal_1 = private constant [1 x i8] c"\0A"
@wosy_utf8_literal_2 = private constant [1 x i8] c"\0A"
@wosy_utf8_literal_3 = private constant [1 x i8] c"\0A"

define void @wosy_fn__706f696e7465722d636173742d6469726563742d6465726566__7372632f6d61696e2e77__31346464306661643536626362366663623063663063626463643633653832306561393231306466313563376263613065336534653764353363643764623866__72756e() {
entry:
  %utf8_literal59 = alloca [16 x i8], align 8
  %aggregate_destination55 = alloca [16 x i8], align 8
  %mutable_field = alloca i8, align 1
  %utf8_literal44 = alloca [16 x i8], align 8
  %aggregate_destination40 = alloca [16 x i8], align 8
  %shared_field = alloca i8, align 1
  %assignment_value25 = alloca i8, align 1
  %utf8_literal21 = alloca [16 x i8], align 8
  %aggregate_destination17 = alloca [16 x i8], align 8
  %mutable_scalar = alloca i8, align 1
  %utf8_literal = alloca [16 x i8], align 8
  %aggregate_destination = alloca [16 x i8], align 8
  %shared_scalar = alloca i8, align 1
  %assignment_value = alloca i8, align 1
  %writer = alloca i32, align 4
  %raw = alloca i32, align 4
  %allocation = call ptr @__wosy_core_alloc(i64 1, i64 1)
  %allocation_address = ptrtoint ptr %allocation to i32
  store i32 %allocation_address, ptr %raw, align 4
  %raw1 = load i32, ptr %raw, align 4
  store i32 %raw1, ptr %writer, align 4
  store i8 65, ptr %assignment_value, align 1
  %assignment_value2 = load i8, ptr %assignment_value, align 1
  %writer3 = load i32, ptr %writer, align 4
  %deref_is_null = icmp eq i32 %writer3, 0
  br i1 %deref_is_null, label %deref_null_panic, label %deref_null_continue

deref_null_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

deref_null_continue:                              ; preds = %entry
  %deref = inttoptr i32 %writer3 to ptr
  %tag = getelementptr inbounds i8, ptr %deref, i8 0
  store i8 %assignment_value2, ptr %tag, align 1
  %raw4 = load i32, ptr %raw, align 4
  %deref_is_null5 = icmp eq i32 %raw4, 0
  br i1 %deref_is_null5, label %deref_null_panic6, label %deref_null_continue7

deref_null_panic6:                                ; preds = %deref_null_continue
  call void @__wosy_core_system_panic()
  unreachable

deref_null_continue7:                             ; preds = %deref_null_continue
  %deref8 = inttoptr i32 %raw4 to ptr
  %place = load i8, ptr %deref8, align 1
  store i8 %place, ptr %shared_scalar, align 1
  %shared_scalar9 = load i8, ptr %shared_scalar, align 1
  %aggregate_original_base = call ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f3337__(i8 %shared_scalar9, ptr %aggregate_destination)
  %aggregate_input_call = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__7072696e74(ptr %aggregate_destination, ptr %aggregate_original_base)
  %data = getelementptr inbounds i8, ptr %utf8_literal, i8 0
  store i32 ptrtoint (ptr @wosy_utf8_literal_0 to i32), ptr %data, align 4
  %length = getelementptr inbounds i8, ptr %utf8_literal, i8 8
  store i64 1, ptr %length, align 4
  %aggregate_input_call10 = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__7072696e74(ptr %utf8_literal, ptr null)
  %raw11 = load i32, ptr %raw, align 4
  %deref_is_null12 = icmp eq i32 %raw11, 0
  br i1 %deref_is_null12, label %deref_null_panic13, label %deref_null_continue14

deref_null_panic13:                               ; preds = %deref_null_continue7
  call void @__wosy_core_system_panic()
  unreachable

deref_null_continue14:                            ; preds = %deref_null_continue7
  %deref15 = inttoptr i32 %raw11 to ptr
  %place16 = load i8, ptr %deref15, align 1
  store i8 %place16, ptr %mutable_scalar, align 1
  %mutable_scalar18 = load i8, ptr %mutable_scalar, align 1
  %aggregate_original_base19 = call ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f3337__(i8 %mutable_scalar18, ptr %aggregate_destination17)
  %aggregate_input_call20 = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__7072696e74(ptr %aggregate_destination17, ptr %aggregate_original_base19)
  %data22 = getelementptr inbounds i8, ptr %utf8_literal21, i8 0
  store i32 ptrtoint (ptr @wosy_utf8_literal_1 to i32), ptr %data22, align 4
  %length23 = getelementptr inbounds i8, ptr %utf8_literal21, i8 8
  store i64 1, ptr %length23, align 4
  %aggregate_input_call24 = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__7072696e74(ptr %utf8_literal21, ptr null)
  store i8 67, ptr %assignment_value25, align 1
  %assignment_value26 = load i8, ptr %assignment_value25, align 1
  %writer27 = load i32, ptr %writer, align 4
  %deref_is_null28 = icmp eq i32 %writer27, 0
  br i1 %deref_is_null28, label %deref_null_panic29, label %deref_null_continue30

deref_null_panic29:                               ; preds = %deref_null_continue14
  call void @__wosy_core_system_panic()
  unreachable

deref_null_continue30:                            ; preds = %deref_null_continue14
  %deref31 = inttoptr i32 %writer27 to ptr
  %tag32 = getelementptr inbounds i8, ptr %deref31, i8 0
  store i8 %assignment_value26, ptr %tag32, align 1
  %raw33 = load i32, ptr %raw, align 4
  %deref_is_null34 = icmp eq i32 %raw33, 0
  br i1 %deref_is_null34, label %deref_null_panic35, label %deref_null_continue36

deref_null_panic35:                               ; preds = %deref_null_continue30
  call void @__wosy_core_system_panic()
  unreachable

deref_null_continue36:                            ; preds = %deref_null_continue30
  %deref37 = inttoptr i32 %raw33 to ptr
  %tag38 = getelementptr inbounds i8, ptr %deref37, i8 0
  %place39 = load i8, ptr %tag38, align 1
  store i8 %place39, ptr %shared_field, align 1
  %shared_field41 = load i8, ptr %shared_field, align 1
  %aggregate_original_base42 = call ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f3337__(i8 %shared_field41, ptr %aggregate_destination40)
  %aggregate_input_call43 = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__7072696e74(ptr %aggregate_destination40, ptr %aggregate_original_base42)
  %data45 = getelementptr inbounds i8, ptr %utf8_literal44, i8 0
  store i32 ptrtoint (ptr @wosy_utf8_literal_2 to i32), ptr %data45, align 4
  %length46 = getelementptr inbounds i8, ptr %utf8_literal44, i8 8
  store i64 1, ptr %length46, align 4
  %aggregate_input_call47 = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__7072696e74(ptr %utf8_literal44, ptr null)
  %raw48 = load i32, ptr %raw, align 4
  %deref_is_null49 = icmp eq i32 %raw48, 0
  br i1 %deref_is_null49, label %deref_null_panic50, label %deref_null_continue51

deref_null_panic50:                               ; preds = %deref_null_continue36
  call void @__wosy_core_system_panic()
  unreachable

deref_null_continue51:                            ; preds = %deref_null_continue36
  %deref52 = inttoptr i32 %raw48 to ptr
  %tag53 = getelementptr inbounds i8, ptr %deref52, i8 0
  %place54 = load i8, ptr %tag53, align 1
  store i8 %place54, ptr %mutable_field, align 1
  %mutable_field56 = load i8, ptr %mutable_field, align 1
  %aggregate_original_base57 = call ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f3337__(i8 %mutable_field56, ptr %aggregate_destination55)
  %aggregate_input_call58 = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__7072696e74(ptr %aggregate_destination55, ptr %aggregate_original_base57)
  %data60 = getelementptr inbounds i8, ptr %utf8_literal59, i8 0
  store i32 ptrtoint (ptr @wosy_utf8_literal_3 to i32), ptr %data60, align 4
  %length61 = getelementptr inbounds i8, ptr %utf8_literal59, i8 8
  store i64 1, ptr %length61, align 4
  %aggregate_input_call62 = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__7072696e74(ptr %utf8_literal59, ptr null)
  %raw63 = load i32, ptr %raw, align 4
  %free_pointer = inttoptr i32 %raw63 to ptr
  call void @__wosy_core_free(ptr %free_pointer)
  ret void
}

define internal { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__7072696e74(ptr %text, ptr %0) #0 {
entry:
  %assignment_value48 = alloca i1, align 1
  %assignment_value42 = alloca i64, align 8
  %assignment_value40 = alloca i1, align 1
  %assignment_value38 = alloca i64, align 8
  %assignment_value36 = alloca i1, align 1
  %assignment_value30 = alloca i64, align 8
  %assignment_value20 = alloca i32, align 4
  %assignment_value = alloca i32, align 4
  %count = alloca i32, align 4
  %result = alloca i32, align 4
  %byte_count = alloca [4 x i8], align 4
  %struct_literal9 = alloca [4 x i8], align 4
  %iovec = alloca [8 x i8], align 4
  %struct_literal = alloca [8 x i8], align 4
  %success = alloca i32, align 4
  %maximum = alloca i64, align 8
  %maximum_u32 = alloca i32, align 4
  %complete = alloca i1, align 1
  %zero = alloca i64, align 8
  %reported = alloca i64, align 8
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
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
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %iovec, ptr align 1 %struct_literal, i32 8, i1 false)
  %value = getelementptr inbounds i8, ptr %struct_literal9, i8 0
  store i32 0, ptr %value, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %byte_count, ptr align 1 %struct_literal9, i32 4, i1 false)
  store i32 0, ptr %result, align 4
  store i32 0, ptr %count, align 4
  %length10 = getelementptr inbounds i8, ptr %text1, i8 8
  %place11 = load i64, ptr %length10, align 4
  %zero12 = load i64, ptr %zero, align 4
  %ne = icmp ne i64 %place11, %zero12
  br i1 %ne, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %length13 = getelementptr inbounds i8, ptr %text1, i8 8
  %place14 = load i64, ptr %length13, align 4
  %maximum15 = load i64, ptr %maximum, align 4
  %le = icmp ule i64 %place14, %maximum15
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %le, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  %address = ptrtoint ptr %iovec to i32
  %address16 = ptrtoint ptr %byte_count to i32
  %call = call i32 @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__66645f77726974655f6f6e6365(i32 1, i32 %address, i32 %address16)
  store i32 %call, ptr %assignment_value, align 4
  %assignment_value17 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value17, ptr %result, align 4
  %value18 = getelementptr inbounds i8, ptr %byte_count, i8 0
  %place19 = load i32, ptr %value18, align 4
  store i32 %place19, ptr %assignment_value20, align 4
  %assignment_value21 = load i32, ptr %assignment_value20, align 4
  store i32 %assignment_value21, ptr %count, align 4
  %result22 = load i32, ptr %result, align 4
  %success23 = load i32, ptr %success, align 4
  %eq24 = icmp eq i32 %result22, %success23
  br i1 %eq24, label %if.then25, label %if.else26

if.else:                                          ; preds = %short_circuit.merge
  store i64 0, ptr %assignment_value42, align 4
  %assignment_value43 = load i64, ptr %assignment_value42, align 4
  store i64 %assignment_value43, ptr %reported, align 4
  %length44 = getelementptr inbounds i8, ptr %text1, i8 8
  %place45 = load i64, ptr %length44, align 4
  %zero46 = load i64, ptr %zero, align 4
  %eq47 = icmp eq i64 %place45, %zero46
  store i1 %eq47, ptr %assignment_value48, align 1
  %assignment_value49 = load i1, ptr %assignment_value48, align 1
  store i1 %assignment_value49, ptr %complete, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.merge27
  %reported50 = load i64, ptr %reported, align 4
  %complete51 = load i1, ptr %complete, align 1
  %output = insertvalue { i64, i1 } zeroinitializer, i64 %reported50, 0
  %output52 = insertvalue { i64, i1 } %output, i1 %complete51, 1
  %aggregate_formal_owned = icmp ne ptr %0, null
  br i1 %aggregate_formal_owned, label %aggregate.formal.release, label %aggregate.formal.end

if.then25:                                        ; preds = %if.then
  %count28 = load i32, ptr %count, align 4
  %int_extend29 = zext i32 %count28 to i64
  store i64 %int_extend29, ptr %assignment_value30, align 4
  %assignment_value31 = load i64, ptr %assignment_value30, align 4
  store i64 %assignment_value31, ptr %reported, align 4
  %reported32 = load i64, ptr %reported, align 4
  %length33 = getelementptr inbounds i8, ptr %text1, i8 8
  %place34 = load i64, ptr %length33, align 4
  %eq35 = icmp eq i64 %reported32, %place34
  store i1 %eq35, ptr %assignment_value36, align 1
  %assignment_value37 = load i1, ptr %assignment_value36, align 1
  store i1 %assignment_value37, ptr %complete, align 1
  br label %if.merge27

if.else26:                                        ; preds = %if.then
  store i64 0, ptr %assignment_value38, align 4
  %assignment_value39 = load i64, ptr %assignment_value38, align 4
  store i64 %assignment_value39, ptr %reported, align 4
  store i1 false, ptr %assignment_value40, align 1
  %assignment_value41 = load i1, ptr %assignment_value40, align 1
  store i1 %assignment_value41, ptr %complete, align 1
  br label %if.merge27

if.merge27:                                       ; preds = %if.else26, %if.then25
  br label %if.merge

aggregate.formal.release:                         ; preds = %if.merge
  call void @__wosy_core_free(ptr %0)
  br label %aggregate.formal.end

aggregate.formal.end:                             ; preds = %aggregate.formal.release, %if.merge
  ret { i64, i1 } %output52
}

define { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__657072696e74(ptr %text) #0 {
entry:
  %assignment_value48 = alloca i1, align 1
  %assignment_value42 = alloca i64, align 8
  %assignment_value40 = alloca i1, align 1
  %assignment_value38 = alloca i64, align 8
  %assignment_value36 = alloca i1, align 1
  %assignment_value30 = alloca i64, align 8
  %assignment_value20 = alloca i32, align 4
  %assignment_value = alloca i32, align 4
  %count = alloca i32, align 4
  %result = alloca i32, align 4
  %byte_count = alloca [4 x i8], align 4
  %struct_literal9 = alloca [4 x i8], align 4
  %iovec = alloca [8 x i8], align 4
  %struct_literal = alloca [8 x i8], align 4
  %success = alloca i32, align 4
  %maximum = alloca i64, align 8
  %maximum_u32 = alloca i32, align 4
  %complete = alloca i1, align 1
  %zero = alloca i64, align 8
  %reported = alloca i64, align 8
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
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
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %iovec, ptr align 1 %struct_literal, i32 8, i1 false)
  %value = getelementptr inbounds i8, ptr %struct_literal9, i8 0
  store i32 0, ptr %value, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %byte_count, ptr align 1 %struct_literal9, i32 4, i1 false)
  store i32 0, ptr %result, align 4
  store i32 0, ptr %count, align 4
  %length10 = getelementptr inbounds i8, ptr %text1, i8 8
  %place11 = load i64, ptr %length10, align 4
  %zero12 = load i64, ptr %zero, align 4
  %ne = icmp ne i64 %place11, %zero12
  br i1 %ne, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %length13 = getelementptr inbounds i8, ptr %text1, i8 8
  %place14 = load i64, ptr %length13, align 4
  %maximum15 = load i64, ptr %maximum, align 4
  %le = icmp ule i64 %place14, %maximum15
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %le, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  %address = ptrtoint ptr %iovec to i32
  %address16 = ptrtoint ptr %byte_count to i32
  %call = call i32 @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__66645f77726974655f6f6e6365(i32 2, i32 %address, i32 %address16)
  store i32 %call, ptr %assignment_value, align 4
  %assignment_value17 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value17, ptr %result, align 4
  %value18 = getelementptr inbounds i8, ptr %byte_count, i8 0
  %place19 = load i32, ptr %value18, align 4
  store i32 %place19, ptr %assignment_value20, align 4
  %assignment_value21 = load i32, ptr %assignment_value20, align 4
  store i32 %assignment_value21, ptr %count, align 4
  %result22 = load i32, ptr %result, align 4
  %success23 = load i32, ptr %success, align 4
  %eq24 = icmp eq i32 %result22, %success23
  br i1 %eq24, label %if.then25, label %if.else26

if.else:                                          ; preds = %short_circuit.merge
  store i64 0, ptr %assignment_value42, align 4
  %assignment_value43 = load i64, ptr %assignment_value42, align 4
  store i64 %assignment_value43, ptr %reported, align 4
  %length44 = getelementptr inbounds i8, ptr %text1, i8 8
  %place45 = load i64, ptr %length44, align 4
  %zero46 = load i64, ptr %zero, align 4
  %eq47 = icmp eq i64 %place45, %zero46
  store i1 %eq47, ptr %assignment_value48, align 1
  %assignment_value49 = load i1, ptr %assignment_value48, align 1
  store i1 %assignment_value49, ptr %complete, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.merge27
  %reported50 = load i64, ptr %reported, align 4
  %complete51 = load i1, ptr %complete, align 1
  %output = insertvalue { i64, i1 } zeroinitializer, i64 %reported50, 0
  %output52 = insertvalue { i64, i1 } %output, i1 %complete51, 1
  ret { i64, i1 } %output52

if.then25:                                        ; preds = %if.then
  %count28 = load i32, ptr %count, align 4
  %int_extend29 = zext i32 %count28 to i64
  store i64 %int_extend29, ptr %assignment_value30, align 4
  %assignment_value31 = load i64, ptr %assignment_value30, align 4
  store i64 %assignment_value31, ptr %reported, align 4
  %reported32 = load i64, ptr %reported, align 4
  %length33 = getelementptr inbounds i8, ptr %text1, i8 8
  %place34 = load i64, ptr %length33, align 4
  %eq35 = icmp eq i64 %reported32, %place34
  store i1 %eq35, ptr %assignment_value36, align 1
  %assignment_value37 = load i1, ptr %assignment_value36, align 1
  store i1 %assignment_value37, ptr %complete, align 1
  br label %if.merge27

if.else26:                                        ; preds = %if.then
  store i64 0, ptr %assignment_value38, align 4
  %assignment_value39 = load i64, ptr %assignment_value38, align 4
  store i64 %assignment_value39, ptr %reported, align 4
  store i1 false, ptr %assignment_value40, align 1
  %assignment_value41 = load i1, ptr %assignment_value40, align 1
  store i1 %assignment_value41, ptr %complete, align 1
  br label %if.merge27

if.merge27:                                       ; preds = %if.else26, %if.then25
  br label %if.merge
}

define { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__726561645f696e746f(i32 %destination, i64 %capacity) {
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

define void @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__65786974(i32 %code) {
entry:
  %code1 = alloca i32, align 4
  store i32 %code, ptr %code1, align 4
  %code2 = load i32, ptr %code1, align 4
  call void @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__65786974(i32 %code2)
  ret void
}

define void @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__736c6565705f6e73(i64 %nanoseconds) {
entry:
  %nanoseconds1 = alloca i64, align 8
  store i64 %nanoseconds, ptr %nanoseconds1, align 4
  %nanoseconds2 = load i64, ptr %nanoseconds1, align 4
  call void @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__736c6565705f6e73(i64 %nanoseconds2)
  ret void
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f656f665f737461747573(i1 %saw_input, i32 %status) {
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

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f63617061636974795f737461747573(i64 %length, i64 %maximum, i32 %status) {
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

define { ptr, i64, i8, i8, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f757466385f73746570(ptr %bytes, i64 %index, i32 %result, i64 %remaining, i8 %next_minimum, i8 %next_maximum) {
entry:
  %assignment_value111 = alloca i32, align 4
  %assignment_value104 = alloca i8, align 1
  %assignment_value97 = alloca i8, align 1
  %assignment_value90 = alloca i64, align 8
  %assignment_value78 = alloca i8, align 1
  %assignment_value71 = alloca i8, align 1
  %assignment_value65 = alloca i64, align 8
  %assignment_value52 = alloca i64, align 8
  %assignment_value38 = alloca i32, align 4
  %assignment_value26 = alloca i8, align 1
  %assignment_value23 = alloca i8, align 1
  %assignment_value20 = alloca i64, align 8
  %assignment_value = alloca i32, align 4
  %byte = alloca i8, align 1
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
  %index2 = alloca i64, align 8
  store i64 %index, ptr %index2, align 4
  %result3 = alloca i32, align 4
  store i32 %result, ptr %result3, align 4
  %remaining4 = alloca i64, align 8
  store i64 %remaining, ptr %remaining4, align 4
  %next_minimum5 = alloca i8, align 1
  store i8 %next_minimum, ptr %next_minimum5, align 1
  %next_maximum6 = alloca i8, align 1
  store i8 %next_maximum, ptr %next_maximum6, align 1
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
  %bytes7 = load ptr, ptr %bytes1, align 8
  %index8 = load i64, ptr %index2, align 4
  %array_element = getelementptr inbounds i8, ptr %bytes7, i64 %index8
  %place = load i8, ptr %array_element, align 1
  store i8 %place, ptr %byte, align 1
  %remaining9 = load i64, ptr %remaining4, align 4
  %zero10 = load i64, ptr %zero, align 4
  %ne = icmp ne i64 %remaining9, %zero10
  br i1 %ne, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  %byte11 = load i8, ptr %byte, align 1
  %next_minimum12 = load i8, ptr %next_minimum5, align 1
  %lt = icmp ult i8 %byte11, %next_minimum12
  br i1 %lt, label %short_circuit.merge, label %short_circuit.rhs

if.else:                                          ; preds = %entry
  %byte28 = load i8, ptr %byte, align 1
  %continuation_minimum29 = load i8, ptr %continuation_minimum, align 1
  %ge = icmp uge i8 %byte28, %continuation_minimum29
  br i1 %ge, label %short_circuit.rhs30, label %short_circuit.merge31

if.merge:                                         ; preds = %if.merge110, %if.merge16
  %bytes113 = load ptr, ptr %bytes1, align 8
  %remaining114 = load i64, ptr %remaining4, align 4
  %next_minimum115 = load i8, ptr %next_minimum5, align 1
  %next_maximum116 = load i8, ptr %next_maximum6, align 1
  %result117 = load i32, ptr %result3, align 4
  %output = insertvalue { ptr, i64, i8, i8, i32 } zeroinitializer, ptr %bytes113, 0
  %output118 = insertvalue { ptr, i64, i8, i8, i32 } %output, i64 %remaining114, 1
  %output119 = insertvalue { ptr, i64, i8, i8, i32 } %output118, i8 %next_minimum115, 2
  %output120 = insertvalue { ptr, i64, i8, i8, i32 } %output119, i8 %next_maximum116, 3
  %output121 = insertvalue { ptr, i64, i8, i8, i32 } %output120, i32 %result117, 4
  ret { ptr, i64, i8, i8, i32 } %output121

short_circuit.rhs:                                ; preds = %if.then
  %byte13 = load i8, ptr %byte, align 1
  %next_maximum14 = load i8, ptr %next_maximum6, align 1
  %gt = icmp ugt i8 %byte13, %next_maximum14
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %if.then
  %short_circuit = phi i1 [ true, %if.then ], [ %gt, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then15, label %if.merge16

if.then15:                                        ; preds = %short_circuit.merge
  store i32 3, ptr %assignment_value, align 4
  %assignment_value17 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value17, ptr %result3, align 4
  br label %if.merge16

if.merge16:                                       ; preds = %if.then15, %short_circuit.merge
  %remaining18 = load i64, ptr %remaining4, align 4
  %one19 = load i64, ptr %one, align 4
  %sub = sub i64 %remaining18, %one19
  store i64 %sub, ptr %assignment_value20, align 4
  %assignment_value21 = load i64, ptr %assignment_value20, align 4
  store i64 %assignment_value21, ptr %remaining4, align 4
  %continuation_minimum22 = load i8, ptr %continuation_minimum, align 1
  store i8 %continuation_minimum22, ptr %assignment_value23, align 1
  %assignment_value24 = load i8, ptr %assignment_value23, align 1
  store i8 %assignment_value24, ptr %next_minimum5, align 1
  %continuation_maximum25 = load i8, ptr %continuation_maximum, align 1
  store i8 %continuation_maximum25, ptr %assignment_value26, align 1
  %assignment_value27 = load i8, ptr %assignment_value26, align 1
  store i8 %assignment_value27, ptr %next_maximum6, align 1
  br label %if.merge

short_circuit.rhs30:                              ; preds = %if.else
  %byte32 = load i8, ptr %byte, align 1
  %two_byte_minimum33 = load i8, ptr %two_byte_minimum, align 1
  %lt34 = icmp ult i8 %byte32, %two_byte_minimum33
  br label %short_circuit.merge31

short_circuit.merge31:                            ; preds = %short_circuit.rhs30, %if.else
  %short_circuit35 = phi i1 [ false, %if.else ], [ %lt34, %short_circuit.rhs30 ]
  br i1 %short_circuit35, label %if.then36, label %if.merge37

if.then36:                                        ; preds = %short_circuit.merge31
  store i32 3, ptr %assignment_value38, align 4
  %assignment_value39 = load i32, ptr %assignment_value38, align 4
  store i32 %assignment_value39, ptr %result3, align 4
  br label %if.merge37

if.merge37:                                       ; preds = %if.then36, %short_circuit.merge31
  %byte40 = load i8, ptr %byte, align 1
  %two_byte_minimum41 = load i8, ptr %two_byte_minimum, align 1
  %ge42 = icmp uge i8 %byte40, %two_byte_minimum41
  br i1 %ge42, label %short_circuit.rhs43, label %short_circuit.merge44

short_circuit.rhs43:                              ; preds = %if.merge37
  %byte45 = load i8, ptr %byte, align 1
  %three_byte_minimum46 = load i8, ptr %three_byte_minimum, align 1
  %lt47 = icmp ult i8 %byte45, %three_byte_minimum46
  br label %short_circuit.merge44

short_circuit.merge44:                            ; preds = %short_circuit.rhs43, %if.merge37
  %short_circuit48 = phi i1 [ false, %if.merge37 ], [ %lt47, %short_circuit.rhs43 ]
  br i1 %short_circuit48, label %if.then49, label %if.merge50

if.then49:                                        ; preds = %short_circuit.merge44
  %one51 = load i64, ptr %one, align 4
  store i64 %one51, ptr %assignment_value52, align 4
  %assignment_value53 = load i64, ptr %assignment_value52, align 4
  store i64 %assignment_value53, ptr %remaining4, align 4
  br label %if.merge50

if.merge50:                                       ; preds = %if.then49, %short_circuit.merge44
  %byte54 = load i8, ptr %byte, align 1
  %three_byte_minimum55 = load i8, ptr %three_byte_minimum, align 1
  %ge56 = icmp uge i8 %byte54, %three_byte_minimum55
  br i1 %ge56, label %short_circuit.rhs57, label %short_circuit.merge58

short_circuit.rhs57:                              ; preds = %if.merge50
  %byte59 = load i8, ptr %byte, align 1
  %four_byte_minimum60 = load i8, ptr %four_byte_minimum, align 1
  %lt61 = icmp ult i8 %byte59, %four_byte_minimum60
  br label %short_circuit.merge58

short_circuit.merge58:                            ; preds = %short_circuit.rhs57, %if.merge50
  %short_circuit62 = phi i1 [ false, %if.merge50 ], [ %lt61, %short_circuit.rhs57 ]
  br i1 %short_circuit62, label %if.then63, label %if.merge64

if.then63:                                        ; preds = %short_circuit.merge58
  store i64 2, ptr %assignment_value65, align 4
  %assignment_value66 = load i64, ptr %assignment_value65, align 4
  store i64 %assignment_value66, ptr %remaining4, align 4
  %byte67 = load i8, ptr %byte, align 1
  %e068 = load i8, ptr %e0, align 1
  %eq = icmp eq i8 %byte67, %e068
  br i1 %eq, label %if.then69, label %if.merge70

if.merge64:                                       ; preds = %if.merge77, %short_circuit.merge58
  %byte80 = load i8, ptr %byte, align 1
  %four_byte_minimum81 = load i8, ptr %four_byte_minimum, align 1
  %ge82 = icmp uge i8 %byte80, %four_byte_minimum81
  br i1 %ge82, label %short_circuit.rhs83, label %short_circuit.merge84

if.then69:                                        ; preds = %if.then63
  store i8 -96, ptr %assignment_value71, align 1
  %assignment_value72 = load i8, ptr %assignment_value71, align 1
  store i8 %assignment_value72, ptr %next_minimum5, align 1
  br label %if.merge70

if.merge70:                                       ; preds = %if.then69, %if.then63
  %byte73 = load i8, ptr %byte, align 1
  %ed74 = load i8, ptr %ed, align 1
  %eq75 = icmp eq i8 %byte73, %ed74
  br i1 %eq75, label %if.then76, label %if.merge77

if.then76:                                        ; preds = %if.merge70
  store i8 -97, ptr %assignment_value78, align 1
  %assignment_value79 = load i8, ptr %assignment_value78, align 1
  store i8 %assignment_value79, ptr %next_maximum6, align 1
  br label %if.merge77

if.merge77:                                       ; preds = %if.then76, %if.merge70
  br label %if.merge64

short_circuit.rhs83:                              ; preds = %if.merge64
  %byte85 = load i8, ptr %byte, align 1
  %four_byte_limit86 = load i8, ptr %four_byte_limit, align 1
  %le = icmp ule i8 %byte85, %four_byte_limit86
  br label %short_circuit.merge84

short_circuit.merge84:                            ; preds = %short_circuit.rhs83, %if.merge64
  %short_circuit87 = phi i1 [ false, %if.merge64 ], [ %le, %short_circuit.rhs83 ]
  br i1 %short_circuit87, label %if.then88, label %if.merge89

if.then88:                                        ; preds = %short_circuit.merge84
  store i64 3, ptr %assignment_value90, align 4
  %assignment_value91 = load i64, ptr %assignment_value90, align 4
  store i64 %assignment_value91, ptr %remaining4, align 4
  %byte92 = load i8, ptr %byte, align 1
  %f093 = load i8, ptr %f0, align 1
  %eq94 = icmp eq i8 %byte92, %f093
  br i1 %eq94, label %if.then95, label %if.merge96

if.merge89:                                       ; preds = %if.merge103, %short_circuit.merge84
  %byte106 = load i8, ptr %byte, align 1
  %four_byte_limit107 = load i8, ptr %four_byte_limit, align 1
  %gt108 = icmp ugt i8 %byte106, %four_byte_limit107
  br i1 %gt108, label %if.then109, label %if.merge110

if.then95:                                        ; preds = %if.then88
  store i8 -112, ptr %assignment_value97, align 1
  %assignment_value98 = load i8, ptr %assignment_value97, align 1
  store i8 %assignment_value98, ptr %next_minimum5, align 1
  br label %if.merge96

if.merge96:                                       ; preds = %if.then95, %if.then88
  %byte99 = load i8, ptr %byte, align 1
  %f4100 = load i8, ptr %f4, align 1
  %eq101 = icmp eq i8 %byte99, %f4100
  br i1 %eq101, label %if.then102, label %if.merge103

if.then102:                                       ; preds = %if.merge96
  store i8 -113, ptr %assignment_value104, align 1
  %assignment_value105 = load i8, ptr %assignment_value104, align 1
  store i8 %assignment_value105, ptr %next_maximum6, align 1
  br label %if.merge103

if.merge103:                                      ; preds = %if.then102, %if.merge96
  br label %if.merge89

if.then109:                                       ; preds = %if.merge89
  store i32 3, ptr %assignment_value111, align 4
  %assignment_value112 = load i32, ptr %assignment_value111, align 4
  store i32 %assignment_value112, ptr %result3, align 4
  br label %if.merge110

if.merge110:                                      ; preds = %if.then109, %if.merge89
  br label %if.merge
}

define { ptr, i64, i8, i8, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f757466385f72616e6765(ptr %bytes, i64 %low, i64 %high, i64 %remaining, i8 %minimum, i8 %maximum, i32 %status) {
entry:
  %assignment_value82 = alloca i32, align 4
  %assignment_value79 = alloca i8, align 1
  %assignment_value76 = alloca i8, align 1
  %assignment_value73 = alloca i64, align 8
  %assignment_value70 = alloca ptr, align 8
  %assignment_value59 = alloca i32, align 4
  %assignment_value56 = alloca i8, align 1
  %assignment_value53 = alloca i8, align 1
  %assignment_value50 = alloca i64, align 8
  %assignment_value47 = alloca ptr, align 8
  %middle = alloca i64, align 8
  %two = alloca i64, align 8
  %assignment_value33 = alloca i32, align 4
  %assignment_value30 = alloca i8, align 1
  %assignment_value27 = alloca i8, align 1
  %assignment_value24 = alloca i64, align 8
  %assignment_value = alloca ptr, align 8
  %count = alloca i64, align 8
  %one = alloca i64, align 8
  %bytes1 = alloca ptr, align 8
  store ptr %bytes, ptr %bytes1, align 8
  %low2 = alloca i64, align 8
  store i64 %low, ptr %low2, align 4
  %high3 = alloca i64, align 8
  store i64 %high, ptr %high3, align 4
  %remaining4 = alloca i64, align 8
  store i64 %remaining, ptr %remaining4, align 4
  %minimum5 = alloca i8, align 1
  store i8 %minimum, ptr %minimum5, align 1
  %maximum6 = alloca i8, align 1
  store i8 %maximum, ptr %maximum6, align 1
  %status7 = alloca i32, align 4
  store i32 %status, ptr %status7, align 4
  store i64 1, ptr %one, align 4
  %low8 = load i64, ptr %low2, align 4
  %high9 = load i64, ptr %high3, align 4
  %lt = icmp ult i64 %low8, %high9
  br i1 %lt, label %if.then, label %if.merge

if.then:                                          ; preds = %entry
  %high10 = load i64, ptr %high3, align 4
  %low11 = load i64, ptr %low2, align 4
  %sub = sub i64 %high10, %low11
  store i64 %sub, ptr %count, align 4
  %count12 = load i64, ptr %count, align 4
  %one13 = load i64, ptr %one, align 4
  %eq = icmp eq i64 %count12, %one13
  br i1 %eq, label %if.then14, label %if.else

if.merge:                                         ; preds = %if.merge15, %entry
  %bytes84 = load ptr, ptr %bytes1, align 8
  %remaining85 = load i64, ptr %remaining4, align 4
  %minimum86 = load i8, ptr %minimum5, align 1
  %maximum87 = load i8, ptr %maximum6, align 1
  %status88 = load i32, ptr %status7, align 4
  %output89 = insertvalue { ptr, i64, i8, i8, i32 } zeroinitializer, ptr %bytes84, 0
  %output90 = insertvalue { ptr, i64, i8, i8, i32 } %output89, i64 %remaining85, 1
  %output91 = insertvalue { ptr, i64, i8, i8, i32 } %output90, i8 %minimum86, 2
  %output92 = insertvalue { ptr, i64, i8, i8, i32 } %output91, i8 %maximum87, 3
  %output93 = insertvalue { ptr, i64, i8, i8, i32 } %output92, i32 %status88, 4
  ret { ptr, i64, i8, i8, i32 } %output93

if.then14:                                        ; preds = %if.then
  %bytes16 = load ptr, ptr %bytes1, align 8
  %low17 = load i64, ptr %low2, align 4
  %status18 = load i32, ptr %status7, align 4
  %remaining19 = load i64, ptr %remaining4, align 4
  %minimum20 = load i8, ptr %minimum5, align 1
  %maximum21 = load i8, ptr %maximum6, align 1
  %call = call { ptr, i64, i8, i8, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f757466385f73746570(ptr %bytes16, i64 %low17, i32 %status18, i64 %remaining19, i8 %minimum20, i8 %maximum21)
  %output = extractvalue { ptr, i64, i8, i8, i32 } %call, 0
  store ptr %output, ptr %assignment_value, align 8
  %assignment_value22 = load ptr, ptr %assignment_value, align 8
  %output23 = extractvalue { ptr, i64, i8, i8, i32 } %call, 1
  store i64 %output23, ptr %assignment_value24, align 4
  %assignment_value25 = load i64, ptr %assignment_value24, align 4
  %output26 = extractvalue { ptr, i64, i8, i8, i32 } %call, 2
  store i8 %output26, ptr %assignment_value27, align 1
  %assignment_value28 = load i8, ptr %assignment_value27, align 1
  %output29 = extractvalue { ptr, i64, i8, i8, i32 } %call, 3
  store i8 %output29, ptr %assignment_value30, align 1
  %assignment_value31 = load i8, ptr %assignment_value30, align 1
  %output32 = extractvalue { ptr, i64, i8, i8, i32 } %call, 4
  store i32 %output32, ptr %assignment_value33, align 4
  %assignment_value34 = load i32, ptr %assignment_value33, align 4
  store ptr %assignment_value22, ptr %bytes1, align 8
  store i64 %assignment_value25, ptr %remaining4, align 4
  store i8 %assignment_value28, ptr %minimum5, align 1
  store i8 %assignment_value31, ptr %maximum6, align 1
  store i32 %assignment_value34, ptr %status7, align 4
  br label %if.merge15

if.else:                                          ; preds = %if.then
  store i64 2, ptr %two, align 4
  %low35 = load i64, ptr %low2, align 4
  %count36 = load i64, ptr %count, align 4
  %two37 = load i64, ptr %two, align 4
  %div = udiv i64 %count36, %two37
  %add = add i64 %low35, %div
  store i64 %add, ptr %middle, align 4
  %bytes38 = load ptr, ptr %bytes1, align 8
  %low39 = load i64, ptr %low2, align 4
  %middle40 = load i64, ptr %middle, align 4
  %remaining41 = load i64, ptr %remaining4, align 4
  %minimum42 = load i8, ptr %minimum5, align 1
  %maximum43 = load i8, ptr %maximum6, align 1
  %status44 = load i32, ptr %status7, align 4
  %call45 = call { ptr, i64, i8, i8, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f757466385f72616e6765(ptr %bytes38, i64 %low39, i64 %middle40, i64 %remaining41, i8 %minimum42, i8 %maximum43, i32 %status44)
  %output46 = extractvalue { ptr, i64, i8, i8, i32 } %call45, 0
  store ptr %output46, ptr %assignment_value47, align 8
  %assignment_value48 = load ptr, ptr %assignment_value47, align 8
  %output49 = extractvalue { ptr, i64, i8, i8, i32 } %call45, 1
  store i64 %output49, ptr %assignment_value50, align 4
  %assignment_value51 = load i64, ptr %assignment_value50, align 4
  %output52 = extractvalue { ptr, i64, i8, i8, i32 } %call45, 2
  store i8 %output52, ptr %assignment_value53, align 1
  %assignment_value54 = load i8, ptr %assignment_value53, align 1
  %output55 = extractvalue { ptr, i64, i8, i8, i32 } %call45, 3
  store i8 %output55, ptr %assignment_value56, align 1
  %assignment_value57 = load i8, ptr %assignment_value56, align 1
  %output58 = extractvalue { ptr, i64, i8, i8, i32 } %call45, 4
  store i32 %output58, ptr %assignment_value59, align 4
  %assignment_value60 = load i32, ptr %assignment_value59, align 4
  store ptr %assignment_value48, ptr %bytes1, align 8
  store i64 %assignment_value51, ptr %remaining4, align 4
  store i8 %assignment_value54, ptr %minimum5, align 1
  store i8 %assignment_value57, ptr %maximum6, align 1
  store i32 %assignment_value60, ptr %status7, align 4
  %bytes61 = load ptr, ptr %bytes1, align 8
  %middle62 = load i64, ptr %middle, align 4
  %high63 = load i64, ptr %high3, align 4
  %remaining64 = load i64, ptr %remaining4, align 4
  %minimum65 = load i8, ptr %minimum5, align 1
  %maximum66 = load i8, ptr %maximum6, align 1
  %status67 = load i32, ptr %status7, align 4
  %call68 = call { ptr, i64, i8, i8, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f757466385f72616e6765(ptr %bytes61, i64 %middle62, i64 %high63, i64 %remaining64, i8 %minimum65, i8 %maximum66, i32 %status67)
  %output69 = extractvalue { ptr, i64, i8, i8, i32 } %call68, 0
  store ptr %output69, ptr %assignment_value70, align 8
  %assignment_value71 = load ptr, ptr %assignment_value70, align 8
  %output72 = extractvalue { ptr, i64, i8, i8, i32 } %call68, 1
  store i64 %output72, ptr %assignment_value73, align 4
  %assignment_value74 = load i64, ptr %assignment_value73, align 4
  %output75 = extractvalue { ptr, i64, i8, i8, i32 } %call68, 2
  store i8 %output75, ptr %assignment_value76, align 1
  %assignment_value77 = load i8, ptr %assignment_value76, align 1
  %output78 = extractvalue { ptr, i64, i8, i8, i32 } %call68, 3
  store i8 %output78, ptr %assignment_value79, align 1
  %assignment_value80 = load i8, ptr %assignment_value79, align 1
  %output81 = extractvalue { ptr, i64, i8, i8, i32 } %call68, 4
  store i32 %output81, ptr %assignment_value82, align 4
  %assignment_value83 = load i32, ptr %assignment_value82, align 4
  store ptr %assignment_value71, ptr %bytes1, align 8
  store i64 %assignment_value74, ptr %remaining4, align 4
  store i8 %assignment_value77, ptr %minimum5, align 1
  store i8 %assignment_value80, ptr %maximum6, align 1
  store i32 %assignment_value83, ptr %status7, align 4
  br label %if.merge15

if.merge15:                                       ; preds = %if.else, %if.then14
  br label %if.merge
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f646973636172645f73746570(i32 %view) {
entry:
  %assignment_value7 = alloca i1, align 1
  %assignment_value = alloca i64, align 8
  %complete = alloca i1, align 1
  %count = alloca i64, align 8
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %view1 = alloca i32, align 4
  store i32 %view, ptr %view1, align 4
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  %zero2 = load i64, ptr %zero, align 4
  store i64 %zero2, ptr %count, align 4
  store i1 false, ptr %complete, align 1
  %view3 = load i32, ptr %view1, align 4
  %deref_is_null = icmp eq i32 %view3, 0
  br i1 %deref_is_null, label %deref_null_panic, label %deref_null_continue

deref_null_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

deref_null_continue:                              ; preds = %entry
  %deref = inttoptr i32 %view3 to ptr
  %address = ptrtoint ptr %deref to i32
  %one4 = load i64, ptr %one, align 4
  %call = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__726561645f696e746f(i32 %address, i64 %one4)
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
  br i1 %not, label %if.then, label %if.else

if.then:                                          ; preds = %deref_null_continue
  br label %if.merge

if.else:                                          ; preds = %deref_null_continue
  %count10 = load i64, ptr %count, align 4
  %zero11 = load i64, ptr %zero, align 4
  %eq = icmp eq i64 %count10, %zero11
  br i1 %eq, label %if.then12, label %if.else13

if.merge:                                         ; preds = %if.merge14, %if.then
  %if25 = phi i32 [ 2, %if.then ], [ %if24, %if.merge14 ]
  ret i32 %if25

if.then12:                                        ; preds = %if.else
  br label %if.merge14

if.else13:                                        ; preds = %if.else
  %view15 = load i32, ptr %view1, align 4
  %deref_is_null16 = icmp eq i32 %view15, 0
  br i1 %deref_is_null16, label %deref_null_panic17, label %deref_null_continue18

if.merge14:                                       ; preds = %if.merge23, %if.then12
  %if24 = phi i32 [ 1, %if.then12 ], [ %if, %if.merge23 ]
  br label %if.merge

deref_null_panic17:                               ; preds = %if.else13
  call void @__wosy_core_system_panic()
  unreachable

deref_null_continue18:                            ; preds = %if.else13
  %deref19 = inttoptr i32 %view15 to ptr
  %place = load i8, ptr %deref19, align 1
  %eq20 = icmp eq i8 %place, 10
  br i1 %eq20, label %if.then21, label %if.else22

if.then21:                                        ; preds = %deref_null_continue18
  br label %if.merge23

if.else22:                                        ; preds = %deref_null_continue18
  br label %if.merge23

if.merge23:                                       ; preds = %if.else22, %if.then21
  %if = phi i32 [ 1, %if.then21 ], [ 0, %if.else22 ]
  br label %if.merge14
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f646973636172645f636f6e74696e7565(i32 %view, i32 %outcome) {
entry:
  %view1 = alloca i32, align 4
  store i32 %view, ptr %view1, align 4
  %outcome2 = alloca i32, align 4
  store i32 %outcome, ptr %outcome2, align 4
  br label %self_tail.header

self_tail.header:                                 ; preds = %self_tail.then, %entry
  %outcome3 = load i32, ptr %outcome2, align 4
  %eq = icmp eq i32 %outcome3, 0
  br i1 %eq, label %self_tail.then, label %self_tail.else

self_tail.then:                                   ; preds = %self_tail.header
  %view4 = load i32, ptr %view1, align 4
  %view5 = load i32, ptr %view1, align 4
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f646973636172645f73746570(i32 %view5)
  store i32 %view4, ptr %view1, align 4
  store i32 %call, ptr %outcome2, align 4
  br label %self_tail.header

self_tail.else:                                   ; preds = %self_tail.header
  %outcome6 = load i32, ptr %outcome2, align 4
  ret i32 %outcome6
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f7a65726f5f6361706163697479(i8 %first) #0 {
entry:
  %assignment_value60 = alloca i32, align 4
  %outcome = alloca i32, align 4
  %first_outcome = alloca i32, align 4
  %assignment_value46 = alloca i1, align 1
  %assignment_value44 = alloca i32, align 4
  %assignment_value25 = alloca i1, align 1
  %assignment_value17 = alloca i1, align 1
  %assignment_value15 = alloca i32, align 4
  %assignment_value10 = alloca i1, align 1
  %assignment_value = alloca i64, align 8
  %complete = alloca i1, align 1
  %count = alloca i64, align 8
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %discard = alloca i1, align 1
  %status = alloca i32, align 4
  %view = alloca i32, align 4
  %scratch = alloca [1 x i8], align 1
  %array_literal = alloca [1 x i8], align 1
  %first1 = alloca i8, align 1
  store i8 %first, ptr %first1, align 1
  %array_element = getelementptr inbounds [1 x i8], ptr %array_literal, i32 0, i32 0
  store i8 0, ptr %array_element, align 1
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %scratch, ptr align 1 %array_literal, i32 1, i1 false)
  %array_element2 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i32 0
  %address = ptrtoint ptr %array_element2 to i32
  store i32 %address, ptr %view, align 4
  store i32 4, ptr %status, align 4
  store i1 true, ptr %discard, align 1
  %first3 = load i8, ptr %first1, align 1
  %eq = icmp eq i8 %first3, 13
  br i1 %eq, label %if.then, label %if.merge

if.then:                                          ; preds = %entry
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  %zero4 = load i64, ptr %zero, align 4
  store i64 %zero4, ptr %count, align 4
  store i1 false, ptr %complete, align 1
  %view5 = load i32, ptr %view, align 4
  %deref_is_null = icmp eq i32 %view5, 0
  br i1 %deref_is_null, label %deref_null_panic, label %deref_null_continue

if.merge:                                         ; preds = %if.merge43, %entry
  %discard48 = load i1, ptr %discard, align 1
  br i1 %discard48, label %if.then49, label %if.merge50

deref_null_panic:                                 ; preds = %if.then
  call void @__wosy_core_system_panic()
  unreachable

deref_null_continue:                              ; preds = %if.then
  %deref = inttoptr i32 %view5 to ptr
  %address6 = ptrtoint ptr %deref to i32
  %one7 = load i64, ptr %one, align 4
  %call = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__726561645f696e746f(i32 %address6, i64 %one7)
  %output = extractvalue { i64, i1 } %call, 0
  store i64 %output, ptr %assignment_value, align 4
  %assignment_value8 = load i64, ptr %assignment_value, align 4
  %output9 = extractvalue { i64, i1 } %call, 1
  store i1 %output9, ptr %assignment_value10, align 1
  %assignment_value11 = load i1, ptr %assignment_value10, align 1
  store i64 %assignment_value8, ptr %count, align 4
  store i1 %assignment_value11, ptr %complete, align 1
  %complete12 = load i1, ptr %complete, align 1
  %not = xor i1 %complete12, true
  br i1 %not, label %if.then13, label %if.merge14

if.then13:                                        ; preds = %deref_null_continue
  store i32 2, ptr %assignment_value15, align 4
  %assignment_value16 = load i32, ptr %assignment_value15, align 4
  store i32 %assignment_value16, ptr %status, align 4
  store i1 false, ptr %assignment_value17, align 1
  %assignment_value18 = load i1, ptr %assignment_value17, align 1
  store i1 %assignment_value18, ptr %discard, align 1
  br label %if.merge14

if.merge14:                                       ; preds = %if.then13, %deref_null_continue
  %complete19 = load i1, ptr %complete, align 1
  br i1 %complete19, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %if.merge14
  %count20 = load i64, ptr %count, align 4
  %zero21 = load i64, ptr %zero, align 4
  %eq22 = icmp eq i64 %count20, %zero21
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %if.merge14
  %short_circuit = phi i1 [ false, %if.merge14 ], [ %eq22, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then23, label %if.merge24

if.then23:                                        ; preds = %short_circuit.merge
  store i1 false, ptr %assignment_value25, align 1
  %assignment_value26 = load i1, ptr %assignment_value25, align 1
  store i1 %assignment_value26, ptr %discard, align 1
  br label %if.merge24

if.merge24:                                       ; preds = %if.then23, %short_circuit.merge
  %complete27 = load i1, ptr %complete, align 1
  br i1 %complete27, label %short_circuit.rhs28, label %short_circuit.merge29

short_circuit.rhs28:                              ; preds = %if.merge24
  %count30 = load i64, ptr %count, align 4
  %zero31 = load i64, ptr %zero, align 4
  %ne = icmp ne i64 %count30, %zero31
  br label %short_circuit.merge29

short_circuit.merge29:                            ; preds = %short_circuit.rhs28, %if.merge24
  %short_circuit32 = phi i1 [ false, %if.merge24 ], [ %ne, %short_circuit.rhs28 ]
  br i1 %short_circuit32, label %short_circuit.rhs33, label %short_circuit.merge34

short_circuit.rhs33:                              ; preds = %short_circuit.merge29
  %view35 = load i32, ptr %view, align 4
  %deref_is_null36 = icmp eq i32 %view35, 0
  br i1 %deref_is_null36, label %deref_null_panic37, label %deref_null_continue38

short_circuit.merge34:                            ; preds = %deref_null_continue38, %short_circuit.merge29
  %short_circuit41 = phi i1 [ false, %short_circuit.merge29 ], [ %eq40, %deref_null_continue38 ]
  br i1 %short_circuit41, label %if.then42, label %if.merge43

deref_null_panic37:                               ; preds = %short_circuit.rhs33
  call void @__wosy_core_system_panic()
  unreachable

deref_null_continue38:                            ; preds = %short_circuit.rhs33
  %deref39 = inttoptr i32 %view35 to ptr
  %place = load i8, ptr %deref39, align 1
  %eq40 = icmp eq i8 %place, 10
  br label %short_circuit.merge34

if.then42:                                        ; preds = %short_circuit.merge34
  store i32 0, ptr %assignment_value44, align 4
  %assignment_value45 = load i32, ptr %assignment_value44, align 4
  store i32 %assignment_value45, ptr %status, align 4
  store i1 false, ptr %assignment_value46, align 1
  %assignment_value47 = load i1, ptr %assignment_value46, align 1
  store i1 %assignment_value47, ptr %discard, align 1
  br label %if.merge43

if.merge43:                                       ; preds = %if.then42, %short_circuit.merge34
  br label %if.merge

if.then49:                                        ; preds = %if.merge
  %view51 = load i32, ptr %view, align 4
  %call52 = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f646973636172645f73746570(i32 %view51)
  store i32 %call52, ptr %first_outcome, align 4
  %view53 = load i32, ptr %view, align 4
  %first_outcome54 = load i32, ptr %first_outcome, align 4
  %call55 = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f646973636172645f636f6e74696e7565(i32 %view53, i32 %first_outcome54)
  store i32 %call55, ptr %outcome, align 4
  %outcome56 = load i32, ptr %outcome, align 4
  %eq57 = icmp eq i32 %outcome56, 2
  br i1 %eq57, label %if.then58, label %if.merge59

if.merge50:                                       ; preds = %if.merge59, %if.merge
  %status62 = load i32, ptr %status, align 4
  ret i32 %status62

if.then58:                                        ; preds = %if.then49
  store i32 2, ptr %assignment_value60, align 4
  %assignment_value61 = load i32, ptr %assignment_value60, align 4
  store i32 %assignment_value61, ptr %status, align 4
  br label %if.merge59

if.merge59:                                       ; preds = %if.then58, %if.then49
  br label %if.merge50
}

define { ptr, i64, i1, i1, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f616c6c6f63617465645f73746570(ptr %bytes, i32 %view, i64 %max_bytes, i64 %length, i1 %pending_cr, i1 %reading, i32 %status) {
entry:
  %assignment_value99 = alloca i1, align 1
  %assignment_value93 = alloca i64, align 8
  %assignment_value85 = alloca i8, align 1
  %assignment_value77 = alloca i32, align 4
  %assignment_value67 = alloca i1, align 1
  %assignment_value62 = alloca i64, align 8
  %assignment_value56 = alloca i8, align 1
  %assignment_value49 = alloca i32, align 4
  %assignment_value40 = alloca i1, align 1
  %assignment_value38 = alloca i1, align 1
  %byte = alloca i8, align 1
  %assignment_value25 = alloca i1, align 1
  %assignment_value18 = alloca i1, align 1
  %assignment_value16 = alloca i32, align 4
  %assignment_value13 = alloca i1, align 1
  %assignment_value = alloca i64, align 8
  %complete = alloca i1, align 1
  %count = alloca i64, align 8
  %cr = alloca i8, align 1
  %lf = alloca i8, align 1
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %bytes1 = alloca ptr, align 8
  store ptr %bytes, ptr %bytes1, align 8
  %view2 = alloca i32, align 4
  store i32 %view, ptr %view2, align 4
  %max_bytes3 = alloca i64, align 8
  store i64 %max_bytes, ptr %max_bytes3, align 4
  %length4 = alloca i64, align 8
  store i64 %length, ptr %length4, align 4
  %pending_cr5 = alloca i1, align 1
  store i1 %pending_cr, ptr %pending_cr5, align 1
  %reading6 = alloca i1, align 1
  store i1 %reading, ptr %reading6, align 1
  %status7 = alloca i32, align 4
  store i32 %status, ptr %status7, align 4
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store i8 10, ptr %lf, align 1
  store i8 13, ptr %cr, align 1
  %zero8 = load i64, ptr %zero, align 4
  store i64 %zero8, ptr %count, align 4
  store i1 false, ptr %complete, align 1
  %view9 = load i32, ptr %view2, align 4
  %deref_is_null = icmp eq i32 %view9, 0
  br i1 %deref_is_null, label %deref_null_panic, label %deref_null_continue

deref_null_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

deref_null_continue:                              ; preds = %entry
  %deref = inttoptr i32 %view9 to ptr
  %address = ptrtoint ptr %deref to i32
  %one10 = load i64, ptr %one, align 4
  %call = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__726561645f696e746f(i32 %address, i64 %one10)
  %output = extractvalue { i64, i1 } %call, 0
  store i64 %output, ptr %assignment_value, align 4
  %assignment_value11 = load i64, ptr %assignment_value, align 4
  %output12 = extractvalue { i64, i1 } %call, 1
  store i1 %output12, ptr %assignment_value13, align 1
  %assignment_value14 = load i1, ptr %assignment_value13, align 1
  store i64 %assignment_value11, ptr %count, align 4
  store i1 %assignment_value14, ptr %complete, align 1
  %complete15 = load i1, ptr %complete, align 1
  %not = xor i1 %complete15, true
  br i1 %not, label %if.then, label %if.else

if.then:                                          ; preds = %deref_null_continue
  store i32 2, ptr %assignment_value16, align 4
  %assignment_value17 = load i32, ptr %assignment_value16, align 4
  store i32 %assignment_value17, ptr %status7, align 4
  store i1 false, ptr %assignment_value18, align 1
  %assignment_value19 = load i1, ptr %assignment_value18, align 1
  store i1 %assignment_value19, ptr %reading6, align 1
  br label %if.merge

if.else:                                          ; preds = %deref_null_continue
  %count20 = load i64, ptr %count, align 4
  %zero21 = load i64, ptr %zero, align 4
  %eq = icmp eq i64 %count20, %zero21
  br i1 %eq, label %if.then22, label %if.else23

if.merge:                                         ; preds = %if.merge24, %if.then
  %status95 = load i32, ptr %status7, align 4
  %eq96 = icmp eq i32 %status95, 4
  br i1 %eq96, label %if.then97, label %if.merge98

if.then22:                                        ; preds = %if.else
  store i1 false, ptr %assignment_value25, align 1
  %assignment_value26 = load i1, ptr %assignment_value25, align 1
  store i1 %assignment_value26, ptr %reading6, align 1
  br label %if.merge24

if.else23:                                        ; preds = %if.else
  %view27 = load i32, ptr %view2, align 4
  %deref_is_null28 = icmp eq i32 %view27, 0
  br i1 %deref_is_null28, label %deref_null_panic29, label %deref_null_continue30

if.merge24:                                       ; preds = %if.merge37, %if.then22
  br label %if.merge

deref_null_panic29:                               ; preds = %if.else23
  call void @__wosy_core_system_panic()
  unreachable

deref_null_continue30:                            ; preds = %if.else23
  %deref31 = inttoptr i32 %view27 to ptr
  %place = load i8, ptr %deref31, align 1
  store i8 %place, ptr %byte, align 1
  %byte32 = load i8, ptr %byte, align 1
  %lf33 = load i8, ptr %lf, align 1
  %eq34 = icmp eq i8 %byte32, %lf33
  br i1 %eq34, label %if.then35, label %if.else36

if.then35:                                        ; preds = %deref_null_continue30
  store i1 false, ptr %assignment_value38, align 1
  %assignment_value39 = load i1, ptr %assignment_value38, align 1
  store i1 %assignment_value39, ptr %pending_cr5, align 1
  store i1 false, ptr %assignment_value40, align 1
  %assignment_value41 = load i1, ptr %assignment_value40, align 1
  store i1 %assignment_value41, ptr %reading6, align 1
  br label %if.merge37

if.else36:                                        ; preds = %deref_null_continue30
  %pending_cr42 = load i1, ptr %pending_cr5, align 1
  br i1 %pending_cr42, label %if.then43, label %if.merge44

if.merge37:                                       ; preds = %if.merge72, %if.then35
  br label %if.merge24

if.then43:                                        ; preds = %if.else36
  %length45 = load i64, ptr %length4, align 4
  %max_bytes46 = load i64, ptr %max_bytes3, align 4
  %status47 = load i32, ptr %status7, align 4
  %call48 = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f63617061636974795f737461747573(i64 %length45, i64 %max_bytes46, i32 %status47)
  store i32 %call48, ptr %assignment_value49, align 4
  %assignment_value50 = load i32, ptr %assignment_value49, align 4
  store i32 %assignment_value50, ptr %status7, align 4
  %length51 = load i64, ptr %length4, align 4
  %max_bytes52 = load i64, ptr %max_bytes3, align 4
  %lt = icmp ult i64 %length51, %max_bytes52
  br i1 %lt, label %if.then53, label %if.merge54

if.merge44:                                       ; preds = %if.merge54, %if.else36
  %byte64 = load i8, ptr %byte, align 1
  %cr65 = load i8, ptr %cr, align 1
  %eq66 = icmp eq i8 %byte64, %cr65
  store i1 %eq66, ptr %assignment_value67, align 1
  %assignment_value68 = load i1, ptr %assignment_value67, align 1
  store i1 %assignment_value68, ptr %pending_cr5, align 1
  %pending_cr69 = load i1, ptr %pending_cr5, align 1
  %not70 = xor i1 %pending_cr69, true
  br i1 %not70, label %if.then71, label %if.merge72

if.then53:                                        ; preds = %if.then43
  %cr55 = load i8, ptr %cr, align 1
  store i8 %cr55, ptr %assignment_value56, align 1
  %assignment_value57 = load i8, ptr %assignment_value56, align 1
  %bytes58 = load ptr, ptr %bytes1, align 8
  %length59 = load i64, ptr %length4, align 4
  %array_element = getelementptr inbounds i8, ptr %bytes58, i64 %length59
  store i8 %assignment_value57, ptr %array_element, align 1
  %length60 = load i64, ptr %length4, align 4
  %one61 = load i64, ptr %one, align 4
  %add = add i64 %length60, %one61
  store i64 %add, ptr %assignment_value62, align 4
  %assignment_value63 = load i64, ptr %assignment_value62, align 4
  store i64 %assignment_value63, ptr %length4, align 4
  br label %if.merge54

if.merge54:                                       ; preds = %if.then53, %if.then43
  br label %if.merge44

if.then71:                                        ; preds = %if.merge44
  %length73 = load i64, ptr %length4, align 4
  %max_bytes74 = load i64, ptr %max_bytes3, align 4
  %status75 = load i32, ptr %status7, align 4
  %call76 = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f63617061636974795f737461747573(i64 %length73, i64 %max_bytes74, i32 %status75)
  store i32 %call76, ptr %assignment_value77, align 4
  %assignment_value78 = load i32, ptr %assignment_value77, align 4
  store i32 %assignment_value78, ptr %status7, align 4
  %length79 = load i64, ptr %length4, align 4
  %max_bytes80 = load i64, ptr %max_bytes3, align 4
  %lt81 = icmp ult i64 %length79, %max_bytes80
  br i1 %lt81, label %if.then82, label %if.merge83

if.merge72:                                       ; preds = %if.merge83, %if.merge44
  br label %if.merge37

if.then82:                                        ; preds = %if.then71
  %byte84 = load i8, ptr %byte, align 1
  store i8 %byte84, ptr %assignment_value85, align 1
  %assignment_value86 = load i8, ptr %assignment_value85, align 1
  %bytes87 = load ptr, ptr %bytes1, align 8
  %length88 = load i64, ptr %length4, align 4
  %array_element89 = getelementptr inbounds i8, ptr %bytes87, i64 %length88
  store i8 %assignment_value86, ptr %array_element89, align 1
  %length90 = load i64, ptr %length4, align 4
  %one91 = load i64, ptr %one, align 4
  %add92 = add i64 %length90, %one91
  store i64 %add92, ptr %assignment_value93, align 4
  %assignment_value94 = load i64, ptr %assignment_value93, align 4
  store i64 %assignment_value94, ptr %length4, align 4
  br label %if.merge83

if.merge83:                                       ; preds = %if.then82, %if.then71
  br label %if.merge72

if.then97:                                        ; preds = %if.merge
  store i1 false, ptr %assignment_value99, align 1
  %assignment_value100 = load i1, ptr %assignment_value99, align 1
  store i1 %assignment_value100, ptr %reading6, align 1
  br label %if.merge98

if.merge98:                                       ; preds = %if.then97, %if.merge
  %bytes101 = load ptr, ptr %bytes1, align 8
  %length102 = load i64, ptr %length4, align 4
  %pending_cr103 = load i1, ptr %pending_cr5, align 1
  %reading104 = load i1, ptr %reading6, align 1
  %status105 = load i32, ptr %status7, align 4
  %output106 = insertvalue { ptr, i64, i1, i1, i32 } zeroinitializer, ptr %bytes101, 0
  %output107 = insertvalue { ptr, i64, i1, i1, i32 } %output106, i64 %length102, 1
  %output108 = insertvalue { ptr, i64, i1, i1, i32 } %output107, i1 %pending_cr103, 2
  %output109 = insertvalue { ptr, i64, i1, i1, i32 } %output108, i1 %reading104, 3
  %output110 = insertvalue { ptr, i64, i1, i1, i32 } %output109, i32 %status105, 4
  ret { ptr, i64, i1, i1, i32 } %output110
}

define { ptr, i64, i1, i1, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f616c6c6f63617465645f72616e6765(ptr %bytes, i32 %view, i64 %max_bytes, i64 %budget, i64 %length, i1 %pending_cr, i1 %reading, i32 %status) {
entry:
  %assignment_value88 = alloca i32, align 4
  %assignment_value85 = alloca i1, align 1
  %assignment_value82 = alloca i1, align 1
  %assignment_value79 = alloca i64, align 8
  %assignment_value76 = alloca ptr, align 8
  %assignment_value60 = alloca i32, align 4
  %assignment_value57 = alloca i1, align 1
  %assignment_value54 = alloca i1, align 1
  %assignment_value51 = alloca i64, align 8
  %assignment_value48 = alloca ptr, align 8
  %left = alloca i64, align 8
  %two = alloca i64, align 8
  %assignment_value34 = alloca i32, align 4
  %assignment_value31 = alloca i1, align 1
  %assignment_value28 = alloca i1, align 1
  %assignment_value25 = alloca i64, align 8
  %assignment_value = alloca ptr, align 8
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %bytes1 = alloca ptr, align 8
  store ptr %bytes, ptr %bytes1, align 8
  %view2 = alloca i32, align 4
  store i32 %view, ptr %view2, align 4
  %max_bytes3 = alloca i64, align 8
  store i64 %max_bytes, ptr %max_bytes3, align 4
  %budget4 = alloca i64, align 8
  store i64 %budget, ptr %budget4, align 4
  %length5 = alloca i64, align 8
  store i64 %length, ptr %length5, align 4
  %pending_cr6 = alloca i1, align 1
  store i1 %pending_cr, ptr %pending_cr6, align 1
  %reading7 = alloca i1, align 1
  store i1 %reading, ptr %reading7, align 1
  %status8 = alloca i32, align 4
  store i32 %status, ptr %status8, align 4
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  %budget9 = load i64, ptr %budget4, align 4
  %zero10 = load i64, ptr %zero, align 4
  %ne = icmp ne i64 %budget9, %zero10
  br i1 %ne, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %reading11 = load i1, ptr %reading7, align 1
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %reading11, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.merge

if.then:                                          ; preds = %short_circuit.merge
  %budget12 = load i64, ptr %budget4, align 4
  %one13 = load i64, ptr %one, align 4
  %eq = icmp eq i64 %budget12, %one13
  br i1 %eq, label %if.then14, label %if.else

if.merge:                                         ; preds = %if.merge15, %short_circuit.merge
  %bytes90 = load ptr, ptr %bytes1, align 8
  %length91 = load i64, ptr %length5, align 4
  %pending_cr92 = load i1, ptr %pending_cr6, align 1
  %reading93 = load i1, ptr %reading7, align 1
  %status94 = load i32, ptr %status8, align 4
  %output95 = insertvalue { ptr, i64, i1, i1, i32 } zeroinitializer, ptr %bytes90, 0
  %output96 = insertvalue { ptr, i64, i1, i1, i32 } %output95, i64 %length91, 1
  %output97 = insertvalue { ptr, i64, i1, i1, i32 } %output96, i1 %pending_cr92, 2
  %output98 = insertvalue { ptr, i64, i1, i1, i32 } %output97, i1 %reading93, 3
  %output99 = insertvalue { ptr, i64, i1, i1, i32 } %output98, i32 %status94, 4
  ret { ptr, i64, i1, i1, i32 } %output99

if.then14:                                        ; preds = %if.then
  %bytes16 = load ptr, ptr %bytes1, align 8
  %view17 = load i32, ptr %view2, align 4
  %max_bytes18 = load i64, ptr %max_bytes3, align 4
  %length19 = load i64, ptr %length5, align 4
  %pending_cr20 = load i1, ptr %pending_cr6, align 1
  %reading21 = load i1, ptr %reading7, align 1
  %status22 = load i32, ptr %status8, align 4
  %call = call { ptr, i64, i1, i1, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f616c6c6f63617465645f73746570(ptr %bytes16, i32 %view17, i64 %max_bytes18, i64 %length19, i1 %pending_cr20, i1 %reading21, i32 %status22)
  %output = extractvalue { ptr, i64, i1, i1, i32 } %call, 0
  store ptr %output, ptr %assignment_value, align 8
  %assignment_value23 = load ptr, ptr %assignment_value, align 8
  %output24 = extractvalue { ptr, i64, i1, i1, i32 } %call, 1
  store i64 %output24, ptr %assignment_value25, align 4
  %assignment_value26 = load i64, ptr %assignment_value25, align 4
  %output27 = extractvalue { ptr, i64, i1, i1, i32 } %call, 2
  store i1 %output27, ptr %assignment_value28, align 1
  %assignment_value29 = load i1, ptr %assignment_value28, align 1
  %output30 = extractvalue { ptr, i64, i1, i1, i32 } %call, 3
  store i1 %output30, ptr %assignment_value31, align 1
  %assignment_value32 = load i1, ptr %assignment_value31, align 1
  %output33 = extractvalue { ptr, i64, i1, i1, i32 } %call, 4
  store i32 %output33, ptr %assignment_value34, align 4
  %assignment_value35 = load i32, ptr %assignment_value34, align 4
  store ptr %assignment_value23, ptr %bytes1, align 8
  store i64 %assignment_value26, ptr %length5, align 4
  store i1 %assignment_value29, ptr %pending_cr6, align 1
  store i1 %assignment_value32, ptr %reading7, align 1
  store i32 %assignment_value35, ptr %status8, align 4
  br label %if.merge15

if.else:                                          ; preds = %if.then
  store i64 2, ptr %two, align 4
  %budget36 = load i64, ptr %budget4, align 4
  %two37 = load i64, ptr %two, align 4
  %div = udiv i64 %budget36, %two37
  store i64 %div, ptr %left, align 4
  %bytes38 = load ptr, ptr %bytes1, align 8
  %view39 = load i32, ptr %view2, align 4
  %max_bytes40 = load i64, ptr %max_bytes3, align 4
  %left41 = load i64, ptr %left, align 4
  %length42 = load i64, ptr %length5, align 4
  %pending_cr43 = load i1, ptr %pending_cr6, align 1
  %reading44 = load i1, ptr %reading7, align 1
  %status45 = load i32, ptr %status8, align 4
  %call46 = call { ptr, i64, i1, i1, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f616c6c6f63617465645f72616e6765(ptr %bytes38, i32 %view39, i64 %max_bytes40, i64 %left41, i64 %length42, i1 %pending_cr43, i1 %reading44, i32 %status45)
  %output47 = extractvalue { ptr, i64, i1, i1, i32 } %call46, 0
  store ptr %output47, ptr %assignment_value48, align 8
  %assignment_value49 = load ptr, ptr %assignment_value48, align 8
  %output50 = extractvalue { ptr, i64, i1, i1, i32 } %call46, 1
  store i64 %output50, ptr %assignment_value51, align 4
  %assignment_value52 = load i64, ptr %assignment_value51, align 4
  %output53 = extractvalue { ptr, i64, i1, i1, i32 } %call46, 2
  store i1 %output53, ptr %assignment_value54, align 1
  %assignment_value55 = load i1, ptr %assignment_value54, align 1
  %output56 = extractvalue { ptr, i64, i1, i1, i32 } %call46, 3
  store i1 %output56, ptr %assignment_value57, align 1
  %assignment_value58 = load i1, ptr %assignment_value57, align 1
  %output59 = extractvalue { ptr, i64, i1, i1, i32 } %call46, 4
  store i32 %output59, ptr %assignment_value60, align 4
  %assignment_value61 = load i32, ptr %assignment_value60, align 4
  store ptr %assignment_value49, ptr %bytes1, align 8
  store i64 %assignment_value52, ptr %length5, align 4
  store i1 %assignment_value55, ptr %pending_cr6, align 1
  store i1 %assignment_value58, ptr %reading7, align 1
  store i32 %assignment_value61, ptr %status8, align 4
  %reading62 = load i1, ptr %reading7, align 1
  br i1 %reading62, label %if.then63, label %if.merge64

if.merge15:                                       ; preds = %if.merge64, %if.then14
  br label %if.merge

if.then63:                                        ; preds = %if.else
  %bytes65 = load ptr, ptr %bytes1, align 8
  %view66 = load i32, ptr %view2, align 4
  %max_bytes67 = load i64, ptr %max_bytes3, align 4
  %budget68 = load i64, ptr %budget4, align 4
  %left69 = load i64, ptr %left, align 4
  %sub = sub i64 %budget68, %left69
  %length70 = load i64, ptr %length5, align 4
  %pending_cr71 = load i1, ptr %pending_cr6, align 1
  %reading72 = load i1, ptr %reading7, align 1
  %status73 = load i32, ptr %status8, align 4
  %call74 = call { ptr, i64, i1, i1, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f616c6c6f63617465645f72616e6765(ptr %bytes65, i32 %view66, i64 %max_bytes67, i64 %sub, i64 %length70, i1 %pending_cr71, i1 %reading72, i32 %status73)
  %output75 = extractvalue { ptr, i64, i1, i1, i32 } %call74, 0
  store ptr %output75, ptr %assignment_value76, align 8
  %assignment_value77 = load ptr, ptr %assignment_value76, align 8
  %output78 = extractvalue { ptr, i64, i1, i1, i32 } %call74, 1
  store i64 %output78, ptr %assignment_value79, align 4
  %assignment_value80 = load i64, ptr %assignment_value79, align 4
  %output81 = extractvalue { ptr, i64, i1, i1, i32 } %call74, 2
  store i1 %output81, ptr %assignment_value82, align 1
  %assignment_value83 = load i1, ptr %assignment_value82, align 1
  %output84 = extractvalue { ptr, i64, i1, i1, i32 } %call74, 3
  store i1 %output84, ptr %assignment_value85, align 1
  %assignment_value86 = load i1, ptr %assignment_value85, align 1
  %output87 = extractvalue { ptr, i64, i1, i1, i32 } %call74, 4
  store i32 %output87, ptr %assignment_value88, align 4
  %assignment_value89 = load i32, ptr %assignment_value88, align 4
  store ptr %assignment_value77, ptr %bytes1, align 8
  store i64 %assignment_value80, ptr %length5, align 4
  store i1 %assignment_value83, ptr %pending_cr6, align 1
  store i1 %assignment_value86, ptr %reading7, align 1
  store i32 %assignment_value89, ptr %status8, align 4
  br label %if.merge64

if.merge64:                                       ; preds = %if.then63, %if.else
  br label %if.merge15
}

define { i32, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f616c6c6f6361746564(i64 %max_bytes, i8 %first, i1 %pending_cr) #0 {
entry:
  %text = alloca [16 x i8], align 8
  %struct_literal = alloca [16 x i8], align 8
  %assignment_value126 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value111 = alloca i32, align 4
  %assignment_value105 = alloca i32, align 4
  %assignment_value102 = alloca i8, align 1
  %assignment_value99 = alloca i8, align 1
  %assignment_value96 = alloca i64, align 8
  %assignment_value93 = alloca ptr, align 8
  %maximum = alloca i8, align 1
  %minimum = alloca i8, align 1
  %remaining = alloca i64, align 8
  %assignment_value77 = alloca i64, align 8
  %assignment_value69 = alloca i8, align 1
  %assignment_value63 = alloca i32, align 4
  %assignment_value52 = alloca i32, align 4
  %outcome = alloca i32, align 4
  %first_outcome = alloca i32, align 4
  %assignment_value38 = alloca i32, align 4
  %assignment_value35 = alloca i1, align 1
  %assignment_value32 = alloca i1, align 1
  %assignment_value29 = alloca i64, align 8
  %assignment_value26 = alloca ptr, align 8
  %view = alloca i32, align 4
  %two = alloca i64, align 8
  %assignment_value13 = alloca i64, align 8
  %assignment_value = alloca i8, align 1
  %status = alloca i32, align 4
  %reading = alloca i1, align 1
  %length = alloca i64, align 8
  %bytes_live = alloca i1, align 1
  %bytes_length = alloca i64, align 8
  %bytes_backing = alloca ptr, align 8
  %scratch = alloca [1 x i8], align 1
  %array_literal = alloca [1 x i8], align 1
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
  %array_element = getelementptr inbounds [1 x i8], ptr %array_literal, i32 0, i32 0
  store i8 0, ptr %array_element, align 1
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %scratch, ptr align 1 %array_literal, i32 1, i1 false)
  %max_bytes4 = load i64, ptr %max_bytes1, align 4
  %allocation = call ptr @__wosy_core_alloc(i64 %max_bytes4, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  store ptr %allocation, ptr %bytes_backing, align 8
  store i64 %max_bytes4, ptr %bytes_length, align 4
  store i1 true, ptr %bytes_live, align 1
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
  %bytes = load ptr, ptr %bytes_backing, align 8
  %length9 = load i64, ptr %length, align 4
  %array_element10 = getelementptr inbounds i8, ptr %bytes, i64 %length9
  store i8 %assignment_value8, ptr %array_element10, align 1
  %length11 = load i64, ptr %length, align 4
  %one12 = load i64, ptr %one, align 4
  %add = add i64 %length11, %one12
  store i64 %add, ptr %assignment_value13, align 4
  %assignment_value14 = load i64, ptr %assignment_value13, align 4
  store i64 %assignment_value14, ptr %length, align 4
  br label %if.merge

if.merge:                                         ; preds = %if.then, %allocation_continue
  store i64 2, ptr %two, align 4
  %array_element15 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i32 0
  %address = ptrtoint ptr %array_element15 to i32
  store i32 %address, ptr %view, align 4
  %bytes16 = load ptr, ptr %bytes_backing, align 8
  %view17 = load i32, ptr %view, align 4
  %max_bytes18 = load i64, ptr %max_bytes1, align 4
  %max_bytes19 = load i64, ptr %max_bytes1, align 4
  %two20 = load i64, ptr %two, align 4
  %add21 = add i64 %max_bytes19, %two20
  %length22 = load i64, ptr %length, align 4
  %pending_cr23 = load i1, ptr %pending_cr3, align 1
  %reading24 = load i1, ptr %reading, align 1
  %status25 = load i32, ptr %status, align 4
  %call = call { ptr, i64, i1, i1, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f616c6c6f63617465645f72616e6765(ptr %bytes16, i32 %view17, i64 %max_bytes18, i64 %add21, i64 %length22, i1 %pending_cr23, i1 %reading24, i32 %status25)
  %output = extractvalue { ptr, i64, i1, i1, i32 } %call, 0
  store ptr %output, ptr %assignment_value26, align 8
  %assignment_value27 = load ptr, ptr %assignment_value26, align 8
  %output28 = extractvalue { ptr, i64, i1, i1, i32 } %call, 1
  store i64 %output28, ptr %assignment_value29, align 4
  %assignment_value30 = load i64, ptr %assignment_value29, align 4
  %output31 = extractvalue { ptr, i64, i1, i1, i32 } %call, 2
  store i1 %output31, ptr %assignment_value32, align 1
  %assignment_value33 = load i1, ptr %assignment_value32, align 1
  %output34 = extractvalue { ptr, i64, i1, i1, i32 } %call, 3
  store i1 %output34, ptr %assignment_value35, align 1
  %assignment_value36 = load i1, ptr %assignment_value35, align 1
  %output37 = extractvalue { ptr, i64, i1, i1, i32 } %call, 4
  store i32 %output37, ptr %assignment_value38, align 4
  %assignment_value39 = load i32, ptr %assignment_value38, align 4
  store ptr %assignment_value27, ptr %bytes_backing, align 8
  store i64 %assignment_value30, ptr %length, align 4
  store i1 %assignment_value33, ptr %pending_cr3, align 1
  store i1 %assignment_value36, ptr %reading, align 1
  store i32 %assignment_value39, ptr %status, align 4
  %status40 = load i32, ptr %status, align 4
  %eq = icmp eq i32 %status40, 4
  br i1 %eq, label %if.then41, label %if.merge42

if.then41:                                        ; preds = %if.merge
  %view43 = load i32, ptr %view, align 4
  %call44 = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f646973636172645f73746570(i32 %view43)
  store i32 %call44, ptr %first_outcome, align 4
  %view45 = load i32, ptr %view, align 4
  %first_outcome46 = load i32, ptr %first_outcome, align 4
  %call47 = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f646973636172645f636f6e74696e7565(i32 %view45, i32 %first_outcome46)
  store i32 %call47, ptr %outcome, align 4
  %outcome48 = load i32, ptr %outcome, align 4
  %eq49 = icmp eq i32 %outcome48, 2
  br i1 %eq49, label %if.then50, label %if.merge51

if.merge42:                                       ; preds = %if.merge51, %if.merge
  %status54 = load i32, ptr %status, align 4
  %eq55 = icmp eq i32 %status54, 0
  br i1 %eq55, label %short_circuit.rhs, label %short_circuit.merge

if.then50:                                        ; preds = %if.then41
  store i32 2, ptr %assignment_value52, align 4
  %assignment_value53 = load i32, ptr %assignment_value52, align 4
  store i32 %assignment_value53, ptr %status, align 4
  br label %if.merge51

if.merge51:                                       ; preds = %if.then50, %if.then41
  br label %if.merge42

short_circuit.rhs:                                ; preds = %if.merge42
  %pending_cr56 = load i1, ptr %pending_cr3, align 1
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %if.merge42
  %short_circuit = phi i1 [ false, %if.merge42 ], [ %pending_cr56, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then57, label %if.merge58

if.then57:                                        ; preds = %short_circuit.merge
  %length59 = load i64, ptr %length, align 4
  %max_bytes60 = load i64, ptr %max_bytes1, align 4
  %status61 = load i32, ptr %status, align 4
  %call62 = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f63617061636974795f737461747573(i64 %length59, i64 %max_bytes60, i32 %status61)
  store i32 %call62, ptr %assignment_value63, align 4
  %assignment_value64 = load i32, ptr %assignment_value63, align 4
  store i32 %assignment_value64, ptr %status, align 4
  %length65 = load i64, ptr %length, align 4
  %max_bytes66 = load i64, ptr %max_bytes1, align 4
  %lt = icmp ult i64 %length65, %max_bytes66
  br i1 %lt, label %if.then67, label %if.merge68

if.merge58:                                       ; preds = %if.merge68, %short_circuit.merge
  %status79 = load i32, ptr %status, align 4
  %eq80 = icmp eq i32 %status79, 0
  br i1 %eq80, label %if.then81, label %if.merge82

if.then67:                                        ; preds = %if.then57
  store i8 13, ptr %assignment_value69, align 1
  %assignment_value70 = load i8, ptr %assignment_value69, align 1
  %bytes71 = load ptr, ptr %bytes_backing, align 8
  %length72 = load i64, ptr %length, align 4
  %array_element73 = getelementptr inbounds i8, ptr %bytes71, i64 %length72
  store i8 %assignment_value70, ptr %array_element73, align 1
  %length74 = load i64, ptr %length, align 4
  %one75 = load i64, ptr %one, align 4
  %add76 = add i64 %length74, %one75
  store i64 %add76, ptr %assignment_value77, align 4
  %assignment_value78 = load i64, ptr %assignment_value77, align 4
  store i64 %assignment_value78, ptr %length, align 4
  br label %if.merge68

if.merge68:                                       ; preds = %if.then67, %if.then57
  br label %if.merge58

if.then81:                                        ; preds = %if.merge58
  %zero83 = load i64, ptr %zero, align 4
  store i64 %zero83, ptr %remaining, align 4
  store i8 -128, ptr %minimum, align 1
  store i8 -65, ptr %maximum, align 1
  %bytes84 = load ptr, ptr %bytes_backing, align 8
  %zero85 = load i64, ptr %zero, align 4
  %length86 = load i64, ptr %length, align 4
  %remaining87 = load i64, ptr %remaining, align 4
  %minimum88 = load i8, ptr %minimum, align 1
  %maximum89 = load i8, ptr %maximum, align 1
  %status90 = load i32, ptr %status, align 4
  %call91 = call { ptr, i64, i8, i8, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f757466385f72616e6765(ptr %bytes84, i64 %zero85, i64 %length86, i64 %remaining87, i8 %minimum88, i8 %maximum89, i32 %status90)
  %output92 = extractvalue { ptr, i64, i8, i8, i32 } %call91, 0
  store ptr %output92, ptr %assignment_value93, align 8
  %assignment_value94 = load ptr, ptr %assignment_value93, align 8
  %output95 = extractvalue { ptr, i64, i8, i8, i32 } %call91, 1
  store i64 %output95, ptr %assignment_value96, align 4
  %assignment_value97 = load i64, ptr %assignment_value96, align 4
  %output98 = extractvalue { ptr, i64, i8, i8, i32 } %call91, 2
  store i8 %output98, ptr %assignment_value99, align 1
  %assignment_value100 = load i8, ptr %assignment_value99, align 1
  %output101 = extractvalue { ptr, i64, i8, i8, i32 } %call91, 3
  store i8 %output101, ptr %assignment_value102, align 1
  %assignment_value103 = load i8, ptr %assignment_value102, align 1
  %output104 = extractvalue { ptr, i64, i8, i8, i32 } %call91, 4
  store i32 %output104, ptr %assignment_value105, align 4
  %assignment_value106 = load i32, ptr %assignment_value105, align 4
  store ptr %assignment_value94, ptr %bytes_backing, align 8
  store i64 %assignment_value97, ptr %remaining, align 4
  store i8 %assignment_value100, ptr %minimum, align 1
  store i8 %assignment_value103, ptr %maximum, align 1
  store i32 %assignment_value106, ptr %status, align 4
  %remaining107 = load i64, ptr %remaining, align 4
  %zero108 = load i64, ptr %zero, align 4
  %ne = icmp ne i64 %remaining107, %zero108
  br i1 %ne, label %if.then109, label %if.merge110

if.merge82:                                       ; preds = %if.merge110, %if.merge58
  %status113 = load i32, ptr %status, align 4
  %eq114 = icmp eq i32 %status113, 0
  br i1 %eq114, label %if.then115, label %if.else

if.then109:                                       ; preds = %if.then81
  store i32 3, ptr %assignment_value111, align 4
  %assignment_value112 = load i32, ptr %assignment_value111, align 4
  store i32 %assignment_value112, ptr %status, align 4
  br label %if.merge110

if.merge110:                                      ; preds = %if.then109, %if.then81
  br label %if.merge82

if.then115:                                       ; preds = %if.merge82
  store i32 0, ptr %data, align 4
  %length117 = load i64, ptr %length, align 4
  %zero118 = load i64, ptr %zero, align 4
  %ne119 = icmp ne i64 %length117, %zero118
  br i1 %ne119, label %if.then120, label %if.else121

if.else:                                          ; preds = %if.merge82
  %status138 = load i32, ptr %status, align 4
  %output139 = insertvalue { i32, i32 } zeroinitializer, i32 %status138, 1
  %allocation_live = load i1, ptr %bytes_live, align 1
  br i1 %allocation_live, label %release.live, label %release.next

if.merge116:                                      ; preds = %release.next, %if.merge122
  %if = phi { i32, i32 } [ %output137, %if.merge122 ], [ %output139, %release.next ]
  %return_output = extractvalue { i32, i32 } %if, 0
  %return_result_is_null = icmp eq i32 %return_output, 0
  br i1 %return_result_is_null, label %return_result_null_0, label %return_result_copy_1

if.then120:                                       ; preds = %if.then115
  %bytes123 = load ptr, ptr %bytes_backing, align 8
  %array_element124 = getelementptr inbounds i8, ptr %bytes123, i64 0
  %address125 = ptrtoint ptr %array_element124 to i32
  store i32 %address125, ptr %assignment_value126, align 4
  %assignment_value127 = load i32, ptr %assignment_value126, align 4
  store i32 %assignment_value127, ptr %data, align 4
  br label %if.merge122

if.else121:                                       ; preds = %if.then115
  %bytes128 = load ptr, ptr %bytes_backing, align 8
  %address129 = ptrtoint ptr %bytes128 to i32
  %checked_release_backing = load ptr, ptr %bytes_backing, align 8
  call void @__wosy_core_free(ptr %checked_release_backing)
  store i1 false, ptr %bytes_live, align 1
  br label %if.merge122

if.merge122:                                      ; preds = %if.else121, %if.then120
  %data130 = load i32, ptr %data, align 4
  %length131 = load i64, ptr %length, align 4
  %data132 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data130, ptr %data132, align 4
  %length133 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length131, ptr %length133, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text, ptr align 1 %struct_literal, i32 16, i1 false)
  %address134 = ptrtoint ptr %text to i32
  %status135 = load i32, ptr %status, align 4
  %output136 = insertvalue { i32, i32 } zeroinitializer, i32 %address134, 0
  %output137 = insertvalue { i32, i32 } %output136, i32 %status135, 1
  br label %if.merge116

release.live:                                     ; preds = %if.else
  call void @__wosy_core_free(ptr %allocation)
  br label %release.next

release.next:                                     ; preds = %release.live, %if.else
  store i1 false, ptr %bytes_live, align 1
  br label %if.merge116

return_result_null_0:                             ; preds = %if.merge116
  br label %return_result_merge_2

return_result_copy_1:                             ; preds = %if.merge116
  %allocation141 = call ptr @__wosy_core_alloc(i64 16, i64 8)
  %allocation_failed142 = icmp eq ptr %allocation141, null
  br i1 %allocation_failed142, label %allocation_panic140, label %allocation_continue143

return_result_merge_2:                            ; preds = %allocation_continue143, %return_result_null_0
  %return_result = phi i32 [ 0, %return_result_null_0 ], [ %return_result_address, %allocation_continue143 ]
  %return_output144 = extractvalue { i32, i32 } %if, 1
  %output145 = insertvalue { i32, i32 } zeroinitializer, i32 %return_result, 0
  %output146 = insertvalue { i32, i32 } %output145, i32 %return_output144, 1
  ret { i32, i32 } %output146

allocation_panic140:                              ; preds = %return_result_copy_1
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue143:                           ; preds = %return_result_copy_1
  %return_result_source = inttoptr i32 %return_output to ptr
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %allocation141, ptr align 1 %return_result_source, i32 16, i1 false)
  %return_result_address = ptrtoint ptr %allocation141 to i32
  br label %return_result_merge_2
}

define { i32, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__726561645f6c696e65(i64 %max_bytes, ptr %0) #0 {
entry:
  %status77 = alloca i32, align 4
  %text = alloca i32, align 4
  %empty51 = alloca [16 x i8], align 8
  %struct_literal47 = alloca [16 x i8], align 8
  %status = alloca i32, align 4
  %empty = alloca [16 x i8], align 8
  %struct_literal = alloca [16 x i8], align 8
  %first = alloca i8, align 1
  %assignment_value10 = alloca i1, align 1
  %assignment_value = alloca i64, align 8
  %complete = alloca i1, align 1
  %count = alloca i64, align 8
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
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %scratch, ptr align 1 %array_literal, i32 1, i1 false)
  %zero5 = load i64, ptr %zero, align 4
  store i64 %zero5, ptr %count, align 4
  store i1 false, ptr %complete, align 1
  %array_element6 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i32 0
  %address = ptrtoint ptr %array_element6 to i32
  %one7 = load i64, ptr %one, align 4
  %call = call { i64, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__726561645f696e746f(i32 %address, i64 %one7)
  %output = extractvalue { i64, i1 } %call, 0
  store i64 %output, ptr %assignment_value, align 4
  %assignment_value8 = load i64, ptr %assignment_value, align 4
  %output9 = extractvalue { i64, i1 } %call, 1
  store i1 %output9, ptr %assignment_value10, align 1
  %assignment_value11 = load i1, ptr %assignment_value10, align 1
  store i64 %assignment_value8, ptr %count, align 4
  store i1 %assignment_value11, ptr %complete, align 1
  %complete12 = load i1, ptr %complete, align 1
  %not = xor i1 %complete12, true
  br i1 %not, label %if.then13, label %if.else14

if.merge:                                         ; preds = %if.merge15, %if.then
  %return_owner_090 = phi i1 [ false, %if.then ], [ %return_owner_088, %if.merge15 ]
  %if91 = phi { i32, i32 } [ { i32 0, i32 4 }, %if.then ], [ %if89, %if.merge15 ]
  store i1 %return_owner_090, ptr %0, align 1
  ret { i32, i32 } %if91

if.then13:                                        ; preds = %if.else
  br label %if.merge15

if.else14:                                        ; preds = %if.else
  %count16 = load i64, ptr %count, align 4
  %zero17 = load i64, ptr %zero, align 4
  %eq = icmp eq i64 %count16, %zero17
  br i1 %eq, label %if.then18, label %if.else19

if.merge15:                                       ; preds = %if.merge20, %if.then13
  %return_owner_088 = phi i1 [ false, %if.then13 ], [ %return_owner_086, %if.merge20 ]
  %if89 = phi { i32, i32 } [ { i32 0, i32 2 }, %if.then13 ], [ %if87, %if.merge20 ]
  br label %if.merge

if.then18:                                        ; preds = %if.else14
  br label %if.merge20

if.else19:                                        ; preds = %if.else14
  %array_element21 = getelementptr inbounds [1 x i8], ptr %scratch, i32 0, i32 0
  %place = load i8, ptr %array_element21, align 1
  store i8 %place, ptr %first, align 1
  %first22 = load i8, ptr %first, align 1
  %eq23 = icmp eq i8 %first22, 10
  br i1 %eq23, label %if.then24, label %if.else25

if.merge20:                                       ; preds = %if.merge26, %if.then18
  %return_owner_086 = phi i1 [ false, %if.then18 ], [ %return_owner_084, %if.merge26 ]
  %if87 = phi { i32, i32 } [ { i32 0, i32 1 }, %if.then18 ], [ %if85, %if.merge26 ]
  br label %if.merge15

if.then24:                                        ; preds = %if.else19
  %zero27 = load i64, ptr %zero, align 4
  %data = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 0, ptr %data, align 4
  %length = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %zero27, ptr %length, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %empty, ptr align 1 %struct_literal, i32 16, i1 false)
  %address28 = ptrtoint ptr %empty to i32
  %output29 = insertvalue { i32, i32 } zeroinitializer, i32 %address28, 0
  %output30 = insertvalue { i32, i32 } %output29, i32 0, 1
  %return_output = extractvalue { i32, i32 } %output30, 0
  %return_result_is_null = icmp eq i32 %return_output, 0
  br i1 %return_result_is_null, label %return_result_null_0, label %return_result_copy_1

if.else25:                                        ; preds = %if.else19
  %max_bytes34 = load i64, ptr %max_bytes1, align 4
  %zero35 = load i64, ptr %zero, align 4
  %eq36 = icmp eq i64 %max_bytes34, %zero35
  br i1 %eq36, label %if.then37, label %if.else38

if.merge26:                                       ; preds = %if.merge39, %return_result_merge_2
  %return_owner_084 = phi i1 [ true, %return_result_merge_2 ], [ %return_owner_082, %if.merge39 ]
  %if85 = phi { i32, i32 } [ %output33, %return_result_merge_2 ], [ %if83, %if.merge39 ]
  br label %if.merge20

return_result_null_0:                             ; preds = %if.then24
  br label %return_result_merge_2

return_result_copy_1:                             ; preds = %if.then24
  %allocation = call ptr @__wosy_core_alloc(i64 16, i64 8)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

return_result_merge_2:                            ; preds = %allocation_continue, %return_result_null_0
  %return_result = phi i32 [ 0, %return_result_null_0 ], [ %return_result_address, %allocation_continue ]
  %return_output31 = extractvalue { i32, i32 } %output30, 1
  %output32 = insertvalue { i32, i32 } zeroinitializer, i32 %return_result, 0
  %output33 = insertvalue { i32, i32 } %output32, i32 %return_output31, 1
  br label %if.merge26

allocation_panic:                                 ; preds = %return_result_copy_1
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %return_result_copy_1
  %return_result_source = inttoptr i32 %return_output to ptr
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %allocation, ptr align 1 %return_result_source, i32 16, i1 false)
  %return_result_address = ptrtoint ptr %allocation to i32
  br label %return_result_merge_2

if.then37:                                        ; preds = %if.else25
  %first40 = load i8, ptr %first, align 1
  %call41 = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f7a65726f5f6361706163697479(i8 %first40)
  store i32 %call41, ptr %status, align 4
  %status42 = load i32, ptr %status, align 4
  %eq43 = icmp eq i32 %status42, 0
  br i1 %eq43, label %if.then44, label %if.else45

if.else38:                                        ; preds = %if.else25
  %max_bytes70 = load i64, ptr %max_bytes1, align 4
  %first71 = load i8, ptr %first, align 1
  %first72 = load i8, ptr %first, align 1
  %eq73 = icmp eq i8 %first72, 13
  %call74 = call { i32, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f726561645f6c696e655f616c6c6f6361746564(i64 %max_bytes70, i8 %first71, i1 %eq73)
  %output75 = extractvalue { i32, i32 } %call74, 0
  store i32 %output75, ptr %text, align 4
  %output76 = extractvalue { i32, i32 } %call74, 1
  store i32 %output76, ptr %status77, align 4
  %text78 = load i32, ptr %text, align 4
  %status79 = load i32, ptr %status77, align 4
  %output80 = insertvalue { i32, i32 } zeroinitializer, i32 %text78, 0
  %output81 = insertvalue { i32, i32 } %output80, i32 %status79, 1
  br label %if.merge39

if.merge39:                                       ; preds = %if.else38, %if.merge46
  %return_owner_082 = phi i1 [ %return_owner_0, %if.merge46 ], [ true, %if.else38 ]
  %if83 = phi { i32, i32 } [ %if, %if.merge46 ], [ %output81, %if.else38 ]
  br label %if.merge26

if.then44:                                        ; preds = %if.then37
  %zero48 = load i64, ptr %zero, align 4
  %data49 = getelementptr inbounds i8, ptr %struct_literal47, i8 0
  store i32 0, ptr %data49, align 4
  %length50 = getelementptr inbounds i8, ptr %struct_literal47, i8 8
  store i64 %zero48, ptr %length50, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %empty51, ptr align 1 %struct_literal47, i32 16, i1 false)
  %address52 = ptrtoint ptr %empty51 to i32
  %status53 = load i32, ptr %status, align 4
  %output54 = insertvalue { i32, i32 } zeroinitializer, i32 %address52, 0
  %output55 = insertvalue { i32, i32 } %output54, i32 %status53, 1
  %return_output56 = extractvalue { i32, i32 } %output55, 0
  %return_result_is_null57 = icmp eq i32 %return_output56, 0
  br i1 %return_result_is_null57, label %return_result_null_3, label %return_result_copy_4

if.else45:                                        ; preds = %if.then37
  %status68 = load i32, ptr %status, align 4
  %output69 = insertvalue { i32, i32 } zeroinitializer, i32 %status68, 1
  br label %if.merge46

if.merge46:                                       ; preds = %if.else45, %return_result_merge_5
  %return_owner_0 = phi i1 [ true, %return_result_merge_5 ], [ false, %if.else45 ]
  %if = phi { i32, i32 } [ %output67, %return_result_merge_5 ], [ %output69, %if.else45 ]
  br label %if.merge39

return_result_null_3:                             ; preds = %if.then44
  br label %return_result_merge_5

return_result_copy_4:                             ; preds = %if.then44
  %allocation59 = call ptr @__wosy_core_alloc(i64 16, i64 8)
  %allocation_failed60 = icmp eq ptr %allocation59, null
  br i1 %allocation_failed60, label %allocation_panic58, label %allocation_continue61

return_result_merge_5:                            ; preds = %allocation_continue61, %return_result_null_3
  %return_result64 = phi i32 [ 0, %return_result_null_3 ], [ %return_result_address63, %allocation_continue61 ]
  %return_output65 = extractvalue { i32, i32 } %output55, 1
  %output66 = insertvalue { i32, i32 } zeroinitializer, i32 %return_result64, 0
  %output67 = insertvalue { i32, i32 } %output66, i32 %return_output65, 1
  br label %if.merge46

allocation_panic58:                               ; preds = %return_result_copy_4
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue61:                            ; preds = %return_result_copy_4
  %return_result_source62 = inttoptr i32 %return_output56 to ptr
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %allocation59, ptr align 1 %return_result_source62, i32 16, i1 false)
  %return_result_address63 = ptrtoint ptr %allocation59 to i32
  br label %return_result_merge_5
}

define i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f64696769745f76616c7565(i8 %byte) {
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

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f756e64657273636f7265(i8 %byte) {
entry:
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  %byte2 = load i8, ptr %byte1, align 1
  %eq = icmp eq i8 %byte2, 95
  ret i1 %eq
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f7369676e(i8 %byte) {
entry:
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  %byte2 = load i8, ptr %byte1, align 1
  %eq = icmp eq i8 %byte2, 45
  ret i1 %eq
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f706c7573(i8 %byte) {
entry:
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  %byte2 = load i8, ptr %byte1, align 1
  %eq = icmp eq i8 %byte2, 43
  ret i1 %eq
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f7a65726f(i8 %byte) {
entry:
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  %byte2 = load i8, ptr %byte1, align 1
  %eq = icmp eq i8 %byte2, 48
  ret i1 %eq
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f78(i8 %byte) {
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

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f62(i8 %byte) {
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

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f6f(i8 %byte) {
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

define i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616469785f66726f6d5f707265666978(i8 %first, i8 %second) {
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
  %call = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f7a65726f(i8 %first3)
  store i1 %call, ptr %leading, align 1
  %leading4 = load i1, ptr %leading, align 1
  br i1 %leading4, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %second5 = load i8, ptr %second2, align 1
  %call6 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f78(i8 %second5)
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
  %call12 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f62(i8 %second11)
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
  %call22 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f6f(i8 %second21)
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

define { i128, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f696e7465676572(i32 %data, i64 %start, i64 %lo, i64 %hi, i128 %radix, i128 %limit, i128 %remainder, i128 %result, i1 %valid, i1 %saw_digit, i1 %last_underscore) {
entry:
  %assignment_value145 = alloca i1, align 1
  %assignment_value142 = alloca i1, align 1
  %assignment_value139 = alloca i1, align 1
  %assignment_value136 = alloca i128, align 8
  %assignment_value118 = alloca i1, align 1
  %assignment_value115 = alloca i1, align 1
  %assignment_value112 = alloca i1, align 1
  %assignment_value109 = alloca i128, align 8
  %mid = alloca i64, align 8
  %two = alloca i64, align 8
  %assignment_value91 = alloca i1, align 1
  %assignment_value89 = alloca i1, align 1
  %assignment_value83 = alloca i1, align 1
  %assignment_value74 = alloca i1, align 1
  %assignment_value63 = alloca i1, align 1
  %assignment_value61 = alloca i1, align 1
  %assignment_value59 = alloca i128, align 8
  %assignment_value54 = alloca i1, align 1
  %digit_wide = alloca i128, align 8
  %digit = alloca i8, align 1
  %underscore = alloca i1, align 1
  %assignment_value = alloca i8, align 1
  %byte = alloca i8, align 1
  %position = alloca i64, align 8
  %wide_index = alloca i128, align 8
  %length = alloca i64, align 8
  %one = alloca i64, align 8
  %data1 = alloca i32, align 4
  store i32 %data, ptr %data1, align 4
  %start2 = alloca i64, align 8
  store i64 %start, ptr %start2, align 4
  %lo3 = alloca i64, align 8
  store i64 %lo, ptr %lo3, align 4
  %hi4 = alloca i64, align 8
  store i64 %hi, ptr %hi4, align 4
  %radix5 = alloca i128, align 8
  store i128 %radix, ptr %radix5, align 4
  %limit6 = alloca i128, align 8
  store i128 %limit, ptr %limit6, align 4
  %remainder7 = alloca i128, align 8
  store i128 %remainder, ptr %remainder7, align 4
  %result8 = alloca i128, align 8
  store i128 %result, ptr %result8, align 4
  %valid9 = alloca i1, align 1
  store i1 %valid, ptr %valid9, align 1
  %saw_digit10 = alloca i1, align 1
  store i1 %saw_digit, ptr %saw_digit10, align 1
  %last_underscore11 = alloca i1, align 1
  store i1 %last_underscore, ptr %last_underscore11, align 1
  store i64 1, ptr %one, align 4
  %lo12 = load i64, ptr %lo3, align 4
  %hi13 = load i64, ptr %hi4, align 4
  %lt = icmp ult i64 %lo12, %hi13
  br i1 %lt, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %valid14 = load i1, ptr %valid9, align 1
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %valid14, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.merge

if.then:                                          ; preds = %short_circuit.merge
  %hi15 = load i64, ptr %hi4, align 4
  %lo16 = load i64, ptr %lo3, align 4
  %sub = sub i64 %hi15, %lo16
  store i64 %sub, ptr %length, align 4
  %length17 = load i64, ptr %length, align 4
  %one18 = load i64, ptr %one, align 4
  %eq = icmp eq i64 %length17, %one18
  br i1 %eq, label %if.then19, label %if.else

if.merge:                                         ; preds = %if.merge20, %short_circuit.merge
  %result147 = load i128, ptr %result8, align 4
  %valid148 = load i1, ptr %valid9, align 1
  %saw_digit149 = load i1, ptr %saw_digit10, align 1
  %last_underscore150 = load i1, ptr %last_underscore11, align 1
  %output151 = insertvalue { i128, i1, i1, i1 } zeroinitializer, i128 %result147, 0
  %output152 = insertvalue { i128, i1, i1, i1 } %output151, i1 %valid148, 1
  %output153 = insertvalue { i128, i1, i1, i1 } %output152, i1 %saw_digit149, 2
  %output154 = insertvalue { i128, i1, i1, i1 } %output153, i1 %last_underscore150, 3
  ret { i128, i1, i1, i1 } %output154

if.then19:                                        ; preds = %if.then
  %lo21 = load i64, ptr %lo3, align 4
  %int_extend = zext i64 %lo21 to i128
  store i128 %int_extend, ptr %wide_index, align 4
  %wide_index22 = load i128, ptr %wide_index, align 4
  %int_trunc = trunc i128 %wide_index22 to i64
  store i64 %int_trunc, ptr %position, align 4
  store i8 0, ptr %byte, align 1
  %data23 = load i32, ptr %data1, align 4
  %position24 = load i64, ptr %position, align 4
  %offset_base = inttoptr i32 %data23 to ptr
  %offset = getelementptr inbounds i8, ptr %offset_base, i64 %position24
  %offset_address = ptrtoint ptr %offset to i32
  %load_base = inttoptr i32 %offset_address to ptr
  %load = load i8, ptr %load_base, align 1
  store i8 %load, ptr %assignment_value, align 1
  %assignment_value25 = load i8, ptr %assignment_value, align 1
  store i8 %assignment_value25, ptr %byte, align 1
  %byte26 = load i8, ptr %byte, align 1
  %call = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f756e64657273636f7265(i8 %byte26)
  store i1 %call, ptr %underscore, align 1
  %byte27 = load i8, ptr %byte, align 1
  %call28 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f64696769745f76616c7565(i8 %byte27)
  store i8 %call28, ptr %digit, align 1
  %digit29 = load i8, ptr %digit, align 1
  %int_extend30 = zext i8 %digit29 to i128
  store i128 %int_extend30, ptr %digit_wide, align 4
  %digit_wide31 = load i128, ptr %digit_wide, align 4
  %radix32 = load i128, ptr %radix5, align 4
  %lt33 = icmp ult i128 %digit_wide31, %radix32
  br i1 %lt33, label %if.then34, label %if.else35

if.else:                                          ; preds = %if.then
  store i64 2, ptr %two, align 4
  %lo93 = load i64, ptr %lo3, align 4
  %length94 = load i64, ptr %length, align 4
  %two95 = load i64, ptr %two, align 4
  %div = udiv i64 %length94, %two95
  %add96 = add i64 %lo93, %div
  store i64 %add96, ptr %mid, align 4
  %data97 = load i32, ptr %data1, align 4
  %start98 = load i64, ptr %start2, align 4
  %lo99 = load i64, ptr %lo3, align 4
  %mid100 = load i64, ptr %mid, align 4
  %radix101 = load i128, ptr %radix5, align 4
  %limit102 = load i128, ptr %limit6, align 4
  %remainder103 = load i128, ptr %remainder7, align 4
  %result104 = load i128, ptr %result8, align 4
  %valid105 = load i1, ptr %valid9, align 1
  %saw_digit106 = load i1, ptr %saw_digit10, align 1
  %last_underscore107 = load i1, ptr %last_underscore11, align 1
  %call108 = call { i128, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f696e7465676572(i32 %data97, i64 %start98, i64 %lo99, i64 %mid100, i128 %radix101, i128 %limit102, i128 %remainder103, i128 %result104, i1 %valid105, i1 %saw_digit106, i1 %last_underscore107)
  %output = extractvalue { i128, i1, i1, i1 } %call108, 0
  store i128 %output, ptr %assignment_value109, align 4
  %assignment_value110 = load i128, ptr %assignment_value109, align 4
  %output111 = extractvalue { i128, i1, i1, i1 } %call108, 1
  store i1 %output111, ptr %assignment_value112, align 1
  %assignment_value113 = load i1, ptr %assignment_value112, align 1
  %output114 = extractvalue { i128, i1, i1, i1 } %call108, 2
  store i1 %output114, ptr %assignment_value115, align 1
  %assignment_value116 = load i1, ptr %assignment_value115, align 1
  %output117 = extractvalue { i128, i1, i1, i1 } %call108, 3
  store i1 %output117, ptr %assignment_value118, align 1
  %assignment_value119 = load i1, ptr %assignment_value118, align 1
  store i128 %assignment_value110, ptr %result8, align 4
  store i1 %assignment_value113, ptr %valid9, align 1
  store i1 %assignment_value116, ptr %saw_digit10, align 1
  store i1 %assignment_value119, ptr %last_underscore11, align 1
  %valid120 = load i1, ptr %valid9, align 1
  br i1 %valid120, label %if.then121, label %if.merge122

if.merge20:                                       ; preds = %if.merge122, %if.merge36
  br label %if.merge

if.then34:                                        ; preds = %if.then19
  %result37 = load i128, ptr %result8, align 4
  %limit38 = load i128, ptr %limit6, align 4
  %gt = icmp ugt i128 %result37, %limit38
  br i1 %gt, label %short_circuit.merge40, label %short_circuit.rhs39

if.else35:                                        ; preds = %if.then19
  %underscore65 = load i1, ptr %underscore, align 1
  br i1 %underscore65, label %short_circuit.rhs66, label %short_circuit.merge67

if.merge36:                                       ; preds = %if.merge88, %if.merge53
  br label %if.merge20

short_circuit.rhs39:                              ; preds = %if.then34
  %result41 = load i128, ptr %result8, align 4
  %limit42 = load i128, ptr %limit6, align 4
  %eq43 = icmp eq i128 %result41, %limit42
  br i1 %eq43, label %short_circuit.rhs44, label %short_circuit.merge45

short_circuit.merge40:                            ; preds = %short_circuit.merge45, %if.then34
  %short_circuit50 = phi i1 [ true, %if.then34 ], [ %short_circuit49, %short_circuit.merge45 ]
  br i1 %short_circuit50, label %if.then51, label %if.else52

short_circuit.rhs44:                              ; preds = %short_circuit.rhs39
  %digit_wide46 = load i128, ptr %digit_wide, align 4
  %remainder47 = load i128, ptr %remainder7, align 4
  %gt48 = icmp ugt i128 %digit_wide46, %remainder47
  br label %short_circuit.merge45

short_circuit.merge45:                            ; preds = %short_circuit.rhs44, %short_circuit.rhs39
  %short_circuit49 = phi i1 [ false, %short_circuit.rhs39 ], [ %gt48, %short_circuit.rhs44 ]
  br label %short_circuit.merge40

if.then51:                                        ; preds = %short_circuit.merge40
  store i1 false, ptr %assignment_value54, align 1
  %assignment_value55 = load i1, ptr %assignment_value54, align 1
  store i1 %assignment_value55, ptr %valid9, align 1
  br label %if.merge53

if.else52:                                        ; preds = %short_circuit.merge40
  %result56 = load i128, ptr %result8, align 4
  %radix57 = load i128, ptr %radix5, align 4
  %mul = mul i128 %result56, %radix57
  %digit_wide58 = load i128, ptr %digit_wide, align 4
  %add = add i128 %mul, %digit_wide58
  store i128 %add, ptr %assignment_value59, align 4
  %assignment_value60 = load i128, ptr %assignment_value59, align 4
  store i128 %assignment_value60, ptr %result8, align 4
  br label %if.merge53

if.merge53:                                       ; preds = %if.else52, %if.then51
  store i1 false, ptr %assignment_value61, align 1
  %assignment_value62 = load i1, ptr %assignment_value61, align 1
  store i1 %assignment_value62, ptr %last_underscore11, align 1
  store i1 true, ptr %assignment_value63, align 1
  %assignment_value64 = load i1, ptr %assignment_value63, align 1
  store i1 %assignment_value64, ptr %saw_digit10, align 1
  br label %if.merge36

short_circuit.rhs66:                              ; preds = %if.else35
  %lo68 = load i64, ptr %lo3, align 4
  %start69 = load i64, ptr %start2, align 4
  %eq70 = icmp eq i64 %lo68, %start69
  br label %short_circuit.merge67

short_circuit.merge67:                            ; preds = %short_circuit.rhs66, %if.else35
  %short_circuit71 = phi i1 [ false, %if.else35 ], [ %eq70, %short_circuit.rhs66 ]
  br i1 %short_circuit71, label %if.then72, label %if.merge73

if.then72:                                        ; preds = %short_circuit.merge67
  store i1 false, ptr %assignment_value74, align 1
  %assignment_value75 = load i1, ptr %assignment_value74, align 1
  store i1 %assignment_value75, ptr %valid9, align 1
  br label %if.merge73

if.merge73:                                       ; preds = %if.then72, %short_circuit.merge67
  %underscore76 = load i1, ptr %underscore, align 1
  br i1 %underscore76, label %short_circuit.rhs77, label %short_circuit.merge78

short_circuit.rhs77:                              ; preds = %if.merge73
  %last_underscore79 = load i1, ptr %last_underscore11, align 1
  br label %short_circuit.merge78

short_circuit.merge78:                            ; preds = %short_circuit.rhs77, %if.merge73
  %short_circuit80 = phi i1 [ false, %if.merge73 ], [ %last_underscore79, %short_circuit.rhs77 ]
  br i1 %short_circuit80, label %if.then81, label %if.merge82

if.then81:                                        ; preds = %short_circuit.merge78
  store i1 false, ptr %assignment_value83, align 1
  %assignment_value84 = load i1, ptr %assignment_value83, align 1
  store i1 %assignment_value84, ptr %valid9, align 1
  br label %if.merge82

if.merge82:                                       ; preds = %if.then81, %short_circuit.merge78
  %underscore85 = load i1, ptr %underscore, align 1
  br i1 %underscore85, label %if.then86, label %if.else87

if.then86:                                        ; preds = %if.merge82
  store i1 true, ptr %assignment_value89, align 1
  %assignment_value90 = load i1, ptr %assignment_value89, align 1
  store i1 %assignment_value90, ptr %last_underscore11, align 1
  br label %if.merge88

if.else87:                                        ; preds = %if.merge82
  store i1 false, ptr %assignment_value91, align 1
  %assignment_value92 = load i1, ptr %assignment_value91, align 1
  store i1 %assignment_value92, ptr %valid9, align 1
  br label %if.merge88

if.merge88:                                       ; preds = %if.else87, %if.then86
  br label %if.merge36

if.then121:                                       ; preds = %if.else
  %data123 = load i32, ptr %data1, align 4
  %start124 = load i64, ptr %start2, align 4
  %mid125 = load i64, ptr %mid, align 4
  %hi126 = load i64, ptr %hi4, align 4
  %radix127 = load i128, ptr %radix5, align 4
  %limit128 = load i128, ptr %limit6, align 4
  %remainder129 = load i128, ptr %remainder7, align 4
  %result130 = load i128, ptr %result8, align 4
  %valid131 = load i1, ptr %valid9, align 1
  %saw_digit132 = load i1, ptr %saw_digit10, align 1
  %last_underscore133 = load i1, ptr %last_underscore11, align 1
  %call134 = call { i128, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f696e7465676572(i32 %data123, i64 %start124, i64 %mid125, i64 %hi126, i128 %radix127, i128 %limit128, i128 %remainder129, i128 %result130, i1 %valid131, i1 %saw_digit132, i1 %last_underscore133)
  %output135 = extractvalue { i128, i1, i1, i1 } %call134, 0
  store i128 %output135, ptr %assignment_value136, align 4
  %assignment_value137 = load i128, ptr %assignment_value136, align 4
  %output138 = extractvalue { i128, i1, i1, i1 } %call134, 1
  store i1 %output138, ptr %assignment_value139, align 1
  %assignment_value140 = load i1, ptr %assignment_value139, align 1
  %output141 = extractvalue { i128, i1, i1, i1 } %call134, 2
  store i1 %output141, ptr %assignment_value142, align 1
  %assignment_value143 = load i1, ptr %assignment_value142, align 1
  %output144 = extractvalue { i128, i1, i1, i1 } %call134, 3
  store i1 %output144, ptr %assignment_value145, align 1
  %assignment_value146 = load i1, ptr %assignment_value145, align 1
  store i128 %assignment_value137, ptr %result8, align 4
  store i1 %assignment_value140, ptr %valid9, align 1
  store i1 %assignment_value143, ptr %saw_digit10, align 1
  store i1 %assignment_value146, ptr %last_underscore11, align 1
  br label %if.merge122

if.merge122:                                      ; preds = %if.then121, %if.else
  br label %if.merge20
}

define { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f616363756d756c6174655f75313238(i32 %data, i64 %start, i64 %end, i8 %radix) {
entry:
  %assignment_value38 = alloca i1, align 1
  %assignment_value33 = alloca i1, align 1
  %assignment_value30 = alloca i1, align 1
  %assignment_value27 = alloca i1, align 1
  %assignment_value24 = alloca i1, align 1
  %assignment_value = alloca i128, align 8
  %last_underscore = alloca i1, align 1
  %saw_digit = alloca i1, align 1
  %valid = alloca i1, align 1
  %result = alloca i128, align 8
  %remainder = alloca i128, align 8
  %limit = alloca i128, align 8
  %radix_wide = alloca i128, align 8
  %maximum = alloca i128, align 8
  %data1 = alloca i32, align 4
  store i32 %data, ptr %data1, align 4
  %start2 = alloca i64, align 8
  store i64 %start, ptr %start2, align 4
  %end3 = alloca i64, align 8
  store i64 %end, ptr %end3, align 4
  %radix4 = alloca i8, align 1
  store i8 %radix, ptr %radix4, align 1
  store i128 -1, ptr %maximum, align 4
  %radix5 = load i8, ptr %radix4, align 1
  %int_extend = zext i8 %radix5 to i128
  store i128 %int_extend, ptr %radix_wide, align 4
  %maximum6 = load i128, ptr %maximum, align 4
  %radix_wide7 = load i128, ptr %radix_wide, align 4
  %div = udiv i128 %maximum6, %radix_wide7
  store i128 %div, ptr %limit, align 4
  %maximum8 = load i128, ptr %maximum, align 4
  %limit9 = load i128, ptr %limit, align 4
  %radix_wide10 = load i128, ptr %radix_wide, align 4
  %mul = mul i128 %limit9, %radix_wide10
  %sub = sub i128 %maximum8, %mul
  store i128 %sub, ptr %remainder, align 4
  store i128 0, ptr %result, align 4
  store i1 true, ptr %valid, align 1
  store i1 false, ptr %saw_digit, align 1
  store i1 false, ptr %last_underscore, align 1
  %data11 = load i32, ptr %data1, align 4
  %start12 = load i64, ptr %start2, align 4
  %start13 = load i64, ptr %start2, align 4
  %end14 = load i64, ptr %end3, align 4
  %radix_wide15 = load i128, ptr %radix_wide, align 4
  %limit16 = load i128, ptr %limit, align 4
  %remainder17 = load i128, ptr %remainder, align 4
  %result18 = load i128, ptr %result, align 4
  %valid19 = load i1, ptr %valid, align 1
  %saw_digit20 = load i1, ptr %saw_digit, align 1
  %last_underscore21 = load i1, ptr %last_underscore, align 1
  %call = call { i128, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f696e7465676572(i32 %data11, i64 %start12, i64 %start13, i64 %end14, i128 %radix_wide15, i128 %limit16, i128 %remainder17, i128 %result18, i1 %valid19, i1 %saw_digit20, i1 %last_underscore21)
  %output = extractvalue { i128, i1, i1, i1 } %call, 0
  store i128 %output, ptr %assignment_value, align 4
  %assignment_value22 = load i128, ptr %assignment_value, align 4
  %output23 = extractvalue { i128, i1, i1, i1 } %call, 1
  store i1 %output23, ptr %assignment_value24, align 1
  %assignment_value25 = load i1, ptr %assignment_value24, align 1
  %output26 = extractvalue { i128, i1, i1, i1 } %call, 2
  store i1 %output26, ptr %assignment_value27, align 1
  %assignment_value28 = load i1, ptr %assignment_value27, align 1
  %output29 = extractvalue { i128, i1, i1, i1 } %call, 3
  store i1 %output29, ptr %assignment_value30, align 1
  %assignment_value31 = load i1, ptr %assignment_value30, align 1
  store i128 %assignment_value22, ptr %result, align 4
  store i1 %assignment_value25, ptr %valid, align 1
  store i1 %assignment_value28, ptr %saw_digit, align 1
  store i1 %assignment_value31, ptr %last_underscore, align 1
  %saw_digit32 = load i1, ptr %saw_digit, align 1
  %not = xor i1 %saw_digit32, true
  br i1 %not, label %if.then, label %if.merge

if.then:                                          ; preds = %entry
  store i1 false, ptr %assignment_value33, align 1
  %assignment_value34 = load i1, ptr %assignment_value33, align 1
  store i1 %assignment_value34, ptr %valid, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %entry
  %last_underscore35 = load i1, ptr %last_underscore, align 1
  br i1 %last_underscore35, label %if.then36, label %if.merge37

if.then36:                                        ; preds = %if.merge
  store i1 false, ptr %assignment_value38, align 1
  %assignment_value39 = load i1, ptr %assignment_value38, align 1
  store i1 %assignment_value39, ptr %valid, align 1
  br label %if.merge37

if.merge37:                                       ; preds = %if.then36, %if.merge
  %result40 = load i128, ptr %result, align 4
  %valid41 = load i1, ptr %valid, align 1
  %output42 = insertvalue { i128, i1 } zeroinitializer, i128 %result40, 0
  %output43 = insertvalue { i128, i1 } %output42, i1 %valid41, 1
  ret { i128, i1 } %output43
}

define { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f616363756d756c6174655f693132385f6e6567(i32 %data, i64 %start, i64 %end, i8 %radix) {
entry:
  %assignment_value38 = alloca i1, align 1
  %assignment_value33 = alloca i1, align 1
  %assignment_value30 = alloca i1, align 1
  %assignment_value27 = alloca i1, align 1
  %assignment_value24 = alloca i1, align 1
  %assignment_value = alloca i128, align 8
  %last_underscore = alloca i1, align 1
  %saw_digit = alloca i1, align 1
  %valid = alloca i1, align 1
  %result = alloca i128, align 8
  %remainder = alloca i128, align 8
  %limit = alloca i128, align 8
  %radix_wide = alloca i128, align 8
  %maximum = alloca i128, align 8
  %data1 = alloca i32, align 4
  store i32 %data, ptr %data1, align 4
  %start2 = alloca i64, align 8
  store i64 %start, ptr %start2, align 4
  %end3 = alloca i64, align 8
  store i64 %end, ptr %end3, align 4
  %radix4 = alloca i8, align 1
  store i8 %radix, ptr %radix4, align 1
  store i128 -170141183460469231731687303715884105728, ptr %maximum, align 4
  %radix5 = load i8, ptr %radix4, align 1
  %int_extend = zext i8 %radix5 to i128
  store i128 %int_extend, ptr %radix_wide, align 4
  %maximum6 = load i128, ptr %maximum, align 4
  %radix_wide7 = load i128, ptr %radix_wide, align 4
  %div = udiv i128 %maximum6, %radix_wide7
  store i128 %div, ptr %limit, align 4
  %maximum8 = load i128, ptr %maximum, align 4
  %limit9 = load i128, ptr %limit, align 4
  %radix_wide10 = load i128, ptr %radix_wide, align 4
  %mul = mul i128 %limit9, %radix_wide10
  %sub = sub i128 %maximum8, %mul
  store i128 %sub, ptr %remainder, align 4
  store i128 0, ptr %result, align 4
  store i1 true, ptr %valid, align 1
  store i1 false, ptr %saw_digit, align 1
  store i1 false, ptr %last_underscore, align 1
  %data11 = load i32, ptr %data1, align 4
  %start12 = load i64, ptr %start2, align 4
  %start13 = load i64, ptr %start2, align 4
  %end14 = load i64, ptr %end3, align 4
  %radix_wide15 = load i128, ptr %radix_wide, align 4
  %limit16 = load i128, ptr %limit, align 4
  %remainder17 = load i128, ptr %remainder, align 4
  %result18 = load i128, ptr %result, align 4
  %valid19 = load i1, ptr %valid, align 1
  %saw_digit20 = load i1, ptr %saw_digit, align 1
  %last_underscore21 = load i1, ptr %last_underscore, align 1
  %call = call { i128, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f696e7465676572(i32 %data11, i64 %start12, i64 %start13, i64 %end14, i128 %radix_wide15, i128 %limit16, i128 %remainder17, i128 %result18, i1 %valid19, i1 %saw_digit20, i1 %last_underscore21)
  %output = extractvalue { i128, i1, i1, i1 } %call, 0
  store i128 %output, ptr %assignment_value, align 4
  %assignment_value22 = load i128, ptr %assignment_value, align 4
  %output23 = extractvalue { i128, i1, i1, i1 } %call, 1
  store i1 %output23, ptr %assignment_value24, align 1
  %assignment_value25 = load i1, ptr %assignment_value24, align 1
  %output26 = extractvalue { i128, i1, i1, i1 } %call, 2
  store i1 %output26, ptr %assignment_value27, align 1
  %assignment_value28 = load i1, ptr %assignment_value27, align 1
  %output29 = extractvalue { i128, i1, i1, i1 } %call, 3
  store i1 %output29, ptr %assignment_value30, align 1
  %assignment_value31 = load i1, ptr %assignment_value30, align 1
  store i128 %assignment_value22, ptr %result, align 4
  store i1 %assignment_value25, ptr %valid, align 1
  store i1 %assignment_value28, ptr %saw_digit, align 1
  store i1 %assignment_value31, ptr %last_underscore, align 1
  %saw_digit32 = load i1, ptr %saw_digit, align 1
  %not = xor i1 %saw_digit32, true
  br i1 %not, label %if.then, label %if.merge

if.then:                                          ; preds = %entry
  store i1 false, ptr %assignment_value33, align 1
  %assignment_value34 = load i1, ptr %assignment_value33, align 1
  store i1 %assignment_value34, ptr %valid, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %entry
  %last_underscore35 = load i1, ptr %last_underscore, align 1
  br i1 %last_underscore35, label %if.then36, label %if.merge37

if.then36:                                        ; preds = %if.merge
  store i1 false, ptr %assignment_value38, align 1
  %assignment_value39 = load i1, ptr %assignment_value38, align 1
  store i1 %assignment_value39, ptr %valid, align 1
  br label %if.merge37

if.merge37:                                       ; preds = %if.then36, %if.merge
  %result40 = load i128, ptr %result, align 4
  %valid41 = load i1, ptr %valid, align 1
  %output42 = insertvalue { i128, i1 } zeroinitializer, i128 %result40, 0
  %output43 = insertvalue { i128, i1 } %output42, i1 %valid41, 1
  ret { i128, i1 } %output43
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f7538(i128 %value) {
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

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f753136(i128 %value) {
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

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f753332(i128 %value) {
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

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f753634(i128 %value) {
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

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f75313238(i128 %value) {
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

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f6938(i128 %magnitude, i1 %negative) {
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

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f693136(i128 %magnitude, i1 %negative) {
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

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f693332(i128 %magnitude, i1 %negative) {
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

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f693634(i128 %magnitude, i1 %negative) {
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

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f69313238(i128 %magnitude, i1 %negative) {
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

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f7538(ptr %text) #0 {
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
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
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
  %call = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616469785f66726f6d5f707265666978(i8 %first33, i8 %second34)
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
  %call50 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f616363756d756c6174655f75313238(i32 %place45, i64 %start46, i64 %place48, i8 %radix49)
  %output = extractvalue { i128, i1 } %call50, 0
  store i128 %output, ptr %magnitude, align 4
  %output51 = extractvalue { i128, i1 } %call50, 1
  store i1 %output51, ptr %valid, align 1
  %magnitude52 = load i128, ptr %magnitude, align 4
  %call53 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f7538(i128 %magnitude52)
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

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f753136(ptr %text) #0 {
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
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
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
  %call = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616469785f66726f6d5f707265666978(i8 %first33, i8 %second34)
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
  %call50 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f616363756d756c6174655f75313238(i32 %place45, i64 %start46, i64 %place48, i8 %radix49)
  %output = extractvalue { i128, i1 } %call50, 0
  store i128 %output, ptr %magnitude, align 4
  %output51 = extractvalue { i128, i1 } %call50, 1
  store i1 %output51, ptr %valid, align 1
  %magnitude52 = load i128, ptr %magnitude, align 4
  %call53 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f753136(i128 %magnitude52)
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

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f753332(ptr %text) #0 {
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
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
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
  %call = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616469785f66726f6d5f707265666978(i8 %first33, i8 %second34)
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
  %call50 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f616363756d756c6174655f75313238(i32 %place45, i64 %start46, i64 %place48, i8 %radix49)
  %output = extractvalue { i128, i1 } %call50, 0
  store i128 %output, ptr %magnitude, align 4
  %output51 = extractvalue { i128, i1 } %call50, 1
  store i1 %output51, ptr %valid, align 1
  %magnitude52 = load i128, ptr %magnitude, align 4
  %call53 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f753332(i128 %magnitude52)
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

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f753634(ptr %text) #0 {
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
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
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
  %call = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616469785f66726f6d5f707265666978(i8 %first34, i8 %second35)
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
  %call51 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f616363756d756c6174655f75313238(i32 %place46, i64 %start47, i64 %place49, i8 %radix50)
  %output = extractvalue { i128, i1 } %call51, 0
  store i128 %output, ptr %magnitude, align 4
  %output52 = extractvalue { i128, i1 } %call51, 1
  store i1 %output52, ptr %valid, align 1
  %magnitude53 = load i128, ptr %magnitude, align 4
  %call54 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f753634(i128 %magnitude53)
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

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f75313238(ptr %text) #0 {
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
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
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
  %call = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616469785f66726f6d5f707265666978(i8 %first33, i8 %second34)
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
  %call50 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f616363756d756c6174655f75313238(i32 %place45, i64 %start46, i64 %place48, i8 %radix49)
  %output = extractvalue { i128, i1 } %call50, 0
  store i128 %output, ptr %magnitude, align 4
  %output51 = extractvalue { i128, i1 } %call50, 1
  store i1 %output51, ptr %valid, align 1
  %magnitude52 = load i128, ptr %magnitude, align 4
  %call53 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f75313238(i128 %magnitude52)
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

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f6938(ptr %text) #0 {
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
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
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
  %call = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f7369676e(i8 %first16)
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
  %call63 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616469785f66726f6d5f707265666978(i8 %prefix_first61, i8 %prefix_second62)
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
  %call81 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f616363756d756c6174655f693132385f6e6567(i32 %place76, i64 %start77, i64 %place79, i8 %radix80)
  %output = extractvalue { i128, i1 } %call81, 0
  store i128 %output, ptr %magnitude, align 4
  %output82 = extractvalue { i128, i1 } %call81, 1
  store i1 %output82, ptr %valid, align 1
  %magnitude83 = load i128, ptr %magnitude, align 4
  %negative84 = load i1, ptr %negative, align 1
  %call85 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f6938(i128 %magnitude83, i1 %negative84)
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

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f693136(ptr %text) #0 {
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
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
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
  %call = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f7369676e(i8 %first16)
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
  %call63 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616469785f66726f6d5f707265666978(i8 %prefix_first61, i8 %prefix_second62)
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
  %call81 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f616363756d756c6174655f693132385f6e6567(i32 %place76, i64 %start77, i64 %place79, i8 %radix80)
  %output = extractvalue { i128, i1 } %call81, 0
  store i128 %output, ptr %magnitude, align 4
  %output82 = extractvalue { i128, i1 } %call81, 1
  store i1 %output82, ptr %valid, align 1
  %magnitude83 = load i128, ptr %magnitude, align 4
  %negative84 = load i1, ptr %negative, align 1
  %call85 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f693136(i128 %magnitude83, i1 %negative84)
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

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f693332(ptr %text) #0 {
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
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
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
  %call = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f7369676e(i8 %first16)
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
  %call63 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616469785f66726f6d5f707265666978(i8 %prefix_first61, i8 %prefix_second62)
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
  %call81 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f616363756d756c6174655f693132385f6e6567(i32 %place76, i64 %start77, i64 %place79, i8 %radix80)
  %output = extractvalue { i128, i1 } %call81, 0
  store i128 %output, ptr %magnitude, align 4
  %output82 = extractvalue { i128, i1 } %call81, 1
  store i1 %output82, ptr %valid, align 1
  %magnitude83 = load i128, ptr %magnitude, align 4
  %negative84 = load i1, ptr %negative, align 1
  %call85 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f693332(i128 %magnitude83, i1 %negative84)
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

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f693634(ptr %text) #0 {
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
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
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
  %call = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f7369676e(i8 %first16)
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
  %call63 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616469785f66726f6d5f707265666978(i8 %prefix_first61, i8 %prefix_second62)
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
  %call81 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f616363756d756c6174655f693132385f6e6567(i32 %place76, i64 %start77, i64 %place79, i8 %radix80)
  %output = extractvalue { i128, i1 } %call81, 0
  store i128 %output, ptr %magnitude, align 4
  %output82 = extractvalue { i128, i1 } %call81, 1
  store i1 %output82, ptr %valid, align 1
  %magnitude83 = load i128, ptr %magnitude, align 4
  %negative84 = load i1, ptr %negative, align 1
  %call85 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f693634(i128 %magnitude83, i1 %negative84)
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

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69313238(ptr %text) #0 {
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
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
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
  %call = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f7369676e(i8 %first16)
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
  %call63 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616469785f66726f6d5f707265666978(i8 %prefix_first61, i8 %prefix_second62)
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
  %call81 = call { i128, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f616363756d756c6174655f693132385f6e6567(i32 %place76, i64 %start77, i64 %place79, i8 %radix80)
  %output = extractvalue { i128, i1 } %call81, 0
  store i128 %output, ptr %magnitude, align 4
  %output82 = extractvalue { i128, i1 } %call81, 1
  store i1 %output82, ptr %valid, align 1
  %magnitude83 = load i128, ptr %magnitude, align 4
  %negative84 = load i1, ptr %negative, align 1
  %call85 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f636865636b5f69313238(i128 %magnitude83, i1 %negative84)
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

define i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f666c6f61745f62797465(i32 %data, i64 %index) {
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

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f646f74(i8 %byte) {
entry:
  %byte1 = alloca i8, align 1
  store i8 %byte, ptr %byte1, align 1
  %byte2 = load i8, ptr %byte1, align 1
  %eq = icmp eq i8 %byte2, 46
  ret i1 %eq
}

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f65(i8 %byte) {
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

define i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f64696769743130(i8 %byte) {
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

define { i64, double, i32, i1, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f666c6f61745f696e7465676572(i32 %data, i64 %pos, i64 %end, double %mant, i32 %extra_scale, i1 %valid, i1 %saw_int, i1 %last_underscore, i1 %int_done) {
entry:
  %assignment_value161 = alloca i1, align 1
  %assignment_value159 = alloca i64, align 8
  %assignment_value154 = alloca i1, align 1
  %assignment_value152 = alloca i1, align 1
  %assignment_value147 = alloca i1, align 1
  %assignment_value137 = alloca i64, align 8
  %assignment_value132 = alloca i1, align 1
  %assignment_value130 = alloca i1, align 1
  %assignment_value128 = alloca i32, align 4
  %assignment_value123 = alloca double, align 8
  %assignment_value113 = alloca double, align 8
  %digit_value = alloca double, align 8
  %wide_digit = alloca i64, align 8
  %small_int = alloca i8, align 1
  %underscore = alloca i1, align 1
  %digit = alloca i1, align 1
  %byte = alloca i8, align 1
  %assignment_value91 = alloca i1, align 1
  %assignment_value88 = alloca i1, align 1
  %assignment_value85 = alloca i1, align 1
  %assignment_value82 = alloca i1, align 1
  %assignment_value79 = alloca i32, align 4
  %assignment_value76 = alloca double, align 8
  %assignment_value73 = alloca i64, align 8
  %assignment_value52 = alloca i1, align 1
  %assignment_value49 = alloca i1, align 1
  %assignment_value46 = alloca i1, align 1
  %assignment_value43 = alloca i1, align 1
  %assignment_value40 = alloca i32, align 4
  %assignment_value37 = alloca double, align 8
  %assignment_value = alloca i64, align 8
  %mid = alloca i64, align 8
  %length = alloca i64, align 8
  %one_count = alloca i32, align 4
  %zero_digit = alloca i8, align 1
  %big_limit = alloca double, align 8
  %ten_value = alloca double, align 8
  %two = alloca i64, align 8
  %one = alloca i64, align 8
  %data1 = alloca i32, align 4
  store i32 %data, ptr %data1, align 4
  %pos2 = alloca i64, align 8
  store i64 %pos, ptr %pos2, align 4
  %end3 = alloca i64, align 8
  store i64 %end, ptr %end3, align 4
  %mant4 = alloca double, align 8
  store double %mant, ptr %mant4, align 8
  %extra_scale5 = alloca i32, align 4
  store i32 %extra_scale, ptr %extra_scale5, align 4
  %valid6 = alloca i1, align 1
  store i1 %valid, ptr %valid6, align 1
  %saw_int7 = alloca i1, align 1
  store i1 %saw_int, ptr %saw_int7, align 1
  %last_underscore8 = alloca i1, align 1
  store i1 %last_underscore, ptr %last_underscore8, align 1
  %int_done9 = alloca i1, align 1
  store i1 %int_done, ptr %int_done9, align 1
  store i64 1, ptr %one, align 4
  store i64 2, ptr %two, align 4
  store double 1.000000e+01, ptr %ten_value, align 8
  store double 1.000000e+300, ptr %big_limit, align 8
  store i8 48, ptr %zero_digit, align 1
  store i32 1, ptr %one_count, align 4
  %pos10 = load i64, ptr %pos2, align 4
  %end11 = load i64, ptr %end3, align 4
  %lt = icmp ult i64 %pos10, %end11
  br i1 %lt, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %valid12 = load i1, ptr %valid6, align 1
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %valid12, %short_circuit.rhs ]
  br i1 %short_circuit, label %short_circuit.rhs13, label %short_circuit.merge14

short_circuit.rhs13:                              ; preds = %short_circuit.merge
  %int_done15 = load i1, ptr %int_done9, align 1
  %not = xor i1 %int_done15, true
  br label %short_circuit.merge14

short_circuit.merge14:                            ; preds = %short_circuit.rhs13, %short_circuit.merge
  %short_circuit16 = phi i1 [ false, %short_circuit.merge ], [ %not, %short_circuit.rhs13 ]
  br i1 %short_circuit16, label %if.then, label %if.merge

if.then:                                          ; preds = %short_circuit.merge14
  %end17 = load i64, ptr %end3, align 4
  %pos18 = load i64, ptr %pos2, align 4
  %sub = sub i64 %end17, %pos18
  store i64 %sub, ptr %length, align 4
  %length19 = load i64, ptr %length, align 4
  %one20 = load i64, ptr %one, align 4
  %gt = icmp ugt i64 %length19, %one20
  br i1 %gt, label %if.then21, label %if.else

if.merge:                                         ; preds = %if.merge22, %short_circuit.merge14
  %pos163 = load i64, ptr %pos2, align 4
  %mant164 = load double, ptr %mant4, align 8
  %extra_scale165 = load i32, ptr %extra_scale5, align 4
  %valid166 = load i1, ptr %valid6, align 1
  %saw_int167 = load i1, ptr %saw_int7, align 1
  %last_underscore168 = load i1, ptr %last_underscore8, align 1
  %int_done169 = load i1, ptr %int_done9, align 1
  %output170 = insertvalue { i64, double, i32, i1, i1, i1, i1 } zeroinitializer, i64 %pos163, 0
  %output171 = insertvalue { i64, double, i32, i1, i1, i1, i1 } %output170, double %mant164, 1
  %output172 = insertvalue { i64, double, i32, i1, i1, i1, i1 } %output171, i32 %extra_scale165, 2
  %output173 = insertvalue { i64, double, i32, i1, i1, i1, i1 } %output172, i1 %valid166, 3
  %output174 = insertvalue { i64, double, i32, i1, i1, i1, i1 } %output173, i1 %saw_int167, 4
  %output175 = insertvalue { i64, double, i32, i1, i1, i1, i1 } %output174, i1 %last_underscore168, 5
  %output176 = insertvalue { i64, double, i32, i1, i1, i1, i1 } %output175, i1 %int_done169, 6
  ret { i64, double, i32, i1, i1, i1, i1 } %output176

if.then21:                                        ; preds = %if.then
  %pos23 = load i64, ptr %pos2, align 4
  %length24 = load i64, ptr %length, align 4
  %two25 = load i64, ptr %two, align 4
  %div = udiv i64 %length24, %two25
  %add = add i64 %pos23, %div
  store i64 %add, ptr %mid, align 4
  %data26 = load i32, ptr %data1, align 4
  %pos27 = load i64, ptr %pos2, align 4
  %mid28 = load i64, ptr %mid, align 4
  %mant29 = load double, ptr %mant4, align 8
  %extra_scale30 = load i32, ptr %extra_scale5, align 4
  %valid31 = load i1, ptr %valid6, align 1
  %saw_int32 = load i1, ptr %saw_int7, align 1
  %last_underscore33 = load i1, ptr %last_underscore8, align 1
  %int_done34 = load i1, ptr %int_done9, align 1
  %call = call { i64, double, i32, i1, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f666c6f61745f696e7465676572(i32 %data26, i64 %pos27, i64 %mid28, double %mant29, i32 %extra_scale30, i1 %valid31, i1 %saw_int32, i1 %last_underscore33, i1 %int_done34)
  %output = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call, 0
  store i64 %output, ptr %assignment_value, align 4
  %assignment_value35 = load i64, ptr %assignment_value, align 4
  %output36 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call, 1
  store double %output36, ptr %assignment_value37, align 8
  %assignment_value38 = load double, ptr %assignment_value37, align 8
  %output39 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call, 2
  store i32 %output39, ptr %assignment_value40, align 4
  %assignment_value41 = load i32, ptr %assignment_value40, align 4
  %output42 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call, 3
  store i1 %output42, ptr %assignment_value43, align 1
  %assignment_value44 = load i1, ptr %assignment_value43, align 1
  %output45 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call, 4
  store i1 %output45, ptr %assignment_value46, align 1
  %assignment_value47 = load i1, ptr %assignment_value46, align 1
  %output48 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call, 5
  store i1 %output48, ptr %assignment_value49, align 1
  %assignment_value50 = load i1, ptr %assignment_value49, align 1
  %output51 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call, 6
  store i1 %output51, ptr %assignment_value52, align 1
  %assignment_value53 = load i1, ptr %assignment_value52, align 1
  store i64 %assignment_value35, ptr %pos2, align 4
  store double %assignment_value38, ptr %mant4, align 8
  store i32 %assignment_value41, ptr %extra_scale5, align 4
  store i1 %assignment_value44, ptr %valid6, align 1
  store i1 %assignment_value47, ptr %saw_int7, align 1
  store i1 %assignment_value50, ptr %last_underscore8, align 1
  store i1 %assignment_value53, ptr %int_done9, align 1
  %valid54 = load i1, ptr %valid6, align 1
  br i1 %valid54, label %short_circuit.rhs55, label %short_circuit.merge56

if.else:                                          ; preds = %if.then
  %data93 = load i32, ptr %data1, align 4
  %pos94 = load i64, ptr %pos2, align 4
  %call95 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f666c6f61745f62797465(i32 %data93, i64 %pos94)
  store i8 %call95, ptr %byte, align 1
  %byte96 = load i8, ptr %byte, align 1
  %call97 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f64696769743130(i8 %byte96)
  store i1 %call97, ptr %digit, align 1
  %byte98 = load i8, ptr %byte, align 1
  %call99 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f756e64657273636f7265(i8 %byte98)
  store i1 %call99, ptr %underscore, align 1
  %digit100 = load i1, ptr %digit, align 1
  br i1 %digit100, label %if.then101, label %if.else102

if.merge22:                                       ; preds = %if.merge103, %if.merge61
  br label %if.merge

short_circuit.rhs55:                              ; preds = %if.then21
  %int_done57 = load i1, ptr %int_done9, align 1
  %not58 = xor i1 %int_done57, true
  br label %short_circuit.merge56

short_circuit.merge56:                            ; preds = %short_circuit.rhs55, %if.then21
  %short_circuit59 = phi i1 [ false, %if.then21 ], [ %not58, %short_circuit.rhs55 ]
  br i1 %short_circuit59, label %if.then60, label %if.merge61

if.then60:                                        ; preds = %short_circuit.merge56
  %data62 = load i32, ptr %data1, align 4
  %pos63 = load i64, ptr %pos2, align 4
  %end64 = load i64, ptr %end3, align 4
  %mant65 = load double, ptr %mant4, align 8
  %extra_scale66 = load i32, ptr %extra_scale5, align 4
  %valid67 = load i1, ptr %valid6, align 1
  %saw_int68 = load i1, ptr %saw_int7, align 1
  %last_underscore69 = load i1, ptr %last_underscore8, align 1
  %int_done70 = load i1, ptr %int_done9, align 1
  %call71 = call { i64, double, i32, i1, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f666c6f61745f696e7465676572(i32 %data62, i64 %pos63, i64 %end64, double %mant65, i32 %extra_scale66, i1 %valid67, i1 %saw_int68, i1 %last_underscore69, i1 %int_done70)
  %output72 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call71, 0
  store i64 %output72, ptr %assignment_value73, align 4
  %assignment_value74 = load i64, ptr %assignment_value73, align 4
  %output75 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call71, 1
  store double %output75, ptr %assignment_value76, align 8
  %assignment_value77 = load double, ptr %assignment_value76, align 8
  %output78 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call71, 2
  store i32 %output78, ptr %assignment_value79, align 4
  %assignment_value80 = load i32, ptr %assignment_value79, align 4
  %output81 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call71, 3
  store i1 %output81, ptr %assignment_value82, align 1
  %assignment_value83 = load i1, ptr %assignment_value82, align 1
  %output84 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call71, 4
  store i1 %output84, ptr %assignment_value85, align 1
  %assignment_value86 = load i1, ptr %assignment_value85, align 1
  %output87 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call71, 5
  store i1 %output87, ptr %assignment_value88, align 1
  %assignment_value89 = load i1, ptr %assignment_value88, align 1
  %output90 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call71, 6
  store i1 %output90, ptr %assignment_value91, align 1
  %assignment_value92 = load i1, ptr %assignment_value91, align 1
  store i64 %assignment_value74, ptr %pos2, align 4
  store double %assignment_value77, ptr %mant4, align 8
  store i32 %assignment_value80, ptr %extra_scale5, align 4
  store i1 %assignment_value83, ptr %valid6, align 1
  store i1 %assignment_value86, ptr %saw_int7, align 1
  store i1 %assignment_value89, ptr %last_underscore8, align 1
  store i1 %assignment_value92, ptr %int_done9, align 1
  br label %if.merge61

if.merge61:                                       ; preds = %if.then60, %short_circuit.merge56
  br label %if.merge22

if.then101:                                       ; preds = %if.else
  %byte104 = load i8, ptr %byte, align 1
  %zero_digit105 = load i8, ptr %zero_digit, align 1
  %sub106 = sub i8 %byte104, %zero_digit105
  store i8 %sub106, ptr %small_int, align 1
  %small_int107 = load i8, ptr %small_int, align 1
  %int_extend = zext i8 %small_int107 to i64
  store i64 %int_extend, ptr %wide_digit, align 4
  %wide_digit108 = load i64, ptr %wide_digit, align 4
  %uint_to_float = uitofp i64 %wide_digit108 to double
  store double %uint_to_float, ptr %digit_value, align 8
  %mant109 = load double, ptr %mant4, align 8
  %ten_value110 = load double, ptr %ten_value, align 8
  %mul = fmul double %mant109, %ten_value110
  %digit_value111 = load double, ptr %digit_value, align 8
  %add112 = fadd double %mul, %digit_value111
  store double %add112, ptr %assignment_value113, align 8
  %assignment_value114 = load double, ptr %assignment_value113, align 8
  store double %assignment_value114, ptr %mant4, align 8
  %mant115 = load double, ptr %mant4, align 8
  %big_limit116 = load double, ptr %big_limit, align 8
  %gt117 = fcmp ogt double %mant115, %big_limit116
  br i1 %gt117, label %if.then118, label %if.merge119

if.else102:                                       ; preds = %if.else
  %underscore139 = load i1, ptr %underscore, align 1
  br i1 %underscore139, label %if.then140, label %if.else141

if.merge103:                                      ; preds = %if.merge142, %if.merge119
  br label %if.merge22

if.then118:                                       ; preds = %if.then101
  %mant120 = load double, ptr %mant4, align 8
  %ten_value121 = load double, ptr %ten_value, align 8
  %div122 = fdiv double %mant120, %ten_value121
  store double %div122, ptr %assignment_value123, align 8
  %assignment_value124 = load double, ptr %assignment_value123, align 8
  store double %assignment_value124, ptr %mant4, align 8
  %extra_scale125 = load i32, ptr %extra_scale5, align 4
  %one_count126 = load i32, ptr %one_count, align 4
  %add127 = add i32 %extra_scale125, %one_count126
  store i32 %add127, ptr %assignment_value128, align 4
  %assignment_value129 = load i32, ptr %assignment_value128, align 4
  store i32 %assignment_value129, ptr %extra_scale5, align 4
  br label %if.merge119

if.merge119:                                      ; preds = %if.then118, %if.then101
  store i1 false, ptr %assignment_value130, align 1
  %assignment_value131 = load i1, ptr %assignment_value130, align 1
  store i1 %assignment_value131, ptr %last_underscore8, align 1
  store i1 true, ptr %assignment_value132, align 1
  %assignment_value133 = load i1, ptr %assignment_value132, align 1
  store i1 %assignment_value133, ptr %saw_int7, align 1
  %pos134 = load i64, ptr %pos2, align 4
  %one135 = load i64, ptr %one, align 4
  %add136 = add i64 %pos134, %one135
  store i64 %add136, ptr %assignment_value137, align 4
  %assignment_value138 = load i64, ptr %assignment_value137, align 4
  store i64 %assignment_value138, ptr %pos2, align 4
  br label %if.merge103

if.then140:                                       ; preds = %if.else102
  %saw_int143 = load i1, ptr %saw_int7, align 1
  %not144 = xor i1 %saw_int143, true
  br i1 %not144, label %if.then145, label %if.merge146

if.else141:                                       ; preds = %if.else102
  store i1 true, ptr %assignment_value161, align 1
  %assignment_value162 = load i1, ptr %assignment_value161, align 1
  store i1 %assignment_value162, ptr %int_done9, align 1
  br label %if.merge142

if.merge142:                                      ; preds = %if.else141, %if.merge151
  br label %if.merge103

if.then145:                                       ; preds = %if.then140
  store i1 false, ptr %assignment_value147, align 1
  %assignment_value148 = load i1, ptr %assignment_value147, align 1
  store i1 %assignment_value148, ptr %valid6, align 1
  br label %if.merge146

if.merge146:                                      ; preds = %if.then145, %if.then140
  %last_underscore149 = load i1, ptr %last_underscore8, align 1
  br i1 %last_underscore149, label %if.then150, label %if.merge151

if.then150:                                       ; preds = %if.merge146
  store i1 false, ptr %assignment_value152, align 1
  %assignment_value153 = load i1, ptr %assignment_value152, align 1
  store i1 %assignment_value153, ptr %valid6, align 1
  br label %if.merge151

if.merge151:                                      ; preds = %if.then150, %if.merge146
  store i1 true, ptr %assignment_value154, align 1
  %assignment_value155 = load i1, ptr %assignment_value154, align 1
  store i1 %assignment_value155, ptr %last_underscore8, align 1
  %pos156 = load i64, ptr %pos2, align 4
  %one157 = load i64, ptr %one, align 4
  %add158 = add i64 %pos156, %one157
  store i64 %add158, ptr %assignment_value159, align 4
  %assignment_value160 = load i64, ptr %assignment_value159, align 4
  store i64 %assignment_value160, ptr %pos2, align 4
  br label %if.merge142
}

define { i64, double, i32, i32, i1, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f666c6f61745f6672616374696f6e(i32 %data, i64 %pos, i64 %end, double %mant, i32 %extra_scale, i32 %frac_count, i1 %valid, i1 %frac_started, i1 %frac_trail, i1 %frac_done) {
entry:
  %assignment_value180 = alloca i1, align 1
  %assignment_value178 = alloca i64, align 8
  %assignment_value173 = alloca i1, align 1
  %assignment_value171 = alloca i1, align 1
  %assignment_value166 = alloca i1, align 1
  %assignment_value156 = alloca i64, align 8
  %assignment_value151 = alloca i1, align 1
  %assignment_value149 = alloca i1, align 1
  %assignment_value147 = alloca i32, align 4
  %assignment_value137 = alloca i32, align 4
  %assignment_value132 = alloca double, align 8
  %assignment_value122 = alloca double, align 8
  %digit_value = alloca double, align 8
  %wide_digit = alloca i64, align 8
  %small_frac = alloca i8, align 1
  %underscore = alloca i1, align 1
  %digit = alloca i1, align 1
  %byte = alloca i8, align 1
  %assignment_value100 = alloca i1, align 1
  %assignment_value97 = alloca i1, align 1
  %assignment_value94 = alloca i1, align 1
  %assignment_value91 = alloca i1, align 1
  %assignment_value88 = alloca i32, align 4
  %assignment_value85 = alloca i32, align 4
  %assignment_value82 = alloca double, align 8
  %assignment_value79 = alloca i64, align 8
  %assignment_value57 = alloca i1, align 1
  %assignment_value54 = alloca i1, align 1
  %assignment_value51 = alloca i1, align 1
  %assignment_value48 = alloca i1, align 1
  %assignment_value45 = alloca i32, align 4
  %assignment_value42 = alloca i32, align 4
  %assignment_value39 = alloca double, align 8
  %assignment_value = alloca i64, align 8
  %mid = alloca i64, align 8
  %length = alloca i64, align 8
  %cap = alloca i32, align 4
  %one_count = alloca i32, align 4
  %zero_digit = alloca i8, align 1
  %big_limit = alloca double, align 8
  %ten_value = alloca double, align 8
  %two = alloca i64, align 8
  %one = alloca i64, align 8
  %data1 = alloca i32, align 4
  store i32 %data, ptr %data1, align 4
  %pos2 = alloca i64, align 8
  store i64 %pos, ptr %pos2, align 4
  %end3 = alloca i64, align 8
  store i64 %end, ptr %end3, align 4
  %mant4 = alloca double, align 8
  store double %mant, ptr %mant4, align 8
  %extra_scale5 = alloca i32, align 4
  store i32 %extra_scale, ptr %extra_scale5, align 4
  %frac_count6 = alloca i32, align 4
  store i32 %frac_count, ptr %frac_count6, align 4
  %valid7 = alloca i1, align 1
  store i1 %valid, ptr %valid7, align 1
  %frac_started8 = alloca i1, align 1
  store i1 %frac_started, ptr %frac_started8, align 1
  %frac_trail9 = alloca i1, align 1
  store i1 %frac_trail, ptr %frac_trail9, align 1
  %frac_done10 = alloca i1, align 1
  store i1 %frac_done, ptr %frac_done10, align 1
  store i64 1, ptr %one, align 4
  store i64 2, ptr %two, align 4
  store double 1.000000e+01, ptr %ten_value, align 8
  store double 1.000000e+300, ptr %big_limit, align 8
  store i8 48, ptr %zero_digit, align 1
  store i32 1, ptr %one_count, align 4
  store i32 1000000, ptr %cap, align 4
  %pos11 = load i64, ptr %pos2, align 4
  %end12 = load i64, ptr %end3, align 4
  %lt = icmp ult i64 %pos11, %end12
  br i1 %lt, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %valid13 = load i1, ptr %valid7, align 1
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %valid13, %short_circuit.rhs ]
  br i1 %short_circuit, label %short_circuit.rhs14, label %short_circuit.merge15

short_circuit.rhs14:                              ; preds = %short_circuit.merge
  %frac_done16 = load i1, ptr %frac_done10, align 1
  %not = xor i1 %frac_done16, true
  br label %short_circuit.merge15

short_circuit.merge15:                            ; preds = %short_circuit.rhs14, %short_circuit.merge
  %short_circuit17 = phi i1 [ false, %short_circuit.merge ], [ %not, %short_circuit.rhs14 ]
  br i1 %short_circuit17, label %if.then, label %if.merge

if.then:                                          ; preds = %short_circuit.merge15
  %end18 = load i64, ptr %end3, align 4
  %pos19 = load i64, ptr %pos2, align 4
  %sub = sub i64 %end18, %pos19
  store i64 %sub, ptr %length, align 4
  %length20 = load i64, ptr %length, align 4
  %one21 = load i64, ptr %one, align 4
  %gt = icmp ugt i64 %length20, %one21
  br i1 %gt, label %if.then22, label %if.else

if.merge:                                         ; preds = %if.merge23, %short_circuit.merge15
  %pos182 = load i64, ptr %pos2, align 4
  %mant183 = load double, ptr %mant4, align 8
  %extra_scale184 = load i32, ptr %extra_scale5, align 4
  %frac_count185 = load i32, ptr %frac_count6, align 4
  %valid186 = load i1, ptr %valid7, align 1
  %frac_started187 = load i1, ptr %frac_started8, align 1
  %frac_trail188 = load i1, ptr %frac_trail9, align 1
  %frac_done189 = load i1, ptr %frac_done10, align 1
  %output190 = insertvalue { i64, double, i32, i32, i1, i1, i1, i1 } zeroinitializer, i64 %pos182, 0
  %output191 = insertvalue { i64, double, i32, i32, i1, i1, i1, i1 } %output190, double %mant183, 1
  %output192 = insertvalue { i64, double, i32, i32, i1, i1, i1, i1 } %output191, i32 %extra_scale184, 2
  %output193 = insertvalue { i64, double, i32, i32, i1, i1, i1, i1 } %output192, i32 %frac_count185, 3
  %output194 = insertvalue { i64, double, i32, i32, i1, i1, i1, i1 } %output193, i1 %valid186, 4
  %output195 = insertvalue { i64, double, i32, i32, i1, i1, i1, i1 } %output194, i1 %frac_started187, 5
  %output196 = insertvalue { i64, double, i32, i32, i1, i1, i1, i1 } %output195, i1 %frac_trail188, 6
  %output197 = insertvalue { i64, double, i32, i32, i1, i1, i1, i1 } %output196, i1 %frac_done189, 7
  ret { i64, double, i32, i32, i1, i1, i1, i1 } %output197

if.then22:                                        ; preds = %if.then
  %pos24 = load i64, ptr %pos2, align 4
  %length25 = load i64, ptr %length, align 4
  %two26 = load i64, ptr %two, align 4
  %div = udiv i64 %length25, %two26
  %add = add i64 %pos24, %div
  store i64 %add, ptr %mid, align 4
  %data27 = load i32, ptr %data1, align 4
  %pos28 = load i64, ptr %pos2, align 4
  %mid29 = load i64, ptr %mid, align 4
  %mant30 = load double, ptr %mant4, align 8
  %extra_scale31 = load i32, ptr %extra_scale5, align 4
  %frac_count32 = load i32, ptr %frac_count6, align 4
  %valid33 = load i1, ptr %valid7, align 1
  %frac_started34 = load i1, ptr %frac_started8, align 1
  %frac_trail35 = load i1, ptr %frac_trail9, align 1
  %frac_done36 = load i1, ptr %frac_done10, align 1
  %call = call { i64, double, i32, i32, i1, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f666c6f61745f6672616374696f6e(i32 %data27, i64 %pos28, i64 %mid29, double %mant30, i32 %extra_scale31, i32 %frac_count32, i1 %valid33, i1 %frac_started34, i1 %frac_trail35, i1 %frac_done36)
  %output = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call, 0
  store i64 %output, ptr %assignment_value, align 4
  %assignment_value37 = load i64, ptr %assignment_value, align 4
  %output38 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call, 1
  store double %output38, ptr %assignment_value39, align 8
  %assignment_value40 = load double, ptr %assignment_value39, align 8
  %output41 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call, 2
  store i32 %output41, ptr %assignment_value42, align 4
  %assignment_value43 = load i32, ptr %assignment_value42, align 4
  %output44 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call, 3
  store i32 %output44, ptr %assignment_value45, align 4
  %assignment_value46 = load i32, ptr %assignment_value45, align 4
  %output47 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call, 4
  store i1 %output47, ptr %assignment_value48, align 1
  %assignment_value49 = load i1, ptr %assignment_value48, align 1
  %output50 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call, 5
  store i1 %output50, ptr %assignment_value51, align 1
  %assignment_value52 = load i1, ptr %assignment_value51, align 1
  %output53 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call, 6
  store i1 %output53, ptr %assignment_value54, align 1
  %assignment_value55 = load i1, ptr %assignment_value54, align 1
  %output56 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call, 7
  store i1 %output56, ptr %assignment_value57, align 1
  %assignment_value58 = load i1, ptr %assignment_value57, align 1
  store i64 %assignment_value37, ptr %pos2, align 4
  store double %assignment_value40, ptr %mant4, align 8
  store i32 %assignment_value43, ptr %extra_scale5, align 4
  store i32 %assignment_value46, ptr %frac_count6, align 4
  store i1 %assignment_value49, ptr %valid7, align 1
  store i1 %assignment_value52, ptr %frac_started8, align 1
  store i1 %assignment_value55, ptr %frac_trail9, align 1
  store i1 %assignment_value58, ptr %frac_done10, align 1
  %valid59 = load i1, ptr %valid7, align 1
  br i1 %valid59, label %short_circuit.rhs60, label %short_circuit.merge61

if.else:                                          ; preds = %if.then
  %data102 = load i32, ptr %data1, align 4
  %pos103 = load i64, ptr %pos2, align 4
  %call104 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f666c6f61745f62797465(i32 %data102, i64 %pos103)
  store i8 %call104, ptr %byte, align 1
  %byte105 = load i8, ptr %byte, align 1
  %call106 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f64696769743130(i8 %byte105)
  store i1 %call106, ptr %digit, align 1
  %byte107 = load i8, ptr %byte, align 1
  %call108 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f756e64657273636f7265(i8 %byte107)
  store i1 %call108, ptr %underscore, align 1
  %digit109 = load i1, ptr %digit, align 1
  br i1 %digit109, label %if.then110, label %if.else111

if.merge23:                                       ; preds = %if.merge112, %if.merge66
  br label %if.merge

short_circuit.rhs60:                              ; preds = %if.then22
  %frac_done62 = load i1, ptr %frac_done10, align 1
  %not63 = xor i1 %frac_done62, true
  br label %short_circuit.merge61

short_circuit.merge61:                            ; preds = %short_circuit.rhs60, %if.then22
  %short_circuit64 = phi i1 [ false, %if.then22 ], [ %not63, %short_circuit.rhs60 ]
  br i1 %short_circuit64, label %if.then65, label %if.merge66

if.then65:                                        ; preds = %short_circuit.merge61
  %data67 = load i32, ptr %data1, align 4
  %pos68 = load i64, ptr %pos2, align 4
  %end69 = load i64, ptr %end3, align 4
  %mant70 = load double, ptr %mant4, align 8
  %extra_scale71 = load i32, ptr %extra_scale5, align 4
  %frac_count72 = load i32, ptr %frac_count6, align 4
  %valid73 = load i1, ptr %valid7, align 1
  %frac_started74 = load i1, ptr %frac_started8, align 1
  %frac_trail75 = load i1, ptr %frac_trail9, align 1
  %frac_done76 = load i1, ptr %frac_done10, align 1
  %call77 = call { i64, double, i32, i32, i1, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f666c6f61745f6672616374696f6e(i32 %data67, i64 %pos68, i64 %end69, double %mant70, i32 %extra_scale71, i32 %frac_count72, i1 %valid73, i1 %frac_started74, i1 %frac_trail75, i1 %frac_done76)
  %output78 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call77, 0
  store i64 %output78, ptr %assignment_value79, align 4
  %assignment_value80 = load i64, ptr %assignment_value79, align 4
  %output81 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call77, 1
  store double %output81, ptr %assignment_value82, align 8
  %assignment_value83 = load double, ptr %assignment_value82, align 8
  %output84 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call77, 2
  store i32 %output84, ptr %assignment_value85, align 4
  %assignment_value86 = load i32, ptr %assignment_value85, align 4
  %output87 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call77, 3
  store i32 %output87, ptr %assignment_value88, align 4
  %assignment_value89 = load i32, ptr %assignment_value88, align 4
  %output90 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call77, 4
  store i1 %output90, ptr %assignment_value91, align 1
  %assignment_value92 = load i1, ptr %assignment_value91, align 1
  %output93 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call77, 5
  store i1 %output93, ptr %assignment_value94, align 1
  %assignment_value95 = load i1, ptr %assignment_value94, align 1
  %output96 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call77, 6
  store i1 %output96, ptr %assignment_value97, align 1
  %assignment_value98 = load i1, ptr %assignment_value97, align 1
  %output99 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call77, 7
  store i1 %output99, ptr %assignment_value100, align 1
  %assignment_value101 = load i1, ptr %assignment_value100, align 1
  store i64 %assignment_value80, ptr %pos2, align 4
  store double %assignment_value83, ptr %mant4, align 8
  store i32 %assignment_value86, ptr %extra_scale5, align 4
  store i32 %assignment_value89, ptr %frac_count6, align 4
  store i1 %assignment_value92, ptr %valid7, align 1
  store i1 %assignment_value95, ptr %frac_started8, align 1
  store i1 %assignment_value98, ptr %frac_trail9, align 1
  store i1 %assignment_value101, ptr %frac_done10, align 1
  br label %if.merge66

if.merge66:                                       ; preds = %if.then65, %short_circuit.merge61
  br label %if.merge23

if.then110:                                       ; preds = %if.else
  %byte113 = load i8, ptr %byte, align 1
  %zero_digit114 = load i8, ptr %zero_digit, align 1
  %sub115 = sub i8 %byte113, %zero_digit114
  store i8 %sub115, ptr %small_frac, align 1
  %small_frac116 = load i8, ptr %small_frac, align 1
  %int_extend = zext i8 %small_frac116 to i64
  store i64 %int_extend, ptr %wide_digit, align 4
  %wide_digit117 = load i64, ptr %wide_digit, align 4
  %uint_to_float = uitofp i64 %wide_digit117 to double
  store double %uint_to_float, ptr %digit_value, align 8
  %mant118 = load double, ptr %mant4, align 8
  %ten_value119 = load double, ptr %ten_value, align 8
  %mul = fmul double %mant118, %ten_value119
  %digit_value120 = load double, ptr %digit_value, align 8
  %add121 = fadd double %mul, %digit_value120
  store double %add121, ptr %assignment_value122, align 8
  %assignment_value123 = load double, ptr %assignment_value122, align 8
  store double %assignment_value123, ptr %mant4, align 8
  %mant124 = load double, ptr %mant4, align 8
  %big_limit125 = load double, ptr %big_limit, align 8
  %gt126 = fcmp ogt double %mant124, %big_limit125
  br i1 %gt126, label %if.then127, label %if.merge128

if.else111:                                       ; preds = %if.else
  %underscore158 = load i1, ptr %underscore, align 1
  br i1 %underscore158, label %if.then159, label %if.else160

if.merge112:                                      ; preds = %if.merge161, %if.merge143
  br label %if.merge23

if.then127:                                       ; preds = %if.then110
  %mant129 = load double, ptr %mant4, align 8
  %ten_value130 = load double, ptr %ten_value, align 8
  %div131 = fdiv double %mant129, %ten_value130
  store double %div131, ptr %assignment_value132, align 8
  %assignment_value133 = load double, ptr %assignment_value132, align 8
  store double %assignment_value133, ptr %mant4, align 8
  %extra_scale134 = load i32, ptr %extra_scale5, align 4
  %one_count135 = load i32, ptr %one_count, align 4
  %add136 = add i32 %extra_scale134, %one_count135
  store i32 %add136, ptr %assignment_value137, align 4
  %assignment_value138 = load i32, ptr %assignment_value137, align 4
  store i32 %assignment_value138, ptr %extra_scale5, align 4
  br label %if.merge128

if.merge128:                                      ; preds = %if.then127, %if.then110
  %frac_count139 = load i32, ptr %frac_count6, align 4
  %cap140 = load i32, ptr %cap, align 4
  %lt141 = icmp slt i32 %frac_count139, %cap140
  br i1 %lt141, label %if.then142, label %if.merge143

if.then142:                                       ; preds = %if.merge128
  %frac_count144 = load i32, ptr %frac_count6, align 4
  %one_count145 = load i32, ptr %one_count, align 4
  %add146 = add i32 %frac_count144, %one_count145
  store i32 %add146, ptr %assignment_value147, align 4
  %assignment_value148 = load i32, ptr %assignment_value147, align 4
  store i32 %assignment_value148, ptr %frac_count6, align 4
  br label %if.merge143

if.merge143:                                      ; preds = %if.then142, %if.merge128
  store i1 true, ptr %assignment_value149, align 1
  %assignment_value150 = load i1, ptr %assignment_value149, align 1
  store i1 %assignment_value150, ptr %frac_started8, align 1
  store i1 false, ptr %assignment_value151, align 1
  %assignment_value152 = load i1, ptr %assignment_value151, align 1
  store i1 %assignment_value152, ptr %frac_trail9, align 1
  %pos153 = load i64, ptr %pos2, align 4
  %one154 = load i64, ptr %one, align 4
  %add155 = add i64 %pos153, %one154
  store i64 %add155, ptr %assignment_value156, align 4
  %assignment_value157 = load i64, ptr %assignment_value156, align 4
  store i64 %assignment_value157, ptr %pos2, align 4
  br label %if.merge112

if.then159:                                       ; preds = %if.else111
  %frac_started162 = load i1, ptr %frac_started8, align 1
  %not163 = xor i1 %frac_started162, true
  br i1 %not163, label %if.then164, label %if.merge165

if.else160:                                       ; preds = %if.else111
  store i1 true, ptr %assignment_value180, align 1
  %assignment_value181 = load i1, ptr %assignment_value180, align 1
  store i1 %assignment_value181, ptr %frac_done10, align 1
  br label %if.merge161

if.merge161:                                      ; preds = %if.else160, %if.merge170
  br label %if.merge112

if.then164:                                       ; preds = %if.then159
  store i1 false, ptr %assignment_value166, align 1
  %assignment_value167 = load i1, ptr %assignment_value166, align 1
  store i1 %assignment_value167, ptr %valid7, align 1
  br label %if.merge165

if.merge165:                                      ; preds = %if.then164, %if.then159
  %frac_trail168 = load i1, ptr %frac_trail9, align 1
  br i1 %frac_trail168, label %if.then169, label %if.merge170

if.then169:                                       ; preds = %if.merge165
  store i1 false, ptr %assignment_value171, align 1
  %assignment_value172 = load i1, ptr %assignment_value171, align 1
  store i1 %assignment_value172, ptr %valid7, align 1
  br label %if.merge170

if.merge170:                                      ; preds = %if.then169, %if.merge165
  store i1 true, ptr %assignment_value173, align 1
  %assignment_value174 = load i1, ptr %assignment_value173, align 1
  store i1 %assignment_value174, ptr %frac_trail9, align 1
  %pos175 = load i64, ptr %pos2, align 4
  %one176 = load i64, ptr %one, align 4
  %add177 = add i64 %pos175, %one176
  store i64 %add177, ptr %assignment_value178, align 4
  %assignment_value179 = load i64, ptr %assignment_value178, align 4
  store i64 %assignment_value179, ptr %pos2, align 4
  br label %if.merge161
}

define { i64, i32, i1, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f666c6f61745f6578706f6e656e74(i32 %data, i64 %pos, i64 %end, i32 %exp_value, i1 %valid, i1 %exp_started, i1 %exp_trail, i1 %exp_done) {
entry:
  %assignment_value141 = alloca i1, align 1
  %assignment_value139 = alloca i64, align 8
  %assignment_value134 = alloca i1, align 1
  %assignment_value132 = alloca i1, align 1
  %assignment_value127 = alloca i1, align 1
  %assignment_value117 = alloca i64, align 8
  %assignment_value112 = alloca i1, align 1
  %assignment_value110 = alloca i1, align 1
  %assignment_value108 = alloca i32, align 4
  %piece = alloca i32, align 4
  %small = alloca i8, align 1
  %underscore = alloca i1, align 1
  %digit = alloca i1, align 1
  %byte = alloca i8, align 1
  %assignment_value82 = alloca i1, align 1
  %assignment_value79 = alloca i1, align 1
  %assignment_value76 = alloca i1, align 1
  %assignment_value73 = alloca i1, align 1
  %assignment_value70 = alloca i32, align 4
  %assignment_value67 = alloca i64, align 8
  %assignment_value47 = alloca i1, align 1
  %assignment_value44 = alloca i1, align 1
  %assignment_value41 = alloca i1, align 1
  %assignment_value38 = alloca i1, align 1
  %assignment_value35 = alloca i32, align 4
  %assignment_value = alloca i64, align 8
  %mid = alloca i64, align 8
  %length = alloca i64, align 8
  %cap = alloca i32, align 4
  %ten_count = alloca i32, align 4
  %zero_digit = alloca i8, align 1
  %two = alloca i64, align 8
  %one = alloca i64, align 8
  %data1 = alloca i32, align 4
  store i32 %data, ptr %data1, align 4
  %pos2 = alloca i64, align 8
  store i64 %pos, ptr %pos2, align 4
  %end3 = alloca i64, align 8
  store i64 %end, ptr %end3, align 4
  %exp_value4 = alloca i32, align 4
  store i32 %exp_value, ptr %exp_value4, align 4
  %valid5 = alloca i1, align 1
  store i1 %valid, ptr %valid5, align 1
  %exp_started6 = alloca i1, align 1
  store i1 %exp_started, ptr %exp_started6, align 1
  %exp_trail7 = alloca i1, align 1
  store i1 %exp_trail, ptr %exp_trail7, align 1
  %exp_done8 = alloca i1, align 1
  store i1 %exp_done, ptr %exp_done8, align 1
  store i64 1, ptr %one, align 4
  store i64 2, ptr %two, align 4
  store i8 48, ptr %zero_digit, align 1
  store i32 10, ptr %ten_count, align 4
  store i32 1000000, ptr %cap, align 4
  %pos9 = load i64, ptr %pos2, align 4
  %end10 = load i64, ptr %end3, align 4
  %lt = icmp ult i64 %pos9, %end10
  br i1 %lt, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %entry
  %valid11 = load i1, ptr %valid5, align 1
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ false, %entry ], [ %valid11, %short_circuit.rhs ]
  br i1 %short_circuit, label %short_circuit.rhs12, label %short_circuit.merge13

short_circuit.rhs12:                              ; preds = %short_circuit.merge
  %exp_done14 = load i1, ptr %exp_done8, align 1
  %not = xor i1 %exp_done14, true
  br label %short_circuit.merge13

short_circuit.merge13:                            ; preds = %short_circuit.rhs12, %short_circuit.merge
  %short_circuit15 = phi i1 [ false, %short_circuit.merge ], [ %not, %short_circuit.rhs12 ]
  br i1 %short_circuit15, label %if.then, label %if.merge

if.then:                                          ; preds = %short_circuit.merge13
  %end16 = load i64, ptr %end3, align 4
  %pos17 = load i64, ptr %pos2, align 4
  %sub = sub i64 %end16, %pos17
  store i64 %sub, ptr %length, align 4
  %length18 = load i64, ptr %length, align 4
  %one19 = load i64, ptr %one, align 4
  %gt = icmp ugt i64 %length18, %one19
  br i1 %gt, label %if.then20, label %if.else

if.merge:                                         ; preds = %if.merge21, %short_circuit.merge13
  %pos143 = load i64, ptr %pos2, align 4
  %exp_value144 = load i32, ptr %exp_value4, align 4
  %valid145 = load i1, ptr %valid5, align 1
  %exp_started146 = load i1, ptr %exp_started6, align 1
  %exp_trail147 = load i1, ptr %exp_trail7, align 1
  %exp_done148 = load i1, ptr %exp_done8, align 1
  %output149 = insertvalue { i64, i32, i1, i1, i1, i1 } zeroinitializer, i64 %pos143, 0
  %output150 = insertvalue { i64, i32, i1, i1, i1, i1 } %output149, i32 %exp_value144, 1
  %output151 = insertvalue { i64, i32, i1, i1, i1, i1 } %output150, i1 %valid145, 2
  %output152 = insertvalue { i64, i32, i1, i1, i1, i1 } %output151, i1 %exp_started146, 3
  %output153 = insertvalue { i64, i32, i1, i1, i1, i1 } %output152, i1 %exp_trail147, 4
  %output154 = insertvalue { i64, i32, i1, i1, i1, i1 } %output153, i1 %exp_done148, 5
  ret { i64, i32, i1, i1, i1, i1 } %output154

if.then20:                                        ; preds = %if.then
  %pos22 = load i64, ptr %pos2, align 4
  %length23 = load i64, ptr %length, align 4
  %two24 = load i64, ptr %two, align 4
  %div = udiv i64 %length23, %two24
  %add = add i64 %pos22, %div
  store i64 %add, ptr %mid, align 4
  %data25 = load i32, ptr %data1, align 4
  %pos26 = load i64, ptr %pos2, align 4
  %mid27 = load i64, ptr %mid, align 4
  %exp_value28 = load i32, ptr %exp_value4, align 4
  %valid29 = load i1, ptr %valid5, align 1
  %exp_started30 = load i1, ptr %exp_started6, align 1
  %exp_trail31 = load i1, ptr %exp_trail7, align 1
  %exp_done32 = load i1, ptr %exp_done8, align 1
  %call = call { i64, i32, i1, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f666c6f61745f6578706f6e656e74(i32 %data25, i64 %pos26, i64 %mid27, i32 %exp_value28, i1 %valid29, i1 %exp_started30, i1 %exp_trail31, i1 %exp_done32)
  %output = extractvalue { i64, i32, i1, i1, i1, i1 } %call, 0
  store i64 %output, ptr %assignment_value, align 4
  %assignment_value33 = load i64, ptr %assignment_value, align 4
  %output34 = extractvalue { i64, i32, i1, i1, i1, i1 } %call, 1
  store i32 %output34, ptr %assignment_value35, align 4
  %assignment_value36 = load i32, ptr %assignment_value35, align 4
  %output37 = extractvalue { i64, i32, i1, i1, i1, i1 } %call, 2
  store i1 %output37, ptr %assignment_value38, align 1
  %assignment_value39 = load i1, ptr %assignment_value38, align 1
  %output40 = extractvalue { i64, i32, i1, i1, i1, i1 } %call, 3
  store i1 %output40, ptr %assignment_value41, align 1
  %assignment_value42 = load i1, ptr %assignment_value41, align 1
  %output43 = extractvalue { i64, i32, i1, i1, i1, i1 } %call, 4
  store i1 %output43, ptr %assignment_value44, align 1
  %assignment_value45 = load i1, ptr %assignment_value44, align 1
  %output46 = extractvalue { i64, i32, i1, i1, i1, i1 } %call, 5
  store i1 %output46, ptr %assignment_value47, align 1
  %assignment_value48 = load i1, ptr %assignment_value47, align 1
  store i64 %assignment_value33, ptr %pos2, align 4
  store i32 %assignment_value36, ptr %exp_value4, align 4
  store i1 %assignment_value39, ptr %valid5, align 1
  store i1 %assignment_value42, ptr %exp_started6, align 1
  store i1 %assignment_value45, ptr %exp_trail7, align 1
  store i1 %assignment_value48, ptr %exp_done8, align 1
  %valid49 = load i1, ptr %valid5, align 1
  br i1 %valid49, label %short_circuit.rhs50, label %short_circuit.merge51

if.else:                                          ; preds = %if.then
  %data84 = load i32, ptr %data1, align 4
  %pos85 = load i64, ptr %pos2, align 4
  %call86 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f666c6f61745f62797465(i32 %data84, i64 %pos85)
  store i8 %call86, ptr %byte, align 1
  %byte87 = load i8, ptr %byte, align 1
  %call88 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f64696769743130(i8 %byte87)
  store i1 %call88, ptr %digit, align 1
  %byte89 = load i8, ptr %byte, align 1
  %call90 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f756e64657273636f7265(i8 %byte89)
  store i1 %call90, ptr %underscore, align 1
  %digit91 = load i1, ptr %digit, align 1
  br i1 %digit91, label %if.then92, label %if.else93

if.merge21:                                       ; preds = %if.merge94, %if.merge56
  br label %if.merge

short_circuit.rhs50:                              ; preds = %if.then20
  %exp_done52 = load i1, ptr %exp_done8, align 1
  %not53 = xor i1 %exp_done52, true
  br label %short_circuit.merge51

short_circuit.merge51:                            ; preds = %short_circuit.rhs50, %if.then20
  %short_circuit54 = phi i1 [ false, %if.then20 ], [ %not53, %short_circuit.rhs50 ]
  br i1 %short_circuit54, label %if.then55, label %if.merge56

if.then55:                                        ; preds = %short_circuit.merge51
  %data57 = load i32, ptr %data1, align 4
  %pos58 = load i64, ptr %pos2, align 4
  %end59 = load i64, ptr %end3, align 4
  %exp_value60 = load i32, ptr %exp_value4, align 4
  %valid61 = load i1, ptr %valid5, align 1
  %exp_started62 = load i1, ptr %exp_started6, align 1
  %exp_trail63 = load i1, ptr %exp_trail7, align 1
  %exp_done64 = load i1, ptr %exp_done8, align 1
  %call65 = call { i64, i32, i1, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f666c6f61745f6578706f6e656e74(i32 %data57, i64 %pos58, i64 %end59, i32 %exp_value60, i1 %valid61, i1 %exp_started62, i1 %exp_trail63, i1 %exp_done64)
  %output66 = extractvalue { i64, i32, i1, i1, i1, i1 } %call65, 0
  store i64 %output66, ptr %assignment_value67, align 4
  %assignment_value68 = load i64, ptr %assignment_value67, align 4
  %output69 = extractvalue { i64, i32, i1, i1, i1, i1 } %call65, 1
  store i32 %output69, ptr %assignment_value70, align 4
  %assignment_value71 = load i32, ptr %assignment_value70, align 4
  %output72 = extractvalue { i64, i32, i1, i1, i1, i1 } %call65, 2
  store i1 %output72, ptr %assignment_value73, align 1
  %assignment_value74 = load i1, ptr %assignment_value73, align 1
  %output75 = extractvalue { i64, i32, i1, i1, i1, i1 } %call65, 3
  store i1 %output75, ptr %assignment_value76, align 1
  %assignment_value77 = load i1, ptr %assignment_value76, align 1
  %output78 = extractvalue { i64, i32, i1, i1, i1, i1 } %call65, 4
  store i1 %output78, ptr %assignment_value79, align 1
  %assignment_value80 = load i1, ptr %assignment_value79, align 1
  %output81 = extractvalue { i64, i32, i1, i1, i1, i1 } %call65, 5
  store i1 %output81, ptr %assignment_value82, align 1
  %assignment_value83 = load i1, ptr %assignment_value82, align 1
  store i64 %assignment_value68, ptr %pos2, align 4
  store i32 %assignment_value71, ptr %exp_value4, align 4
  store i1 %assignment_value74, ptr %valid5, align 1
  store i1 %assignment_value77, ptr %exp_started6, align 1
  store i1 %assignment_value80, ptr %exp_trail7, align 1
  store i1 %assignment_value83, ptr %exp_done8, align 1
  br label %if.merge56

if.merge56:                                       ; preds = %if.then55, %short_circuit.merge51
  br label %if.merge21

if.then92:                                        ; preds = %if.else
  %byte95 = load i8, ptr %byte, align 1
  %zero_digit96 = load i8, ptr %zero_digit, align 1
  %sub97 = sub i8 %byte95, %zero_digit96
  store i8 %sub97, ptr %small, align 1
  %small98 = load i8, ptr %small, align 1
  %int_extend = zext i8 %small98 to i32
  store i32 %int_extend, ptr %piece, align 4
  %exp_value99 = load i32, ptr %exp_value4, align 4
  %cap100 = load i32, ptr %cap, align 4
  %lt101 = icmp slt i32 %exp_value99, %cap100
  br i1 %lt101, label %if.then102, label %if.merge103

if.else93:                                        ; preds = %if.else
  %underscore119 = load i1, ptr %underscore, align 1
  br i1 %underscore119, label %if.then120, label %if.else121

if.merge94:                                       ; preds = %if.merge122, %if.merge103
  br label %if.merge21

if.then102:                                       ; preds = %if.then92
  %exp_value104 = load i32, ptr %exp_value4, align 4
  %ten_count105 = load i32, ptr %ten_count, align 4
  %mul = mul i32 %exp_value104, %ten_count105
  %piece106 = load i32, ptr %piece, align 4
  %add107 = add i32 %mul, %piece106
  store i32 %add107, ptr %assignment_value108, align 4
  %assignment_value109 = load i32, ptr %assignment_value108, align 4
  store i32 %assignment_value109, ptr %exp_value4, align 4
  br label %if.merge103

if.merge103:                                      ; preds = %if.then102, %if.then92
  store i1 true, ptr %assignment_value110, align 1
  %assignment_value111 = load i1, ptr %assignment_value110, align 1
  store i1 %assignment_value111, ptr %exp_started6, align 1
  store i1 false, ptr %assignment_value112, align 1
  %assignment_value113 = load i1, ptr %assignment_value112, align 1
  store i1 %assignment_value113, ptr %exp_trail7, align 1
  %pos114 = load i64, ptr %pos2, align 4
  %one115 = load i64, ptr %one, align 4
  %add116 = add i64 %pos114, %one115
  store i64 %add116, ptr %assignment_value117, align 4
  %assignment_value118 = load i64, ptr %assignment_value117, align 4
  store i64 %assignment_value118, ptr %pos2, align 4
  br label %if.merge94

if.then120:                                       ; preds = %if.else93
  %exp_started123 = load i1, ptr %exp_started6, align 1
  %not124 = xor i1 %exp_started123, true
  br i1 %not124, label %if.then125, label %if.merge126

if.else121:                                       ; preds = %if.else93
  store i1 true, ptr %assignment_value141, align 1
  %assignment_value142 = load i1, ptr %assignment_value141, align 1
  store i1 %assignment_value142, ptr %exp_done8, align 1
  br label %if.merge122

if.merge122:                                      ; preds = %if.else121, %if.merge131
  br label %if.merge94

if.then125:                                       ; preds = %if.then120
  store i1 false, ptr %assignment_value127, align 1
  %assignment_value128 = load i1, ptr %assignment_value127, align 1
  store i1 %assignment_value128, ptr %valid5, align 1
  br label %if.merge126

if.merge126:                                      ; preds = %if.then125, %if.then120
  %exp_trail129 = load i1, ptr %exp_trail7, align 1
  br i1 %exp_trail129, label %if.then130, label %if.merge131

if.then130:                                       ; preds = %if.merge126
  store i1 false, ptr %assignment_value132, align 1
  %assignment_value133 = load i1, ptr %assignment_value132, align 1
  store i1 %assignment_value133, ptr %valid5, align 1
  br label %if.merge131

if.merge131:                                      ; preds = %if.then130, %if.merge126
  store i1 true, ptr %assignment_value134, align 1
  %assignment_value135 = load i1, ptr %assignment_value134, align 1
  store i1 %assignment_value135, ptr %exp_trail7, align 1
  %pos136 = load i64, ptr %pos2, align 4
  %one137 = load i64, ptr %one, align 4
  %add138 = add i64 %pos136, %one137
  store i64 %add138, ptr %assignment_value139, align 4
  %assignment_value140 = load i64, ptr %assignment_value139, align 4
  store i64 %assignment_value140, ptr %pos2, align 4
  br label %if.merge122
}

define double @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f7363616c655f666c6f61745f7570(double %scaled, i32 %steps) {
entry:
  %assignment_value19 = alloca double, align 8
  %assignment_value13 = alloca double, align 8
  %half = alloca i32, align 4
  %assignment_value = alloca double, align 8
  %ten_value = alloca double, align 8
  %scaled1 = alloca double, align 8
  store double %scaled, ptr %scaled1, align 8
  %steps2 = alloca i32, align 4
  store i32 %steps, ptr %steps2, align 4
  %steps3 = load i32, ptr %steps2, align 4
  %gt = icmp sgt i32 %steps3, 0
  br i1 %gt, label %if.then, label %if.merge

if.then:                                          ; preds = %entry
  %steps4 = load i32, ptr %steps2, align 4
  %eq = icmp eq i32 %steps4, 1
  br i1 %eq, label %if.then5, label %if.else

if.merge:                                         ; preds = %if.merge6, %entry
  %scaled21 = load double, ptr %scaled1, align 8
  ret double %scaled21

if.then5:                                         ; preds = %if.then
  store double 1.000000e+01, ptr %ten_value, align 8
  %scaled7 = load double, ptr %scaled1, align 8
  %ten_value8 = load double, ptr %ten_value, align 8
  %mul = fmul double %scaled7, %ten_value8
  store double %mul, ptr %assignment_value, align 8
  %assignment_value9 = load double, ptr %assignment_value, align 8
  store double %assignment_value9, ptr %scaled1, align 8
  br label %if.merge6

if.else:                                          ; preds = %if.then
  %steps10 = load i32, ptr %steps2, align 4
  %div = sdiv i32 %steps10, 2
  store i32 %div, ptr %half, align 4
  %scaled11 = load double, ptr %scaled1, align 8
  %half12 = load i32, ptr %half, align 4
  %call = call double @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f7363616c655f666c6f61745f7570(double %scaled11, i32 %half12)
  store double %call, ptr %assignment_value13, align 8
  %assignment_value14 = load double, ptr %assignment_value13, align 8
  store double %assignment_value14, ptr %scaled1, align 8
  %scaled15 = load double, ptr %scaled1, align 8
  %steps16 = load i32, ptr %steps2, align 4
  %half17 = load i32, ptr %half, align 4
  %sub = sub i32 %steps16, %half17
  %call18 = call double @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f7363616c655f666c6f61745f7570(double %scaled15, i32 %sub)
  store double %call18, ptr %assignment_value19, align 8
  %assignment_value20 = load double, ptr %assignment_value19, align 8
  store double %assignment_value20, ptr %scaled1, align 8
  br label %if.merge6

if.merge6:                                        ; preds = %if.else, %if.then5
  br label %if.merge
}

define double @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f7363616c655f666c6f61745f646f776e(double %scaled, i32 %steps) {
entry:
  %assignment_value20 = alloca double, align 8
  %assignment_value14 = alloca double, align 8
  %half = alloca i32, align 4
  %assignment_value = alloca double, align 8
  %ten_value = alloca double, align 8
  %scaled1 = alloca double, align 8
  store double %scaled, ptr %scaled1, align 8
  %steps2 = alloca i32, align 4
  store i32 %steps, ptr %steps2, align 4
  %steps3 = load i32, ptr %steps2, align 4
  %gt = icmp sgt i32 %steps3, 0
  br i1 %gt, label %if.then, label %if.merge

if.then:                                          ; preds = %entry
  %steps4 = load i32, ptr %steps2, align 4
  %eq = icmp eq i32 %steps4, 1
  br i1 %eq, label %if.then5, label %if.else

if.merge:                                         ; preds = %if.merge6, %entry
  %scaled22 = load double, ptr %scaled1, align 8
  ret double %scaled22

if.then5:                                         ; preds = %if.then
  store double 1.000000e+01, ptr %ten_value, align 8
  %scaled7 = load double, ptr %scaled1, align 8
  %ten_value8 = load double, ptr %ten_value, align 8
  %div = fdiv double %scaled7, %ten_value8
  store double %div, ptr %assignment_value, align 8
  %assignment_value9 = load double, ptr %assignment_value, align 8
  store double %assignment_value9, ptr %scaled1, align 8
  br label %if.merge6

if.else:                                          ; preds = %if.then
  %steps10 = load i32, ptr %steps2, align 4
  %div11 = sdiv i32 %steps10, 2
  store i32 %div11, ptr %half, align 4
  %scaled12 = load double, ptr %scaled1, align 8
  %half13 = load i32, ptr %half, align 4
  %call = call double @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f7363616c655f666c6f61745f646f776e(double %scaled12, i32 %half13)
  store double %call, ptr %assignment_value14, align 8
  %assignment_value15 = load double, ptr %assignment_value14, align 8
  store double %assignment_value15, ptr %scaled1, align 8
  %scaled16 = load double, ptr %scaled1, align 8
  %steps17 = load i32, ptr %steps2, align 4
  %half18 = load i32, ptr %half, align 4
  %sub = sub i32 %steps17, %half18
  %call19 = call double @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f7363616c655f666c6f61745f646f776e(double %scaled16, i32 %sub)
  store double %call19, ptr %assignment_value20, align 8
  %assignment_value21 = load double, ptr %assignment_value20, align 8
  store double %assignment_value21, ptr %scaled1, align 8
  br label %if.merge6

if.merge6:                                        ; preds = %if.else, %if.then5
  br label %if.merge
}

define { double, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f666c6f61745f663634(i32 %data, i64 %start, i64 %end) {
entry:
  %assignment_value298 = alloca i1, align 1
  %assignment_value288 = alloca i1, align 1
  %assignment_value277 = alloca double, align 8
  %assignment_value272 = alloca double, align 8
  %assignment_value261 = alloca i32, align 4
  %steps = alloca i32, align 4
  %assignment_value250 = alloca double, align 8
  %assignment_value241 = alloca i1, align 1
  %adjusted = alloca i32, align 4
  %assignment_value229 = alloca i32, align 4
  %exp_signed = alloca i32, align 4
  %assignment_value220 = alloca double, align 8
  %scaled = alloca double, align 8
  %assignment_value208 = alloca i1, align 1
  %assignment_value203 = alloca i1, align 1
  %assignment_value197 = alloca i1, align 1
  %assignment_value194 = alloca i1, align 1
  %assignment_value191 = alloca i1, align 1
  %assignment_value188 = alloca i1, align 1
  %assignment_value185 = alloca i32, align 4
  %assignment_value182 = alloca i64, align 8
  %exp_done = alloca i1, align 1
  %exp_trail = alloca i1, align 1
  %exp_started = alloca i1, align 1
  %assignment_value170 = alloca i64, align 8
  %assignment_value161 = alloca i64, align 8
  %assignment_value156 = alloca i1, align 1
  %byte151 = alloca i8, align 1
  %assignment_value134 = alloca i64, align 8
  %assignment_value129 = alloca i1, align 1
  %byte124 = alloca i8, align 1
  %has_exp = alloca i1, align 1
  %assignment_value110 = alloca i1, align 1
  %assignment_value105 = alloca i1, align 1
  %assignment_value102 = alloca i1, align 1
  %assignment_value99 = alloca i1, align 1
  %assignment_value96 = alloca i1, align 1
  %assignment_value93 = alloca i32, align 4
  %assignment_value90 = alloca i32, align 4
  %assignment_value87 = alloca double, align 8
  %assignment_value84 = alloca i64, align 8
  %frac_done = alloca i1, align 1
  %frac_trail = alloca i1, align 1
  %frac_started = alloca i1, align 1
  %assignment_value63 = alloca i64, align 8
  %assignment_value59 = alloca i1, align 1
  %byte = alloca i8, align 1
  %has_dot = alloca i1, align 1
  %assignment_value45 = alloca i1, align 1
  %assignment_value40 = alloca i1, align 1
  %exp_negative = alloca i1, align 1
  %exp_value = alloca i32, align 4
  %frac_count = alloca i32, align 4
  %assignment_value35 = alloca i1, align 1
  %assignment_value32 = alloca i1, align 1
  %assignment_value29 = alloca i1, align 1
  %assignment_value26 = alloca i1, align 1
  %assignment_value23 = alloca i32, align 4
  %assignment_value20 = alloca double, align 8
  %assignment_value = alloca i64, align 8
  %int_done = alloca i1, align 1
  %last_underscore = alloca i1, align 1
  %saw_int = alloca i1, align 1
  %valid = alloca i1, align 1
  %extra_scale = alloca i32, align 4
  %mant = alloca double, align 8
  %pos = alloca i64, align 8
  %neg_limit = alloca i32, align 4
  %limit = alloca i32, align 4
  %zero_count = alloca i32, align 4
  %f64_max = alloca double, align 8
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
  store double 0x7FEFFFFFFFFFFFFF, ptr %f64_max, align 8
  store i32 0, ptr %zero_count, align 4
  store i32 1000, ptr %limit, align 4
  %zero_count4 = load i32, ptr %zero_count, align 4
  %limit5 = load i32, ptr %limit, align 4
  %sub = sub i32 %zero_count4, %limit5
  store i32 %sub, ptr %neg_limit, align 4
  %start6 = load i64, ptr %start2, align 4
  store i64 %start6, ptr %pos, align 4
  %zero_value7 = load double, ptr %zero_value, align 8
  store double %zero_value7, ptr %mant, align 8
  %zero_count8 = load i32, ptr %zero_count, align 4
  store i32 %zero_count8, ptr %extra_scale, align 4
  store i1 true, ptr %valid, align 1
  store i1 false, ptr %saw_int, align 1
  store i1 false, ptr %last_underscore, align 1
  store i1 false, ptr %int_done, align 1
  %data9 = load i32, ptr %data1, align 4
  %pos10 = load i64, ptr %pos, align 4
  %end11 = load i64, ptr %end3, align 4
  %mant12 = load double, ptr %mant, align 8
  %extra_scale13 = load i32, ptr %extra_scale, align 4
  %valid14 = load i1, ptr %valid, align 1
  %saw_int15 = load i1, ptr %saw_int, align 1
  %last_underscore16 = load i1, ptr %last_underscore, align 1
  %int_done17 = load i1, ptr %int_done, align 1
  %call = call { i64, double, i32, i1, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f666c6f61745f696e7465676572(i32 %data9, i64 %pos10, i64 %end11, double %mant12, i32 %extra_scale13, i1 %valid14, i1 %saw_int15, i1 %last_underscore16, i1 %int_done17)
  %output = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call, 0
  store i64 %output, ptr %assignment_value, align 4
  %assignment_value18 = load i64, ptr %assignment_value, align 4
  %output19 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call, 1
  store double %output19, ptr %assignment_value20, align 8
  %assignment_value21 = load double, ptr %assignment_value20, align 8
  %output22 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call, 2
  store i32 %output22, ptr %assignment_value23, align 4
  %assignment_value24 = load i32, ptr %assignment_value23, align 4
  %output25 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call, 3
  store i1 %output25, ptr %assignment_value26, align 1
  %assignment_value27 = load i1, ptr %assignment_value26, align 1
  %output28 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call, 4
  store i1 %output28, ptr %assignment_value29, align 1
  %assignment_value30 = load i1, ptr %assignment_value29, align 1
  %output31 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call, 5
  store i1 %output31, ptr %assignment_value32, align 1
  %assignment_value33 = load i1, ptr %assignment_value32, align 1
  %output34 = extractvalue { i64, double, i32, i1, i1, i1, i1 } %call, 6
  store i1 %output34, ptr %assignment_value35, align 1
  %assignment_value36 = load i1, ptr %assignment_value35, align 1
  store i64 %assignment_value18, ptr %pos, align 4
  store double %assignment_value21, ptr %mant, align 8
  store i32 %assignment_value24, ptr %extra_scale, align 4
  store i1 %assignment_value27, ptr %valid, align 1
  store i1 %assignment_value30, ptr %saw_int, align 1
  store i1 %assignment_value33, ptr %last_underscore, align 1
  store i1 %assignment_value36, ptr %int_done, align 1
  %zero_count37 = load i32, ptr %zero_count, align 4
  store i32 %zero_count37, ptr %frac_count, align 4
  %zero_count38 = load i32, ptr %zero_count, align 4
  store i32 %zero_count38, ptr %exp_value, align 4
  store i1 false, ptr %exp_negative, align 1
  %last_underscore39 = load i1, ptr %last_underscore, align 1
  br i1 %last_underscore39, label %if.then, label %if.merge

if.then:                                          ; preds = %entry
  store i1 false, ptr %assignment_value40, align 1
  %assignment_value41 = load i1, ptr %assignment_value40, align 1
  store i1 %assignment_value41, ptr %valid, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.then, %entry
  %saw_int42 = load i1, ptr %saw_int, align 1
  %not = xor i1 %saw_int42, true
  br i1 %not, label %if.then43, label %if.merge44

if.then43:                                        ; preds = %if.merge
  store i1 false, ptr %assignment_value45, align 1
  %assignment_value46 = load i1, ptr %assignment_value45, align 1
  store i1 %assignment_value46, ptr %valid, align 1
  br label %if.merge44

if.merge44:                                       ; preds = %if.then43, %if.merge
  store i1 false, ptr %has_dot, align 1
  %valid47 = load i1, ptr %valid, align 1
  br i1 %valid47, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %if.merge44
  %pos48 = load i64, ptr %pos, align 4
  %end49 = load i64, ptr %end3, align 4
  %lt = icmp ult i64 %pos48, %end49
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %if.merge44
  %short_circuit = phi i1 [ false, %if.merge44 ], [ %lt, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then50, label %if.merge51

if.then50:                                        ; preds = %short_circuit.merge
  %data52 = load i32, ptr %data1, align 4
  %pos53 = load i64, ptr %pos, align 4
  %call54 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f666c6f61745f62797465(i32 %data52, i64 %pos53)
  store i8 %call54, ptr %byte, align 1
  %byte55 = load i8, ptr %byte, align 1
  %call56 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f646f74(i8 %byte55)
  br i1 %call56, label %if.then57, label %if.merge58

if.merge51:                                       ; preds = %if.merge58, %short_circuit.merge
  %valid65 = load i1, ptr %valid, align 1
  br i1 %valid65, label %short_circuit.rhs66, label %short_circuit.merge67

if.then57:                                        ; preds = %if.then50
  store i1 true, ptr %assignment_value59, align 1
  %assignment_value60 = load i1, ptr %assignment_value59, align 1
  store i1 %assignment_value60, ptr %has_dot, align 1
  %pos61 = load i64, ptr %pos, align 4
  %one62 = load i64, ptr %one, align 4
  %add = add i64 %pos61, %one62
  store i64 %add, ptr %assignment_value63, align 4
  %assignment_value64 = load i64, ptr %assignment_value63, align 4
  store i64 %assignment_value64, ptr %pos, align 4
  br label %if.merge58

if.merge58:                                       ; preds = %if.then57, %if.then50
  br label %if.merge51

short_circuit.rhs66:                              ; preds = %if.merge51
  %has_dot68 = load i1, ptr %has_dot, align 1
  br label %short_circuit.merge67

short_circuit.merge67:                            ; preds = %short_circuit.rhs66, %if.merge51
  %short_circuit69 = phi i1 [ false, %if.merge51 ], [ %has_dot68, %short_circuit.rhs66 ]
  br i1 %short_circuit69, label %if.then70, label %if.merge71

if.then70:                                        ; preds = %short_circuit.merge67
  store i1 false, ptr %frac_started, align 1
  store i1 false, ptr %frac_trail, align 1
  store i1 false, ptr %frac_done, align 1
  %data72 = load i32, ptr %data1, align 4
  %pos73 = load i64, ptr %pos, align 4
  %end74 = load i64, ptr %end3, align 4
  %mant75 = load double, ptr %mant, align 8
  %extra_scale76 = load i32, ptr %extra_scale, align 4
  %frac_count77 = load i32, ptr %frac_count, align 4
  %valid78 = load i1, ptr %valid, align 1
  %frac_started79 = load i1, ptr %frac_started, align 1
  %frac_trail80 = load i1, ptr %frac_trail, align 1
  %frac_done81 = load i1, ptr %frac_done, align 1
  %call82 = call { i64, double, i32, i32, i1, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f666c6f61745f6672616374696f6e(i32 %data72, i64 %pos73, i64 %end74, double %mant75, i32 %extra_scale76, i32 %frac_count77, i1 %valid78, i1 %frac_started79, i1 %frac_trail80, i1 %frac_done81)
  %output83 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call82, 0
  store i64 %output83, ptr %assignment_value84, align 4
  %assignment_value85 = load i64, ptr %assignment_value84, align 4
  %output86 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call82, 1
  store double %output86, ptr %assignment_value87, align 8
  %assignment_value88 = load double, ptr %assignment_value87, align 8
  %output89 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call82, 2
  store i32 %output89, ptr %assignment_value90, align 4
  %assignment_value91 = load i32, ptr %assignment_value90, align 4
  %output92 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call82, 3
  store i32 %output92, ptr %assignment_value93, align 4
  %assignment_value94 = load i32, ptr %assignment_value93, align 4
  %output95 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call82, 4
  store i1 %output95, ptr %assignment_value96, align 1
  %assignment_value97 = load i1, ptr %assignment_value96, align 1
  %output98 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call82, 5
  store i1 %output98, ptr %assignment_value99, align 1
  %assignment_value100 = load i1, ptr %assignment_value99, align 1
  %output101 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call82, 6
  store i1 %output101, ptr %assignment_value102, align 1
  %assignment_value103 = load i1, ptr %assignment_value102, align 1
  %output104 = extractvalue { i64, double, i32, i32, i1, i1, i1, i1 } %call82, 7
  store i1 %output104, ptr %assignment_value105, align 1
  %assignment_value106 = load i1, ptr %assignment_value105, align 1
  store i64 %assignment_value85, ptr %pos, align 4
  store double %assignment_value88, ptr %mant, align 8
  store i32 %assignment_value91, ptr %extra_scale, align 4
  store i32 %assignment_value94, ptr %frac_count, align 4
  store i1 %assignment_value97, ptr %valid, align 1
  store i1 %assignment_value100, ptr %frac_started, align 1
  store i1 %assignment_value103, ptr %frac_trail, align 1
  store i1 %assignment_value106, ptr %frac_done, align 1
  %frac_trail107 = load i1, ptr %frac_trail, align 1
  br i1 %frac_trail107, label %if.then108, label %if.merge109

if.merge71:                                       ; preds = %if.merge109, %short_circuit.merge67
  store i1 false, ptr %has_exp, align 1
  %valid112 = load i1, ptr %valid, align 1
  br i1 %valid112, label %short_circuit.rhs113, label %short_circuit.merge114

if.then108:                                       ; preds = %if.then70
  store i1 false, ptr %assignment_value110, align 1
  %assignment_value111 = load i1, ptr %assignment_value110, align 1
  store i1 %assignment_value111, ptr %valid, align 1
  br label %if.merge109

if.merge109:                                      ; preds = %if.then108, %if.then70
  br label %if.merge71

short_circuit.rhs113:                             ; preds = %if.merge71
  %pos115 = load i64, ptr %pos, align 4
  %end116 = load i64, ptr %end3, align 4
  %lt117 = icmp ult i64 %pos115, %end116
  br label %short_circuit.merge114

short_circuit.merge114:                           ; preds = %short_circuit.rhs113, %if.merge71
  %short_circuit118 = phi i1 [ false, %if.merge71 ], [ %lt117, %short_circuit.rhs113 ]
  br i1 %short_circuit118, label %if.then119, label %if.merge120

if.then119:                                       ; preds = %short_circuit.merge114
  %data121 = load i32, ptr %data1, align 4
  %pos122 = load i64, ptr %pos, align 4
  %call123 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f666c6f61745f62797465(i32 %data121, i64 %pos122)
  store i8 %call123, ptr %byte124, align 1
  %byte125 = load i8, ptr %byte124, align 1
  %call126 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f65(i8 %byte125)
  br i1 %call126, label %if.then127, label %if.merge128

if.merge120:                                      ; preds = %if.merge128, %short_circuit.merge114
  %valid136 = load i1, ptr %valid, align 1
  br i1 %valid136, label %short_circuit.rhs137, label %short_circuit.merge138

if.then127:                                       ; preds = %if.then119
  store i1 true, ptr %assignment_value129, align 1
  %assignment_value130 = load i1, ptr %assignment_value129, align 1
  store i1 %assignment_value130, ptr %has_exp, align 1
  %pos131 = load i64, ptr %pos, align 4
  %one132 = load i64, ptr %one, align 4
  %add133 = add i64 %pos131, %one132
  store i64 %add133, ptr %assignment_value134, align 4
  %assignment_value135 = load i64, ptr %assignment_value134, align 4
  store i64 %assignment_value135, ptr %pos, align 4
  br label %if.merge128

if.merge128:                                      ; preds = %if.then127, %if.then119
  br label %if.merge120

short_circuit.rhs137:                             ; preds = %if.merge120
  %has_exp139 = load i1, ptr %has_exp, align 1
  br label %short_circuit.merge138

short_circuit.merge138:                           ; preds = %short_circuit.rhs137, %if.merge120
  %short_circuit140 = phi i1 [ false, %if.merge120 ], [ %has_exp139, %short_circuit.rhs137 ]
  br i1 %short_circuit140, label %if.then141, label %if.merge142

if.then141:                                       ; preds = %short_circuit.merge138
  %pos143 = load i64, ptr %pos, align 4
  %end144 = load i64, ptr %end3, align 4
  %lt145 = icmp ult i64 %pos143, %end144
  br i1 %lt145, label %if.then146, label %if.merge147

if.merge142:                                      ; preds = %if.merge207, %short_circuit.merge138
  %mant210 = load double, ptr %mant, align 8
  store double %mant210, ptr %scaled, align 8
  %valid211 = load i1, ptr %valid, align 1
  br i1 %valid211, label %if.then212, label %if.merge213

if.then146:                                       ; preds = %if.then141
  %data148 = load i32, ptr %data1, align 4
  %pos149 = load i64, ptr %pos, align 4
  %call150 = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f666c6f61745f62797465(i32 %data148, i64 %pos149)
  store i8 %call150, ptr %byte151, align 1
  %byte152 = load i8, ptr %byte151, align 1
  %call153 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f7369676e(i8 %byte152)
  br i1 %call153, label %if.then154, label %if.else

if.merge147:                                      ; preds = %if.merge155, %if.then141
  store i1 false, ptr %exp_started, align 1
  store i1 false, ptr %exp_trail, align 1
  store i1 false, ptr %exp_done, align 1
  %data172 = load i32, ptr %data1, align 4
  %pos173 = load i64, ptr %pos, align 4
  %end174 = load i64, ptr %end3, align 4
  %exp_value175 = load i32, ptr %exp_value, align 4
  %valid176 = load i1, ptr %valid, align 1
  %exp_started177 = load i1, ptr %exp_started, align 1
  %exp_trail178 = load i1, ptr %exp_trail, align 1
  %exp_done179 = load i1, ptr %exp_done, align 1
  %call180 = call { i64, i32, i1, i1, i1, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f72616e67655f666c6f61745f6578706f6e656e74(i32 %data172, i64 %pos173, i64 %end174, i32 %exp_value175, i1 %valid176, i1 %exp_started177, i1 %exp_trail178, i1 %exp_done179)
  %output181 = extractvalue { i64, i32, i1, i1, i1, i1 } %call180, 0
  store i64 %output181, ptr %assignment_value182, align 4
  %assignment_value183 = load i64, ptr %assignment_value182, align 4
  %output184 = extractvalue { i64, i32, i1, i1, i1, i1 } %call180, 1
  store i32 %output184, ptr %assignment_value185, align 4
  %assignment_value186 = load i32, ptr %assignment_value185, align 4
  %output187 = extractvalue { i64, i32, i1, i1, i1, i1 } %call180, 2
  store i1 %output187, ptr %assignment_value188, align 1
  %assignment_value189 = load i1, ptr %assignment_value188, align 1
  %output190 = extractvalue { i64, i32, i1, i1, i1, i1 } %call180, 3
  store i1 %output190, ptr %assignment_value191, align 1
  %assignment_value192 = load i1, ptr %assignment_value191, align 1
  %output193 = extractvalue { i64, i32, i1, i1, i1, i1 } %call180, 4
  store i1 %output193, ptr %assignment_value194, align 1
  %assignment_value195 = load i1, ptr %assignment_value194, align 1
  %output196 = extractvalue { i64, i32, i1, i1, i1, i1 } %call180, 5
  store i1 %output196, ptr %assignment_value197, align 1
  %assignment_value198 = load i1, ptr %assignment_value197, align 1
  store i64 %assignment_value183, ptr %pos, align 4
  store i32 %assignment_value186, ptr %exp_value, align 4
  store i1 %assignment_value189, ptr %valid, align 1
  store i1 %assignment_value192, ptr %exp_started, align 1
  store i1 %assignment_value195, ptr %exp_trail, align 1
  store i1 %assignment_value198, ptr %exp_done, align 1
  %exp_started199 = load i1, ptr %exp_started, align 1
  %not200 = xor i1 %exp_started199, true
  br i1 %not200, label %if.then201, label %if.merge202

if.then154:                                       ; preds = %if.then146
  store i1 true, ptr %assignment_value156, align 1
  %assignment_value157 = load i1, ptr %assignment_value156, align 1
  store i1 %assignment_value157, ptr %exp_negative, align 1
  %pos158 = load i64, ptr %pos, align 4
  %one159 = load i64, ptr %one, align 4
  %add160 = add i64 %pos158, %one159
  store i64 %add160, ptr %assignment_value161, align 4
  %assignment_value162 = load i64, ptr %assignment_value161, align 4
  store i64 %assignment_value162, ptr %pos, align 4
  br label %if.merge155

if.else:                                          ; preds = %if.then146
  %byte163 = load i8, ptr %byte151, align 1
  %call164 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f706c7573(i8 %byte163)
  br i1 %call164, label %if.then165, label %if.merge166

if.merge155:                                      ; preds = %if.merge166, %if.then154
  br label %if.merge147

if.then165:                                       ; preds = %if.else
  %pos167 = load i64, ptr %pos, align 4
  %one168 = load i64, ptr %one, align 4
  %add169 = add i64 %pos167, %one168
  store i64 %add169, ptr %assignment_value170, align 4
  %assignment_value171 = load i64, ptr %assignment_value170, align 4
  store i64 %assignment_value171, ptr %pos, align 4
  br label %if.merge166

if.merge166:                                      ; preds = %if.then165, %if.else
  br label %if.merge155

if.then201:                                       ; preds = %if.merge147
  store i1 false, ptr %assignment_value203, align 1
  %assignment_value204 = load i1, ptr %assignment_value203, align 1
  store i1 %assignment_value204, ptr %valid, align 1
  br label %if.merge202

if.merge202:                                      ; preds = %if.then201, %if.merge147
  %exp_trail205 = load i1, ptr %exp_trail, align 1
  br i1 %exp_trail205, label %if.then206, label %if.merge207

if.then206:                                       ; preds = %if.merge202
  store i1 false, ptr %assignment_value208, align 1
  %assignment_value209 = load i1, ptr %assignment_value208, align 1
  store i1 %assignment_value209, ptr %valid, align 1
  br label %if.merge207

if.merge207:                                      ; preds = %if.then206, %if.merge202
  br label %if.merge142

if.then212:                                       ; preds = %if.merge142
  %mant214 = load double, ptr %mant, align 8
  %zero_value215 = load double, ptr %zero_value, align 8
  %eq = fcmp oeq double %mant214, %zero_value215
  br i1 %eq, label %if.then216, label %if.else217

if.merge213:                                      ; preds = %if.merge218, %if.merge142
  %valid279 = load i1, ptr %valid, align 1
  br i1 %valid279, label %short_circuit.rhs280, label %short_circuit.merge281

if.then216:                                       ; preds = %if.then212
  %zero_value219 = load double, ptr %zero_value, align 8
  store double %zero_value219, ptr %assignment_value220, align 8
  %assignment_value221 = load double, ptr %assignment_value220, align 8
  store double %assignment_value221, ptr %scaled, align 8
  br label %if.merge218

if.else217:                                       ; preds = %if.then212
  %exp_value222 = load i32, ptr %exp_value, align 4
  store i32 %exp_value222, ptr %exp_signed, align 4
  %exp_negative223 = load i1, ptr %exp_negative, align 1
  br i1 %exp_negative223, label %if.then224, label %if.merge225

if.merge218:                                      ; preds = %if.merge240, %if.then216
  br label %if.merge213

if.then224:                                       ; preds = %if.else217
  %zero_count226 = load i32, ptr %zero_count, align 4
  %exp_value227 = load i32, ptr %exp_value, align 4
  %sub228 = sub i32 %zero_count226, %exp_value227
  store i32 %sub228, ptr %assignment_value229, align 4
  %assignment_value230 = load i32, ptr %assignment_value229, align 4
  store i32 %assignment_value230, ptr %exp_signed, align 4
  br label %if.merge225

if.merge225:                                      ; preds = %if.then224, %if.else217
  %exp_signed231 = load i32, ptr %exp_signed, align 4
  %frac_count232 = load i32, ptr %frac_count, align 4
  %sub233 = sub i32 %exp_signed231, %frac_count232
  %extra_scale234 = load i32, ptr %extra_scale, align 4
  %add235 = add i32 %sub233, %extra_scale234
  store i32 %add235, ptr %adjusted, align 4
  %adjusted236 = load i32, ptr %adjusted, align 4
  %limit237 = load i32, ptr %limit, align 4
  %gt = icmp sgt i32 %adjusted236, %limit237
  br i1 %gt, label %if.then238, label %if.else239

if.then238:                                       ; preds = %if.merge225
  store i1 false, ptr %assignment_value241, align 1
  %assignment_value242 = load i1, ptr %assignment_value241, align 1
  store i1 %assignment_value242, ptr %valid, align 1
  br label %if.merge240

if.else239:                                       ; preds = %if.merge225
  %adjusted243 = load i32, ptr %adjusted, align 4
  %neg_limit244 = load i32, ptr %neg_limit, align 4
  %lt245 = icmp slt i32 %adjusted243, %neg_limit244
  br i1 %lt245, label %if.then246, label %if.else247

if.merge240:                                      ; preds = %if.merge248, %if.then238
  br label %if.merge218

if.then246:                                       ; preds = %if.else239
  %zero_value249 = load double, ptr %zero_value, align 8
  store double %zero_value249, ptr %assignment_value250, align 8
  %assignment_value251 = load double, ptr %assignment_value250, align 8
  store double %assignment_value251, ptr %scaled, align 8
  br label %if.merge248

if.else247:                                       ; preds = %if.else239
  %adjusted252 = load i32, ptr %adjusted, align 4
  store i32 %adjusted252, ptr %steps, align 4
  %steps253 = load i32, ptr %steps, align 4
  %zero_count254 = load i32, ptr %zero_count, align 4
  %lt255 = icmp slt i32 %steps253, %zero_count254
  br i1 %lt255, label %if.then256, label %if.merge257

if.merge248:                                      ; preds = %if.merge268, %if.then246
  br label %if.merge240

if.then256:                                       ; preds = %if.else247
  %zero_count258 = load i32, ptr %zero_count, align 4
  %steps259 = load i32, ptr %steps, align 4
  %sub260 = sub i32 %zero_count258, %steps259
  store i32 %sub260, ptr %assignment_value261, align 4
  %assignment_value262 = load i32, ptr %assignment_value261, align 4
  store i32 %assignment_value262, ptr %steps, align 4
  br label %if.merge257

if.merge257:                                      ; preds = %if.then256, %if.else247
  %adjusted263 = load i32, ptr %adjusted, align 4
  %zero_count264 = load i32, ptr %zero_count, align 4
  %gt265 = icmp sgt i32 %adjusted263, %zero_count264
  br i1 %gt265, label %if.then266, label %if.else267

if.then266:                                       ; preds = %if.merge257
  %scaled269 = load double, ptr %scaled, align 8
  %steps270 = load i32, ptr %steps, align 4
  %call271 = call double @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f7363616c655f666c6f61745f7570(double %scaled269, i32 %steps270)
  store double %call271, ptr %assignment_value272, align 8
  %assignment_value273 = load double, ptr %assignment_value272, align 8
  store double %assignment_value273, ptr %scaled, align 8
  br label %if.merge268

if.else267:                                       ; preds = %if.merge257
  %scaled274 = load double, ptr %scaled, align 8
  %steps275 = load i32, ptr %steps, align 4
  %call276 = call double @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f7363616c655f666c6f61745f646f776e(double %scaled274, i32 %steps275)
  store double %call276, ptr %assignment_value277, align 8
  %assignment_value278 = load double, ptr %assignment_value277, align 8
  store double %assignment_value278, ptr %scaled, align 8
  br label %if.merge268

if.merge268:                                      ; preds = %if.else267, %if.then266
  br label %if.merge248

short_circuit.rhs280:                             ; preds = %if.merge213
  %scaled282 = load double, ptr %scaled, align 8
  %f64_max283 = load double, ptr %f64_max, align 8
  %gt284 = fcmp ogt double %scaled282, %f64_max283
  br label %short_circuit.merge281

short_circuit.merge281:                           ; preds = %short_circuit.rhs280, %if.merge213
  %short_circuit285 = phi i1 [ false, %if.merge213 ], [ %gt284, %short_circuit.rhs280 ]
  br i1 %short_circuit285, label %if.then286, label %if.merge287

if.then286:                                       ; preds = %short_circuit.merge281
  store i1 false, ptr %assignment_value288, align 1
  %assignment_value289 = load i1, ptr %assignment_value288, align 1
  store i1 %assignment_value289, ptr %valid, align 1
  br label %if.merge287

if.merge287:                                      ; preds = %if.then286, %short_circuit.merge281
  %valid290 = load i1, ptr %valid, align 1
  br i1 %valid290, label %short_circuit.rhs291, label %short_circuit.merge292

short_circuit.rhs291:                             ; preds = %if.merge287
  %pos293 = load i64, ptr %pos, align 4
  %end294 = load i64, ptr %end3, align 4
  %ne = icmp ne i64 %pos293, %end294
  br label %short_circuit.merge292

short_circuit.merge292:                           ; preds = %short_circuit.rhs291, %if.merge287
  %short_circuit295 = phi i1 [ false, %if.merge287 ], [ %ne, %short_circuit.rhs291 ]
  br i1 %short_circuit295, label %if.then296, label %if.merge297

if.then296:                                       ; preds = %short_circuit.merge292
  store i1 false, ptr %assignment_value298, align 1
  %assignment_value299 = load i1, ptr %assignment_value298, align 1
  store i1 %assignment_value299, ptr %valid, align 1
  br label %if.merge297

if.merge297:                                      ; preds = %if.then296, %short_circuit.merge292
  %scaled300 = load double, ptr %scaled, align 8
  %valid301 = load i1, ptr %valid, align 1
  %output302 = insertvalue { double, i1 } zeroinitializer, double %scaled300, 0
  %output303 = insertvalue { double, i1 } %output302, i1 %valid301, 1
  ret { double, i1 } %output303
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f663332(ptr %text) #0 {
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
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
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
  %call = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f666c6f61745f62797465(i32 %place7, i64 %zero8)
  store i8 %call, ptr %first, align 1
  %first9 = load i8, ptr %first, align 1
  %call10 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f706c7573(i8 %first9)
  store i1 %call10, ptr %rejected, align 1
  %zero11 = load i64, ptr %zero, align 4
  store i64 %zero11, ptr %pos, align 4
  store i1 false, ptr %negative, align 1
  %first12 = load i8, ptr %first, align 1
  %call13 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f7369676e(i8 %first12)
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
  %call29 = call { double, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f666c6f61745f663634(i32 %place25, i64 %pos26, i64 %place28)
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

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f663634(ptr %text) #0 {
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
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
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
  %call = call i8 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f666c6f61745f62797465(i32 %place7, i64 %zero8)
  store i8 %call, ptr %first, align 1
  %first9 = load i8, ptr %first, align 1
  %call10 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f706c7573(i8 %first9)
  store i1 %call10, ptr %rejected, align 1
  %zero11 = load i64, ptr %zero, align 4
  store i64 %zero11, ptr %pos, align 4
  store i1 false, ptr %negative, align 1
  %first12 = load i8, ptr %first, align 1
  %call13 = call i1 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69735f7369676e(i8 %first12)
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
  %call29 = call { double, i1 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f666c6f61745f663634(i32 %place25, i64 %pos26, i64 %place28)
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

define internal ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f626f6f6c(i1 %value, ptr %0) #0 {
entry:
  %text = alloca [16 x i8], align 8
  %struct_literal = alloca [16 x i8], align 8
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
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text, ptr align 1 %struct_literal, i32 16, i1 false)
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %0, ptr align 1 %text, i32 16, i1 false)
  ret ptr %allocation
}

define internal ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f63686172(i32 %value, ptr %0) #0 {
entry:
  %text = alloca [16 x i8], align 8
  %struct_literal = alloca [16 x i8], align 8
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
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text, ptr align 1 %struct_literal, i32 16, i1 false)
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %0, ptr align 1 %text, i32 16, i1 false)
  ret ptr %allocation
}

define i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f6938(ptr %scratch, i64 %index, i8 %remaining, i1 %negative, i64 %capacity) {
entry:
  %assignment_value46 = alloca i8, align 1
  %assignment_value42 = alloca i64, align 8
  %zero_digit = alloca i8, align 1
  %one38 = alloca i64, align 8
  %digit = alloca i8, align 1
  %wide_digit = alloca i16, align 2
  %assignment_value34 = alloca i8, align 1
  %digit_signed = alloca i8, align 1
  %ten = alloca i8, align 1
  %length = alloca i64, align 8
  %assignment_value15 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %minus = alloca i8, align 1
  %one = alloca i64, align 8
  %zero = alloca i8, align 1
  %scratch1 = alloca ptr, align 8
  store ptr %scratch, ptr %scratch1, align 8
  %index2 = alloca i64, align 8
  store i64 %index, ptr %index2, align 4
  %remaining3 = alloca i8, align 1
  store i8 %remaining, ptr %remaining3, align 1
  %negative4 = alloca i1, align 1
  store i1 %negative, ptr %negative4, align 1
  %capacity5 = alloca i64, align 8
  store i64 %capacity, ptr %capacity5, align 4
  store i8 0, ptr %zero, align 1
  %remaining6 = load i8, ptr %remaining3, align 1
  %zero7 = load i8, ptr %zero, align 1
  %eq = icmp eq i8 %remaining6, %zero7
  br i1 %eq, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  %negative8 = load i1, ptr %negative4, align 1
  br i1 %negative8, label %if.then9, label %if.merge10

if.else:                                          ; preds = %entry
  store i8 10, ptr %ten, align 1
  %remaining26 = load i8, ptr %remaining3, align 1
  %ten27 = load i8, ptr %ten, align 1
  %rem = srem i8 %remaining26, %ten27
  store i8 %rem, ptr %digit_signed, align 1
  %negative28 = load i1, ptr %negative4, align 1
  br i1 %negative28, label %if.then29, label %if.merge30

if.merge:                                         ; preds = %if.merge30, %if.merge10
  %if = phi i64 [ %length25, %if.merge10 ], [ %call, %if.merge30 ]
  ret i64 %if

if.then9:                                         ; preds = %if.then
  store i64 1, ptr %one, align 4
  store i8 45, ptr %minus, align 1
  %index11 = load i64, ptr %index2, align 4
  %one12 = load i64, ptr %one, align 4
  %sub = sub i64 %index11, %one12
  store i64 %sub, ptr %assignment_value, align 4
  %assignment_value13 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value13, ptr %index2, align 4
  %minus14 = load i8, ptr %minus, align 1
  store i8 %minus14, ptr %assignment_value15, align 1
  %assignment_value16 = load i8, ptr %assignment_value15, align 1
  %scratch17 = load ptr, ptr %scratch1, align 8
  %index18 = load i64, ptr %index2, align 4
  %array_element = getelementptr inbounds i8, ptr %scratch17, i64 %index18
  store i8 %assignment_value16, ptr %array_element, align 1
  br label %if.merge10

if.merge10:                                       ; preds = %if.then9, %if.then
  %capacity19 = load i64, ptr %capacity5, align 4
  %index20 = load i64, ptr %index2, align 4
  %sub21 = sub i64 %capacity19, %index20
  store i64 %sub21, ptr %length, align 4
  %scratch22 = load ptr, ptr %scratch1, align 8
  %index23 = load i64, ptr %index2, align 4
  %capacity24 = load i64, ptr %capacity5, align 4
  call void @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f636f6d706163745f6279746573(ptr %scratch22, i64 0, i64 %index23, i64 %capacity24)
  %length25 = load i64, ptr %length, align 4
  br label %if.merge

if.then29:                                        ; preds = %if.else
  %zero31 = load i8, ptr %zero, align 1
  %digit_signed32 = load i8, ptr %digit_signed, align 1
  %sub33 = sub i8 %zero31, %digit_signed32
  store i8 %sub33, ptr %assignment_value34, align 1
  %assignment_value35 = load i8, ptr %assignment_value34, align 1
  store i8 %assignment_value35, ptr %digit_signed, align 1
  br label %if.merge30

if.merge30:                                       ; preds = %if.then29, %if.else
  %digit_signed36 = load i8, ptr %digit_signed, align 1
  %int_extend = sext i8 %digit_signed36 to i16
  store i16 %int_extend, ptr %wide_digit, align 2
  %wide_digit37 = load i16, ptr %wide_digit, align 2
  %int_trunc = trunc i16 %wide_digit37 to i8
  store i8 %int_trunc, ptr %digit, align 1
  store i64 1, ptr %one38, align 4
  store i8 48, ptr %zero_digit, align 1
  %index39 = load i64, ptr %index2, align 4
  %one40 = load i64, ptr %one38, align 4
  %sub41 = sub i64 %index39, %one40
  store i64 %sub41, ptr %assignment_value42, align 4
  %assignment_value43 = load i64, ptr %assignment_value42, align 4
  store i64 %assignment_value43, ptr %index2, align 4
  %zero_digit44 = load i8, ptr %zero_digit, align 1
  %digit45 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit44, %digit45
  store i8 %add, ptr %assignment_value46, align 1
  %assignment_value47 = load i8, ptr %assignment_value46, align 1
  %scratch48 = load ptr, ptr %scratch1, align 8
  %index49 = load i64, ptr %index2, align 4
  %array_element50 = getelementptr inbounds i8, ptr %scratch48, i64 %index49
  store i8 %assignment_value47, ptr %array_element50, align 1
  %scratch51 = load ptr, ptr %scratch1, align 8
  %index52 = load i64, ptr %index2, align 4
  %remaining53 = load i8, ptr %remaining3, align 1
  %ten54 = load i8, ptr %ten, align 1
  %div = sdiv i8 %remaining53, %ten54
  %negative55 = load i1, ptr %negative4, align 1
  %capacity56 = load i64, ptr %capacity5, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f6938(ptr %scratch51, i64 %index52, i8 %div, i1 %negative55, i64 %capacity56)
  br label %if.merge
}

define i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f693136(ptr %scratch, i64 %index, i16 %remaining, i1 %negative, i64 %capacity) {
entry:
  %assignment_value45 = alloca i8, align 1
  %assignment_value41 = alloca i64, align 8
  %zero_digit = alloca i8, align 1
  %one37 = alloca i64, align 8
  %digit = alloca i8, align 1
  %assignment_value34 = alloca i16, align 2
  %digit_signed = alloca i16, align 2
  %ten = alloca i16, align 2
  %length = alloca i64, align 8
  %assignment_value15 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %minus = alloca i8, align 1
  %one = alloca i64, align 8
  %zero = alloca i16, align 2
  %scratch1 = alloca ptr, align 8
  store ptr %scratch, ptr %scratch1, align 8
  %index2 = alloca i64, align 8
  store i64 %index, ptr %index2, align 4
  %remaining3 = alloca i16, align 2
  store i16 %remaining, ptr %remaining3, align 2
  %negative4 = alloca i1, align 1
  store i1 %negative, ptr %negative4, align 1
  %capacity5 = alloca i64, align 8
  store i64 %capacity, ptr %capacity5, align 4
  store i16 0, ptr %zero, align 2
  %remaining6 = load i16, ptr %remaining3, align 2
  %zero7 = load i16, ptr %zero, align 2
  %eq = icmp eq i16 %remaining6, %zero7
  br i1 %eq, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  %negative8 = load i1, ptr %negative4, align 1
  br i1 %negative8, label %if.then9, label %if.merge10

if.else:                                          ; preds = %entry
  store i16 10, ptr %ten, align 2
  %remaining26 = load i16, ptr %remaining3, align 2
  %ten27 = load i16, ptr %ten, align 2
  %rem = srem i16 %remaining26, %ten27
  store i16 %rem, ptr %digit_signed, align 2
  %negative28 = load i1, ptr %negative4, align 1
  br i1 %negative28, label %if.then29, label %if.merge30

if.merge:                                         ; preds = %if.merge30, %if.merge10
  %if = phi i64 [ %length25, %if.merge10 ], [ %call, %if.merge30 ]
  ret i64 %if

if.then9:                                         ; preds = %if.then
  store i64 1, ptr %one, align 4
  store i8 45, ptr %minus, align 1
  %index11 = load i64, ptr %index2, align 4
  %one12 = load i64, ptr %one, align 4
  %sub = sub i64 %index11, %one12
  store i64 %sub, ptr %assignment_value, align 4
  %assignment_value13 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value13, ptr %index2, align 4
  %minus14 = load i8, ptr %minus, align 1
  store i8 %minus14, ptr %assignment_value15, align 1
  %assignment_value16 = load i8, ptr %assignment_value15, align 1
  %scratch17 = load ptr, ptr %scratch1, align 8
  %index18 = load i64, ptr %index2, align 4
  %array_element = getelementptr inbounds i8, ptr %scratch17, i64 %index18
  store i8 %assignment_value16, ptr %array_element, align 1
  br label %if.merge10

if.merge10:                                       ; preds = %if.then9, %if.then
  %capacity19 = load i64, ptr %capacity5, align 4
  %index20 = load i64, ptr %index2, align 4
  %sub21 = sub i64 %capacity19, %index20
  store i64 %sub21, ptr %length, align 4
  %scratch22 = load ptr, ptr %scratch1, align 8
  %index23 = load i64, ptr %index2, align 4
  %capacity24 = load i64, ptr %capacity5, align 4
  call void @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f636f6d706163745f6279746573(ptr %scratch22, i64 0, i64 %index23, i64 %capacity24)
  %length25 = load i64, ptr %length, align 4
  br label %if.merge

if.then29:                                        ; preds = %if.else
  %zero31 = load i16, ptr %zero, align 2
  %digit_signed32 = load i16, ptr %digit_signed, align 2
  %sub33 = sub i16 %zero31, %digit_signed32
  store i16 %sub33, ptr %assignment_value34, align 2
  %assignment_value35 = load i16, ptr %assignment_value34, align 2
  store i16 %assignment_value35, ptr %digit_signed, align 2
  br label %if.merge30

if.merge30:                                       ; preds = %if.then29, %if.else
  %digit_signed36 = load i16, ptr %digit_signed, align 2
  %int_trunc = trunc i16 %digit_signed36 to i8
  store i8 %int_trunc, ptr %digit, align 1
  store i64 1, ptr %one37, align 4
  store i8 48, ptr %zero_digit, align 1
  %index38 = load i64, ptr %index2, align 4
  %one39 = load i64, ptr %one37, align 4
  %sub40 = sub i64 %index38, %one39
  store i64 %sub40, ptr %assignment_value41, align 4
  %assignment_value42 = load i64, ptr %assignment_value41, align 4
  store i64 %assignment_value42, ptr %index2, align 4
  %zero_digit43 = load i8, ptr %zero_digit, align 1
  %digit44 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit43, %digit44
  store i8 %add, ptr %assignment_value45, align 1
  %assignment_value46 = load i8, ptr %assignment_value45, align 1
  %scratch47 = load ptr, ptr %scratch1, align 8
  %index48 = load i64, ptr %index2, align 4
  %array_element49 = getelementptr inbounds i8, ptr %scratch47, i64 %index48
  store i8 %assignment_value46, ptr %array_element49, align 1
  %scratch50 = load ptr, ptr %scratch1, align 8
  %index51 = load i64, ptr %index2, align 4
  %remaining52 = load i16, ptr %remaining3, align 2
  %ten53 = load i16, ptr %ten, align 2
  %div = sdiv i16 %remaining52, %ten53
  %negative54 = load i1, ptr %negative4, align 1
  %capacity55 = load i64, ptr %capacity5, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f693136(ptr %scratch50, i64 %index51, i16 %div, i1 %negative54, i64 %capacity55)
  br label %if.merge
}

define i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f693332(ptr %scratch, i64 %index, i32 %remaining, i1 %negative, i64 %capacity) {
entry:
  %assignment_value45 = alloca i8, align 1
  %assignment_value41 = alloca i64, align 8
  %zero_digit = alloca i8, align 1
  %one37 = alloca i64, align 8
  %digit = alloca i8, align 1
  %assignment_value34 = alloca i32, align 4
  %digit_signed = alloca i32, align 4
  %ten = alloca i32, align 4
  %length = alloca i64, align 8
  %assignment_value15 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %minus = alloca i8, align 1
  %one = alloca i64, align 8
  %zero = alloca i32, align 4
  %scratch1 = alloca ptr, align 8
  store ptr %scratch, ptr %scratch1, align 8
  %index2 = alloca i64, align 8
  store i64 %index, ptr %index2, align 4
  %remaining3 = alloca i32, align 4
  store i32 %remaining, ptr %remaining3, align 4
  %negative4 = alloca i1, align 1
  store i1 %negative, ptr %negative4, align 1
  %capacity5 = alloca i64, align 8
  store i64 %capacity, ptr %capacity5, align 4
  store i32 0, ptr %zero, align 4
  %remaining6 = load i32, ptr %remaining3, align 4
  %zero7 = load i32, ptr %zero, align 4
  %eq = icmp eq i32 %remaining6, %zero7
  br i1 %eq, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  %negative8 = load i1, ptr %negative4, align 1
  br i1 %negative8, label %if.then9, label %if.merge10

if.else:                                          ; preds = %entry
  store i32 10, ptr %ten, align 4
  %remaining26 = load i32, ptr %remaining3, align 4
  %ten27 = load i32, ptr %ten, align 4
  %rem = srem i32 %remaining26, %ten27
  store i32 %rem, ptr %digit_signed, align 4
  %negative28 = load i1, ptr %negative4, align 1
  br i1 %negative28, label %if.then29, label %if.merge30

if.merge:                                         ; preds = %if.merge30, %if.merge10
  %if = phi i64 [ %length25, %if.merge10 ], [ %call, %if.merge30 ]
  ret i64 %if

if.then9:                                         ; preds = %if.then
  store i64 1, ptr %one, align 4
  store i8 45, ptr %minus, align 1
  %index11 = load i64, ptr %index2, align 4
  %one12 = load i64, ptr %one, align 4
  %sub = sub i64 %index11, %one12
  store i64 %sub, ptr %assignment_value, align 4
  %assignment_value13 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value13, ptr %index2, align 4
  %minus14 = load i8, ptr %minus, align 1
  store i8 %minus14, ptr %assignment_value15, align 1
  %assignment_value16 = load i8, ptr %assignment_value15, align 1
  %scratch17 = load ptr, ptr %scratch1, align 8
  %index18 = load i64, ptr %index2, align 4
  %array_element = getelementptr inbounds i8, ptr %scratch17, i64 %index18
  store i8 %assignment_value16, ptr %array_element, align 1
  br label %if.merge10

if.merge10:                                       ; preds = %if.then9, %if.then
  %capacity19 = load i64, ptr %capacity5, align 4
  %index20 = load i64, ptr %index2, align 4
  %sub21 = sub i64 %capacity19, %index20
  store i64 %sub21, ptr %length, align 4
  %scratch22 = load ptr, ptr %scratch1, align 8
  %index23 = load i64, ptr %index2, align 4
  %capacity24 = load i64, ptr %capacity5, align 4
  call void @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f636f6d706163745f6279746573(ptr %scratch22, i64 0, i64 %index23, i64 %capacity24)
  %length25 = load i64, ptr %length, align 4
  br label %if.merge

if.then29:                                        ; preds = %if.else
  %zero31 = load i32, ptr %zero, align 4
  %digit_signed32 = load i32, ptr %digit_signed, align 4
  %sub33 = sub i32 %zero31, %digit_signed32
  store i32 %sub33, ptr %assignment_value34, align 4
  %assignment_value35 = load i32, ptr %assignment_value34, align 4
  store i32 %assignment_value35, ptr %digit_signed, align 4
  br label %if.merge30

if.merge30:                                       ; preds = %if.then29, %if.else
  %digit_signed36 = load i32, ptr %digit_signed, align 4
  %int_trunc = trunc i32 %digit_signed36 to i8
  store i8 %int_trunc, ptr %digit, align 1
  store i64 1, ptr %one37, align 4
  store i8 48, ptr %zero_digit, align 1
  %index38 = load i64, ptr %index2, align 4
  %one39 = load i64, ptr %one37, align 4
  %sub40 = sub i64 %index38, %one39
  store i64 %sub40, ptr %assignment_value41, align 4
  %assignment_value42 = load i64, ptr %assignment_value41, align 4
  store i64 %assignment_value42, ptr %index2, align 4
  %zero_digit43 = load i8, ptr %zero_digit, align 1
  %digit44 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit43, %digit44
  store i8 %add, ptr %assignment_value45, align 1
  %assignment_value46 = load i8, ptr %assignment_value45, align 1
  %scratch47 = load ptr, ptr %scratch1, align 8
  %index48 = load i64, ptr %index2, align 4
  %array_element49 = getelementptr inbounds i8, ptr %scratch47, i64 %index48
  store i8 %assignment_value46, ptr %array_element49, align 1
  %scratch50 = load ptr, ptr %scratch1, align 8
  %index51 = load i64, ptr %index2, align 4
  %remaining52 = load i32, ptr %remaining3, align 4
  %ten53 = load i32, ptr %ten, align 4
  %div = sdiv i32 %remaining52, %ten53
  %negative54 = load i1, ptr %negative4, align 1
  %capacity55 = load i64, ptr %capacity5, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f693332(ptr %scratch50, i64 %index51, i32 %div, i1 %negative54, i64 %capacity55)
  br label %if.merge
}

define i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f693634(ptr %scratch, i64 %index, i64 %remaining, i1 %negative, i64 %capacity) {
entry:
  %assignment_value45 = alloca i8, align 1
  %assignment_value41 = alloca i64, align 8
  %zero_digit = alloca i8, align 1
  %one37 = alloca i64, align 8
  %digit = alloca i8, align 1
  %assignment_value34 = alloca i64, align 8
  %digit_signed = alloca i64, align 8
  %ten = alloca i64, align 8
  %length = alloca i64, align 8
  %assignment_value15 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %minus = alloca i8, align 1
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %scratch1 = alloca ptr, align 8
  store ptr %scratch, ptr %scratch1, align 8
  %index2 = alloca i64, align 8
  store i64 %index, ptr %index2, align 4
  %remaining3 = alloca i64, align 8
  store i64 %remaining, ptr %remaining3, align 4
  %negative4 = alloca i1, align 1
  store i1 %negative, ptr %negative4, align 1
  %capacity5 = alloca i64, align 8
  store i64 %capacity, ptr %capacity5, align 4
  store i64 0, ptr %zero, align 4
  %remaining6 = load i64, ptr %remaining3, align 4
  %zero7 = load i64, ptr %zero, align 4
  %eq = icmp eq i64 %remaining6, %zero7
  br i1 %eq, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  %negative8 = load i1, ptr %negative4, align 1
  br i1 %negative8, label %if.then9, label %if.merge10

if.else:                                          ; preds = %entry
  store i64 10, ptr %ten, align 4
  %remaining26 = load i64, ptr %remaining3, align 4
  %ten27 = load i64, ptr %ten, align 4
  %rem = srem i64 %remaining26, %ten27
  store i64 %rem, ptr %digit_signed, align 4
  %negative28 = load i1, ptr %negative4, align 1
  br i1 %negative28, label %if.then29, label %if.merge30

if.merge:                                         ; preds = %if.merge30, %if.merge10
  %if = phi i64 [ %length25, %if.merge10 ], [ %call, %if.merge30 ]
  ret i64 %if

if.then9:                                         ; preds = %if.then
  store i64 1, ptr %one, align 4
  store i8 45, ptr %minus, align 1
  %index11 = load i64, ptr %index2, align 4
  %one12 = load i64, ptr %one, align 4
  %sub = sub i64 %index11, %one12
  store i64 %sub, ptr %assignment_value, align 4
  %assignment_value13 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value13, ptr %index2, align 4
  %minus14 = load i8, ptr %minus, align 1
  store i8 %minus14, ptr %assignment_value15, align 1
  %assignment_value16 = load i8, ptr %assignment_value15, align 1
  %scratch17 = load ptr, ptr %scratch1, align 8
  %index18 = load i64, ptr %index2, align 4
  %array_element = getelementptr inbounds i8, ptr %scratch17, i64 %index18
  store i8 %assignment_value16, ptr %array_element, align 1
  br label %if.merge10

if.merge10:                                       ; preds = %if.then9, %if.then
  %capacity19 = load i64, ptr %capacity5, align 4
  %index20 = load i64, ptr %index2, align 4
  %sub21 = sub i64 %capacity19, %index20
  store i64 %sub21, ptr %length, align 4
  %scratch22 = load ptr, ptr %scratch1, align 8
  %index23 = load i64, ptr %index2, align 4
  %capacity24 = load i64, ptr %capacity5, align 4
  call void @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f636f6d706163745f6279746573(ptr %scratch22, i64 0, i64 %index23, i64 %capacity24)
  %length25 = load i64, ptr %length, align 4
  br label %if.merge

if.then29:                                        ; preds = %if.else
  %zero31 = load i64, ptr %zero, align 4
  %digit_signed32 = load i64, ptr %digit_signed, align 4
  %sub33 = sub i64 %zero31, %digit_signed32
  store i64 %sub33, ptr %assignment_value34, align 4
  %assignment_value35 = load i64, ptr %assignment_value34, align 4
  store i64 %assignment_value35, ptr %digit_signed, align 4
  br label %if.merge30

if.merge30:                                       ; preds = %if.then29, %if.else
  %digit_signed36 = load i64, ptr %digit_signed, align 4
  %int_trunc = trunc i64 %digit_signed36 to i8
  store i8 %int_trunc, ptr %digit, align 1
  store i64 1, ptr %one37, align 4
  store i8 48, ptr %zero_digit, align 1
  %index38 = load i64, ptr %index2, align 4
  %one39 = load i64, ptr %one37, align 4
  %sub40 = sub i64 %index38, %one39
  store i64 %sub40, ptr %assignment_value41, align 4
  %assignment_value42 = load i64, ptr %assignment_value41, align 4
  store i64 %assignment_value42, ptr %index2, align 4
  %zero_digit43 = load i8, ptr %zero_digit, align 1
  %digit44 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit43, %digit44
  store i8 %add, ptr %assignment_value45, align 1
  %assignment_value46 = load i8, ptr %assignment_value45, align 1
  %scratch47 = load ptr, ptr %scratch1, align 8
  %index48 = load i64, ptr %index2, align 4
  %array_element49 = getelementptr inbounds i8, ptr %scratch47, i64 %index48
  store i8 %assignment_value46, ptr %array_element49, align 1
  %scratch50 = load ptr, ptr %scratch1, align 8
  %index51 = load i64, ptr %index2, align 4
  %remaining52 = load i64, ptr %remaining3, align 4
  %ten53 = load i64, ptr %ten, align 4
  %div = sdiv i64 %remaining52, %ten53
  %negative54 = load i1, ptr %negative4, align 1
  %capacity55 = load i64, ptr %capacity5, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f693634(ptr %scratch50, i64 %index51, i64 %div, i1 %negative54, i64 %capacity55)
  br label %if.merge
}

define i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f7538(ptr %scratch, i64 %index, i8 %remaining, i64 %capacity) {
entry:
  %assignment_value21 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %zero_digit = alloca i8, align 1
  %one = alloca i64, align 8
  %digit = alloca i8, align 1
  %ten = alloca i8, align 1
  %length = alloca i64, align 8
  %zero = alloca i8, align 1
  %scratch1 = alloca ptr, align 8
  store ptr %scratch, ptr %scratch1, align 8
  %index2 = alloca i64, align 8
  store i64 %index, ptr %index2, align 4
  %remaining3 = alloca i8, align 1
  store i8 %remaining, ptr %remaining3, align 1
  %capacity4 = alloca i64, align 8
  store i64 %capacity, ptr %capacity4, align 4
  store i8 0, ptr %zero, align 1
  %remaining5 = load i8, ptr %remaining3, align 1
  %zero6 = load i8, ptr %zero, align 1
  %eq = icmp eq i8 %remaining5, %zero6
  br i1 %eq, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  %capacity7 = load i64, ptr %capacity4, align 4
  %index8 = load i64, ptr %index2, align 4
  %sub = sub i64 %capacity7, %index8
  store i64 %sub, ptr %length, align 4
  %scratch9 = load ptr, ptr %scratch1, align 8
  %index10 = load i64, ptr %index2, align 4
  %capacity11 = load i64, ptr %capacity4, align 4
  call void @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f636f6d706163745f6279746573(ptr %scratch9, i64 0, i64 %index10, i64 %capacity11)
  %length12 = load i64, ptr %length, align 4
  br label %if.merge

if.else:                                          ; preds = %entry
  store i8 10, ptr %ten, align 1
  %remaining13 = load i8, ptr %remaining3, align 1
  %ten14 = load i8, ptr %ten, align 1
  %rem = urem i8 %remaining13, %ten14
  store i8 %rem, ptr %digit, align 1
  store i64 1, ptr %one, align 4
  store i8 48, ptr %zero_digit, align 1
  %index15 = load i64, ptr %index2, align 4
  %one16 = load i64, ptr %one, align 4
  %sub17 = sub i64 %index15, %one16
  store i64 %sub17, ptr %assignment_value, align 4
  %assignment_value18 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value18, ptr %index2, align 4
  %zero_digit19 = load i8, ptr %zero_digit, align 1
  %digit20 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit19, %digit20
  store i8 %add, ptr %assignment_value21, align 1
  %assignment_value22 = load i8, ptr %assignment_value21, align 1
  %scratch23 = load ptr, ptr %scratch1, align 8
  %index24 = load i64, ptr %index2, align 4
  %array_element = getelementptr inbounds i8, ptr %scratch23, i64 %index24
  store i8 %assignment_value22, ptr %array_element, align 1
  %scratch25 = load ptr, ptr %scratch1, align 8
  %index26 = load i64, ptr %index2, align 4
  %remaining27 = load i8, ptr %remaining3, align 1
  %ten28 = load i8, ptr %ten, align 1
  %div = udiv i8 %remaining27, %ten28
  %capacity29 = load i64, ptr %capacity4, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f7538(ptr %scratch25, i64 %index26, i8 %div, i64 %capacity29)
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.then
  %if = phi i64 [ %length12, %if.then ], [ %call, %if.else ]
  ret i64 %if
}

define i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f753136(ptr %scratch, i64 %index, i16 %remaining, i64 %capacity) {
entry:
  %assignment_value22 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %zero_digit = alloca i8, align 1
  %one = alloca i64, align 8
  %digit = alloca i8, align 1
  %digit_wide = alloca i16, align 2
  %ten = alloca i16, align 2
  %length = alloca i64, align 8
  %zero = alloca i16, align 2
  %scratch1 = alloca ptr, align 8
  store ptr %scratch, ptr %scratch1, align 8
  %index2 = alloca i64, align 8
  store i64 %index, ptr %index2, align 4
  %remaining3 = alloca i16, align 2
  store i16 %remaining, ptr %remaining3, align 2
  %capacity4 = alloca i64, align 8
  store i64 %capacity, ptr %capacity4, align 4
  store i16 0, ptr %zero, align 2
  %remaining5 = load i16, ptr %remaining3, align 2
  %zero6 = load i16, ptr %zero, align 2
  %eq = icmp eq i16 %remaining5, %zero6
  br i1 %eq, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  %capacity7 = load i64, ptr %capacity4, align 4
  %index8 = load i64, ptr %index2, align 4
  %sub = sub i64 %capacity7, %index8
  store i64 %sub, ptr %length, align 4
  %scratch9 = load ptr, ptr %scratch1, align 8
  %index10 = load i64, ptr %index2, align 4
  %capacity11 = load i64, ptr %capacity4, align 4
  call void @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f636f6d706163745f6279746573(ptr %scratch9, i64 0, i64 %index10, i64 %capacity11)
  %length12 = load i64, ptr %length, align 4
  br label %if.merge

if.else:                                          ; preds = %entry
  store i16 10, ptr %ten, align 2
  %remaining13 = load i16, ptr %remaining3, align 2
  %ten14 = load i16, ptr %ten, align 2
  %rem = urem i16 %remaining13, %ten14
  store i16 %rem, ptr %digit_wide, align 2
  %digit_wide15 = load i16, ptr %digit_wide, align 2
  %int_trunc = trunc i16 %digit_wide15 to i8
  store i8 %int_trunc, ptr %digit, align 1
  store i64 1, ptr %one, align 4
  store i8 48, ptr %zero_digit, align 1
  %index16 = load i64, ptr %index2, align 4
  %one17 = load i64, ptr %one, align 4
  %sub18 = sub i64 %index16, %one17
  store i64 %sub18, ptr %assignment_value, align 4
  %assignment_value19 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value19, ptr %index2, align 4
  %zero_digit20 = load i8, ptr %zero_digit, align 1
  %digit21 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit20, %digit21
  store i8 %add, ptr %assignment_value22, align 1
  %assignment_value23 = load i8, ptr %assignment_value22, align 1
  %scratch24 = load ptr, ptr %scratch1, align 8
  %index25 = load i64, ptr %index2, align 4
  %array_element = getelementptr inbounds i8, ptr %scratch24, i64 %index25
  store i8 %assignment_value23, ptr %array_element, align 1
  %scratch26 = load ptr, ptr %scratch1, align 8
  %index27 = load i64, ptr %index2, align 4
  %remaining28 = load i16, ptr %remaining3, align 2
  %ten29 = load i16, ptr %ten, align 2
  %div = udiv i16 %remaining28, %ten29
  %capacity30 = load i64, ptr %capacity4, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f753136(ptr %scratch26, i64 %index27, i16 %div, i64 %capacity30)
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.then
  %if = phi i64 [ %length12, %if.then ], [ %call, %if.else ]
  ret i64 %if
}

define i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f753332(ptr %scratch, i64 %index, i32 %remaining, i64 %capacity) {
entry:
  %assignment_value22 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %zero_digit = alloca i8, align 1
  %one = alloca i64, align 8
  %digit = alloca i8, align 1
  %digit_wide = alloca i32, align 4
  %ten = alloca i32, align 4
  %length = alloca i64, align 8
  %zero = alloca i32, align 4
  %scratch1 = alloca ptr, align 8
  store ptr %scratch, ptr %scratch1, align 8
  %index2 = alloca i64, align 8
  store i64 %index, ptr %index2, align 4
  %remaining3 = alloca i32, align 4
  store i32 %remaining, ptr %remaining3, align 4
  %capacity4 = alloca i64, align 8
  store i64 %capacity, ptr %capacity4, align 4
  store i32 0, ptr %zero, align 4
  %remaining5 = load i32, ptr %remaining3, align 4
  %zero6 = load i32, ptr %zero, align 4
  %eq = icmp eq i32 %remaining5, %zero6
  br i1 %eq, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  %capacity7 = load i64, ptr %capacity4, align 4
  %index8 = load i64, ptr %index2, align 4
  %sub = sub i64 %capacity7, %index8
  store i64 %sub, ptr %length, align 4
  %scratch9 = load ptr, ptr %scratch1, align 8
  %index10 = load i64, ptr %index2, align 4
  %capacity11 = load i64, ptr %capacity4, align 4
  call void @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f636f6d706163745f6279746573(ptr %scratch9, i64 0, i64 %index10, i64 %capacity11)
  %length12 = load i64, ptr %length, align 4
  br label %if.merge

if.else:                                          ; preds = %entry
  store i32 10, ptr %ten, align 4
  %remaining13 = load i32, ptr %remaining3, align 4
  %ten14 = load i32, ptr %ten, align 4
  %rem = urem i32 %remaining13, %ten14
  store i32 %rem, ptr %digit_wide, align 4
  %digit_wide15 = load i32, ptr %digit_wide, align 4
  %int_trunc = trunc i32 %digit_wide15 to i8
  store i8 %int_trunc, ptr %digit, align 1
  store i64 1, ptr %one, align 4
  store i8 48, ptr %zero_digit, align 1
  %index16 = load i64, ptr %index2, align 4
  %one17 = load i64, ptr %one, align 4
  %sub18 = sub i64 %index16, %one17
  store i64 %sub18, ptr %assignment_value, align 4
  %assignment_value19 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value19, ptr %index2, align 4
  %zero_digit20 = load i8, ptr %zero_digit, align 1
  %digit21 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit20, %digit21
  store i8 %add, ptr %assignment_value22, align 1
  %assignment_value23 = load i8, ptr %assignment_value22, align 1
  %scratch24 = load ptr, ptr %scratch1, align 8
  %index25 = load i64, ptr %index2, align 4
  %array_element = getelementptr inbounds i8, ptr %scratch24, i64 %index25
  store i8 %assignment_value23, ptr %array_element, align 1
  %scratch26 = load ptr, ptr %scratch1, align 8
  %index27 = load i64, ptr %index2, align 4
  %remaining28 = load i32, ptr %remaining3, align 4
  %ten29 = load i32, ptr %ten, align 4
  %div = udiv i32 %remaining28, %ten29
  %capacity30 = load i64, ptr %capacity4, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f753332(ptr %scratch26, i64 %index27, i32 %div, i64 %capacity30)
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.then
  %if = phi i64 [ %length12, %if.then ], [ %call, %if.else ]
  ret i64 %if
}

define i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f753634(ptr %scratch, i64 %index, i64 %remaining, i64 %capacity) {
entry:
  %assignment_value22 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %zero_digit = alloca i8, align 1
  %one = alloca i64, align 8
  %digit = alloca i8, align 1
  %digit_wide = alloca i64, align 8
  %ten = alloca i64, align 8
  %length = alloca i64, align 8
  %zero = alloca i64, align 8
  %scratch1 = alloca ptr, align 8
  store ptr %scratch, ptr %scratch1, align 8
  %index2 = alloca i64, align 8
  store i64 %index, ptr %index2, align 4
  %remaining3 = alloca i64, align 8
  store i64 %remaining, ptr %remaining3, align 4
  %capacity4 = alloca i64, align 8
  store i64 %capacity, ptr %capacity4, align 4
  store i64 0, ptr %zero, align 4
  %remaining5 = load i64, ptr %remaining3, align 4
  %zero6 = load i64, ptr %zero, align 4
  %eq = icmp eq i64 %remaining5, %zero6
  br i1 %eq, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  %capacity7 = load i64, ptr %capacity4, align 4
  %index8 = load i64, ptr %index2, align 4
  %sub = sub i64 %capacity7, %index8
  store i64 %sub, ptr %length, align 4
  %scratch9 = load ptr, ptr %scratch1, align 8
  %index10 = load i64, ptr %index2, align 4
  %capacity11 = load i64, ptr %capacity4, align 4
  call void @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f636f6d706163745f6279746573(ptr %scratch9, i64 0, i64 %index10, i64 %capacity11)
  %length12 = load i64, ptr %length, align 4
  br label %if.merge

if.else:                                          ; preds = %entry
  store i64 10, ptr %ten, align 4
  %remaining13 = load i64, ptr %remaining3, align 4
  %ten14 = load i64, ptr %ten, align 4
  %rem = urem i64 %remaining13, %ten14
  store i64 %rem, ptr %digit_wide, align 4
  %digit_wide15 = load i64, ptr %digit_wide, align 4
  %int_trunc = trunc i64 %digit_wide15 to i8
  store i8 %int_trunc, ptr %digit, align 1
  store i64 1, ptr %one, align 4
  store i8 48, ptr %zero_digit, align 1
  %index16 = load i64, ptr %index2, align 4
  %one17 = load i64, ptr %one, align 4
  %sub18 = sub i64 %index16, %one17
  store i64 %sub18, ptr %assignment_value, align 4
  %assignment_value19 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value19, ptr %index2, align 4
  %zero_digit20 = load i8, ptr %zero_digit, align 1
  %digit21 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit20, %digit21
  store i8 %add, ptr %assignment_value22, align 1
  %assignment_value23 = load i8, ptr %assignment_value22, align 1
  %scratch24 = load ptr, ptr %scratch1, align 8
  %index25 = load i64, ptr %index2, align 4
  %array_element = getelementptr inbounds i8, ptr %scratch24, i64 %index25
  store i8 %assignment_value23, ptr %array_element, align 1
  %scratch26 = load ptr, ptr %scratch1, align 8
  %index27 = load i64, ptr %index2, align 4
  %remaining28 = load i64, ptr %remaining3, align 4
  %ten29 = load i64, ptr %ten, align 4
  %div = udiv i64 %remaining28, %ten29
  %capacity30 = load i64, ptr %capacity4, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f753634(ptr %scratch26, i64 %index27, i64 %div, i64 %capacity30)
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.then
  %if = phi i64 [ %length12, %if.then ], [ %call, %if.else ]
  ret i64 %if
}

define void @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f636f6d706163745f6279746573(ptr %scratch, i64 %target, i64 %source, i64 %capacity) {
entry:
  %one = alloca i64, align 8
  %assignment_value = alloca i8, align 1
  %scratch1 = alloca ptr, align 8
  store ptr %scratch, ptr %scratch1, align 8
  %target2 = alloca i64, align 8
  store i64 %target, ptr %target2, align 4
  %source3 = alloca i64, align 8
  store i64 %source, ptr %source3, align 4
  %capacity4 = alloca i64, align 8
  store i64 %capacity, ptr %capacity4, align 4
  %source5 = load i64, ptr %source3, align 4
  %capacity6 = load i64, ptr %capacity4, align 4
  %lt = icmp ult i64 %source5, %capacity6
  br i1 %lt, label %if.then, label %if.merge

if.then:                                          ; preds = %entry
  %scratch7 = load ptr, ptr %scratch1, align 8
  %source8 = load i64, ptr %source3, align 4
  %array_element = getelementptr inbounds i8, ptr %scratch7, i64 %source8
  %place = load i8, ptr %array_element, align 1
  store i8 %place, ptr %assignment_value, align 1
  %assignment_value9 = load i8, ptr %assignment_value, align 1
  %scratch10 = load ptr, ptr %scratch1, align 8
  %target11 = load i64, ptr %target2, align 4
  %array_element12 = getelementptr inbounds i8, ptr %scratch10, i64 %target11
  store i8 %assignment_value9, ptr %array_element12, align 1
  store i64 1, ptr %one, align 4
  %scratch13 = load ptr, ptr %scratch1, align 8
  %target14 = load i64, ptr %target2, align 4
  %one15 = load i64, ptr %one, align 4
  %add = add i64 %target14, %one15
  %source16 = load i64, ptr %source3, align 4
  %one17 = load i64, ptr %one, align 4
  %add18 = add i64 %source16, %one17
  %capacity19 = load i64, ptr %capacity4, align 4
  call void @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f636f6d706163745f6279746573(ptr %scratch13, i64 %add, i64 %add18, i64 %capacity19)
  br label %if.merge

if.merge:                                         ; preds = %if.then, %entry
  ret void
}

define internal ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6938(i8 %value, ptr %0) #0 {
entry:
  %text = alloca [16 x i8], align 8
  %struct_literal = alloca [16 x i8], align 8
  %assignment_value20 = alloca i32, align 4
  %data = alloca i32, align 4
  %length = alloca i64, align 8
  %assignment_value12 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %negative = alloca i1, align 1
  %remaining = alloca i8, align 1
  %index = alloca i64, align 8
  %zero_digit = alloca i8, align 1
  %zero = alloca i8, align 1
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
  store i8 0, ptr %zero, align 1
  store i8 48, ptr %zero_digit, align 1
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
  %index15 = load i64, ptr %index, align 4
  %remaining16 = load i8, ptr %remaining, align 1
  %negative17 = load i1, ptr %negative, align 1
  %capacity18 = load i64, ptr %capacity, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f6938(ptr %allocation, i64 %index15, i8 %remaining16, i1 %negative17, i64 %capacity18)
  store i64 %call, ptr %length, align 4
  store i32 0, ptr %data, align 4
  %array_element19 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element19 to i32
  store i32 %address, ptr %assignment_value20, align 4
  %assignment_value21 = load i32, ptr %assignment_value20, align 4
  store i32 %assignment_value21, ptr %data, align 4
  %data22 = load i32, ptr %data, align 4
  %length23 = load i64, ptr %length, align 4
  %data24 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data22, ptr %data24, align 4
  %length25 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length23, ptr %length25, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text, ptr align 1 %struct_literal, i32 16, i1 false)
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %0, ptr align 1 %text, i32 16, i1 false)
  ret ptr %allocation
}

define internal ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f693136(i16 %value, ptr %0) #0 {
entry:
  %text = alloca [16 x i8], align 8
  %struct_literal = alloca [16 x i8], align 8
  %assignment_value20 = alloca i32, align 4
  %data = alloca i32, align 4
  %length = alloca i64, align 8
  %assignment_value12 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %negative = alloca i1, align 1
  %remaining = alloca i16, align 2
  %index = alloca i64, align 8
  %zero_digit = alloca i8, align 1
  %zero = alloca i16, align 2
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
  store i16 0, ptr %zero, align 2
  store i8 48, ptr %zero_digit, align 1
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
  %index15 = load i64, ptr %index, align 4
  %remaining16 = load i16, ptr %remaining, align 2
  %negative17 = load i1, ptr %negative, align 1
  %capacity18 = load i64, ptr %capacity, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f693136(ptr %allocation, i64 %index15, i16 %remaining16, i1 %negative17, i64 %capacity18)
  store i64 %call, ptr %length, align 4
  store i32 0, ptr %data, align 4
  %array_element19 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element19 to i32
  store i32 %address, ptr %assignment_value20, align 4
  %assignment_value21 = load i32, ptr %assignment_value20, align 4
  store i32 %assignment_value21, ptr %data, align 4
  %data22 = load i32, ptr %data, align 4
  %length23 = load i64, ptr %length, align 4
  %data24 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data22, ptr %data24, align 4
  %length25 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length23, ptr %length25, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text, ptr align 1 %struct_literal, i32 16, i1 false)
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %0, ptr align 1 %text, i32 16, i1 false)
  ret ptr %allocation
}

define internal ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f693332(i32 %value, ptr %0) #0 {
entry:
  %text = alloca [16 x i8], align 8
  %struct_literal = alloca [16 x i8], align 8
  %assignment_value20 = alloca i32, align 4
  %data = alloca i32, align 4
  %length = alloca i64, align 8
  %assignment_value12 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %negative = alloca i1, align 1
  %remaining = alloca i32, align 4
  %index = alloca i64, align 8
  %zero_digit = alloca i8, align 1
  %zero = alloca i32, align 4
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
  store i32 0, ptr %zero, align 4
  store i8 48, ptr %zero_digit, align 1
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
  %index15 = load i64, ptr %index, align 4
  %remaining16 = load i32, ptr %remaining, align 4
  %negative17 = load i1, ptr %negative, align 1
  %capacity18 = load i64, ptr %capacity, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f693332(ptr %allocation, i64 %index15, i32 %remaining16, i1 %negative17, i64 %capacity18)
  store i64 %call, ptr %length, align 4
  store i32 0, ptr %data, align 4
  %array_element19 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element19 to i32
  store i32 %address, ptr %assignment_value20, align 4
  %assignment_value21 = load i32, ptr %assignment_value20, align 4
  store i32 %assignment_value21, ptr %data, align 4
  %data22 = load i32, ptr %data, align 4
  %length23 = load i64, ptr %length, align 4
  %data24 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data22, ptr %data24, align 4
  %length25 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length23, ptr %length25, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text, ptr align 1 %struct_literal, i32 16, i1 false)
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %0, ptr align 1 %text, i32 16, i1 false)
  ret ptr %allocation
}

define internal ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f693634(i64 %value, ptr %0) #0 {
entry:
  %text = alloca [16 x i8], align 8
  %struct_literal = alloca [16 x i8], align 8
  %assignment_value20 = alloca i32, align 4
  %data = alloca i32, align 4
  %length = alloca i64, align 8
  %assignment_value12 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %negative = alloca i1, align 1
  %remaining = alloca i64, align 8
  %index = alloca i64, align 8
  %zero_digit = alloca i8, align 1
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
  store i8 48, ptr %zero_digit, align 1
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
  %index15 = load i64, ptr %index, align 4
  %remaining16 = load i64, ptr %remaining, align 4
  %negative17 = load i1, ptr %negative, align 1
  %capacity18 = load i64, ptr %capacity, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f693634(ptr %allocation, i64 %index15, i64 %remaining16, i1 %negative17, i64 %capacity18)
  store i64 %call, ptr %length, align 4
  store i32 0, ptr %data, align 4
  %array_element19 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element19 to i32
  store i32 %address, ptr %assignment_value20, align 4
  %assignment_value21 = load i32, ptr %assignment_value20, align 4
  store i32 %assignment_value21, ptr %data, align 4
  %data22 = load i32, ptr %data, align 4
  %length23 = load i64, ptr %length, align 4
  %data24 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data22, ptr %data24, align 4
  %length25 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length23, ptr %length25, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text, ptr align 1 %struct_literal, i32 16, i1 false)
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %0, ptr align 1 %text, i32 16, i1 false)
  ret ptr %allocation
}

define internal ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f69313238(i128 %value, ptr %0) #0 {
entry:
  %text = alloca [16 x i8], align 8
  %struct_literal = alloca [16 x i8], align 8
  %assignment_value = alloca i32, align 4
  %data = alloca i32, align 4
  %length = alloca i64, align 8
  %magnitude = alloca i128, align 8
  %before_last = alloca i128, align 8
  %negative = alloca i1, align 1
  %zero = alloca i128, align 8
  %capacity = alloca i64, align 8
  %value1 = alloca i128, align 8
  store i128 %value, ptr %value1, align 4
  %allocation = call ptr @__wosy_core_alloc(i64 40, i64 1)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %allocation_panic, label %allocation_continue

allocation_panic:                                 ; preds = %entry
  call void @__wosy_core_system_panic()
  unreachable

allocation_continue:                              ; preds = %entry
  store i64 40, ptr %capacity, align 4
  store i128 0, ptr %zero, align 4
  %value2 = load i128, ptr %value1, align 4
  %zero3 = load i128, ptr %zero, align 4
  %lt = icmp slt i128 %value2, %zero3
  store i1 %lt, ptr %negative, align 1
  %negative4 = load i1, ptr %negative, align 1
  br i1 %negative4, label %if.then, label %if.else

if.then:                                          ; preds = %allocation_continue
  %zero5 = load i128, ptr %zero, align 4
  %value6 = load i128, ptr %value1, align 4
  %add = add i128 %value6, 1
  %sub = sub i128 %zero5, %add
  store i128 %sub, ptr %before_last, align 4
  %before_last7 = load i128, ptr %before_last, align 4
  %add8 = add i128 %before_last7, 1
  br label %if.merge

if.else:                                          ; preds = %allocation_continue
  %value9 = load i128, ptr %value1, align 4
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.then
  %if = phi i128 [ %add8, %if.then ], [ %value9, %if.else ]
  store i128 %if, ptr %magnitude, align 4
  %magnitude10 = load i128, ptr %magnitude, align 4
  %negative11 = load i1, ptr %negative, align 1
  %capacity12 = load i64, ptr %capacity, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f776964655f646563696d616c(ptr %allocation, i128 %magnitude10, i1 %negative11, i64 %capacity12)
  store i64 %call, ptr %length, align 4
  store i32 0, ptr %data, align 4
  %array_element = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element to i32
  store i32 %address, ptr %assignment_value, align 4
  %assignment_value13 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value13, ptr %data, align 4
  %data14 = load i32, ptr %data, align 4
  %length15 = load i64, ptr %length, align 4
  %data16 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data14, ptr %data16, align 4
  %length17 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length15, ptr %length17, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text, ptr align 1 %struct_literal, i32 16, i1 false)
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %0, ptr align 1 %text, i32 16, i1 false)
  ret ptr %allocation
}

define internal ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f7538(i8 %value, ptr %0) #0 {
entry:
  %text = alloca [16 x i8], align 8
  %struct_literal = alloca [16 x i8], align 8
  %assignment_value17 = alloca i32, align 4
  %data = alloca i32, align 4
  %length = alloca i64, align 8
  %assignment_value10 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %remaining = alloca i8, align 1
  %index = alloca i64, align 8
  %zero_value = alloca i8, align 1
  %zero_digit = alloca i8, align 1
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
  %index13 = load i64, ptr %index, align 4
  %remaining14 = load i8, ptr %remaining, align 1
  %capacity15 = load i64, ptr %capacity, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f7538(ptr %allocation, i64 %index13, i8 %remaining14, i64 %capacity15)
  store i64 %call, ptr %length, align 4
  store i32 0, ptr %data, align 4
  %array_element16 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element16 to i32
  store i32 %address, ptr %assignment_value17, align 4
  %assignment_value18 = load i32, ptr %assignment_value17, align 4
  store i32 %assignment_value18, ptr %data, align 4
  %data19 = load i32, ptr %data, align 4
  %length20 = load i64, ptr %length, align 4
  %data21 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data19, ptr %data21, align 4
  %length22 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length20, ptr %length22, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text, ptr align 1 %struct_literal, i32 16, i1 false)
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %0, ptr align 1 %text, i32 16, i1 false)
  ret ptr %allocation
}

define internal ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f753136(i16 %value, ptr %0) #0 {
entry:
  %text = alloca [16 x i8], align 8
  %struct_literal = alloca [16 x i8], align 8
  %assignment_value17 = alloca i32, align 4
  %data = alloca i32, align 4
  %length = alloca i64, align 8
  %assignment_value10 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %remaining = alloca i16, align 2
  %index = alloca i64, align 8
  %zero_value = alloca i16, align 2
  %zero_digit = alloca i8, align 1
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
  %index13 = load i64, ptr %index, align 4
  %remaining14 = load i16, ptr %remaining, align 2
  %capacity15 = load i64, ptr %capacity, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f753136(ptr %allocation, i64 %index13, i16 %remaining14, i64 %capacity15)
  store i64 %call, ptr %length, align 4
  store i32 0, ptr %data, align 4
  %array_element16 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element16 to i32
  store i32 %address, ptr %assignment_value17, align 4
  %assignment_value18 = load i32, ptr %assignment_value17, align 4
  store i32 %assignment_value18, ptr %data, align 4
  %data19 = load i32, ptr %data, align 4
  %length20 = load i64, ptr %length, align 4
  %data21 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data19, ptr %data21, align 4
  %length22 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length20, ptr %length22, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text, ptr align 1 %struct_literal, i32 16, i1 false)
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %0, ptr align 1 %text, i32 16, i1 false)
  ret ptr %allocation
}

define internal ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f753332(i32 %value, ptr %0) #0 {
entry:
  %text = alloca [16 x i8], align 8
  %struct_literal = alloca [16 x i8], align 8
  %assignment_value17 = alloca i32, align 4
  %data = alloca i32, align 4
  %length = alloca i64, align 8
  %assignment_value10 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %remaining = alloca i32, align 4
  %index = alloca i64, align 8
  %zero_value = alloca i32, align 4
  %zero_digit = alloca i8, align 1
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
  %index13 = load i64, ptr %index, align 4
  %remaining14 = load i32, ptr %remaining, align 4
  %capacity15 = load i64, ptr %capacity, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f753332(ptr %allocation, i64 %index13, i32 %remaining14, i64 %capacity15)
  store i64 %call, ptr %length, align 4
  store i32 0, ptr %data, align 4
  %array_element16 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element16 to i32
  store i32 %address, ptr %assignment_value17, align 4
  %assignment_value18 = load i32, ptr %assignment_value17, align 4
  store i32 %assignment_value18, ptr %data, align 4
  %data19 = load i32, ptr %data, align 4
  %length20 = load i64, ptr %length, align 4
  %data21 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data19, ptr %data21, align 4
  %length22 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length20, ptr %length22, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text, ptr align 1 %struct_literal, i32 16, i1 false)
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %0, ptr align 1 %text, i32 16, i1 false)
  ret ptr %allocation
}

define internal ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f753634(i64 %value, ptr %0) #0 {
entry:
  %text = alloca [16 x i8], align 8
  %struct_literal = alloca [16 x i8], align 8
  %assignment_value17 = alloca i32, align 4
  %data = alloca i32, align 4
  %length = alloca i64, align 8
  %assignment_value10 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %remaining = alloca i64, align 8
  %index = alloca i64, align 8
  %zero_value = alloca i64, align 8
  %zero_digit = alloca i8, align 1
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
  %index13 = load i64, ptr %index, align 4
  %remaining14 = load i64, ptr %remaining, align 4
  %capacity15 = load i64, ptr %capacity, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f753634(ptr %allocation, i64 %index13, i64 %remaining14, i64 %capacity15)
  store i64 %call, ptr %length, align 4
  store i32 0, ptr %data, align 4
  %array_element16 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element16 to i32
  store i32 %address, ptr %assignment_value17, align 4
  %assignment_value18 = load i32, ptr %assignment_value17, align 4
  store i32 %assignment_value18, ptr %data, align 4
  %data19 = load i32, ptr %data, align 4
  %length20 = load i64, ptr %length, align 4
  %data21 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data19, ptr %data21, align 4
  %length22 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length20, ptr %length22, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text, ptr align 1 %struct_literal, i32 16, i1 false)
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %0, ptr align 1 %text, i32 16, i1 false)
  ret ptr %allocation
}

define { i128, i8 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f73756274726163745f75313238(i128 %remaining, i128 %scale, i8 %digit) {
entry:
  %next_digit = alloca i8, align 1
  %rest = alloca i128, align 8
  %one_digit = alloca i8, align 1
  %remaining1 = alloca i128, align 8
  store i128 %remaining, ptr %remaining1, align 4
  %scale2 = alloca i128, align 8
  store i128 %scale, ptr %scale2, align 4
  %digit3 = alloca i8, align 1
  store i8 %digit, ptr %digit3, align 1
  %remaining4 = load i128, ptr %remaining1, align 4
  %scale5 = load i128, ptr %scale2, align 4
  %ge = icmp uge i128 %remaining4, %scale5
  br i1 %ge, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  store i8 1, ptr %one_digit, align 1
  %remaining6 = load i128, ptr %remaining1, align 4
  %scale7 = load i128, ptr %scale2, align 4
  %sub = sub i128 %remaining6, %scale7
  %scale8 = load i128, ptr %scale2, align 4
  %digit9 = load i8, ptr %digit3, align 1
  %one_digit10 = load i8, ptr %one_digit, align 1
  %add = add i8 %digit9, %one_digit10
  %call = call { i128, i8 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f73756274726163745f75313238(i128 %sub, i128 %scale8, i8 %add)
  %output = extractvalue { i128, i8 } %call, 0
  store i128 %output, ptr %rest, align 4
  %output11 = extractvalue { i128, i8 } %call, 1
  store i8 %output11, ptr %next_digit, align 1
  %rest12 = load i128, ptr %rest, align 4
  %next_digit13 = load i8, ptr %next_digit, align 1
  %output14 = insertvalue { i128, i8 } zeroinitializer, i128 %rest12, 0
  %output15 = insertvalue { i128, i8 } %output14, i8 %next_digit13, 1
  br label %if.merge

if.else:                                          ; preds = %entry
  %remaining16 = load i128, ptr %remaining1, align 4
  %digit17 = load i8, ptr %digit3, align 1
  %output18 = insertvalue { i128, i8 } zeroinitializer, i128 %remaining16, 0
  %output19 = insertvalue { i128, i8 } %output18, i8 %digit17, 1
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.then
  %if = phi { i128, i8 } [ %output15, %if.then ], [ %output19, %if.else ]
  ret { i128, i8 } %if
}

define i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f646563696d616c5f6368756e6b73(ptr %scratch, i64 %index, i64 %remaining, i64 %upper, i64 %width, i8 %leading, i1 %has_upper, i1 %negative, i64 %capacity) {
entry:
  %length = alloca i64, align 8
  %assignment_value94 = alloca i8, align 1
  %assignment_value92 = alloca i64, align 8
  %assignment_value81 = alloca i8, align 1
  %assignment_value76 = alloca i64, align 8
  %padded = alloca i64, align 8
  %assignment_value39 = alloca i64, align 8
  %assignment_value27 = alloca i8, align 1
  %assignment_value = alloca i64, align 8
  %digit = alloca i8, align 1
  %remainder = alloca i64, align 8
  %quotient = alloca i64, align 8
  %ten = alloca i64, align 8
  %zero_digit = alloca i8, align 1
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %scratch1 = alloca ptr, align 8
  store ptr %scratch, ptr %scratch1, align 8
  %index2 = alloca i64, align 8
  store i64 %index, ptr %index2, align 4
  %remaining3 = alloca i64, align 8
  store i64 %remaining, ptr %remaining3, align 4
  %upper4 = alloca i64, align 8
  store i64 %upper, ptr %upper4, align 4
  %width5 = alloca i64, align 8
  store i64 %width, ptr %width5, align 4
  %leading6 = alloca i8, align 1
  store i8 %leading, ptr %leading6, align 1
  %has_upper7 = alloca i1, align 1
  store i1 %has_upper, ptr %has_upper7, align 1
  %negative8 = alloca i1, align 1
  store i1 %negative, ptr %negative8, align 1
  %capacity9 = alloca i64, align 8
  store i64 %capacity, ptr %capacity9, align 4
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  store i8 48, ptr %zero_digit, align 1
  %remaining10 = load i64, ptr %remaining3, align 4
  %zero11 = load i64, ptr %zero, align 4
  %ne = icmp ne i64 %remaining10, %zero11
  br i1 %ne, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %width12 = load i64, ptr %width5, align 4
  %zero13 = load i64, ptr %zero, align 4
  %ne14 = icmp ne i64 %width12, %zero13
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %ne14, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  store i64 10, ptr %ten, align 4
  %remaining15 = load i64, ptr %remaining3, align 4
  %ten16 = load i64, ptr %ten, align 4
  %div = udiv i64 %remaining15, %ten16
  store i64 %div, ptr %quotient, align 4
  %remaining17 = load i64, ptr %remaining3, align 4
  %quotient18 = load i64, ptr %quotient, align 4
  %ten19 = load i64, ptr %ten, align 4
  %mul = mul i64 %quotient18, %ten19
  %sub = sub i64 %remaining17, %mul
  store i64 %sub, ptr %remainder, align 4
  %remainder20 = load i64, ptr %remainder, align 4
  %int_trunc = trunc i64 %remainder20 to i8
  store i8 %int_trunc, ptr %digit, align 1
  %index21 = load i64, ptr %index2, align 4
  %one22 = load i64, ptr %one, align 4
  %sub23 = sub i64 %index21, %one22
  store i64 %sub23, ptr %assignment_value, align 4
  %assignment_value24 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value24, ptr %index2, align 4
  %zero_digit25 = load i8, ptr %zero_digit, align 1
  %digit26 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit25, %digit26
  store i8 %add, ptr %assignment_value27, align 1
  %assignment_value28 = load i8, ptr %assignment_value27, align 1
  %scratch29 = load ptr, ptr %scratch1, align 8
  %index30 = load i64, ptr %index2, align 4
  %array_element = getelementptr inbounds i8, ptr %scratch29, i64 %index30
  store i8 %assignment_value28, ptr %array_element, align 1
  %width31 = load i64, ptr %width5, align 4
  %zero32 = load i64, ptr %zero, align 4
  %ne33 = icmp ne i64 %width31, %zero32
  br i1 %ne33, label %if.then34, label %if.merge35

if.else:                                          ; preds = %short_circuit.merge
  %has_upper50 = load i1, ptr %has_upper7, align 1
  br i1 %has_upper50, label %if.then51, label %if.else52

if.merge:                                         ; preds = %if.merge53, %if.merge35
  %if108 = phi i64 [ %call, %if.merge35 ], [ %if107, %if.merge53 ]
  ret i64 %if108

if.then34:                                        ; preds = %if.then
  %width36 = load i64, ptr %width5, align 4
  %one37 = load i64, ptr %one, align 4
  %sub38 = sub i64 %width36, %one37
  store i64 %sub38, ptr %assignment_value39, align 4
  %assignment_value40 = load i64, ptr %assignment_value39, align 4
  store i64 %assignment_value40, ptr %width5, align 4
  br label %if.merge35

if.merge35:                                       ; preds = %if.then34, %if.then
  %scratch41 = load ptr, ptr %scratch1, align 8
  %index42 = load i64, ptr %index2, align 4
  %quotient43 = load i64, ptr %quotient, align 4
  %upper44 = load i64, ptr %upper4, align 4
  %width45 = load i64, ptr %width5, align 4
  %leading46 = load i8, ptr %leading6, align 1
  %has_upper47 = load i1, ptr %has_upper7, align 1
  %negative48 = load i1, ptr %negative8, align 1
  %capacity49 = load i64, ptr %capacity9, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f646563696d616c5f6368756e6b73(ptr %scratch41, i64 %index42, i64 %quotient43, i64 %upper44, i64 %width45, i8 %leading46, i1 %has_upper47, i1 %negative48, i64 %capacity49)
  br label %if.merge

if.then51:                                        ; preds = %if.else
  %leading54 = load i8, ptr %leading6, align 1
  %ne55 = icmp ne i8 %leading54, 0
  br i1 %ne55, label %if.then56, label %if.else57

if.else52:                                        ; preds = %if.else
  %leading69 = load i8, ptr %leading6, align 1
  %ne70 = icmp ne i8 %leading69, 0
  br i1 %ne70, label %if.then71, label %if.merge72

if.merge53:                                       ; preds = %if.merge88, %if.merge58
  %if107 = phi i64 [ %call68, %if.merge58 ], [ %length106, %if.merge88 ]
  br label %if.merge

if.then56:                                        ; preds = %if.then51
  br label %if.merge58

if.else57:                                        ; preds = %if.then51
  %zero59 = load i64, ptr %zero, align 4
  br label %if.merge58

if.merge58:                                       ; preds = %if.else57, %if.then56
  %if = phi i64 [ 19, %if.then56 ], [ %zero59, %if.else57 ]
  store i64 %if, ptr %padded, align 4
  %scratch60 = load ptr, ptr %scratch1, align 8
  %index61 = load i64, ptr %index2, align 4
  %upper62 = load i64, ptr %upper4, align 4
  %zero63 = load i64, ptr %zero, align 4
  %padded64 = load i64, ptr %padded, align 4
  %leading65 = load i8, ptr %leading6, align 1
  %negative66 = load i1, ptr %negative8, align 1
  %capacity67 = load i64, ptr %capacity9, align 4
  %call68 = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f646563696d616c5f6368756e6b73(ptr %scratch60, i64 %index61, i64 %upper62, i64 %zero63, i64 %padded64, i8 %leading65, i1 false, i1 %negative66, i64 %capacity67)
  br label %if.merge53

if.then71:                                        ; preds = %if.else52
  %index73 = load i64, ptr %index2, align 4
  %one74 = load i64, ptr %one, align 4
  %sub75 = sub i64 %index73, %one74
  store i64 %sub75, ptr %assignment_value76, align 4
  %assignment_value77 = load i64, ptr %assignment_value76, align 4
  store i64 %assignment_value77, ptr %index2, align 4
  %zero_digit78 = load i8, ptr %zero_digit, align 1
  %leading79 = load i8, ptr %leading6, align 1
  %add80 = add i8 %zero_digit78, %leading79
  store i8 %add80, ptr %assignment_value81, align 1
  %assignment_value82 = load i8, ptr %assignment_value81, align 1
  %scratch83 = load ptr, ptr %scratch1, align 8
  %index84 = load i64, ptr %index2, align 4
  %array_element85 = getelementptr inbounds i8, ptr %scratch83, i64 %index84
  store i8 %assignment_value82, ptr %array_element85, align 1
  br label %if.merge72

if.merge72:                                       ; preds = %if.then71, %if.else52
  %negative86 = load i1, ptr %negative8, align 1
  br i1 %negative86, label %if.then87, label %if.merge88

if.then87:                                        ; preds = %if.merge72
  %index89 = load i64, ptr %index2, align 4
  %one90 = load i64, ptr %one, align 4
  %sub91 = sub i64 %index89, %one90
  store i64 %sub91, ptr %assignment_value92, align 4
  %assignment_value93 = load i64, ptr %assignment_value92, align 4
  store i64 %assignment_value93, ptr %index2, align 4
  store i8 45, ptr %assignment_value94, align 1
  %assignment_value95 = load i8, ptr %assignment_value94, align 1
  %scratch96 = load ptr, ptr %scratch1, align 8
  %index97 = load i64, ptr %index2, align 4
  %array_element98 = getelementptr inbounds i8, ptr %scratch96, i64 %index97
  store i8 %assignment_value95, ptr %array_element98, align 1
  br label %if.merge88

if.merge88:                                       ; preds = %if.then87, %if.merge72
  %capacity99 = load i64, ptr %capacity9, align 4
  %index100 = load i64, ptr %index2, align 4
  %sub101 = sub i64 %capacity99, %index100
  store i64 %sub101, ptr %length, align 4
  %scratch102 = load ptr, ptr %scratch1, align 8
  %zero103 = load i64, ptr %zero, align 4
  %index104 = load i64, ptr %index2, align 4
  %capacity105 = load i64, ptr %capacity9, align 4
  call void @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f636f6d706163745f6279746573(ptr %scratch102, i64 %zero103, i64 %index104, i64 %capacity105)
  %length106 = load i64, ptr %length, align 4
  br label %if.merge53
}

define i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f776964655f646563696d616c(ptr %scratch, i128 %value, i1 %negative, i64 %capacity) {
entry:
  %assignment_value31 = alloca i64, align 8
  %assignment_value27 = alloca i64, align 8
  %assignment_value25 = alloca i64, align 8
  %assignment_value21 = alloca i8, align 1
  %assignment_value18 = alloca i128, align 8
  %assignment_value = alloca i64, align 8
  %remainder = alloca i128, align 8
  %quotient = alloca i128, align 8
  %has_upper = alloca i1, align 1
  %leading = alloca i8, align 1
  %width = alloca i64, align 8
  %upper = alloca i64, align 8
  %remaining = alloca i64, align 8
  %chunk_base = alloca i128, align 8
  %scratch1 = alloca ptr, align 8
  store ptr %scratch, ptr %scratch1, align 8
  %value2 = alloca i128, align 8
  store i128 %value, ptr %value2, align 4
  %negative3 = alloca i1, align 1
  store i1 %negative, ptr %negative3, align 1
  %capacity4 = alloca i64, align 8
  store i64 %capacity, ptr %capacity4, align 4
  store i128 10000000000000000000, ptr %chunk_base, align 4
  store i64 0, ptr %remaining, align 4
  store i64 0, ptr %upper, align 4
  store i64 1, ptr %width, align 4
  store i8 0, ptr %leading, align 1
  %value5 = load i128, ptr %value2, align 4
  %chunk_base6 = load i128, ptr %chunk_base, align 4
  %ge = icmp uge i128 %value5, %chunk_base6
  store i1 %ge, ptr %has_upper, align 1
  %has_upper7 = load i1, ptr %has_upper, align 1
  br i1 %has_upper7, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  %value8 = load i128, ptr %value2, align 4
  %chunk_base9 = load i128, ptr %chunk_base, align 4
  %div = udiv i128 %value8, %chunk_base9
  store i128 %div, ptr %quotient, align 4
  %value10 = load i128, ptr %value2, align 4
  %quotient11 = load i128, ptr %quotient, align 4
  %chunk_base12 = load i128, ptr %chunk_base, align 4
  %mul = mul i128 %quotient11, %chunk_base12
  %sub = sub i128 %value10, %mul
  store i128 %sub, ptr %remainder, align 4
  %remainder13 = load i128, ptr %remainder, align 4
  %int_trunc = trunc i128 %remainder13 to i64
  store i64 %int_trunc, ptr %assignment_value, align 4
  %assignment_value14 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value14, ptr %remaining, align 4
  %quotient15 = load i128, ptr %quotient, align 4
  %chunk_base16 = load i128, ptr %chunk_base, align 4
  %leading17 = load i8, ptr %leading, align 1
  %call = call { i128, i8 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f73756274726163745f75313238(i128 %quotient15, i128 %chunk_base16, i8 %leading17)
  %output = extractvalue { i128, i8 } %call, 0
  store i128 %output, ptr %assignment_value18, align 4
  %assignment_value19 = load i128, ptr %assignment_value18, align 4
  %output20 = extractvalue { i128, i8 } %call, 1
  store i8 %output20, ptr %assignment_value21, align 1
  %assignment_value22 = load i8, ptr %assignment_value21, align 1
  store i128 %assignment_value19, ptr %quotient, align 4
  store i8 %assignment_value22, ptr %leading, align 1
  %quotient23 = load i128, ptr %quotient, align 4
  %int_trunc24 = trunc i128 %quotient23 to i64
  store i64 %int_trunc24, ptr %assignment_value25, align 4
  %assignment_value26 = load i64, ptr %assignment_value25, align 4
  store i64 %assignment_value26, ptr %upper, align 4
  store i64 19, ptr %assignment_value27, align 4
  %assignment_value28 = load i64, ptr %assignment_value27, align 4
  store i64 %assignment_value28, ptr %width, align 4
  br label %if.merge

if.else:                                          ; preds = %entry
  %value29 = load i128, ptr %value2, align 4
  %int_trunc30 = trunc i128 %value29 to i64
  store i64 %int_trunc30, ptr %assignment_value31, align 4
  %assignment_value32 = load i64, ptr %assignment_value31, align 4
  store i64 %assignment_value32, ptr %remaining, align 4
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.then
  %scratch33 = load ptr, ptr %scratch1, align 8
  %capacity34 = load i64, ptr %capacity4, align 4
  %remaining35 = load i64, ptr %remaining, align 4
  %upper36 = load i64, ptr %upper, align 4
  %width37 = load i64, ptr %width, align 4
  %leading38 = load i8, ptr %leading, align 1
  %has_upper39 = load i1, ptr %has_upper, align 1
  %negative40 = load i1, ptr %negative3, align 1
  %capacity41 = load i64, ptr %capacity4, align 4
  %call42 = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f646563696d616c5f6368756e6b73(ptr %scratch33, i64 %capacity34, i64 %remaining35, i64 %upper36, i64 %width37, i8 %leading38, i1 %has_upper39, i1 %negative40, i64 %capacity41)
  ret i64 %call42
}

define internal ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f75313238(i128 %value, ptr %0) #0 {
entry:
  %text = alloca [16 x i8], align 8
  %struct_literal = alloca [16 x i8], align 8
  %assignment_value = alloca i32, align 4
  %data = alloca i32, align 4
  %length = alloca i64, align 8
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
  %value2 = load i128, ptr %value1, align 4
  %capacity3 = load i64, ptr %capacity, align 4
  %call = call i64 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f776964655f646563696d616c(ptr %allocation, i128 %value2, i1 false, i64 %capacity3)
  store i64 %call, ptr %length, align 4
  store i32 0, ptr %data, align 4
  %array_element = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element to i32
  store i32 %address, ptr %assignment_value, align 4
  %assignment_value4 = load i32, ptr %assignment_value, align 4
  store i32 %assignment_value4, ptr %data, align 4
  %data5 = load i32, ptr %data, align 4
  %length6 = load i64, ptr %length, align 4
  %data7 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data5, ptr %data7, align 4
  %length8 = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %length6, ptr %length8, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text, ptr align 1 %struct_literal, i32 16, i1 false)
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %0, ptr align 1 %text, i32 16, i1 false)
  ret ptr %allocation
}

define { double, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6e6f726d616c697a655f666c6f61745f646f776e(double %scaled, i32 %exponent, i64 %budget) {
entry:
  %right_exponent = alloca i32, align 4
  %right = alloca double, align 8
  %left_exponent = alloca i32, align 4
  %left = alloca double, align 8
  %half = alloca i64, align 8
  %two = alloca i64, align 8
  %one_count = alloca i32, align 4
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %ten_value = alloca double, align 8
  %scaled1 = alloca double, align 8
  store double %scaled, ptr %scaled1, align 8
  %exponent2 = alloca i32, align 4
  store i32 %exponent, ptr %exponent2, align 4
  %budget3 = alloca i64, align 8
  store i64 %budget, ptr %budget3, align 4
  store double 1.000000e+01, ptr %ten_value, align 8
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  %budget4 = load i64, ptr %budget3, align 4
  %zero5 = load i64, ptr %zero, align 4
  %eq = icmp eq i64 %budget4, %zero5
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %scaled6 = load double, ptr %scaled1, align 8
  %ten_value7 = load double, ptr %ten_value, align 8
  %ge = fcmp oge double %scaled6, %ten_value7
  %not = xor i1 %ge, true
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %not, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  %scaled8 = load double, ptr %scaled1, align 8
  %exponent9 = load i32, ptr %exponent2, align 4
  %output = insertvalue { double, i32 } zeroinitializer, double %scaled8, 0
  %output10 = insertvalue { double, i32 } %output, i32 %exponent9, 1
  br label %if.merge

if.else:                                          ; preds = %short_circuit.merge
  %budget11 = load i64, ptr %budget3, align 4
  %one12 = load i64, ptr %one, align 4
  %eq13 = icmp eq i64 %budget11, %one12
  br i1 %eq13, label %if.then14, label %if.else15

if.merge:                                         ; preds = %if.merge16, %if.then
  %if53 = phi { double, i32 } [ %output10, %if.then ], [ %if52, %if.merge16 ]
  ret { double, i32 } %if53

if.then14:                                        ; preds = %if.else
  store i32 1, ptr %one_count, align 4
  %scaled17 = load double, ptr %scaled1, align 8
  %ten_value18 = load double, ptr %ten_value, align 8
  %div = fdiv double %scaled17, %ten_value18
  %exponent19 = load i32, ptr %exponent2, align 4
  %one_count20 = load i32, ptr %one_count, align 4
  %add = add i32 %exponent19, %one_count20
  %output21 = insertvalue { double, i32 } zeroinitializer, double %div, 0
  %output22 = insertvalue { double, i32 } %output21, i32 %add, 1
  br label %if.merge16

if.else15:                                        ; preds = %if.else
  store i64 2, ptr %two, align 4
  %budget23 = load i64, ptr %budget3, align 4
  %two24 = load i64, ptr %two, align 4
  %div25 = udiv i64 %budget23, %two24
  store i64 %div25, ptr %half, align 4
  %scaled26 = load double, ptr %scaled1, align 8
  %exponent27 = load i32, ptr %exponent2, align 4
  %half28 = load i64, ptr %half, align 4
  %call = call { double, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6e6f726d616c697a655f666c6f61745f646f776e(double %scaled26, i32 %exponent27, i64 %half28)
  %output29 = extractvalue { double, i32 } %call, 0
  store double %output29, ptr %left, align 8
  %output30 = extractvalue { double, i32 } %call, 1
  store i32 %output30, ptr %left_exponent, align 4
  %left31 = load double, ptr %left, align 8
  %ten_value32 = load double, ptr %ten_value, align 8
  %ge33 = fcmp oge double %left31, %ten_value32
  br i1 %ge33, label %if.then34, label %if.else35

if.merge16:                                       ; preds = %if.merge36, %if.then14
  %if52 = phi { double, i32 } [ %output22, %if.then14 ], [ %if, %if.merge36 ]
  br label %if.merge

if.then34:                                        ; preds = %if.else15
  %left37 = load double, ptr %left, align 8
  %left_exponent38 = load i32, ptr %left_exponent, align 4
  %budget39 = load i64, ptr %budget3, align 4
  %half40 = load i64, ptr %half, align 4
  %sub = sub i64 %budget39, %half40
  %call41 = call { double, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6e6f726d616c697a655f666c6f61745f646f776e(double %left37, i32 %left_exponent38, i64 %sub)
  %output42 = extractvalue { double, i32 } %call41, 0
  store double %output42, ptr %right, align 8
  %output43 = extractvalue { double, i32 } %call41, 1
  store i32 %output43, ptr %right_exponent, align 4
  %right44 = load double, ptr %right, align 8
  %right_exponent45 = load i32, ptr %right_exponent, align 4
  %output46 = insertvalue { double, i32 } zeroinitializer, double %right44, 0
  %output47 = insertvalue { double, i32 } %output46, i32 %right_exponent45, 1
  br label %if.merge36

if.else35:                                        ; preds = %if.else15
  %left48 = load double, ptr %left, align 8
  %left_exponent49 = load i32, ptr %left_exponent, align 4
  %output50 = insertvalue { double, i32 } zeroinitializer, double %left48, 0
  %output51 = insertvalue { double, i32 } %output50, i32 %left_exponent49, 1
  br label %if.merge36

if.merge36:                                       ; preds = %if.else35, %if.then34
  %if = phi { double, i32 } [ %output47, %if.then34 ], [ %output51, %if.else35 ]
  br label %if.merge16
}

define { double, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6e6f726d616c697a655f666c6f61745f7570(double %scaled, i32 %exponent, i64 %budget) {
entry:
  %right_exponent = alloca i32, align 4
  %right = alloca double, align 8
  %left_exponent = alloca i32, align 4
  %left = alloca double, align 8
  %half = alloca i64, align 8
  %two = alloca i64, align 8
  %one_count = alloca i32, align 4
  %ten_value = alloca double, align 8
  %one = alloca i64, align 8
  %zero = alloca i64, align 8
  %one_value = alloca double, align 8
  %scaled1 = alloca double, align 8
  store double %scaled, ptr %scaled1, align 8
  %exponent2 = alloca i32, align 4
  store i32 %exponent, ptr %exponent2, align 4
  %budget3 = alloca i64, align 8
  store i64 %budget, ptr %budget3, align 4
  store double 1.000000e+00, ptr %one_value, align 8
  store i64 0, ptr %zero, align 4
  store i64 1, ptr %one, align 4
  %budget4 = load i64, ptr %budget3, align 4
  %zero5 = load i64, ptr %zero, align 4
  %eq = icmp eq i64 %budget4, %zero5
  br i1 %eq, label %short_circuit.merge, label %short_circuit.rhs

short_circuit.rhs:                                ; preds = %entry
  %scaled6 = load double, ptr %scaled1, align 8
  %one_value7 = load double, ptr %one_value, align 8
  %lt = fcmp olt double %scaled6, %one_value7
  %not = xor i1 %lt, true
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %entry
  %short_circuit = phi i1 [ true, %entry ], [ %not, %short_circuit.rhs ]
  br i1 %short_circuit, label %if.then, label %if.else

if.then:                                          ; preds = %short_circuit.merge
  %scaled8 = load double, ptr %scaled1, align 8
  %exponent9 = load i32, ptr %exponent2, align 4
  %output = insertvalue { double, i32 } zeroinitializer, double %scaled8, 0
  %output10 = insertvalue { double, i32 } %output, i32 %exponent9, 1
  br label %if.merge

if.else:                                          ; preds = %short_circuit.merge
  %budget11 = load i64, ptr %budget3, align 4
  %one12 = load i64, ptr %one, align 4
  %eq13 = icmp eq i64 %budget11, %one12
  br i1 %eq13, label %if.then14, label %if.else15

if.merge:                                         ; preds = %if.merge16, %if.then
  %if53 = phi { double, i32 } [ %output10, %if.then ], [ %if52, %if.merge16 ]
  ret { double, i32 } %if53

if.then14:                                        ; preds = %if.else
  store double 1.000000e+01, ptr %ten_value, align 8
  store i32 1, ptr %one_count, align 4
  %scaled17 = load double, ptr %scaled1, align 8
  %ten_value18 = load double, ptr %ten_value, align 8
  %mul = fmul double %scaled17, %ten_value18
  %exponent19 = load i32, ptr %exponent2, align 4
  %one_count20 = load i32, ptr %one_count, align 4
  %sub = sub i32 %exponent19, %one_count20
  %output21 = insertvalue { double, i32 } zeroinitializer, double %mul, 0
  %output22 = insertvalue { double, i32 } %output21, i32 %sub, 1
  br label %if.merge16

if.else15:                                        ; preds = %if.else
  store i64 2, ptr %two, align 4
  %budget23 = load i64, ptr %budget3, align 4
  %two24 = load i64, ptr %two, align 4
  %div = udiv i64 %budget23, %two24
  store i64 %div, ptr %half, align 4
  %scaled25 = load double, ptr %scaled1, align 8
  %exponent26 = load i32, ptr %exponent2, align 4
  %half27 = load i64, ptr %half, align 4
  %call = call { double, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6e6f726d616c697a655f666c6f61745f7570(double %scaled25, i32 %exponent26, i64 %half27)
  %output28 = extractvalue { double, i32 } %call, 0
  store double %output28, ptr %left, align 8
  %output29 = extractvalue { double, i32 } %call, 1
  store i32 %output29, ptr %left_exponent, align 4
  %left30 = load double, ptr %left, align 8
  %one_value31 = load double, ptr %one_value, align 8
  %lt32 = fcmp olt double %left30, %one_value31
  br i1 %lt32, label %if.then33, label %if.else34

if.merge16:                                       ; preds = %if.merge35, %if.then14
  %if52 = phi { double, i32 } [ %output22, %if.then14 ], [ %if, %if.merge35 ]
  br label %if.merge

if.then33:                                        ; preds = %if.else15
  %left36 = load double, ptr %left, align 8
  %left_exponent37 = load i32, ptr %left_exponent, align 4
  %budget38 = load i64, ptr %budget3, align 4
  %half39 = load i64, ptr %half, align 4
  %sub40 = sub i64 %budget38, %half39
  %call41 = call { double, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6e6f726d616c697a655f666c6f61745f7570(double %left36, i32 %left_exponent37, i64 %sub40)
  %output42 = extractvalue { double, i32 } %call41, 0
  store double %output42, ptr %right, align 8
  %output43 = extractvalue { double, i32 } %call41, 1
  store i32 %output43, ptr %right_exponent, align 4
  %right44 = load double, ptr %right, align 8
  %right_exponent45 = load i32, ptr %right_exponent, align 4
  %output46 = insertvalue { double, i32 } zeroinitializer, double %right44, 0
  %output47 = insertvalue { double, i32 } %output46, i32 %right_exponent45, 1
  br label %if.merge35

if.else34:                                        ; preds = %if.else15
  %left48 = load double, ptr %left, align 8
  %left_exponent49 = load i32, ptr %left_exponent, align 4
  %output50 = insertvalue { double, i32 } zeroinitializer, double %left48, 0
  %output51 = insertvalue { double, i32 } %output50, i32 %left_exponent49, 1
  br label %if.merge35

if.merge35:                                       ; preds = %if.else34, %if.then33
  %if = phi { double, i32 } [ %output47, %if.then33 ], [ %output51, %if.else34 ]
  br label %if.merge16
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f63617272795f663332(ptr %scratch, i64 %round_position, i64 %digit_start, i32 %exponent, i1 %rounding) {
entry:
  %assignment_value107 = alloca i32, align 4
  %assignment_value99 = alloca i8, align 1
  %one_count = alloca i32, align 4
  %one_char = alloca i8, align 1
  %leading = alloca i8, align 1
  %assignment_value82 = alloca i1, align 1
  %assignment_value77 = alloca i8, align 1
  %assignment_value70 = alloca i1, align 1
  %assignment_value65 = alloca i8, align 1
  %assignment_value59 = alloca i64, align 8
  %assignment_value51 = alloca i8, align 1
  %carry_done = alloca i1, align 1
  %carry_stop = alloca i1, align 1
  %carry_back = alloca i1, align 1
  %assignment_value = alloca i64, align 8
  %at_start = alloca i1, align 1
  %is_nine = alloca i1, align 1
  %is_point = alloca i1, align 1
  %current = alloca i8, align 1
  %one_digit = alloca i8, align 1
  %nine_digit = alloca i8, align 1
  %point = alloca i8, align 1
  %one = alloca i64, align 8
  %zero_digit = alloca i8, align 1
  %scratch1 = alloca ptr, align 8
  store ptr %scratch, ptr %scratch1, align 8
  %round_position2 = alloca i64, align 8
  store i64 %round_position, ptr %round_position2, align 4
  %digit_start3 = alloca i64, align 8
  store i64 %digit_start, ptr %digit_start3, align 4
  %exponent4 = alloca i32, align 4
  store i32 %exponent, ptr %exponent4, align 4
  %rounding5 = alloca i1, align 1
  store i1 %rounding, ptr %rounding5, align 1
  store i8 48, ptr %zero_digit, align 1
  %rounding6 = load i1, ptr %rounding5, align 1
  br i1 %rounding6, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  store i64 1, ptr %one, align 4
  store i8 46, ptr %point, align 1
  store i8 57, ptr %nine_digit, align 1
  store i8 1, ptr %one_digit, align 1
  %scratch7 = load ptr, ptr %scratch1, align 8
  %round_position8 = load i64, ptr %round_position2, align 4
  %array_element = getelementptr inbounds i8, ptr %scratch7, i64 %round_position8
  %place = load i8, ptr %array_element, align 1
  store i8 %place, ptr %current, align 1
  %current9 = load i8, ptr %current, align 1
  %point10 = load i8, ptr %point, align 1
  %eq = icmp eq i8 %current9, %point10
  store i1 %eq, ptr %is_point, align 1
  %current11 = load i8, ptr %current, align 1
  %nine_digit12 = load i8, ptr %nine_digit, align 1
  %eq13 = icmp eq i8 %current11, %nine_digit12
  store i1 %eq13, ptr %is_nine, align 1
  %round_position14 = load i64, ptr %round_position2, align 4
  %digit_start15 = load i64, ptr %digit_start3, align 4
  %eq16 = icmp eq i64 %round_position14, %digit_start15
  store i1 %eq16, ptr %at_start, align 1
  %is_point17 = load i1, ptr %is_point, align 1
  br i1 %is_point17, label %if.then18, label %if.merge19

if.else:                                          ; preds = %entry
  %scratch89 = load ptr, ptr %scratch1, align 8
  %digit_start90 = load i64, ptr %digit_start3, align 4
  %array_element91 = getelementptr inbounds i8, ptr %scratch89, i64 %digit_start90
  %place92 = load i8, ptr %array_element91, align 1
  store i8 %place92, ptr %leading, align 1
  %leading93 = load i8, ptr %leading, align 1
  %zero_digit94 = load i8, ptr %zero_digit, align 1
  %eq95 = icmp eq i8 %leading93, %zero_digit94
  br i1 %eq95, label %if.then96, label %if.merge97

if.merge:                                         ; preds = %if.merge97, %if.merge74
  %if = phi i32 [ %call, %if.merge74 ], [ %exponent109, %if.merge97 ]
  ret i32 %if

if.then18:                                        ; preds = %if.then
  %round_position20 = load i64, ptr %round_position2, align 4
  %one21 = load i64, ptr %one, align 4
  %sub = sub i64 %round_position20, %one21
  store i64 %sub, ptr %assignment_value, align 4
  %assignment_value22 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value22, ptr %round_position2, align 4
  br label %if.merge19

if.merge19:                                       ; preds = %if.then18, %if.then
  %is_point23 = load i1, ptr %is_point, align 1
  %not = xor i1 %is_point23, true
  br i1 %not, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %if.merge19
  %is_nine24 = load i1, ptr %is_nine, align 1
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %if.merge19
  %short_circuit = phi i1 [ false, %if.merge19 ], [ %is_nine24, %short_circuit.rhs ]
  br i1 %short_circuit, label %short_circuit.rhs25, label %short_circuit.merge26

short_circuit.rhs25:                              ; preds = %short_circuit.merge
  %at_start27 = load i1, ptr %at_start, align 1
  %not28 = xor i1 %at_start27, true
  br label %short_circuit.merge26

short_circuit.merge26:                            ; preds = %short_circuit.rhs25, %short_circuit.merge
  %short_circuit29 = phi i1 [ false, %short_circuit.merge ], [ %not28, %short_circuit.rhs25 ]
  store i1 %short_circuit29, ptr %carry_back, align 1
  %is_point30 = load i1, ptr %is_point, align 1
  %not31 = xor i1 %is_point30, true
  br i1 %not31, label %short_circuit.rhs32, label %short_circuit.merge33

short_circuit.rhs32:                              ; preds = %short_circuit.merge26
  %is_nine34 = load i1, ptr %is_nine, align 1
  br label %short_circuit.merge33

short_circuit.merge33:                            ; preds = %short_circuit.rhs32, %short_circuit.merge26
  %short_circuit35 = phi i1 [ false, %short_circuit.merge26 ], [ %is_nine34, %short_circuit.rhs32 ]
  br i1 %short_circuit35, label %short_circuit.rhs36, label %short_circuit.merge37

short_circuit.rhs36:                              ; preds = %short_circuit.merge33
  %at_start38 = load i1, ptr %at_start, align 1
  br label %short_circuit.merge37

short_circuit.merge37:                            ; preds = %short_circuit.rhs36, %short_circuit.merge33
  %short_circuit39 = phi i1 [ false, %short_circuit.merge33 ], [ %at_start38, %short_circuit.rhs36 ]
  store i1 %short_circuit39, ptr %carry_stop, align 1
  %is_point40 = load i1, ptr %is_point, align 1
  %not41 = xor i1 %is_point40, true
  br i1 %not41, label %short_circuit.rhs42, label %short_circuit.merge43

short_circuit.rhs42:                              ; preds = %short_circuit.merge37
  %is_nine44 = load i1, ptr %is_nine, align 1
  %not45 = xor i1 %is_nine44, true
  br label %short_circuit.merge43

short_circuit.merge43:                            ; preds = %short_circuit.rhs42, %short_circuit.merge37
  %short_circuit46 = phi i1 [ false, %short_circuit.merge37 ], [ %not45, %short_circuit.rhs42 ]
  store i1 %short_circuit46, ptr %carry_done, align 1
  %carry_back47 = load i1, ptr %carry_back, align 1
  br i1 %carry_back47, label %if.then48, label %if.merge49

if.then48:                                        ; preds = %short_circuit.merge43
  %zero_digit50 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit50, ptr %assignment_value51, align 1
  %assignment_value52 = load i8, ptr %assignment_value51, align 1
  %scratch53 = load ptr, ptr %scratch1, align 8
  %round_position54 = load i64, ptr %round_position2, align 4
  %array_element55 = getelementptr inbounds i8, ptr %scratch53, i64 %round_position54
  store i8 %assignment_value52, ptr %array_element55, align 1
  %round_position56 = load i64, ptr %round_position2, align 4
  %one57 = load i64, ptr %one, align 4
  %sub58 = sub i64 %round_position56, %one57
  store i64 %sub58, ptr %assignment_value59, align 4
  %assignment_value60 = load i64, ptr %assignment_value59, align 4
  store i64 %assignment_value60, ptr %round_position2, align 4
  br label %if.merge49

if.merge49:                                       ; preds = %if.then48, %short_circuit.merge43
  %carry_stop61 = load i1, ptr %carry_stop, align 1
  br i1 %carry_stop61, label %if.then62, label %if.merge63

if.then62:                                        ; preds = %if.merge49
  %zero_digit64 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit64, ptr %assignment_value65, align 1
  %assignment_value66 = load i8, ptr %assignment_value65, align 1
  %scratch67 = load ptr, ptr %scratch1, align 8
  %round_position68 = load i64, ptr %round_position2, align 4
  %array_element69 = getelementptr inbounds i8, ptr %scratch67, i64 %round_position68
  store i8 %assignment_value66, ptr %array_element69, align 1
  store i1 false, ptr %assignment_value70, align 1
  %assignment_value71 = load i1, ptr %assignment_value70, align 1
  store i1 %assignment_value71, ptr %rounding5, align 1
  br label %if.merge63

if.merge63:                                       ; preds = %if.then62, %if.merge49
  %carry_done72 = load i1, ptr %carry_done, align 1
  br i1 %carry_done72, label %if.then73, label %if.merge74

if.then73:                                        ; preds = %if.merge63
  %current75 = load i8, ptr %current, align 1
  %one_digit76 = load i8, ptr %one_digit, align 1
  %add = add i8 %current75, %one_digit76
  store i8 %add, ptr %assignment_value77, align 1
  %assignment_value78 = load i8, ptr %assignment_value77, align 1
  %scratch79 = load ptr, ptr %scratch1, align 8
  %round_position80 = load i64, ptr %round_position2, align 4
  %array_element81 = getelementptr inbounds i8, ptr %scratch79, i64 %round_position80
  store i8 %assignment_value78, ptr %array_element81, align 1
  store i1 false, ptr %assignment_value82, align 1
  %assignment_value83 = load i1, ptr %assignment_value82, align 1
  store i1 %assignment_value83, ptr %rounding5, align 1
  br label %if.merge74

if.merge74:                                       ; preds = %if.then73, %if.merge63
  %scratch84 = load ptr, ptr %scratch1, align 8
  %round_position85 = load i64, ptr %round_position2, align 4
  %digit_start86 = load i64, ptr %digit_start3, align 4
  %exponent87 = load i32, ptr %exponent4, align 4
  %rounding88 = load i1, ptr %rounding5, align 1
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f63617272795f663332(ptr %scratch84, i64 %round_position85, i64 %digit_start86, i32 %exponent87, i1 %rounding88)
  br label %if.merge

if.then96:                                        ; preds = %if.else
  store i8 49, ptr %one_char, align 1
  store i32 1, ptr %one_count, align 4
  %one_char98 = load i8, ptr %one_char, align 1
  store i8 %one_char98, ptr %assignment_value99, align 1
  %assignment_value100 = load i8, ptr %assignment_value99, align 1
  %scratch101 = load ptr, ptr %scratch1, align 8
  %digit_start102 = load i64, ptr %digit_start3, align 4
  %array_element103 = getelementptr inbounds i8, ptr %scratch101, i64 %digit_start102
  store i8 %assignment_value100, ptr %array_element103, align 1
  %exponent104 = load i32, ptr %exponent4, align 4
  %one_count105 = load i32, ptr %one_count, align 4
  %add106 = add i32 %exponent104, %one_count105
  store i32 %add106, ptr %assignment_value107, align 4
  %assignment_value108 = load i32, ptr %assignment_value107, align 4
  store i32 %assignment_value108, ptr %exponent4, align 4
  br label %if.merge97

if.merge97:                                       ; preds = %if.then96, %if.else
  %exponent109 = load i32, ptr %exponent4, align 4
  br label %if.merge
}

define { i64, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f663332(ptr %scratch, i64 %digit_start, i64 %emit, i64 %produced, double %scaled, i32 %exponent) {
entry:
  %assignment_value69 = alloca i32, align 4
  %five_value = alloca double, align 8
  %result_exponent = alloca i32, align 4
  %result_emit = alloca i64, align 8
  %assignment_value43 = alloca double, align 8
  %assignment_value39 = alloca double, align 8
  %whole = alloca double, align 8
  %assignment_value34 = alloca i64, align 8
  %assignment_value26 = alloca i8, align 1
  %assignment_value19 = alloca i64, align 8
  %assignment_value = alloca i8, align 1
  %digit = alloca i8, align 1
  %digit_value = alloca i32, align 4
  %ten_value = alloca double, align 8
  %point = alloca i8, align 1
  %zero_digit = alloca i8, align 1
  %zero = alloca i64, align 8
  %one = alloca i64, align 8
  %nine = alloca i64, align 8
  %scratch1 = alloca ptr, align 8
  store ptr %scratch, ptr %scratch1, align 8
  %digit_start2 = alloca i64, align 8
  store i64 %digit_start, ptr %digit_start2, align 4
  %emit3 = alloca i64, align 8
  store i64 %emit, ptr %emit3, align 4
  %produced4 = alloca i64, align 8
  store i64 %produced, ptr %produced4, align 4
  %scaled5 = alloca double, align 8
  store double %scaled, ptr %scaled5, align 8
  %exponent6 = alloca i32, align 4
  store i32 %exponent, ptr %exponent6, align 4
  store i64 9, ptr %nine, align 4
  store i64 1, ptr %one, align 4
  %produced7 = load i64, ptr %produced4, align 4
  %nine8 = load i64, ptr %nine, align 4
  %lt = icmp ult i64 %produced7, %nine8
  br i1 %lt, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  store i64 0, ptr %zero, align 4
  store i8 48, ptr %zero_digit, align 1
  store i8 46, ptr %point, align 1
  store double 1.000000e+01, ptr %ten_value, align 8
  %scaled9 = load double, ptr %scaled5, align 8
  %float_to_sint_trunc = fptosi double %scaled9 to i32
  store i32 %float_to_sint_trunc, ptr %digit_value, align 4
  %digit_value10 = load i32, ptr %digit_value, align 4
  %int_trunc = trunc i32 %digit_value10 to i8
  store i8 %int_trunc, ptr %digit, align 1
  %zero_digit11 = load i8, ptr %zero_digit, align 1
  %digit12 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit11, %digit12
  store i8 %add, ptr %assignment_value, align 1
  %assignment_value13 = load i8, ptr %assignment_value, align 1
  %scratch14 = load ptr, ptr %scratch1, align 8
  %emit15 = load i64, ptr %emit3, align 4
  %array_element = getelementptr inbounds i8, ptr %scratch14, i64 %emit15
  store i8 %assignment_value13, ptr %array_element, align 1
  %emit16 = load i64, ptr %emit3, align 4
  %one17 = load i64, ptr %one, align 4
  %add18 = add i64 %emit16, %one17
  store i64 %add18, ptr %assignment_value19, align 4
  %assignment_value20 = load i64, ptr %assignment_value19, align 4
  store i64 %assignment_value20, ptr %emit3, align 4
  %produced21 = load i64, ptr %produced4, align 4
  %zero22 = load i64, ptr %zero, align 4
  %eq = icmp eq i64 %produced21, %zero22
  br i1 %eq, label %if.then23, label %if.merge24

if.else:                                          ; preds = %entry
  store double 5.000000e+00, ptr %five_value, align 8
  %scaled58 = load double, ptr %scaled5, align 8
  %five_value59 = load double, ptr %five_value, align 8
  %ge = fcmp oge double %scaled58, %five_value59
  br i1 %ge, label %if.then60, label %if.merge61

if.merge:                                         ; preds = %if.merge61, %if.merge24
  %if = phi { i64, i32 } [ %output57, %if.merge24 ], [ %output74, %if.merge61 ]
  ret { i64, i32 } %if

if.then23:                                        ; preds = %if.then
  %point25 = load i8, ptr %point, align 1
  store i8 %point25, ptr %assignment_value26, align 1
  %assignment_value27 = load i8, ptr %assignment_value26, align 1
  %scratch28 = load ptr, ptr %scratch1, align 8
  %emit29 = load i64, ptr %emit3, align 4
  %array_element30 = getelementptr inbounds i8, ptr %scratch28, i64 %emit29
  store i8 %assignment_value27, ptr %array_element30, align 1
  %emit31 = load i64, ptr %emit3, align 4
  %one32 = load i64, ptr %one, align 4
  %add33 = add i64 %emit31, %one32
  store i64 %add33, ptr %assignment_value34, align 4
  %assignment_value35 = load i64, ptr %assignment_value34, align 4
  store i64 %assignment_value35, ptr %emit3, align 4
  br label %if.merge24

if.merge24:                                       ; preds = %if.then23, %if.then
  %digit_value36 = load i32, ptr %digit_value, align 4
  %sint_to_float = sitofp i32 %digit_value36 to double
  store double %sint_to_float, ptr %whole, align 8
  %scaled37 = load double, ptr %scaled5, align 8
  %whole38 = load double, ptr %whole, align 8
  %sub = fsub double %scaled37, %whole38
  store double %sub, ptr %assignment_value39, align 8
  %assignment_value40 = load double, ptr %assignment_value39, align 8
  store double %assignment_value40, ptr %scaled5, align 8
  %scaled41 = load double, ptr %scaled5, align 8
  %ten_value42 = load double, ptr %ten_value, align 8
  %mul = fmul double %scaled41, %ten_value42
  store double %mul, ptr %assignment_value43, align 8
  %assignment_value44 = load double, ptr %assignment_value43, align 8
  store double %assignment_value44, ptr %scaled5, align 8
  %scratch45 = load ptr, ptr %scratch1, align 8
  %digit_start46 = load i64, ptr %digit_start2, align 4
  %emit47 = load i64, ptr %emit3, align 4
  %produced48 = load i64, ptr %produced4, align 4
  %one49 = load i64, ptr %one, align 4
  %add50 = add i64 %produced48, %one49
  %scaled51 = load double, ptr %scaled5, align 8
  %exponent52 = load i32, ptr %exponent6, align 4
  %call = call { i64, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f663332(ptr %scratch45, i64 %digit_start46, i64 %emit47, i64 %add50, double %scaled51, i32 %exponent52)
  %output = extractvalue { i64, i32 } %call, 0
  store i64 %output, ptr %result_emit, align 4
  %output53 = extractvalue { i64, i32 } %call, 1
  store i32 %output53, ptr %result_exponent, align 4
  %result_emit54 = load i64, ptr %result_emit, align 4
  %result_exponent55 = load i32, ptr %result_exponent, align 4
  %output56 = insertvalue { i64, i32 } zeroinitializer, i64 %result_emit54, 0
  %output57 = insertvalue { i64, i32 } %output56, i32 %result_exponent55, 1
  br label %if.merge

if.then60:                                        ; preds = %if.else
  %scratch62 = load ptr, ptr %scratch1, align 8
  %emit63 = load i64, ptr %emit3, align 4
  %one64 = load i64, ptr %one, align 4
  %sub65 = sub i64 %emit63, %one64
  %digit_start66 = load i64, ptr %digit_start2, align 4
  %exponent67 = load i32, ptr %exponent6, align 4
  %call68 = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f63617272795f663332(ptr %scratch62, i64 %sub65, i64 %digit_start66, i32 %exponent67, i1 true)
  store i32 %call68, ptr %assignment_value69, align 4
  %assignment_value70 = load i32, ptr %assignment_value69, align 4
  store i32 %assignment_value70, ptr %exponent6, align 4
  br label %if.merge61

if.merge61:                                       ; preds = %if.then60, %if.else
  %emit71 = load i64, ptr %emit3, align 4
  %exponent72 = load i32, ptr %exponent6, align 4
  %output73 = insertvalue { i64, i32 } zeroinitializer, i64 %emit71, 0
  %output74 = insertvalue { i64, i32 } %output73, i32 %exponent72, 1
  br label %if.merge
}

define internal ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f663332(float %value, ptr %0) #0 {
entry:
  %text = alloca [16 x i8], align 8
  %struct_literal = alloca [16 x i8], align 8
  %assignment_value180 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value177 = alloca i64, align 8
  %assignment_value170 = alloca i8, align 1
  %one_digit_out = alloca i8, align 1
  %assignment_value163 = alloca i64, align 8
  %assignment_value156 = alloca i8, align 1
  %ten_digit = alloca i8, align 1
  %assignment_value146 = alloca i64, align 8
  %assignment_value139 = alloca i8, align 1
  %hundred_digit = alloca i8, align 1
  %assignment_value128 = alloca i1, align 1
  %assignment_value121 = alloca i1, align 1
  %show_tens = alloca i1, align 1
  %ones = alloca i32, align 4
  %tens = alloca i32, align 4
  %after_hundreds = alloca i32, align 4
  %hundreds = alloca i32, align 4
  %ten_scale = alloca i32, align 4
  %hundred_scale = alloca i32, align 4
  %assignment_value100 = alloca i32, align 4
  %assignment_value95 = alloca i64, align 8
  %assignment_value88 = alloca i8, align 1
  %remaining_exponent = alloca i32, align 4
  %assignment_value79 = alloca i64, align 8
  %assignment_value72 = alloca i8, align 1
  %assignment_value69 = alloca i64, align 8
  %exponent = alloca i32, align 4
  %digit_emit = alloca i64, align 8
  %normalized_exponent = alloca i32, align 4
  %scaled = alloca double, align 8
  %down_exponent = alloca i32, align 4
  %down = alloca double, align 8
  %budget = alloca i64, align 8
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
  %zero_count = alloca i32, align 4
  %exponent_mark = alloca i8, align 1
  %minus = alloca i8, align 1
  %zero_digit = alloca i8, align 1
  %one_value = alloca double, align 8
  %zero_value = alloca double, align 8
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
  store double 0.000000e+00, ptr %zero_value, align 8
  store double 1.000000e+00, ptr %one_value, align 8
  store i8 48, ptr %zero_digit, align 1
  store i8 45, ptr %minus, align 1
  store i8 101, ptr %exponent_mark, align 1
  store i32 0, ptr %zero_count, align 4
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
  store i64 50, ptr %budget, align 4
  %magnitude50 = load double, ptr %magnitude, align 8
  %zero_count51 = load i32, ptr %zero_count, align 4
  %budget52 = load i64, ptr %budget, align 4
  %call = call { double, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6e6f726d616c697a655f666c6f61745f646f776e(double %magnitude50, i32 %zero_count51, i64 %budget52)
  %output = extractvalue { double, i32 } %call, 0
  store double %output, ptr %down, align 8
  %output53 = extractvalue { double, i32 } %call, 1
  store i32 %output53, ptr %down_exponent, align 4
  %down54 = load double, ptr %down, align 8
  %down_exponent55 = load i32, ptr %down_exponent, align 4
  %budget56 = load i64, ptr %budget, align 4
  %call57 = call { double, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6e6f726d616c697a655f666c6f61745f7570(double %down54, i32 %down_exponent55, i64 %budget56)
  %output58 = extractvalue { double, i32 } %call57, 0
  store double %output58, ptr %scaled, align 8
  %output59 = extractvalue { double, i32 } %call57, 1
  store i32 %output59, ptr %normalized_exponent, align 4
  %emit60 = load i64, ptr %emit, align 4
  %emit61 = load i64, ptr %emit, align 4
  %zero62 = load i64, ptr %zero, align 4
  %scaled63 = load double, ptr %scaled, align 8
  %normalized_exponent64 = load i32, ptr %normalized_exponent, align 4
  %call65 = call { i64, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f663332(ptr %allocation, i64 %emit60, i64 %emit61, i64 %zero62, double %scaled63, i32 %normalized_exponent64)
  %output66 = extractvalue { i64, i32 } %call65, 0
  store i64 %output66, ptr %digit_emit, align 4
  %output67 = extractvalue { i64, i32 } %call65, 1
  store i32 %output67, ptr %exponent, align 4
  %digit_emit68 = load i64, ptr %digit_emit, align 4
  store i64 %digit_emit68, ptr %assignment_value69, align 4
  %assignment_value70 = load i64, ptr %assignment_value69, align 4
  store i64 %assignment_value70, ptr %emit, align 4
  %exponent_mark71 = load i8, ptr %exponent_mark, align 1
  store i8 %exponent_mark71, ptr %assignment_value72, align 1
  %assignment_value73 = load i8, ptr %assignment_value72, align 1
  %emit74 = load i64, ptr %emit, align 4
  %array_element75 = getelementptr inbounds i8, ptr %allocation, i64 %emit74
  store i8 %assignment_value73, ptr %array_element75, align 1
  %emit76 = load i64, ptr %emit, align 4
  %one77 = load i64, ptr %one, align 4
  %add78 = add i64 %emit76, %one77
  store i64 %add78, ptr %assignment_value79, align 4
  %assignment_value80 = load i64, ptr %assignment_value79, align 4
  store i64 %assignment_value80, ptr %emit, align 4
  %exponent81 = load i32, ptr %exponent, align 4
  store i32 %exponent81, ptr %remaining_exponent, align 4
  %remaining_exponent82 = load i32, ptr %remaining_exponent, align 4
  %zero_count83 = load i32, ptr %zero_count, align 4
  %lt84 = icmp slt i32 %remaining_exponent82, %zero_count83
  br i1 %lt84, label %if.then85, label %if.merge86

if.merge23:                                       ; preds = %if.merge150, %if.merge30
  store i32 0, ptr %data, align 4
  %array_element179 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element179 to i32
  store i32 %address, ptr %assignment_value180, align 4
  %assignment_value181 = load i32, ptr %assignment_value180, align 4
  store i32 %assignment_value181, ptr %data, align 4
  %data182 = load i32, ptr %data, align 4
  %emit183 = load i64, ptr %emit, align 4
  %data184 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data182, ptr %data184, align 4
  %length = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %emit183, ptr %length, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text, ptr align 1 %struct_literal, i32 16, i1 false)
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %0, ptr align 1 %text, i32 16, i1 false)
  ret ptr %allocation

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

if.then85:                                        ; preds = %if.else
  %minus87 = load i8, ptr %minus, align 1
  store i8 %minus87, ptr %assignment_value88, align 1
  %assignment_value89 = load i8, ptr %assignment_value88, align 1
  %emit90 = load i64, ptr %emit, align 4
  %array_element91 = getelementptr inbounds i8, ptr %allocation, i64 %emit90
  store i8 %assignment_value89, ptr %array_element91, align 1
  %emit92 = load i64, ptr %emit, align 4
  %one93 = load i64, ptr %one, align 4
  %add94 = add i64 %emit92, %one93
  store i64 %add94, ptr %assignment_value95, align 4
  %assignment_value96 = load i64, ptr %assignment_value95, align 4
  store i64 %assignment_value96, ptr %emit, align 4
  %zero_count97 = load i32, ptr %zero_count, align 4
  %remaining_exponent98 = load i32, ptr %remaining_exponent, align 4
  %sub99 = sub i32 %zero_count97, %remaining_exponent98
  store i32 %sub99, ptr %assignment_value100, align 4
  %assignment_value101 = load i32, ptr %assignment_value100, align 4
  store i32 %assignment_value101, ptr %remaining_exponent, align 4
  br label %if.merge86

if.merge86:                                       ; preds = %if.then85, %if.else
  store i32 100, ptr %hundred_scale, align 4
  store i32 10, ptr %ten_scale, align 4
  %remaining_exponent102 = load i32, ptr %remaining_exponent, align 4
  %hundred_scale103 = load i32, ptr %hundred_scale, align 4
  %div104 = sdiv i32 %remaining_exponent102, %hundred_scale103
  store i32 %div104, ptr %hundreds, align 4
  %remaining_exponent105 = load i32, ptr %remaining_exponent, align 4
  %hundreds106 = load i32, ptr %hundreds, align 4
  %hundred_scale107 = load i32, ptr %hundred_scale, align 4
  %mul = mul i32 %hundreds106, %hundred_scale107
  %sub108 = sub i32 %remaining_exponent105, %mul
  store i32 %sub108, ptr %after_hundreds, align 4
  %after_hundreds109 = load i32, ptr %after_hundreds, align 4
  %ten_scale110 = load i32, ptr %ten_scale, align 4
  %div111 = sdiv i32 %after_hundreds109, %ten_scale110
  store i32 %div111, ptr %tens, align 4
  %after_hundreds112 = load i32, ptr %after_hundreds, align 4
  %tens113 = load i32, ptr %tens, align 4
  %ten_scale114 = load i32, ptr %ten_scale, align 4
  %mul115 = mul i32 %tens113, %ten_scale114
  %sub116 = sub i32 %after_hundreds112, %mul115
  store i32 %sub116, ptr %ones, align 4
  store i1 false, ptr %show_tens, align 1
  %hundreds117 = load i32, ptr %hundreds, align 4
  %zero_count118 = load i32, ptr %zero_count, align 4
  %ne = icmp ne i32 %hundreds117, %zero_count118
  br i1 %ne, label %if.then119, label %if.merge120

if.then119:                                       ; preds = %if.merge86
  store i1 true, ptr %assignment_value121, align 1
  %assignment_value122 = load i1, ptr %assignment_value121, align 1
  store i1 %assignment_value122, ptr %show_tens, align 1
  br label %if.merge120

if.merge120:                                      ; preds = %if.then119, %if.merge86
  %tens123 = load i32, ptr %tens, align 4
  %zero_count124 = load i32, ptr %zero_count, align 4
  %ne125 = icmp ne i32 %tens123, %zero_count124
  br i1 %ne125, label %if.then126, label %if.merge127

if.then126:                                       ; preds = %if.merge120
  store i1 true, ptr %assignment_value128, align 1
  %assignment_value129 = load i1, ptr %assignment_value128, align 1
  store i1 %assignment_value129, ptr %show_tens, align 1
  br label %if.merge127

if.merge127:                                      ; preds = %if.then126, %if.merge120
  %hundreds130 = load i32, ptr %hundreds, align 4
  %zero_count131 = load i32, ptr %zero_count, align 4
  %ne132 = icmp ne i32 %hundreds130, %zero_count131
  br i1 %ne132, label %if.then133, label %if.merge134

if.then133:                                       ; preds = %if.merge127
  %hundreds135 = load i32, ptr %hundreds, align 4
  %int_trunc = trunc i32 %hundreds135 to i8
  store i8 %int_trunc, ptr %hundred_digit, align 1
  %zero_digit136 = load i8, ptr %zero_digit, align 1
  %hundred_digit137 = load i8, ptr %hundred_digit, align 1
  %add138 = add i8 %zero_digit136, %hundred_digit137
  store i8 %add138, ptr %assignment_value139, align 1
  %assignment_value140 = load i8, ptr %assignment_value139, align 1
  %emit141 = load i64, ptr %emit, align 4
  %array_element142 = getelementptr inbounds i8, ptr %allocation, i64 %emit141
  store i8 %assignment_value140, ptr %array_element142, align 1
  %emit143 = load i64, ptr %emit, align 4
  %one144 = load i64, ptr %one, align 4
  %add145 = add i64 %emit143, %one144
  store i64 %add145, ptr %assignment_value146, align 4
  %assignment_value147 = load i64, ptr %assignment_value146, align 4
  store i64 %assignment_value147, ptr %emit, align 4
  br label %if.merge134

if.merge134:                                      ; preds = %if.then133, %if.merge127
  %show_tens148 = load i1, ptr %show_tens, align 1
  br i1 %show_tens148, label %if.then149, label %if.merge150

if.then149:                                       ; preds = %if.merge134
  %tens151 = load i32, ptr %tens, align 4
  %int_trunc152 = trunc i32 %tens151 to i8
  store i8 %int_trunc152, ptr %ten_digit, align 1
  %zero_digit153 = load i8, ptr %zero_digit, align 1
  %ten_digit154 = load i8, ptr %ten_digit, align 1
  %add155 = add i8 %zero_digit153, %ten_digit154
  store i8 %add155, ptr %assignment_value156, align 1
  %assignment_value157 = load i8, ptr %assignment_value156, align 1
  %emit158 = load i64, ptr %emit, align 4
  %array_element159 = getelementptr inbounds i8, ptr %allocation, i64 %emit158
  store i8 %assignment_value157, ptr %array_element159, align 1
  %emit160 = load i64, ptr %emit, align 4
  %one161 = load i64, ptr %one, align 4
  %add162 = add i64 %emit160, %one161
  store i64 %add162, ptr %assignment_value163, align 4
  %assignment_value164 = load i64, ptr %assignment_value163, align 4
  store i64 %assignment_value164, ptr %emit, align 4
  br label %if.merge150

if.merge150:                                      ; preds = %if.then149, %if.merge134
  %ones165 = load i32, ptr %ones, align 4
  %int_trunc166 = trunc i32 %ones165 to i8
  store i8 %int_trunc166, ptr %one_digit_out, align 1
  %zero_digit167 = load i8, ptr %zero_digit, align 1
  %one_digit_out168 = load i8, ptr %one_digit_out, align 1
  %add169 = add i8 %zero_digit167, %one_digit_out168
  store i8 %add169, ptr %assignment_value170, align 1
  %assignment_value171 = load i8, ptr %assignment_value170, align 1
  %emit172 = load i64, ptr %emit, align 4
  %array_element173 = getelementptr inbounds i8, ptr %allocation, i64 %emit172
  store i8 %assignment_value171, ptr %array_element173, align 1
  %emit174 = load i64, ptr %emit, align 4
  %one175 = load i64, ptr %one, align 4
  %add176 = add i64 %emit174, %one175
  store i64 %add176, ptr %assignment_value177, align 4
  %assignment_value178 = load i64, ptr %assignment_value177, align 4
  store i64 %assignment_value178, ptr %emit, align 4
  br label %if.merge23
}

define i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f63617272795f663634(ptr %scratch, i64 %round_position, i64 %digit_start, i32 %exponent, i1 %rounding) {
entry:
  %assignment_value107 = alloca i32, align 4
  %assignment_value99 = alloca i8, align 1
  %one_count = alloca i32, align 4
  %one_char = alloca i8, align 1
  %leading = alloca i8, align 1
  %assignment_value82 = alloca i1, align 1
  %assignment_value77 = alloca i8, align 1
  %assignment_value70 = alloca i1, align 1
  %assignment_value65 = alloca i8, align 1
  %assignment_value59 = alloca i64, align 8
  %assignment_value51 = alloca i8, align 1
  %carry_done = alloca i1, align 1
  %carry_stop = alloca i1, align 1
  %carry_back = alloca i1, align 1
  %assignment_value = alloca i64, align 8
  %at_start = alloca i1, align 1
  %is_nine = alloca i1, align 1
  %is_point = alloca i1, align 1
  %current = alloca i8, align 1
  %one_digit = alloca i8, align 1
  %nine_digit = alloca i8, align 1
  %point = alloca i8, align 1
  %one = alloca i64, align 8
  %zero_digit = alloca i8, align 1
  %scratch1 = alloca ptr, align 8
  store ptr %scratch, ptr %scratch1, align 8
  %round_position2 = alloca i64, align 8
  store i64 %round_position, ptr %round_position2, align 4
  %digit_start3 = alloca i64, align 8
  store i64 %digit_start, ptr %digit_start3, align 4
  %exponent4 = alloca i32, align 4
  store i32 %exponent, ptr %exponent4, align 4
  %rounding5 = alloca i1, align 1
  store i1 %rounding, ptr %rounding5, align 1
  store i8 48, ptr %zero_digit, align 1
  %rounding6 = load i1, ptr %rounding5, align 1
  br i1 %rounding6, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  store i64 1, ptr %one, align 4
  store i8 46, ptr %point, align 1
  store i8 57, ptr %nine_digit, align 1
  store i8 1, ptr %one_digit, align 1
  %scratch7 = load ptr, ptr %scratch1, align 8
  %round_position8 = load i64, ptr %round_position2, align 4
  %array_element = getelementptr inbounds i8, ptr %scratch7, i64 %round_position8
  %place = load i8, ptr %array_element, align 1
  store i8 %place, ptr %current, align 1
  %current9 = load i8, ptr %current, align 1
  %point10 = load i8, ptr %point, align 1
  %eq = icmp eq i8 %current9, %point10
  store i1 %eq, ptr %is_point, align 1
  %current11 = load i8, ptr %current, align 1
  %nine_digit12 = load i8, ptr %nine_digit, align 1
  %eq13 = icmp eq i8 %current11, %nine_digit12
  store i1 %eq13, ptr %is_nine, align 1
  %round_position14 = load i64, ptr %round_position2, align 4
  %digit_start15 = load i64, ptr %digit_start3, align 4
  %eq16 = icmp eq i64 %round_position14, %digit_start15
  store i1 %eq16, ptr %at_start, align 1
  %is_point17 = load i1, ptr %is_point, align 1
  br i1 %is_point17, label %if.then18, label %if.merge19

if.else:                                          ; preds = %entry
  %scratch89 = load ptr, ptr %scratch1, align 8
  %digit_start90 = load i64, ptr %digit_start3, align 4
  %array_element91 = getelementptr inbounds i8, ptr %scratch89, i64 %digit_start90
  %place92 = load i8, ptr %array_element91, align 1
  store i8 %place92, ptr %leading, align 1
  %leading93 = load i8, ptr %leading, align 1
  %zero_digit94 = load i8, ptr %zero_digit, align 1
  %eq95 = icmp eq i8 %leading93, %zero_digit94
  br i1 %eq95, label %if.then96, label %if.merge97

if.merge:                                         ; preds = %if.merge97, %if.merge74
  %if = phi i32 [ %call, %if.merge74 ], [ %exponent109, %if.merge97 ]
  ret i32 %if

if.then18:                                        ; preds = %if.then
  %round_position20 = load i64, ptr %round_position2, align 4
  %one21 = load i64, ptr %one, align 4
  %sub = sub i64 %round_position20, %one21
  store i64 %sub, ptr %assignment_value, align 4
  %assignment_value22 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value22, ptr %round_position2, align 4
  br label %if.merge19

if.merge19:                                       ; preds = %if.then18, %if.then
  %is_point23 = load i1, ptr %is_point, align 1
  %not = xor i1 %is_point23, true
  br i1 %not, label %short_circuit.rhs, label %short_circuit.merge

short_circuit.rhs:                                ; preds = %if.merge19
  %is_nine24 = load i1, ptr %is_nine, align 1
  br label %short_circuit.merge

short_circuit.merge:                              ; preds = %short_circuit.rhs, %if.merge19
  %short_circuit = phi i1 [ false, %if.merge19 ], [ %is_nine24, %short_circuit.rhs ]
  br i1 %short_circuit, label %short_circuit.rhs25, label %short_circuit.merge26

short_circuit.rhs25:                              ; preds = %short_circuit.merge
  %at_start27 = load i1, ptr %at_start, align 1
  %not28 = xor i1 %at_start27, true
  br label %short_circuit.merge26

short_circuit.merge26:                            ; preds = %short_circuit.rhs25, %short_circuit.merge
  %short_circuit29 = phi i1 [ false, %short_circuit.merge ], [ %not28, %short_circuit.rhs25 ]
  store i1 %short_circuit29, ptr %carry_back, align 1
  %is_point30 = load i1, ptr %is_point, align 1
  %not31 = xor i1 %is_point30, true
  br i1 %not31, label %short_circuit.rhs32, label %short_circuit.merge33

short_circuit.rhs32:                              ; preds = %short_circuit.merge26
  %is_nine34 = load i1, ptr %is_nine, align 1
  br label %short_circuit.merge33

short_circuit.merge33:                            ; preds = %short_circuit.rhs32, %short_circuit.merge26
  %short_circuit35 = phi i1 [ false, %short_circuit.merge26 ], [ %is_nine34, %short_circuit.rhs32 ]
  br i1 %short_circuit35, label %short_circuit.rhs36, label %short_circuit.merge37

short_circuit.rhs36:                              ; preds = %short_circuit.merge33
  %at_start38 = load i1, ptr %at_start, align 1
  br label %short_circuit.merge37

short_circuit.merge37:                            ; preds = %short_circuit.rhs36, %short_circuit.merge33
  %short_circuit39 = phi i1 [ false, %short_circuit.merge33 ], [ %at_start38, %short_circuit.rhs36 ]
  store i1 %short_circuit39, ptr %carry_stop, align 1
  %is_point40 = load i1, ptr %is_point, align 1
  %not41 = xor i1 %is_point40, true
  br i1 %not41, label %short_circuit.rhs42, label %short_circuit.merge43

short_circuit.rhs42:                              ; preds = %short_circuit.merge37
  %is_nine44 = load i1, ptr %is_nine, align 1
  %not45 = xor i1 %is_nine44, true
  br label %short_circuit.merge43

short_circuit.merge43:                            ; preds = %short_circuit.rhs42, %short_circuit.merge37
  %short_circuit46 = phi i1 [ false, %short_circuit.merge37 ], [ %not45, %short_circuit.rhs42 ]
  store i1 %short_circuit46, ptr %carry_done, align 1
  %carry_back47 = load i1, ptr %carry_back, align 1
  br i1 %carry_back47, label %if.then48, label %if.merge49

if.then48:                                        ; preds = %short_circuit.merge43
  %zero_digit50 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit50, ptr %assignment_value51, align 1
  %assignment_value52 = load i8, ptr %assignment_value51, align 1
  %scratch53 = load ptr, ptr %scratch1, align 8
  %round_position54 = load i64, ptr %round_position2, align 4
  %array_element55 = getelementptr inbounds i8, ptr %scratch53, i64 %round_position54
  store i8 %assignment_value52, ptr %array_element55, align 1
  %round_position56 = load i64, ptr %round_position2, align 4
  %one57 = load i64, ptr %one, align 4
  %sub58 = sub i64 %round_position56, %one57
  store i64 %sub58, ptr %assignment_value59, align 4
  %assignment_value60 = load i64, ptr %assignment_value59, align 4
  store i64 %assignment_value60, ptr %round_position2, align 4
  br label %if.merge49

if.merge49:                                       ; preds = %if.then48, %short_circuit.merge43
  %carry_stop61 = load i1, ptr %carry_stop, align 1
  br i1 %carry_stop61, label %if.then62, label %if.merge63

if.then62:                                        ; preds = %if.merge49
  %zero_digit64 = load i8, ptr %zero_digit, align 1
  store i8 %zero_digit64, ptr %assignment_value65, align 1
  %assignment_value66 = load i8, ptr %assignment_value65, align 1
  %scratch67 = load ptr, ptr %scratch1, align 8
  %round_position68 = load i64, ptr %round_position2, align 4
  %array_element69 = getelementptr inbounds i8, ptr %scratch67, i64 %round_position68
  store i8 %assignment_value66, ptr %array_element69, align 1
  store i1 false, ptr %assignment_value70, align 1
  %assignment_value71 = load i1, ptr %assignment_value70, align 1
  store i1 %assignment_value71, ptr %rounding5, align 1
  br label %if.merge63

if.merge63:                                       ; preds = %if.then62, %if.merge49
  %carry_done72 = load i1, ptr %carry_done, align 1
  br i1 %carry_done72, label %if.then73, label %if.merge74

if.then73:                                        ; preds = %if.merge63
  %current75 = load i8, ptr %current, align 1
  %one_digit76 = load i8, ptr %one_digit, align 1
  %add = add i8 %current75, %one_digit76
  store i8 %add, ptr %assignment_value77, align 1
  %assignment_value78 = load i8, ptr %assignment_value77, align 1
  %scratch79 = load ptr, ptr %scratch1, align 8
  %round_position80 = load i64, ptr %round_position2, align 4
  %array_element81 = getelementptr inbounds i8, ptr %scratch79, i64 %round_position80
  store i8 %assignment_value78, ptr %array_element81, align 1
  store i1 false, ptr %assignment_value82, align 1
  %assignment_value83 = load i1, ptr %assignment_value82, align 1
  store i1 %assignment_value83, ptr %rounding5, align 1
  br label %if.merge74

if.merge74:                                       ; preds = %if.then73, %if.merge63
  %scratch84 = load ptr, ptr %scratch1, align 8
  %round_position85 = load i64, ptr %round_position2, align 4
  %digit_start86 = load i64, ptr %digit_start3, align 4
  %exponent87 = load i32, ptr %exponent4, align 4
  %rounding88 = load i1, ptr %rounding5, align 1
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f63617272795f663634(ptr %scratch84, i64 %round_position85, i64 %digit_start86, i32 %exponent87, i1 %rounding88)
  br label %if.merge

if.then96:                                        ; preds = %if.else
  store i8 49, ptr %one_char, align 1
  store i32 1, ptr %one_count, align 4
  %one_char98 = load i8, ptr %one_char, align 1
  store i8 %one_char98, ptr %assignment_value99, align 1
  %assignment_value100 = load i8, ptr %assignment_value99, align 1
  %scratch101 = load ptr, ptr %scratch1, align 8
  %digit_start102 = load i64, ptr %digit_start3, align 4
  %array_element103 = getelementptr inbounds i8, ptr %scratch101, i64 %digit_start102
  store i8 %assignment_value100, ptr %array_element103, align 1
  %exponent104 = load i32, ptr %exponent4, align 4
  %one_count105 = load i32, ptr %one_count, align 4
  %add106 = add i32 %exponent104, %one_count105
  store i32 %add106, ptr %assignment_value107, align 4
  %assignment_value108 = load i32, ptr %assignment_value107, align 4
  store i32 %assignment_value108, ptr %exponent4, align 4
  br label %if.merge97

if.merge97:                                       ; preds = %if.then96, %if.else
  %exponent109 = load i32, ptr %exponent4, align 4
  br label %if.merge
}

define { i64, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f663634(ptr %scratch, i64 %digit_start, i64 %emit, i64 %produced, double %scaled, i32 %exponent) {
entry:
  %assignment_value69 = alloca i32, align 4
  %five_value = alloca double, align 8
  %result_exponent = alloca i32, align 4
  %result_emit = alloca i64, align 8
  %assignment_value43 = alloca double, align 8
  %assignment_value39 = alloca double, align 8
  %whole = alloca double, align 8
  %assignment_value34 = alloca i64, align 8
  %assignment_value26 = alloca i8, align 1
  %assignment_value19 = alloca i64, align 8
  %assignment_value = alloca i8, align 1
  %digit = alloca i8, align 1
  %digit_value = alloca i32, align 4
  %ten_value = alloca double, align 8
  %point = alloca i8, align 1
  %zero_digit = alloca i8, align 1
  %zero = alloca i64, align 8
  %one = alloca i64, align 8
  %seventeen = alloca i64, align 8
  %scratch1 = alloca ptr, align 8
  store ptr %scratch, ptr %scratch1, align 8
  %digit_start2 = alloca i64, align 8
  store i64 %digit_start, ptr %digit_start2, align 4
  %emit3 = alloca i64, align 8
  store i64 %emit, ptr %emit3, align 4
  %produced4 = alloca i64, align 8
  store i64 %produced, ptr %produced4, align 4
  %scaled5 = alloca double, align 8
  store double %scaled, ptr %scaled5, align 8
  %exponent6 = alloca i32, align 4
  store i32 %exponent, ptr %exponent6, align 4
  store i64 17, ptr %seventeen, align 4
  store i64 1, ptr %one, align 4
  %produced7 = load i64, ptr %produced4, align 4
  %seventeen8 = load i64, ptr %seventeen, align 4
  %lt = icmp ult i64 %produced7, %seventeen8
  br i1 %lt, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  store i64 0, ptr %zero, align 4
  store i8 48, ptr %zero_digit, align 1
  store i8 46, ptr %point, align 1
  store double 1.000000e+01, ptr %ten_value, align 8
  %scaled9 = load double, ptr %scaled5, align 8
  %float_to_sint_trunc = fptosi double %scaled9 to i32
  store i32 %float_to_sint_trunc, ptr %digit_value, align 4
  %digit_value10 = load i32, ptr %digit_value, align 4
  %int_trunc = trunc i32 %digit_value10 to i8
  store i8 %int_trunc, ptr %digit, align 1
  %zero_digit11 = load i8, ptr %zero_digit, align 1
  %digit12 = load i8, ptr %digit, align 1
  %add = add i8 %zero_digit11, %digit12
  store i8 %add, ptr %assignment_value, align 1
  %assignment_value13 = load i8, ptr %assignment_value, align 1
  %scratch14 = load ptr, ptr %scratch1, align 8
  %emit15 = load i64, ptr %emit3, align 4
  %array_element = getelementptr inbounds i8, ptr %scratch14, i64 %emit15
  store i8 %assignment_value13, ptr %array_element, align 1
  %emit16 = load i64, ptr %emit3, align 4
  %one17 = load i64, ptr %one, align 4
  %add18 = add i64 %emit16, %one17
  store i64 %add18, ptr %assignment_value19, align 4
  %assignment_value20 = load i64, ptr %assignment_value19, align 4
  store i64 %assignment_value20, ptr %emit3, align 4
  %produced21 = load i64, ptr %produced4, align 4
  %zero22 = load i64, ptr %zero, align 4
  %eq = icmp eq i64 %produced21, %zero22
  br i1 %eq, label %if.then23, label %if.merge24

if.else:                                          ; preds = %entry
  store double 5.000000e+00, ptr %five_value, align 8
  %scaled58 = load double, ptr %scaled5, align 8
  %five_value59 = load double, ptr %five_value, align 8
  %ge = fcmp oge double %scaled58, %five_value59
  br i1 %ge, label %if.then60, label %if.merge61

if.merge:                                         ; preds = %if.merge61, %if.merge24
  %if = phi { i64, i32 } [ %output57, %if.merge24 ], [ %output74, %if.merge61 ]
  ret { i64, i32 } %if

if.then23:                                        ; preds = %if.then
  %point25 = load i8, ptr %point, align 1
  store i8 %point25, ptr %assignment_value26, align 1
  %assignment_value27 = load i8, ptr %assignment_value26, align 1
  %scratch28 = load ptr, ptr %scratch1, align 8
  %emit29 = load i64, ptr %emit3, align 4
  %array_element30 = getelementptr inbounds i8, ptr %scratch28, i64 %emit29
  store i8 %assignment_value27, ptr %array_element30, align 1
  %emit31 = load i64, ptr %emit3, align 4
  %one32 = load i64, ptr %one, align 4
  %add33 = add i64 %emit31, %one32
  store i64 %add33, ptr %assignment_value34, align 4
  %assignment_value35 = load i64, ptr %assignment_value34, align 4
  store i64 %assignment_value35, ptr %emit3, align 4
  br label %if.merge24

if.merge24:                                       ; preds = %if.then23, %if.then
  %digit_value36 = load i32, ptr %digit_value, align 4
  %sint_to_float = sitofp i32 %digit_value36 to double
  store double %sint_to_float, ptr %whole, align 8
  %scaled37 = load double, ptr %scaled5, align 8
  %whole38 = load double, ptr %whole, align 8
  %sub = fsub double %scaled37, %whole38
  store double %sub, ptr %assignment_value39, align 8
  %assignment_value40 = load double, ptr %assignment_value39, align 8
  store double %assignment_value40, ptr %scaled5, align 8
  %scaled41 = load double, ptr %scaled5, align 8
  %ten_value42 = load double, ptr %ten_value, align 8
  %mul = fmul double %scaled41, %ten_value42
  store double %mul, ptr %assignment_value43, align 8
  %assignment_value44 = load double, ptr %assignment_value43, align 8
  store double %assignment_value44, ptr %scaled5, align 8
  %scratch45 = load ptr, ptr %scratch1, align 8
  %digit_start46 = load i64, ptr %digit_start2, align 4
  %emit47 = load i64, ptr %emit3, align 4
  %produced48 = load i64, ptr %produced4, align 4
  %one49 = load i64, ptr %one, align 4
  %add50 = add i64 %produced48, %one49
  %scaled51 = load double, ptr %scaled5, align 8
  %exponent52 = load i32, ptr %exponent6, align 4
  %call = call { i64, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f663634(ptr %scratch45, i64 %digit_start46, i64 %emit47, i64 %add50, double %scaled51, i32 %exponent52)
  %output = extractvalue { i64, i32 } %call, 0
  store i64 %output, ptr %result_emit, align 4
  %output53 = extractvalue { i64, i32 } %call, 1
  store i32 %output53, ptr %result_exponent, align 4
  %result_emit54 = load i64, ptr %result_emit, align 4
  %result_exponent55 = load i32, ptr %result_exponent, align 4
  %output56 = insertvalue { i64, i32 } zeroinitializer, i64 %result_emit54, 0
  %output57 = insertvalue { i64, i32 } %output56, i32 %result_exponent55, 1
  br label %if.merge

if.then60:                                        ; preds = %if.else
  %scratch62 = load ptr, ptr %scratch1, align 8
  %emit63 = load i64, ptr %emit3, align 4
  %one64 = load i64, ptr %one, align 4
  %sub65 = sub i64 %emit63, %one64
  %digit_start66 = load i64, ptr %digit_start2, align 4
  %exponent67 = load i32, ptr %exponent6, align 4
  %call68 = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f63617272795f663634(ptr %scratch62, i64 %sub65, i64 %digit_start66, i32 %exponent67, i1 true)
  store i32 %call68, ptr %assignment_value69, align 4
  %assignment_value70 = load i32, ptr %assignment_value69, align 4
  store i32 %assignment_value70, ptr %exponent6, align 4
  br label %if.merge61

if.merge61:                                       ; preds = %if.then60, %if.else
  %emit71 = load i64, ptr %emit3, align 4
  %exponent72 = load i32, ptr %exponent6, align 4
  %output73 = insertvalue { i64, i32 } zeroinitializer, i64 %emit71, 0
  %output74 = insertvalue { i64, i32 } %output73, i32 %exponent72, 1
  br label %if.merge
}

define internal ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f663634(double %value, ptr %0) #0 {
entry:
  %text = alloca [16 x i8], align 8
  %struct_literal = alloca [16 x i8], align 8
  %assignment_value179 = alloca i32, align 4
  %data = alloca i32, align 4
  %assignment_value176 = alloca i64, align 8
  %assignment_value169 = alloca i8, align 1
  %one_digit_out = alloca i8, align 1
  %assignment_value162 = alloca i64, align 8
  %assignment_value155 = alloca i8, align 1
  %ten_digit = alloca i8, align 1
  %assignment_value145 = alloca i64, align 8
  %assignment_value138 = alloca i8, align 1
  %hundred_digit = alloca i8, align 1
  %assignment_value127 = alloca i1, align 1
  %assignment_value120 = alloca i1, align 1
  %show_tens = alloca i1, align 1
  %ones = alloca i32, align 4
  %tens = alloca i32, align 4
  %after_hundreds = alloca i32, align 4
  %hundreds = alloca i32, align 4
  %ten_scale = alloca i32, align 4
  %hundred_scale = alloca i32, align 4
  %assignment_value99 = alloca i32, align 4
  %assignment_value94 = alloca i64, align 8
  %assignment_value87 = alloca i8, align 1
  %remaining_exponent = alloca i32, align 4
  %assignment_value78 = alloca i64, align 8
  %assignment_value71 = alloca i8, align 1
  %assignment_value68 = alloca i64, align 8
  %exponent = alloca i32, align 4
  %digit_emit = alloca i64, align 8
  %normalized_exponent = alloca i32, align 4
  %scaled = alloca double, align 8
  %down_exponent = alloca i32, align 4
  %down = alloca double, align 8
  %budget = alloca i64, align 8
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
  %zero_count = alloca i32, align 4
  %exponent_mark = alloca i8, align 1
  %minus = alloca i8, align 1
  %zero_digit = alloca i8, align 1
  %one_value = alloca double, align 8
  %zero_value = alloca double, align 8
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
  store double 0.000000e+00, ptr %zero_value, align 8
  store double 1.000000e+00, ptr %one_value, align 8
  store i8 48, ptr %zero_digit, align 1
  store i8 45, ptr %minus, align 1
  store i8 101, ptr %exponent_mark, align 1
  store i32 0, ptr %zero_count, align 4
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
  store i64 400, ptr %budget, align 4
  %magnitude49 = load double, ptr %magnitude, align 8
  %zero_count50 = load i32, ptr %zero_count, align 4
  %budget51 = load i64, ptr %budget, align 4
  %call = call { double, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6e6f726d616c697a655f666c6f61745f646f776e(double %magnitude49, i32 %zero_count50, i64 %budget51)
  %output = extractvalue { double, i32 } %call, 0
  store double %output, ptr %down, align 8
  %output52 = extractvalue { double, i32 } %call, 1
  store i32 %output52, ptr %down_exponent, align 4
  %down53 = load double, ptr %down, align 8
  %down_exponent54 = load i32, ptr %down_exponent, align 4
  %budget55 = load i64, ptr %budget, align 4
  %call56 = call { double, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6e6f726d616c697a655f666c6f61745f7570(double %down53, i32 %down_exponent54, i64 %budget55)
  %output57 = extractvalue { double, i32 } %call56, 0
  store double %output57, ptr %scaled, align 8
  %output58 = extractvalue { double, i32 } %call56, 1
  store i32 %output58, ptr %normalized_exponent, align 4
  %emit59 = load i64, ptr %emit, align 4
  %emit60 = load i64, ptr %emit, align 4
  %zero61 = load i64, ptr %zero, align 4
  %scaled62 = load double, ptr %scaled, align 8
  %normalized_exponent63 = load i32, ptr %normalized_exponent, align 4
  %call64 = call { i64, i32 } @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6469676974735f663634(ptr %allocation, i64 %emit59, i64 %emit60, i64 %zero61, double %scaled62, i32 %normalized_exponent63)
  %output65 = extractvalue { i64, i32 } %call64, 0
  store i64 %output65, ptr %digit_emit, align 4
  %output66 = extractvalue { i64, i32 } %call64, 1
  store i32 %output66, ptr %exponent, align 4
  %digit_emit67 = load i64, ptr %digit_emit, align 4
  store i64 %digit_emit67, ptr %assignment_value68, align 4
  %assignment_value69 = load i64, ptr %assignment_value68, align 4
  store i64 %assignment_value69, ptr %emit, align 4
  %exponent_mark70 = load i8, ptr %exponent_mark, align 1
  store i8 %exponent_mark70, ptr %assignment_value71, align 1
  %assignment_value72 = load i8, ptr %assignment_value71, align 1
  %emit73 = load i64, ptr %emit, align 4
  %array_element74 = getelementptr inbounds i8, ptr %allocation, i64 %emit73
  store i8 %assignment_value72, ptr %array_element74, align 1
  %emit75 = load i64, ptr %emit, align 4
  %one76 = load i64, ptr %one, align 4
  %add77 = add i64 %emit75, %one76
  store i64 %add77, ptr %assignment_value78, align 4
  %assignment_value79 = load i64, ptr %assignment_value78, align 4
  store i64 %assignment_value79, ptr %emit, align 4
  %exponent80 = load i32, ptr %exponent, align 4
  store i32 %exponent80, ptr %remaining_exponent, align 4
  %remaining_exponent81 = load i32, ptr %remaining_exponent, align 4
  %zero_count82 = load i32, ptr %zero_count, align 4
  %lt83 = icmp slt i32 %remaining_exponent81, %zero_count82
  br i1 %lt83, label %if.then84, label %if.merge85

if.merge22:                                       ; preds = %if.merge149, %if.merge29
  store i32 0, ptr %data, align 4
  %array_element178 = getelementptr inbounds i8, ptr %allocation, i64 0
  %address = ptrtoint ptr %array_element178 to i32
  store i32 %address, ptr %assignment_value179, align 4
  %assignment_value180 = load i32, ptr %assignment_value179, align 4
  store i32 %assignment_value180, ptr %data, align 4
  %data181 = load i32, ptr %data, align 4
  %emit182 = load i64, ptr %emit, align 4
  %data183 = getelementptr inbounds i8, ptr %struct_literal, i8 0
  store i32 %data181, ptr %data183, align 4
  %length = getelementptr inbounds i8, ptr %struct_literal, i8 8
  store i64 %emit182, ptr %length, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text, ptr align 1 %struct_literal, i32 16, i1 false)
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %0, ptr align 1 %text, i32 16, i1 false)
  ret ptr %allocation

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

if.then84:                                        ; preds = %if.else
  %minus86 = load i8, ptr %minus, align 1
  store i8 %minus86, ptr %assignment_value87, align 1
  %assignment_value88 = load i8, ptr %assignment_value87, align 1
  %emit89 = load i64, ptr %emit, align 4
  %array_element90 = getelementptr inbounds i8, ptr %allocation, i64 %emit89
  store i8 %assignment_value88, ptr %array_element90, align 1
  %emit91 = load i64, ptr %emit, align 4
  %one92 = load i64, ptr %one, align 4
  %add93 = add i64 %emit91, %one92
  store i64 %add93, ptr %assignment_value94, align 4
  %assignment_value95 = load i64, ptr %assignment_value94, align 4
  store i64 %assignment_value95, ptr %emit, align 4
  %zero_count96 = load i32, ptr %zero_count, align 4
  %remaining_exponent97 = load i32, ptr %remaining_exponent, align 4
  %sub98 = sub i32 %zero_count96, %remaining_exponent97
  store i32 %sub98, ptr %assignment_value99, align 4
  %assignment_value100 = load i32, ptr %assignment_value99, align 4
  store i32 %assignment_value100, ptr %remaining_exponent, align 4
  br label %if.merge85

if.merge85:                                       ; preds = %if.then84, %if.else
  store i32 100, ptr %hundred_scale, align 4
  store i32 10, ptr %ten_scale, align 4
  %remaining_exponent101 = load i32, ptr %remaining_exponent, align 4
  %hundred_scale102 = load i32, ptr %hundred_scale, align 4
  %div103 = sdiv i32 %remaining_exponent101, %hundred_scale102
  store i32 %div103, ptr %hundreds, align 4
  %remaining_exponent104 = load i32, ptr %remaining_exponent, align 4
  %hundreds105 = load i32, ptr %hundreds, align 4
  %hundred_scale106 = load i32, ptr %hundred_scale, align 4
  %mul = mul i32 %hundreds105, %hundred_scale106
  %sub107 = sub i32 %remaining_exponent104, %mul
  store i32 %sub107, ptr %after_hundreds, align 4
  %after_hundreds108 = load i32, ptr %after_hundreds, align 4
  %ten_scale109 = load i32, ptr %ten_scale, align 4
  %div110 = sdiv i32 %after_hundreds108, %ten_scale109
  store i32 %div110, ptr %tens, align 4
  %after_hundreds111 = load i32, ptr %after_hundreds, align 4
  %tens112 = load i32, ptr %tens, align 4
  %ten_scale113 = load i32, ptr %ten_scale, align 4
  %mul114 = mul i32 %tens112, %ten_scale113
  %sub115 = sub i32 %after_hundreds111, %mul114
  store i32 %sub115, ptr %ones, align 4
  store i1 false, ptr %show_tens, align 1
  %hundreds116 = load i32, ptr %hundreds, align 4
  %zero_count117 = load i32, ptr %zero_count, align 4
  %ne = icmp ne i32 %hundreds116, %zero_count117
  br i1 %ne, label %if.then118, label %if.merge119

if.then118:                                       ; preds = %if.merge85
  store i1 true, ptr %assignment_value120, align 1
  %assignment_value121 = load i1, ptr %assignment_value120, align 1
  store i1 %assignment_value121, ptr %show_tens, align 1
  br label %if.merge119

if.merge119:                                      ; preds = %if.then118, %if.merge85
  %tens122 = load i32, ptr %tens, align 4
  %zero_count123 = load i32, ptr %zero_count, align 4
  %ne124 = icmp ne i32 %tens122, %zero_count123
  br i1 %ne124, label %if.then125, label %if.merge126

if.then125:                                       ; preds = %if.merge119
  store i1 true, ptr %assignment_value127, align 1
  %assignment_value128 = load i1, ptr %assignment_value127, align 1
  store i1 %assignment_value128, ptr %show_tens, align 1
  br label %if.merge126

if.merge126:                                      ; preds = %if.then125, %if.merge119
  %hundreds129 = load i32, ptr %hundreds, align 4
  %zero_count130 = load i32, ptr %zero_count, align 4
  %ne131 = icmp ne i32 %hundreds129, %zero_count130
  br i1 %ne131, label %if.then132, label %if.merge133

if.then132:                                       ; preds = %if.merge126
  %hundreds134 = load i32, ptr %hundreds, align 4
  %int_trunc = trunc i32 %hundreds134 to i8
  store i8 %int_trunc, ptr %hundred_digit, align 1
  %zero_digit135 = load i8, ptr %zero_digit, align 1
  %hundred_digit136 = load i8, ptr %hundred_digit, align 1
  %add137 = add i8 %zero_digit135, %hundred_digit136
  store i8 %add137, ptr %assignment_value138, align 1
  %assignment_value139 = load i8, ptr %assignment_value138, align 1
  %emit140 = load i64, ptr %emit, align 4
  %array_element141 = getelementptr inbounds i8, ptr %allocation, i64 %emit140
  store i8 %assignment_value139, ptr %array_element141, align 1
  %emit142 = load i64, ptr %emit, align 4
  %one143 = load i64, ptr %one, align 4
  %add144 = add i64 %emit142, %one143
  store i64 %add144, ptr %assignment_value145, align 4
  %assignment_value146 = load i64, ptr %assignment_value145, align 4
  store i64 %assignment_value146, ptr %emit, align 4
  br label %if.merge133

if.merge133:                                      ; preds = %if.then132, %if.merge126
  %show_tens147 = load i1, ptr %show_tens, align 1
  br i1 %show_tens147, label %if.then148, label %if.merge149

if.then148:                                       ; preds = %if.merge133
  %tens150 = load i32, ptr %tens, align 4
  %int_trunc151 = trunc i32 %tens150 to i8
  store i8 %int_trunc151, ptr %ten_digit, align 1
  %zero_digit152 = load i8, ptr %zero_digit, align 1
  %ten_digit153 = load i8, ptr %ten_digit, align 1
  %add154 = add i8 %zero_digit152, %ten_digit153
  store i8 %add154, ptr %assignment_value155, align 1
  %assignment_value156 = load i8, ptr %assignment_value155, align 1
  %emit157 = load i64, ptr %emit, align 4
  %array_element158 = getelementptr inbounds i8, ptr %allocation, i64 %emit157
  store i8 %assignment_value156, ptr %array_element158, align 1
  %emit159 = load i64, ptr %emit, align 4
  %one160 = load i64, ptr %one, align 4
  %add161 = add i64 %emit159, %one160
  store i64 %add161, ptr %assignment_value162, align 4
  %assignment_value163 = load i64, ptr %assignment_value162, align 4
  store i64 %assignment_value163, ptr %emit, align 4
  br label %if.merge149

if.merge149:                                      ; preds = %if.then148, %if.merge133
  %ones164 = load i32, ptr %ones, align 4
  %int_trunc165 = trunc i32 %ones164 to i8
  store i8 %int_trunc165, ptr %one_digit_out, align 1
  %zero_digit166 = load i8, ptr %zero_digit, align 1
  %one_digit_out167 = load i8, ptr %one_digit_out, align 1
  %add168 = add i8 %zero_digit166, %one_digit_out167
  store i8 %add168, ptr %assignment_value169, align 1
  %assignment_value170 = load i8, ptr %assignment_value169, align 1
  %emit171 = load i64, ptr %emit, align 4
  %array_element172 = getelementptr inbounds i8, ptr %allocation, i64 %emit171
  store i8 %assignment_value170, ptr %array_element172, align 1
  %emit173 = load i64, ptr %emit, align 4
  %one174 = load i64, ptr %one, align 4
  %add175 = add i64 %emit173, %one174
  store i64 %add175, ptr %assignment_value176, align 4
  %assignment_value177 = load i64, ptr %assignment_value176, align 4
  store i64 %assignment_value177, ptr %emit, align 4
  br label %if.merge22
}





define { i64, i1 } @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f66645f726561645f6f6e6365(i32 %destination, i64 %capacity) #0 {
entry:
  %assignment_value14 = alloca i1, align 1
  %assignment_value12 = alloca i64, align 8
  %assignment_value10 = alloca i1, align 1
  %assignment_value = alloca i64, align 8
  %complete = alloca i1, align 1
  %reported = alloca i64, align 8
  %_result = alloca i32, align 4
  %_byte_count = alloca [4 x i8], align 4
  %struct_literal5 = alloca [4 x i8], align 4
  %_iovec = alloca [8 x i8], align 4
  %struct_literal = alloca [8 x i8], align 4
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
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %_iovec, ptr align 1 %struct_literal, i32 8, i1 false)
  %value = getelementptr inbounds i8, ptr %struct_literal5, i8 0
  store i32 0, ptr %value, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %_byte_count, ptr align 1 %struct_literal5, i32 4, i1 false)
  %address = ptrtoint ptr %_iovec to i32
  %address6 = ptrtoint ptr %_byte_count to i32
  %call = call i32 @wosy_extern__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f77617369__776173695f736e617073686f745f7072657669657731__5f66645f72656164(i32 0, i32 %address, i32 1, i32 %address6)
  store i32 %call, ptr %_result, align 4
  store i64 0, ptr %reported, align 4
  store i1 false, ptr %complete, align 1
  %_result7 = load i32, ptr %_result, align 4
  %eq = icmp eq i32 %_result7, 0
  br i1 %eq, label %if.then, label %if.else

if.then:                                          ; preds = %entry
  %value8 = getelementptr inbounds i8, ptr %_byte_count, i8 0
  %place = load i32, ptr %value8, align 4
  %int_extend = zext i32 %place to i64
  store i64 %int_extend, ptr %assignment_value, align 4
  %assignment_value9 = load i64, ptr %assignment_value, align 4
  store i64 %assignment_value9, ptr %reported, align 4
  store i1 true, ptr %assignment_value10, align 1
  %assignment_value11 = load i1, ptr %assignment_value10, align 1
  store i1 %assignment_value11, ptr %complete, align 1
  br label %if.merge

if.else:                                          ; preds = %entry
  store i64 0, ptr %assignment_value12, align 4
  %assignment_value13 = load i64, ptr %assignment_value12, align 4
  store i64 %assignment_value13, ptr %reported, align 4
  store i1 false, ptr %assignment_value14, align 1
  %assignment_value15 = load i1, ptr %assignment_value14, align 1
  store i1 %assignment_value15, ptr %complete, align 1
  br label %if.merge

if.merge:                                         ; preds = %if.else, %if.then
  %reported16 = load i64, ptr %reported, align 4
  %complete17 = load i1, ptr %complete, align 1
  %output = insertvalue { i64, i1 } zeroinitializer, i64 %reported16, 0
  %output18 = insertvalue { i64, i1 } %output, i1 %complete17, 1
  ret { i64, i1 } %output18
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

define void @wosy_fn__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__736c6565705f6e73(i64 %nanoseconds) #0 {
entry:
  %fired = alloca [4 x i8], align 4
  %struct_literal20 = alloca [4 x i8], align 4
  %event = alloca [32 x i8], align 8
  %struct_literal3 = alloca [32 x i8], align 8
  %sub = alloca [48 x i8], align 8
  %struct_literal = alloca [48 x i8], align 8
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
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %sub, ptr align 1 %struct_literal, i32 48, i1 false)
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
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %event, ptr align 1 %struct_literal3, i32 32, i1 false)
  %value = getelementptr inbounds i8, ptr %struct_literal20, i8 0
  store i32 0, ptr %value, align 4
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %fired, ptr align 1 %struct_literal20, i32 4, i1 false)
  %address = ptrtoint ptr %sub to i32
  %address21 = ptrtoint ptr %event to i32
  %address22 = ptrtoint ptr %fired to i32
  %call = call i32 @wosy_extern__737464__7372632f776173692f70726576696577312e77__33646430646165626662636338353638356539383066353032626235623166376635316565646131613130313432626637646534396536666635373831373138__5f77617369__776173695f736e617073686f745f7072657669657731__5f706f6c6c5f6f6e656f6666(i32 %address, i32 %address21, i32 1, i32 %address22)
  ret void
}

define internal ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f3330__(i1 %value, ptr %0) {
entry:
  %value1 = alloca i1, align 1
  store i1 %value, ptr %value1, align 1
  %value2 = load i1, ptr %value1, align 1
  %aggregate_original_base = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f626f6f6c(i1 %value2, ptr %0)
  ret ptr %aggregate_original_base
}

define internal ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f3331__(i32 %value, ptr %0) {
entry:
  %value1 = alloca i32, align 4
  store i32 %value, ptr %value1, align 4
  %value2 = load i32, ptr %value1, align 4
  %aggregate_original_base = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f63686172(i32 %value2, ptr %0)
  ret ptr %aggregate_original_base
}

define internal ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f33313330__(i64 %value, ptr %0) {
entry:
  %value1 = alloca i64, align 8
  store i64 %value, ptr %value1, align 4
  %value2 = load i64, ptr %value1, align 4
  %aggregate_original_base = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f753634(i64 %value2, ptr %0)
  ret ptr %aggregate_original_base
}

define internal ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f33313331__(i128 %value, ptr %0) {
entry:
  %value1 = alloca i128, align 8
  store i128 %value, ptr %value1, align 4
  %value2 = load i128, ptr %value1, align 4
  %aggregate_original_base = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f75313238(i128 %value2, ptr %0)
  ret ptr %aggregate_original_base
}

define internal ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f33313332__(float %value, ptr %0) {
entry:
  %value1 = alloca float, align 4
  store float %value, ptr %value1, align 4
  %value2 = load float, ptr %value1, align 4
  %aggregate_original_base = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f663332(float %value2, ptr %0)
  ret ptr %aggregate_original_base
}

define internal ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f33313333__(double %value, ptr %0) {
entry:
  %value1 = alloca double, align 8
  store double %value, ptr %value1, align 8
  %value2 = load double, ptr %value1, align 8
  %aggregate_original_base = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f663634(double %value2, ptr %0)
  ret ptr %aggregate_original_base
}

define internal ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f3332__(i8 %value, ptr %0) {
entry:
  %value1 = alloca i8, align 1
  store i8 %value, ptr %value1, align 1
  %value2 = load i8, ptr %value1, align 1
  %aggregate_original_base = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f6938(i8 %value2, ptr %0)
  ret ptr %aggregate_original_base
}

define internal ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f3333__(i16 %value, ptr %0) {
entry:
  %value1 = alloca i16, align 2
  store i16 %value, ptr %value1, align 2
  %value2 = load i16, ptr %value1, align 2
  %aggregate_original_base = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f693136(i16 %value2, ptr %0)
  ret ptr %aggregate_original_base
}

define internal ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f3334__(i32 %value, ptr %0) {
entry:
  %value1 = alloca i32, align 4
  store i32 %value, ptr %value1, align 4
  %value2 = load i32, ptr %value1, align 4
  %aggregate_original_base = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f693332(i32 %value2, ptr %0)
  ret ptr %aggregate_original_base
}

define internal ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f3335__(i64 %value, ptr %0) {
entry:
  %value1 = alloca i64, align 8
  store i64 %value, ptr %value1, align 4
  %value2 = load i64, ptr %value1, align 4
  %aggregate_original_base = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f693634(i64 %value2, ptr %0)
  ret ptr %aggregate_original_base
}

define internal ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f3336__(i128 %value, ptr %0) {
entry:
  %value1 = alloca i128, align 8
  store i128 %value, ptr %value1, align 4
  %value2 = load i128, ptr %value1, align 4
  %aggregate_original_base = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f69313238(i128 %value2, ptr %0)
  ret ptr %aggregate_original_base
}

define internal ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f3337__(i8 %value, ptr %0) {
entry:
  %value1 = alloca i8, align 1
  store i8 %value, ptr %value1, align 1
  %value2 = load i8, ptr %value1, align 1
  %aggregate_original_base = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f7538(i8 %value2, ptr %0)
  ret ptr %aggregate_original_base
}

define internal ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f3338__(i16 %value, ptr %0) {
entry:
  %value1 = alloca i16, align 2
  store i16 %value, ptr %value1, align 2
  %value2 = load i16, ptr %value1, align 2
  %aggregate_original_base = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f753136(i16 %value2, ptr %0)
  ret ptr %aggregate_original_base
}

define internal ptr @wosy_generic__776f73795f6f7665726c6f61645f5f373736663733373935663636366535663566333733333337333433363334356635663337333333373332333633333332363633363332333636363336363633373334333733333337333433373332333633313337333033323635333733373566356633333330333633343333333733333338333333343333333433333337333633343336333233333332333633343336333233333331333333363336333333333337333633353333333233363332333333353336333133333333333633333336333333333335333633363333333033333333333333363333333633363331333633363333333833333332333633353333333033333338333333373333333933363331333333383333333933333333333633343333333033333338333633343333333533363335333333393333333333363334333633333333333833333337333333333333333933363334333333353333333233363331333333303333333733363331356635663336333633363636333733323336363433363331333733345f5f3339__(i32 %value, ptr %0) {
entry:
  %value1 = alloca i32, align 4
  store i32 %value, ptr %value1, align 4
  %value2 = load i32, ptr %value1, align 4
  %aggregate_original_base = call ptr @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f666f726d61745f753332(i32 %value2, ptr %0)
  ret ptr %aggregate_original_base
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333303336333433333337333333383333333433333334333333373336333433363332333333323336333433363332333333313333333633363333333333373336333533333332333633323333333533363331333333333336333333363333333333353336333633333330333333333333333633333336333633313336333633333338333333323336333533333330333333383333333733333339333633313333333833333339333333333336333433333330333333383336333433333335333633353333333933333333333633343336333333333338333333373333333333333339333633343333333533333332333633313333333033333337333633313566356633373330333633313337333233373333333633355f5f3330__(ptr %text) #0 {
entry:
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f7538(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333303336333433333337333333383333333433333334333333373336333433363332333333323336333433363332333333313333333633363333333333373336333533333332333633323333333533363331333333333336333333363333333333353336333633333330333333333333333633333336333633313336333633333338333333323336333533333330333333383333333733333339333633313333333833333339333333333336333433333330333333383336333433333335333633353333333933333333333633343336333333333338333333373333333333333339333633343333333533333332333633313333333033333337333633313566356633373330333633313337333233373333333633355f5f3331__(ptr %text) #0 {
entry:
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f753136(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333303336333433333337333333383333333433333334333333373336333433363332333333323336333433363332333333313333333633363333333333373336333533333332333633323333333533363331333333333336333333363333333333353336333633333330333333333333333633333336333633313336333633333338333333323336333533333330333333383333333733333339333633313333333833333339333333333336333433333330333333383336333433333335333633353333333933333333333633343336333333333338333333373333333333333339333633343333333533333332333633313333333033333337333633313566356633373330333633313337333233373333333633355f5f33313330__(ptr %text) #0 {
entry:
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f663332(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333303336333433333337333333383333333433333334333333373336333433363332333333323336333433363332333333313333333633363333333333373336333533333332333633323333333533363331333333333336333333363333333333353336333633333330333333333333333633333336333633313336333633333338333333323336333533333330333333383333333733333339333633313333333833333339333333333336333433333330333333383336333433333335333633353333333933333333333633343336333333333338333333373333333333333339333633343333333533333332333633313333333033333337333633313566356633373330333633313337333233373333333633355f5f33313331__(ptr %text) #0 {
entry:
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f663634(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333303336333433333337333333383333333433333334333333373336333433363332333333323336333433363332333333313333333633363333333333373336333533333332333633323333333533363331333333333336333333363333333333353336333633333330333333333333333633333336333633313336333633333338333333323336333533333330333333383333333733333339333633313333333833333339333333333336333433333330333333383336333433333335333633353333333933333333333633343336333333333338333333373333333333333339333633343333333533333332333633313333333033333337333633313566356633373330333633313337333233373333333633355f5f3332__(ptr %text) #0 {
entry:
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f753332(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333303336333433333337333333383333333433333334333333373336333433363332333333323336333433363332333333313333333633363333333333373336333533333332333633323333333533363331333333333336333333363333333333353336333633333330333333333333333633333336333633313336333633333338333333323336333533333330333333383333333733333339333633313333333833333339333333333336333433333330333333383336333433333335333633353333333933333333333633343336333333333338333333373333333333333339333633343333333533333332333633313333333033333337333633313566356633373330333633313337333233373333333633355f5f3333__(ptr %text) #0 {
entry:
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f753634(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333303336333433333337333333383333333433333334333333373336333433363332333333323336333433363332333333313333333633363333333333373336333533333332333633323333333533363331333333333336333333363333333333353336333633333330333333333333333633333336333633313336333633333338333333323336333533333330333333383333333733333339333633313333333833333339333333333336333433333330333333383336333433333335333633353333333933333333333633343336333333333338333333373333333333333339333633343333333533333332333633313333333033333337333633313566356633373330333633313337333233373333333633355f5f3334__(ptr %text) #0 {
entry:
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f75313238(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333303336333433333337333333383333333433333334333333373336333433363332333333323336333433363332333333313333333633363333333333373336333533333332333633323333333533363331333333333336333333363333333333353336333633333330333333333333333633333336333633313336333633333338333333323336333533333330333333383333333733333339333633313333333833333339333333333336333433333330333333383336333433333335333633353333333933333333333633343336333333333338333333373333333333333339333633343333333533333332333633313333333033333337333633313566356633373330333633313337333233373333333633355f5f3335__(ptr %text) #0 {
entry:
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f6938(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333303336333433333337333333383333333433333334333333373336333433363332333333323336333433363332333333313333333633363333333333373336333533333332333633323333333533363331333333333336333333363333333333353336333633333330333333333333333633333336333633313336333633333338333333323336333533333330333333383333333733333339333633313333333833333339333333333336333433333330333333383336333433333335333633353333333933333333333633343336333333333338333333373333333333333339333633343333333533333332333633313333333033333337333633313566356633373330333633313337333233373333333633355f5f3336__(ptr %text) #0 {
entry:
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f693136(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333303336333433333337333333383333333433333334333333373336333433363332333333323336333433363332333333313333333633363333333333373336333533333332333633323333333533363331333333333336333333363333333333353336333633333330333333333333333633333336333633313336333633333338333333323336333533333330333333383333333733333339333633313333333833333339333333333336333433333330333333383336333433333335333633353333333933333333333633343336333333333338333333373333333333333339333633343333333533333332333633313333333033333337333633313566356633373330333633313337333233373333333633355f5f3337__(ptr %text) #0 {
entry:
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f693332(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333303336333433333337333333383333333433333334333333373336333433363332333333323336333433363332333333313333333633363333333333373336333533333332333633323333333533363331333333333336333333363333333333353336333633333330333333333333333633333336333633313336333633333338333333323336333533333330333333383333333733333339333633313333333833333339333333333336333433333330333333383336333433333335333633353333333933333333333633343336333333333338333333373333333333333339333633343333333533333332333633313333333033333337333633313566356633373330333633313337333233373333333633355f5f3338__(ptr %text) #0 {
entry:
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f693634(ptr %text1)
  ret i32 %call
}

define i32 @wosy_generic__776f73795f6f7665726c6f61645f5f3737366637333739356636363665356635663337333333373334333633343566356633373333333733323336333333323636333633323336363633363636333733343337333333373334333733323336333133373330333236353337333735663566333333303336333433333337333333383333333433333334333333373336333433363332333333323336333433363332333333313333333633363333333333373336333533333332333633323333333533363331333333333336333333363333333333353336333633333330333333333333333633333336333633313336333633333338333333323336333533333330333333383333333733333339333633313333333833333339333333333336333433333330333333383336333433333335333633353333333933333333333633343336333333333338333333373333333333333339333633343333333533333332333633313333333033333337333633313566356633373330333633313337333233373333333633355f5f3339__(ptr %text) #0 {
entry:
  %text1 = alloca [16 x i8], align 8
  call void @llvm.memmove.p0.p0.i32(ptr align 1 %text1, ptr align 1 %text, i32 16, i1 false)
  %call = call i32 @wosy_fn__737464__7372632f626f6f7473747261702e77__30643738343437646232646231366337653262356133636335663033363661663832653038373961383933643038643565393364633837333964353261303761__5f70617273655f69313238(ptr %text1)
  ret i32 %call
}

define i32 @main() {
entry:
  call void @wosy_fn__706f696e7465722d636173742d6469726563742d6465726566__7372632f6d61696e2e77__31346464306661643536626362366663623063663063626463643633653832306561393231306466313563376263613065336534653764353363643764623866__72756e()
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

; Function Attrs: nocallback nofree nounwind willreturn memory(argmem: readwrite)
declare void @llvm.memmove.p0.p0.i32(ptr nocapture writeonly, ptr nocapture readonly, i32, i1 immarg) #1

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

declare ptr @__wosy_core_alloc.91(i64, i64)

declare void @__wosy_core_free.92(ptr)

declare void @__wosy_core_system_panic.93()

declare ptr @__wosy_core_alloc.94(i64, i64)

declare void @__wosy_core_free.95(ptr)

declare void @__wosy_core_system_panic.96()

declare ptr @__wosy_core_alloc.97(i64, i64)

declare void @__wosy_core_free.98(ptr)

declare void @__wosy_core_system_panic.99()

declare ptr @__wosy_core_alloc.100(i64, i64)

declare void @__wosy_core_free.101(ptr)

declare void @__wosy_core_system_panic.102()

declare ptr @__wosy_core_alloc.103(i64, i64)

declare void @__wosy_core_free.104(ptr)

declare void @__wosy_core_system_panic.105()

declare ptr @__wosy_core_alloc.106(i64, i64)

declare void @__wosy_core_free.107(ptr)

declare void @__wosy_core_system_panic.108()

declare ptr @__wosy_core_alloc.109(i64, i64)

declare void @__wosy_core_free.110(ptr)

declare void @__wosy_core_system_panic.111()

declare ptr @__wosy_core_alloc.112(i64, i64)

declare void @__wosy_core_free.113(ptr)

declare void @__wosy_core_system_panic.114()

declare ptr @__wosy_core_alloc.115(i64, i64)

declare void @__wosy_core_free.116(ptr)

declare void @__wosy_core_system_panic.117()

declare ptr @__wosy_core_alloc.118(i64, i64)

declare void @__wosy_core_free.119(ptr)

declare void @__wosy_core_system_panic.120()

declare ptr @__wosy_core_alloc.121(i64, i64)

declare void @__wosy_core_free.122(ptr)

declare void @__wosy_core_system_panic.123()

declare ptr @__wosy_core_alloc.124(i64, i64)

declare void @__wosy_core_free.125(ptr)

declare void @__wosy_core_system_panic.126()

declare ptr @__wosy_core_alloc.127(i64, i64)

declare void @__wosy_core_free.128(ptr)

declare void @__wosy_core_system_panic.129()

declare ptr @__wosy_core_alloc.130(i64, i64)

declare void @__wosy_core_free.131(ptr)

declare void @__wosy_core_system_panic.132()

declare ptr @__wosy_core_alloc.133(i64, i64)

declare void @__wosy_core_free.134(ptr)

declare void @__wosy_core_system_panic.135()

declare ptr @__wosy_core_alloc.136(i64, i64)

declare void @__wosy_core_free.137(ptr)

declare void @__wosy_core_system_panic.138()

attributes #0 = { "target-features"="+bulk-memory" }
attributes #1 = { nocallback nofree nounwind willreturn memory(argmem: readwrite) }