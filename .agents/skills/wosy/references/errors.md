# Diagnostics

Each diagnostic carries a code, a message, and labeled spans.

## Syntax

- `S0001`: reported when `wosy-syntax` cannot consume the current item. The report points at `syntax error originates here`. Recovery preserves neighboring items, but `parse` and `build` still fail.

## Names, duplicates, and arity

- `B0001 unknown name | unknown assignment target | unknown callable name | unknown field`: the name is absent from local scope, module members, or the selected namespace.
- `M0002 unknown module member | unknown enum variant`: the module or enum has no such member.
- `B0002 duplicate declaration | duplicate assignment target | assignment targets require a distinctness proof`: the same declaration appears twice, the same place is assigned twice without order, or two dynamic indexes cannot be proven distinct.
- `B0008 declaration collides with an existing name under ASCII case folding`: names match after lowercasing. The report labels the first conflicting declaration.
- `B0004`: arity and overload failures, including parameter/output count mismatches, calls with fewer outputs than receivers, explicit type arguments on overload calls, missing or ambiguous overload candidates, and integer literals without a unique target type.

## Types and effects

- `B0003`: type and form errors, including unknown scalar types, missing fields, invalid struct types, naming-convention violations, unsupported raw pointer or extern shapes, direct foreign arrays or checked references, type mismatches against the expected type, integer operations over non-integers, context-free `null`, string, or array literals, and dereference assignment through an immutable reference.
- `B0005`: an `if` condition has a type other than `bool`.
- `B0006 conditional branches must have equal types`: `if/else` value branches must agree.
- `B0007 assignment targets a SCREAMING_SNAKE_CASE const binding`: constants never accept assignment.
- `B0009 local binding or parameter has no required static use`: every local must be read.
- `B0010 invalid integer literal | invalid finite floating-point literal | integer literal is outside the resolved target type range`.
- `B0012 raw address requires an unsafe block | core.alloc/free requires an unsafe block`: address and allocation operations must appear inside `unsafe`.

## Memory and ownership

`wosy build` rejects these cases before producing an executable:

- `B0003 read or move of unavailable place`: the value or a field was moved, or the access conflicts with a live borrow. If shown, check the secondary span labeled `origin of conflicting place or loan`. Read from the binding that received the move, or finish using the overlapping borrow before moving or writing to its owner.
- `B0003 a field of a temporary is not an assignable or borrowable place`: you can read `(make(42)).value`. To assign to or borrow that field, bind the returned struct to a local first.
- `B0003 array elements cannot be moved individually`: the element is not a Copy value. Borrow the element instead of moving it, or move the whole array.
- `B0003 fallible allocation requires an exact proven null guard for its local binding`: guard the original fallible binding against `null` before reading it.

A checked null dereference or out-of-bounds access can instead fail **at
runtime** through `core.system_panic()`, not with a `B0003` diagnostic. See
[checked access](types.md#checked-access).

## Modules and projects

- `M0001 missing module <package>::<path>`: the imported file does not exist.
- `M0003 import cycle`: namespace edges form a cycle. The report lists every participating edge.
- `P0001`: malformed `wosy.toml`, including unexpected dependency fields and malformed builder, runner, target, or artifact tables.
- `P0002`: an unresolvable local dependency, including a missing dependency `wosy.toml`, a mismatched `package.name`, or a missing `source_root` directory.
