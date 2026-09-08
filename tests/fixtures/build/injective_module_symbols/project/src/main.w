%%start
left = namespace injective_module_symbols "src/a-b.w";
right = namespace injective_module_symbols "src/a/b.w";
i32 first = left.same();
i32 second = right.same();
%%end
