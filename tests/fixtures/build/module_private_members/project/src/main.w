%%start
child = namespace app "src/child.w";

i32 binding = child._binding;
i32 function = child._function();
i32 external = child._extern();
child._Private private_value = { .value = 1; };
i32 ordinary = child._ordinary(1);
char generic = child._identity('g');

i32 public_binding = child.public_binding;
i32 public_function = child.public_function();
child.Public public_value = { .value = 1; };
%%end
