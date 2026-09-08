%%start
child = namespace local_namespace_initialization_order "src/child.w";
i32 first = child.read();
i32 root = 7;
again = namespace local_namespace_initialization_order "src/child.w";
i32 second = again.read();
%%end
