%%start
child = namespace short_circuit_project "src/child.w";

bool skipped_and = false && child.right();
bool called_and = true && child.right();
bool skipped_or = true || child.right();
bool called_or = false || child.right();
%%end
