from pathlib import Path
import subprocess, sys, tempfile
cli=Path(sys.argv[1]).resolve()
manifest='''
[package]
name = "app"
default_target = "app"
source_root = "src"
[comments]
categories = []
[builders]
[runners]
[targets.app]
root = "src/main.w"
[targets.app.artifacts.main]
artifact = "RuntimeStdio"
[targets.app.artifacts.main.profiles.dev]
backend = "llvm"
output = "{output}"
builder = "never"
run_runner = "never"
[targets.app.profiles.dev]
main_artifact = "main"
'''
def report(code,label):
    return '{ .stdout = ""; .stderr = "'+label+'"; .code = '+str(code)+'; .files = []; }'
def make(body, helper=""):
    return f"""%%start
    i8(i8) affine = fn(n) {{
        if (n >= 0) {{ if (n <= 4) {{ 4 - n }} else {{ -9 }} }} else {{ -9 }}
    }};
    {helper}
    unit(i8) unused = fn(input) {{
        i8 value = affine(input);
        {body}
    }};
    %%end"""

unary_body = f"""
    prune(value);
    core.require(input == 1 || input == 3, {report(31,"INVERSE")});
    if (input == 1) {{ core.require(value == 3, {report(32,"LINK")}); }};
"""
unary = make(unary_body, f"""unit(i8) prune = fn(n) {{ core.narrow(n == 1 || n == 3, {report(30,"PRUNE")}); }};""")
fixed = make(f"""
    prune(9, input);
    core.require(value == 1 || value == 4, {report(31,"FORWARD")});
    if (value == 1) {{ core.require(input == 3, {report(32,"LINK")}); }};
""", f"""unit(i8, i8) prune = fn(tag, n) {{
    if (tag == 9) {{ core.narrow(n == 0 || n == 3, {report(30,"PRUNE")}); }};
}};""")

callee_failure = make("prune(value);", f"""
    unit(i8) prune = fn(n) {{ core.require(n >= 0, {report(30,"PRUNE")}); }};
""")
# The startup call passes and marks prune as used. Declared-package must
# still report the failing call from the unused caller's affine context.
callee_failure_through_caller = callee_failure.replace("%%end", "prune(0);\n    %%end")

cases = [
    ("unary-projection-narrows-affine-input",unary,0,b""),
    ("unary-projection-preserves-exact-counterexample",unary.replace("value == 3","value == 1"),32,b"LINK"),
    ("reversed-fixed-companion-narrows-affine-output",fixed,0,b""),
    ("reversed-fixed-companion-preserves-exact-counterexample",fixed.replace("input == 3", "input == 0"),32,b"LINK"),
    ("unit-callee-failure-retains-direct-and-nested-symbolic-contexts",callee_failure,30,b"PRUNEPRUNE"),
    ("unit-callee-failure-is-not-swallowed-when-callee-is-used",callee_failure_through_caller,30,b"PRUNE"),
]
# All rows with value=1 originate at input=3. Merely intersecting the
# input/output marginals (or losing the anchor) yields a false witness.
equal = make(f"""
    i8 alias = echo(value, value);
    if (alias == 1) {{ core.require(input == 3, {report(33,"EQUAL")}); }};
""", "i8(i8,i8) echo = fn(left,right) { if (left < 0) { left } else { right } };")
equal_unit = make(unary_body.replace("prune(value)", "prune(value,value)"), f"""
    unit(i8,i8) prune = fn(other,n) {{
        if (other >= -128) {{ core.narrow(n == 1 || n == 3, {report(30,"PRUNE")}); }};
    }};
""")
fixed_integer = make(f"""
    i8 choice = split(9, value);
    if (choice == 1) {{ core.require(input == 3, {report(34,"FIXED")}); }};
    if (choice == 7) {{ core.require(input != 3, {report(35,"CONSTANT")}); }};
""", "i8(i8,i8) split = fn(tag,n) { if (tag == 9) { if (n < 2) { n } else { 7 } } else { 6 } };")
paired = make(f"""
    i8 picked = pick(value, side);
    if (side == 0) {{
        if (picked == 1) {{ core.require(input == 3, {report(36,"ANCHORED")}); }};
    }} else {{ core.require(picked == 8, {report(37,"SIBLING")}); }};
""", "i8(i8,i8) pick = fn(n,tag) { if (tag == 0) { n } else { 8 } };").replace(
    "unit(i8) unused = fn(input)", "unit(i8,i8) unused = fn(input,side)")
paired_unit = make(f"""
    prune(side, value);
    if (side == 0) {{ core.require(input == 3, {report(38,"PAIRUNIT")}); }};
""", f"""unit(i8,i8) prune = fn(tag,n) {{
    if (tag == 0) {{ core.narrow(n == 1, {report(39,"PRUNE")}); }};
}};""").replace("unit(i8) unused = fn(input)", "unit(i8,i8) unused = fn(input,side)")
unary_integer = make(f"""
    i8 original = echo(input);
    if (value == 1) {{ core.require(original == 3, {report(40,"UNARY")}); }};
""", "i8(i8) echo = fn(n) { n };")
for name,program,correct,incorrect,code,label in [
    ("equal-integer",equal,"input == 3","input == 2",33,b"EQUAL"),
    ("equal-unit",equal_unit,"value == 3","value == 1",32,b"LINK"),
    ("fixed-integer",fixed_integer,"input != 3","input == 3",35,b"CONSTANT"),
    ("independent-integer",paired,"input == 3","input == 2",36,b"ANCHORED"),
    ("independent-unit",paired_unit,"input == 3","input == 2",38,b"PAIRUNIT"),
    ("unary-integer",unary_integer,"original == 3","original == 2",40,b"UNARY"),
]:
    cases.extend([(name,program,0,b""),(name+"-witness",program.replace(correct,incorrect),code,label)])
# Use the second formal's cut to force inverse transport inside the child.
# Startup reaches the helper with an exact passing pair, ensuring any reported
# CHILD failure comes from the unused caller rather than a standalone root.
dependent_unit = make(f"""
    prune(input, value);
    if (input >= 0) {{ if (input <= 4) {{
        core.require(value == 1 || value == 3, {report(42,"PARENT")});
    }}; }};
""", f"""unit(i8,i8) prune = fn(n,mapped) {{
    if (n >= 0) {{
        if (n <= 4) {{
            core.narrow(mapped == 1 || mapped == 3, {report(43,"CUT")});
            core.require(n == 1 || n == 3, {report(44,"CHILD")});
        }};
    }};
}};""").replace("%%end", "prune(1,3);\n    %%end")
# Obtain an actual tracked alias through the ordinary integer return lane.
# A direct nonliteral lexical initializer is outside this checker slice.
dependent_integer = make(f"""
    i8 alias = echo(value);
    i8 picked = pick(alias, input);
    if (picked == 1) {{ core.require(input == 3, {report(45,"REVERSED")}); }};
    if (picked == 8) {{ core.require(input != 3, {report(46,"PARTITION")}); }};
""", """
    i8(i8) echo = fn(n) { n };
    i8(i8,i8) pick = fn(mapped,n) { if (n == 3) { mapped } else { 8 } };
""")
composed = make(f"""
    i8 image = bump(value);
    if (input == 1) {{
        core.require(value == 3, {report(47,"FORWARD3")});
        core.require(image == 4, {report(47,"FORWARD3")});
    }};
    if (image == 2) {{
        core.require(value == 1, {report(48,"BACK3")});
        core.require(input == 3, {report(48,"BACK3")});
    }};
    if (value == 2) {{
        core.require(image == 3, {report(49,"MIDDLE3")});
        core.require(input == 2, {report(49,"MIDDLE3")});
    }};
""", "i8(i8) bump = fn(n) { n + 1 };")
identity = make(f"""
    i8 again = affine(value);
    if (input >= 0) {{ if (input <= 4) {{
        if (again == 2) {{
            core.require(input == 2, {report(51,"IDENTITY")});
            core.require(value == 2, {report(51,"IDENTITY")});
        }};
    }}; }};
""")
paired_composition = make(f"""
    i8 image = bump(value, input);
    if (image == 2) {{
        core.require(input == 3, {report(52,"PAIRSUM")});
        core.require(value == 1, {report(52,"PAIRSUM")});
    }};
""", "i8(i8,i8) bump = fn(mapped,n) { if (n >= 0) { if (n <= 4) { mapped + 1 } else { 8 } } else { 8 } };")
independent_affine = make(f"""
    i8 second = affine(side);
    if (second == 1) {{ core.require(value >= -9, {report(53,"OTHER")}); }};
""").replace("unit(i8) unused = fn(input)", "unit(i8,i8) unused = fn(input,side)")
overflow = make(f"""
    i8 image = bump(value);
    core.require(image > 0, {report(54,"OVERFLOW")});
""", "i8(i8) bump = fn(n) { n + 127 };")
for name,program,correct,incorrect,code,label in [
    ("dependent-unit",dependent_unit,"core.require(n == 1 || n == 3","core.require(n == 1",44,b"CHILD"),
    ("dependent-integer-reversed-alias",dependent_integer,"input == 3,","input == 2,",45,b"REVERSED"),
    ("composed-unary",composed,"require(input == 3","require(input == 1",48,b"BACK3"),
    ("composed-identity",identity,"require(input == 2","require(input == 1",51,b"IDENTITY"),
    ("composed-dependent-pair",paired_composition,"require(input == 3","require(input == 2",52,b"PAIRSUM"),
]:
    cases.extend([(name,program,0,b""),(name+"-witness",program.replace(correct,incorrect),code,label)])
cases.extend([
    ("independent-second-affine-class-remains-unsupported",independent_affine,2,b"unsupported analysis"),
    ("composed-affine-overflow-remains-unsupported",overflow,2,b"unsupported analysis"),
])
cases.append(("previously-unsupported-lexical-exact-assertion", make(f"""
    i8 fixed = echo(input);
    core.require(fixed == 2, {report(30,"EXACT")});
    if (value == 1) {{ core.require(input == 3, {report(32,"CORRELATION")}); }};
""", "i8(i8) echo = fn(n) { n };"), 30, b"EXACT"))
direct_alias_pair = dependent_integer.replace("echo(value)", "value").replace(
    "    i8(i8) echo = fn(n) { n };\n", "")
standalone_copy = f"""%%start
    unit(i8) snapshot = fn(entry) {{
        i8 first = entry;
        i8 second = first;
        if (entry == 3) {{
            i8 scoped = second;
            core.require(scoped == 3, {report(55,"SCOPED")});
        }} else {{
            i8 scoped = first;
            core.require(scoped != 3, {report(55,"SCOPED")});
        }};
        if (second == 1) {{ core.require(entry == 1, {report(56,"STANDALONE")}); }};
    }};
    %%end"""
standalone_return = f"""%%start
    i8(i8) snapshot = fn(entry) {{
        i8 copy = entry;
        if (entry == 3) {{ core.require(copy == 3, {report(57,"RETURN-COPY")}); }};
        copy
    }};
    %%end"""
# The copy-free wrapper is itself unused. Mark the helper reached by one
# passing startup call, so its failing symbolic context can only come through
# the wrapper rather than through an independently selected symbolic root.
unary_wrapper_copy = f"""%%start
    unit(i8) helper = fn(input) {{
        i8 copy = input;
        if (input == 1) {{ core.require(copy == 1, {report(63,"WRAPPER")}); }};
    }};
    unit(i8) bridge = fn(input) {{ helper(input); }};
    unit(i8) wrapper = fn(input) {{ bridge(input); }};
    bridge(0);
    %%end"""
snapshot_write = f"""%%start
    unit(i8) snapshot = fn(entry) {{
        i8 first = entry;
        i8 saved = first;
        first = 7;
        core.require(first == 7, {report(58,"NEW")});
        if (entry == 1) {{ core.require(saved == 1, {report(59,"OLD")}); }};
    }};
    %%end"""
startup_copy = f"""%%start
    i8(i8) relay = fn(input) {{ i8 copy = input; copy }};
    unit(i8) snapshot = fn(entry) {{
        i8 copied = entry;
        i8 result = relay(copied);
        core.require(result == 3, {report(60,"STARTUP")});
    }};
    snapshot(2);
    %%end"""
affine_write = make(f"""
    i8 saved = value;
    value = 7;
    if (input == 1) {{ core.require(saved == 3, {report(61,"AFFINE-WRITE")}); }};
    core.require(value == 7, {report(62,"CURRENT")});
""")
for name,program,correct,incorrect,code,label in [
    ("direct-lexical-affine-pair",direct_alias_pair,"input == 3,","input == 2,",45,b"REVERSED"),
    ("standalone-unit-lexical-copies",standalone_copy,"entry == 1,","entry == 2,",56,b"STANDALONE"),
    ("standalone-integer-lexical-copy",standalone_return,"copy == 3","copy == 1",57,b"RETURN-COPY"),
    ("copy-free-transitive-unary-wrapper",unary_wrapper_copy,"copy == 1","copy == 2",63,b"WRAPPER"),
    ("lexical-copy-snapshot-on-local-rebind",snapshot_write,"saved == 1","saved == 7",59,b"OLD"),
]:
    cases.extend([(name,program,0,b""),(name+"-witness",program.replace(correct,incorrect),code,label)])
cases.extend([
    ("exact-startup-lexical-copy",startup_copy,60,b"STARTUP"),
    ("overwrite-affine-call-result",affine_write,0,b""),
    ("overwrite-affine-call-result-witness",affine_write.replace("saved == 3","saved == 7"),61,b"AFFINE-WRITE"),
    ("unguarded-inline-overflow",standalone_return.replace("copy = entry;", "copy = entry + 1;"),2,b"unsupported analysis"),
])
def bounds(axis, low, high):
    return (f'core.narrow({axis} >= {low}, {report(64,"RANGE")});\n'
            f'core.narrow({axis} <= {high}, {report(64,"RANGE")});')

inline_affine = f"""%%start
    unit(i8) transform = fn(entry) {{
        {bounds("entry",0,4)}
        i8 reflected = 4 - entry;
        i8 adjusted = reflected + -2;
        i8 identity = adjusted + 0;
        if (identity == 0) {{
            core.require(entry == 2, {report(65,"INLINE-BACK")});
            core.require(reflected == 2, {report(65,"INLINE-BACK")});
        }};
        if (entry == 3) {{ core.require(adjusted == -1, {report(66,"INLINE-FORWARD")}); }};
    }};
    %%end"""
inline_pair = f"""%%start
    unit(i8,i8) prune = fn(mapped, base) {{
        core.narrow(mapped == 1 || mapped == 3, {report(67,"PAIR-CUT")});
        core.require(base == 1 || base == 3, {report(68,"INLINE-PAIR")});
    }};
    unit(i8) caller = fn(entry) {{
        {bounds("entry",0,4)}
        i8 reflected = 4 - entry;
        prune(reflected, entry);
        core.require(entry == 1 || entry == 3, {report(69,"PAIR-PARENT")});
    }};
    prune(1,3);
    %%end"""
inline_return = f"""%%start
    i8(i8) transform = fn(entry) {{
        {bounds("entry",0,4)}
        i8 adjusted = entry - 1;
        if (adjusted == 2) {{ core.require(entry == 3, {report(70,"INLINE-RETURN")}); }};
        adjusted
    }};
    %%end"""
inline_wrapper = f"""%%start
    unit(i8) helper = fn(entry) {{
        {bounds("entry",0,4)}
        i8 adjusted = entry + 1;
        if (adjusted == 3) {{ core.require(entry == 2, {report(71,"INLINE-WRAPPER")}); }};
    }};
    unit(i8) wrapper = fn(entry) {{ helper(entry); }};
    helper(0);
    %%end"""
inline_startup = f"""%%start
    unit(i8) exact = fn(entry) {{
        i8 adjusted = -2 - entry;
        core.require(adjusted == 0, {report(72,"INLINE-STARTUP")});
    }};
    exact(1);
    %%end"""
inline_two_broad = f"""%%start
    unit(i8,i8) exact = fn(entry, offset) {{
        {bounds("entry",0,4)}
        {bounds("offset",0,4)}
        i8 adjusted = entry + offset;
        core.require(adjusted >= 0, {report(73,"INLINE-DUAL")});
    }};
    %%end"""
inline_second_class = f"""%%start
    unit(i8,i8) exact = fn(entry, offset) {{
        {bounds("entry",0,4)}
        {bounds("offset",0,4)}
        i8 first = 4 - entry;
        i8 other = offset - 1;
        if (first == 0) {{ core.require(other <= 3, {report(74,"INLINE-OTHER")}); }};
    }};
    %%end"""
for name,program,correct,incorrect,code,label in [
    ("inline-reflected-composed",inline_affine,"entry == 2,","entry == 3,",65,b"INLINE-BACK"),
    ("inline-dependent-pair",inline_pair,"core.require(base == 1 || base == 3","core.require(base == 3",68,b"INLINE-PAIR"),
    ("inline-integer-root",inline_return,"entry == 3,","entry == 4,",70,b"INLINE-RETURN"),
    ("inline-arithmetic-through-copy-free-wrapper",inline_wrapper,"entry == 2,","entry == 4,",71,b"INLINE-WRAPPER"),
]:
    cases.extend([(name,program,0,b""),(name+"-witness",program.replace(correct,incorrect),code,label)])
cases.extend([
    ("exact-startup-inline-arithmetic",inline_startup,72,b"INLINE-STARTUP"),
    ("inline-finite-multiplication-refines-integer-root",inline_return.replace("entry - 1","entry * 1").replace(
        report(70,"INLINE-RETURN"), report(70,"INLINE-RETURN").replace(".files = []", '.files = [{ .path = "state.json"; .state = entry; }]'),
    ),70,b"INLINE-RETURN"),
    ("inline-two-lexical-multiplication-remains-unsupported",inline_return.replace("entry - 1","entry * entry"),2,b"unsupported analysis"),
    ("inline-two-broad-operands-remain-unsupported",inline_two_broad,2,b"unsupported analysis"),
    ("inline-unrelated-second-class-remains-unsupported",inline_second_class,2,b"unsupported analysis"),
])
overwrite_member = f"""%%start
    unit(i8) transform = fn(entry) {{
        {bounds("entry",0,4)}
        i8 first = 4 - entry;
        i8 second = first + 1;
        i8 saved = first;
        first = 7;
        core.require(first == 7, {report(75,"NEW-MEMBER")});
        if (second == 2) {{
            core.require(saved == 1, {report(76,"OLD-MEMBER")});
            core.require(entry == 3, {report(77,"OTHER-MEMBER")});
        }};
    }};
    %%end"""
overwrite_anchor = f"""%%start
    unit(i8) transform = fn(entry) {{
        {bounds("entry",0,4)}
        i8 first = 4 - entry;
        i8 second = first + 1;
        entry = 7;
        core.require(entry == 7, {report(78,"NEW-ANCHOR")});
        if (second == 2) {{ core.require(first == 1, {report(79,"REANCHOR")}); }};
    }};
    %%end"""
overwrite_anchor_alias = overwrite_anchor.replace(
    "entry = 7;", "i8 saved = entry;\n        entry = 7;").replace(
    "core.require(first == 1", f'core.require(saved == 3, {report(80,"OLD-ANCHOR")}); core.require(first == 1')
overwrite_alias = overwrite_member.replace("first = 7;", "saved = 7;").replace(
    "core.require(first == 7", "core.require(saved == 7").replace(
    "core.require(saved == 1", "core.require(first == 1")
projected_snapshot = f"""%%start
    unit(i8) gate = fn(entry) {{
        if (entry < 5) {{
            i8 saved = entry;
            i8 flipped = 4 - entry;
            entry = 7;
            core.require(entry == 7, {report(81,"NEW-PROJECTED")});
            core.narrow(flipped == 1 || flipped == 3, {report(82,"PROJECTED-CUT")});
            if (flipped == 1) {{ core.require(saved == 3, {report(83,"PROJECTED-ENTRY")}); }};
        }};
    }};
    unit(i8) original = fn(input) {{
        {bounds("input",0,4)}
        gate(input);
        core.require(input == 1 || input == 3, {report(84,"CALLER-ENTRY")});
    }};
    gate(5);
    %%end"""
for name,program,correct,incorrect,code,label in [
    ("overwrite-affine-canonical-member",overwrite_member,"saved == 1","saved == 7",76,b"OLD-MEMBER"),
    ("overwrite-affine-anchor-without-snapshot",overwrite_anchor,"first == 1","first == 3",79,b"REANCHOR"),
    ("overwrite-affine-anchor-with-snapshot",overwrite_anchor_alias,"saved == 3","saved == 7",80,b"OLD-ANCHOR"),
    ("overwrite-affine-alias-only",overwrite_alias,"first == 1","first == 7",76,b"OLD-MEMBER"),
    ("overwrite-projected-formal-through-snapshot",projected_snapshot,"saved == 3","saved == 7",83,b"PROJECTED-ENTRY"),
]:
    cases.extend([(name,program,0,b""),(name+"-witness",program.replace(correct,incorrect),code,label)])
cases.extend([
    ("overwrite-projected-formal-without-snapshot-refused",projected_snapshot.replace(
        "i8 saved = entry;","").replace("core.require(saved == 3","core.require(flipped == 1"),2,b"unsupported analysis"),
    ("nonliteral-affine-reassignment-remains-unsupported",overwrite_member.replace("first = 7;","first = second;"),2,b"unsupported analysis"),
])
compare_odd = f"""%%start
    unit(i8) compare = fn(entry) {{
        {bounds("entry",-4,1)}
        i8 reflected = -3 - entry;
        if (reflected > entry) {{ core.require(entry <= -2, {report(85,"THEN")}); }}
        else {{ core.require(entry >= -1, {report(86,"ELSE")}); }};
        core.require(reflected != entry, {report(87,"ODD")});
    }};
    %%end"""
compare_unsigned = f"""%%start
    unit(u8) compare = fn(entry) {{
        core.narrow(entry <= 4, {report(88,"BOUNDS")});
        u8 reflected = 4 - entry;
        core.narrow(reflected == entry, {report(89,"UNSIGNED")});
        core.require(entry == 2, {report(90,"EQUALITY")});
    }};
    %%end"""
compare_member = f"""%%start
    unit(i8) compare = fn(entry) {{
        {bounds("entry",0,4)}
        i8 elevated = entry + 1;
        i8 same = elevated;
        core.require(same >= elevated, {report(91,"ALIAS")});
        core.require(entry < elevated, {report(92,"SAME-SLOPE")});
    }};
    %%end"""
compare_pair = f"""%%start
    unit(i8,i8) verify = fn(mapped,base) {{
        if (mapped < base) {{ core.require(base >= 3, {report(93,"PREPARED-PAIR")}); }};
    }};
    unit(i8) caller = fn(entry) {{
        {bounds("entry",0,4)}
        i8 reflected = 4 - entry;
        verify(reflected, entry);
    }};
    verify(0,0);
    %%end"""
compare_startup = f"""%%start
    unit(i8,i8) check = fn(left,right) {{
        core.require(left == right, {report(94,"ACTUAL-COMPARE")});
    }};
    check(1,2);
    %%end"""
independent_compare = f"""%%start
    unit(i8,i8) check = fn(left,right) {{
        core.require(left <= right, {report(95,"INDEPENDENT")});
    }};
    %%end"""
for name,program,correct,incorrect,code,label in [
    ("odd-negative-comparison",compare_odd,"entry <= -2","entry <= -3",85,b"THEN"),
    ("unsigned-reflexive-cut",compare_unsigned,"entry == 2","entry == 0",90,b"EQUALITY"),
    ("same-slope-and-alias-comparison",compare_member,"entry < elevated","entry > elevated",92,b"SAME-SLOPE"),
    ("related-comparison-inside-pair",compare_pair,"base >= 3","base >= 4",93,b"PREPARED-PAIR"),
]:
    cases.extend([(name,program,0,b""),(name+"-witness",program.replace(correct,incorrect),code,label)])
cases.extend([
    ("lexical-comparisons-keep-arm-diagnostic-order",compare_odd.replace("entry <= -2","entry <= -3")
     .replace("entry >= -1","entry >= 0"),85,b"THENELSE"),
    ("exact-startup-lexical-comparison",compare_startup,94,b"ACTUAL-COMPARE"),
    ("independent-lexical-comparison-remains-unsupported",independent_compare,2,b"unsupported analysis"),
])
compound_branch = f"""%%start
    unit(i8) check = fn(entry) {{
        {bounds("entry",0,6)}
        i8 reflected = 6 - entry;
        if (reflected > entry && entry >= 2) {{
            core.require(entry == 2, {report(96,"COMPOUND-THEN")});
        }} else {{
            core.require(entry != 2, {report(97,"COMPOUND-ELSE")});
        }};
    }};
    %%end"""
compound_require = f"""%%start
    unit(i8) check = fn(entry) {{
        {bounds("entry",-4,1)}
        i8 reflected = -3 - entry;
        core.require((entry <= -2 && reflected > entry) || (entry >= -1 && reflected < entry),
            {report(98,"COMPOUND-REQUIRE")});
    }};
    %%end"""
compound_narrow = f"""%%start
    unit(u8) check = fn(entry) {{
        core.narrow(entry <= 4, {report(99,"BOUND")});
        u8 reflected = 4 - entry;
        core.narrow(!(entry == reflected) && (entry == 1 || reflected == 1),
            {report(100,"COMPOUND-NARROW")});
        core.require(entry == 1 || entry == 3, {report(101,"COMPOUND-REFINE")});
    }};
    %%end"""
compound_short = f"""%%start
    unit(i8,i8) check = fn(entry, other) {{
        {bounds("entry",0,4)}
        if (entry > 4 && entry < other) {{ core.require(false, {report(102,"DEAD-RHS")}); }};
        core.require(entry <= 4 || entry < other, {report(103,"SHORT-OR")});
    }};
    %%end"""
compound_narrow_empty = f"""%%start
    unit(u8) check = fn(entry) {{
        core.narrow(entry <= 4, {report(104,"BOUND")});
        u8 reflected = 4 - entry;
        core.narrow(entry == reflected && entry != reflected, {report(105,"EMPTY-COMPOUND")});
    }};
    %%end"""
for name,program,correct,incorrect,code,label in [
    ("compound-lexical-branch",compound_branch,"entry == 2,","entry == 3,",96,b"COMPOUND-THEN"),
    ("compound-signed-require",compound_require,"entry >= -1 &&","entry >= 0 &&",98,b"COMPOUND-REQUIRE"),
    ("compound-unsigned-narrow",compound_narrow,"entry == 1 || entry == 3","entry == 1 || entry == 2",101,b"COMPOUND-REFINE"),
]:
    cases.extend([(name,program,0,b""),(name+"-witness",program.replace(correct,incorrect),code,label)])
cases.extend([
    ("compound-skips-unreachable-independent-rhs",compound_short,0,b""),
    # The RHS compares two independent broad inputs only when at least two
    # left values survive. A singleton left value is an admissible literal cut.
    ("compound-reachable-small-independent-rhs",compound_short.replace("entry > 4 &&","entry >= 3 &&"),102,b"DEAD-RHS"),
    ("compound-reachable-large-independent-rhs-refused",compound_short.replace(
        "core.narrow(entry <= 4","core.narrow(entry <= 30").replace("entry > 4 &&","entry >= 3 &&"),
     2,b"unsupported analysis"),
    ("compound-reached-singleton-rhs-can-report",compound_short.replace("entry > 4 &&","entry >= 4 &&"),102,b"DEAD-RHS"),
    ("compound-contradiction-reports-narrow",compound_narrow_empty,105,b"EMPTY-COMPOUND"),
])
# Repeated always-true legacy formal/literal ORs must not grow alternative
# rows or run out of the new compound lane's transfer budget.
legacy_tautology = f"""%%start
    unit(i8) check = fn(entry) {{
        {" ".join(f'if (entry < 0 || entry >= 0) {{ core.require(entry >= -128, {report(106,"LEGACY")}); }};' for _ in range(96))}
    }};
    %%end"""
oversized_rhs = " && ".join(["(entry >= 0)"] * 34)
short_oversized = f"""%%start
    unit(i8) check = fn(entry) {{
        {bounds("entry",0,4)}
        core.require(entry <= 4 || ({oversized_rhs}), {report(107,"OVERSIZED")});
    }};
    %%end"""
cases.extend([
    ("legacy-formal-tautologies-stay-compact",legacy_tautology,0,b""),
    ("compound-dead-oversized-rhs-is-not-analyzed",short_oversized,0,b""),
    ("compound-reached-oversized-rhs-is-unrepresentable",short_oversized.replace("entry <= 4 ||","entry < 4 ||"),2,b"unsupported analysis"),
])
independent_rectangle = f"""%%start
    unit(i8,i8) check = fn(left,right) {{
        {bounds("left",0,4)}
        core.narrow(left < right, {report(108,"RELATIVE-CUT")});
        core.require(left < right, {report(109,"NO-FALSE-PAIR")});
        if (left == 3) {{ core.require(right >= 4, {report(110,"RECTANGLE")}); }};
    }};
    %%end"""
independent_holes = f"""%%start
    unit(i16,i16) check = fn(left,right) {{
        core.narrow(left <= -100 || left >= 100, {report(111,"HOLES")});
        {bounds("right",-64,64)}
        core.require(left != right, {report(112,"DISJOINT-HOLES")});
    }};
    %%end"""
independent_anchored = f"""%%start
    unit(i8,i8) clip = fn(left,right) {{
        core.narrow(left < right, {report(113,"CHILD-CUT")});
    }};
    unit(i8,i8) caller = fn(entry,other) {{
        {bounds("entry",0,4)}
        i8 reflected = 4 - entry;
        clip(reflected,other);
        core.require(reflected < other, {report(114,"ANCHORED-RELATIVE")});
        if (entry == 4) {{ core.require(other >= 1, {report(115,"ANCHORED-INVERSE")}); }};
    }};
    clip(0,1);
    %%end"""
for name,program,correct,incorrect,code,label in [
    ("bounded-independent-relative-narrow",independent_rectangle,"right >= 4","right >= 5",110,b"RECTANGLE"),
    ("large-independent-holed-disjoint-shortcut",independent_holes,"left != right","left == right",112,b"DISJOINT-HOLES"),
    ("anchored-independent-comparison-plus-live-affine",independent_anchored,"other >= 1","other >= 2",115,b"ANCHORED-INVERSE"),
]:
    cases.extend([(name,program,0,b""),(name+"-witness",program.replace(correct,incorrect),code,label)])
bool_local = f"""%%start
    unit(bool) check = fn(enabled) {{
        bool copied = enabled;
        bool valid = true;
        if (copied) {{
            core.require(enabled && valid, {report(116,"COPIED-TRUE")});
        }} else {{
            core.require(!enabled, {report(117,"COPIED-FALSE")});
        }};
    }};
    %%end"""
bool_startup = f"""%%start
    unit(bool) check = fn(enabled) {{
        if (enabled) {{ core.require(false, {report(118,"ACTUAL-TRUE")}); }};
    }};
    check(false);
    check(true);
    %%end"""
bool_unary = f"""%%start
    unit(bool) gate = fn(enabled) {{
        core.narrow(enabled, {report(119,"BOOL-CUT")});
    }};
    unit(bool) caller = fn(enabled) {{
        gate(enabled);
        core.require(enabled, {report(120,"BOOL-UNARY")});
    }};
    gate(true);
    %%end"""
bool_pair = f"""%%start
    unit(bool,i8) prune = fn(enabled, number) {{
        if (enabled) {{ core.narrow(number <= 0, {report(121,"BOOL-PAIR-CUT")}); }}
        else {{ core.narrow(number >= 1, {report(121,"BOOL-PAIR-CUT")}); }};
    }};
    unit(bool,i8) caller = fn(enabled, number) {{
        prune(enabled, number);
        if (number > 0) {{ core.require(!enabled, {report(122,"BOOL-PAIR")}); }};
        if (enabled) {{ core.require(number <= 0, {report(123,"BOOL-FORWARD")}); }};
    }};
    prune(true,0);
    %%end"""
bool_fixed = f"""%%start
    unit(bool,i8) prune = fn(enabled, number) {{
        if (enabled) {{ core.narrow(number <= 0, {report(124,"FIXED-CUT")}); }};
    }};
    unit(i8) caller = fn(number) {{
        prune(true, number);
        core.require(number <= 0, {report(125,"BOOL-FIXED")});
    }};
    %%end"""
for name,program,correct,incorrect,code,label in [
    ("bool-symbolic-local-copy",bool_local,"!enabled,","enabled,",117,b"COPIED-FALSE"),
    ("bool-projected-unary",bool_unary,"require(enabled,","require(!enabled,",120,b"BOOL-UNARY"),
    ("mixed-bool-integer-anchored-pair",bool_pair,"require(!enabled,","require(enabled,",122,b"BOOL-PAIR"),
    ("fixed-bool-literal-with-symbolic-integer",bool_fixed,"require(number <= 0","require(number >= 0",125,b"BOOL-FIXED"),
]:
    cases.extend([(name,program,0,b""),(name+"-witness",program.replace(correct,incorrect),code,label)])
cases.append(("exact-startup-bool-argument",bool_startup,118,b"ACTUAL-TRUE"))
bool_result = f"""%%start
    bool(i8) allowed = fn(entry) {{ if (entry < 4) {{ true }} else {{ false }} }};
    unit(i8) check = fn(entry) {{
        bool result = allowed(entry);
        if (result && entry >= 0) {{
            core.require(entry <= 3, {report(126,"BOOL-TO-ENTRY")});
        }};
        if (entry >= 4) {{ core.require(!result, {report(127,"ENTRY-TO-BOOL")}); }};
    }};
    %%end"""
bool_mixed_result = f"""%%start
    bool(bool,i8) allowed = fn(enabled, number) {{
        if (enabled) {{ if (number > 2) {{ true }} else {{ false }} }} else {{ false }}
    }};
    unit(bool,i8) check = fn(enabled, number) {{
        bool result = allowed(enabled, number);
        core.narrow(result, {report(128,"BOOL-RESULT-CUT")});
        core.require(enabled && number > 2, {report(129,"PAIR-BOOL-RESULT")});
    }};
    %%end"""
bool_alias_result = f"""%%start
    bool(bool) echo = fn(enabled) {{ enabled }};
    unit(bool) check = fn(enabled) {{
        bool result = echo(enabled);
        if (result) {{ core.require(enabled, {report(130,"BOOL-RESULT-ALIAS")}); }}
        else {{ core.require(!enabled, {report(131,"BOOL-RESULT-ALIAS-NO")}); }};
    }};
    %%end"""
bool_fixed_result = f"""%%start
    bool(i8,i8) allowed = fn(entry, limit) {{ if (entry < limit) {{ true }} else {{ false }} }};
    unit(i8) check = fn(entry) {{
        bool result = allowed(entry, -1);
        if (result) {{ core.require(entry <= -2, {report(132,"BOOL-RESULT-FIXED")}); }};
    }};
    %%end"""
bool_exact_result = f"""%%start
    bool() ready = fn {{ true }};
    unit() check = fn {{
        bool result = ready();
        core.require(!result, {report(133,"EXACT-BOOL-RESULT")});
    }};
    check();
    %%end"""
bool_independent_root = f"""%%start
    bool(i8) classified = fn(entry) {{
        core.require(entry >= 0, {report(134,"UNUSED-BOOL-ROOT")});
        if (entry < 4) {{ true }} else {{ false }}
    }};
    %%end"""
for name,program,correct,incorrect,code,label in [
    ("staged-bool-result-feeds-compound",bool_result,"entry <= 3","entry <= 2",126,b"BOOL-TO-ENTRY"),
    ("staged-bool-result-mixed-pair",bool_mixed_result,"enabled && number > 2","enabled && number > 3",129,b"PAIR-BOOL-RESULT"),
    ("staged-bool-result-keeps-input-alias",bool_alias_result,"require(enabled,","require(!enabled,",130,b"BOOL-RESULT-ALIAS"),
    ("staged-bool-result-fixed-signed-argument",bool_fixed_result,"entry <= -2","entry <= -3",132,b"BOOL-RESULT-FIXED"),
]:
    cases.extend([(name,program,0,b""),(name+"-witness",program.replace(correct,incorrect),code,label)])
cases.extend([
    ("exact-startup-staged-bool-result",bool_exact_result,133,b"EXACT-BOOL-RESULT"),
    ("unused-bool-return-root-keeps-requirement",bool_independent_root,134,b"UNUSED-BOOL-ROOT"),
    ("direct-bool-call-feeds-compound-guard",bool_result.replace(
        "if (result && entry >= 0)", "if (allowed(entry) && entry >= 0)"),0,b""),
    ("elided-directive-bool-call-remains-unsupported",bool_result.replace(
        "core.require(!result,", "core.require(!result && allowed(entry),"),2,b"unsupported analysis"),
])
bool_staged_root = f"""%%start
    bool(i8) selected = fn(entry) {{ if (entry < 4) {{ true }} else {{ false }} }};
    bool(i8) check = fn(entry) {{
        bool result = selected(entry);
        if (result) {{ core.require(entry <= 3, {report(135,"BOOL-ROOT-CHILD")}); }};
        result
    }};
    %%end"""
cases.extend([
    ("unused-bool-root-with-staged-return-needs-no-receipt",bool_staged_root,0,b""),
    ("unused-bool-root-with-staged-return-still-reports-witness",
     bool_staged_root.replace("entry <= 3","entry <= 2"),135,b"BOOL-ROOT-CHILD"),
])
guard_dead = f"""%%start
    bool(i8) observed = fn(entry) {{
        core.require(entry == 0, {report(136,"REACHED-CALLEE")});
        true
    }};
    unit() startup = fn {{
        bool result = observed(0);
        core.require(result, {report(137,"STARTUP-CALLEE")});
    }};
    unit(i8) check = fn(entry) {{
        {bounds("entry",0,4)}
        if (entry > 4 && observed(entry)) {{ core.require(entry <= 4, {report(138,"GUARD-AND")}); }};
        if (entry <= 4 || observed(entry)) {{ core.require(entry <= 4, {report(139,"GUARD-OR")}); }};
    }};
    startup();
    %%end"""
guard_mixed = f"""%%start
    bool(bool,i8) allowed = fn(enabled, number) {{
        if (enabled) {{ if (number > 2) {{ true }} else {{ false }} }} else {{ false }}
    }};
    unit(bool,i8) check = fn(enabled, number) {{
        if (allowed(enabled, number)) {{
            core.require(enabled && number > 2, {report(140,"DIRECT-PAIR")});
        }} else {{
            if (enabled) {{ core.require(number <= 2, {report(141,"DIRECT-PAIR-FALSE")}); }};
        }};
    }};
    %%end"""
guard_terminal = f"""%%start
    bool(i8) first = fn(entry) {{ if (entry < 4) {{ true }} else {{ false }} }};
    bool(i8) second = fn(entry) {{ if (entry < 2) {{ true }} else {{ false }} }};
    bool(i8) selected = fn(entry) {{
        if (first(entry) && !second(entry)) {{ true }} else {{ false }}
    }};
    unit(i8) check = fn(entry) {{
        bool result = selected(entry);
        if (result) {{ core.require(entry >= 2 && entry <= 3, {report(142,"TERMINAL-GUARD")}); }};
    }};
    %%end"""
guard_alias = f"""%%start
    bool(bool) echo = fn(flag) {{ flag }};
    unit(bool) check = fn(flag) {{
        if (echo(flag)) {{ core.require(flag, {report(143,"TRUE-ALIAS")}); }}
        else {{ core.require(!flag, {report(144,"FALSE-ALIAS")}); }};
    }};
    %%end"""
for name,program,correct,incorrect,code,label in [
    ("direct-bool-call-retains-pair-anchors",guard_mixed,"enabled && number > 2","enabled && number > 3",140,b"DIRECT-PAIR"),
    ("direct-bool-call-terminal-compound",guard_terminal,"entry >= 2 && entry <= 3","entry >= 3 && entry <= 3",142,b"TERMINAL-GUARD"),
    ("direct-bool-call-raw-alias-does-not-escape",guard_alias,"require(flag,","require(!flag,",143,b"TRUE-ALIAS"),
]:
    cases.extend([(name,program,0,b""),(name+"-witness",program.replace(correct,incorrect),code,label)])
cases.extend([
    ("direct-bool-call-dead-rhs-never-activates",guard_dead,0,b""),
    ("direct-bool-call-reached-rhs-propagates-callee-failure",guard_dead.replace("entry > 4 &&","entry >= 4 &&"),
     136,b"REACHED-CALLEE"),
])
guard_actual = guard_dead.replace("startup();", "startup();\n    check(4);")
cases.extend([
    ("exact-startup-direct-guard-skips-dead-call",guard_actual,0,b""),
    ("exact-startup-direct-guard-reached-call-reports",
     guard_actual.replace("entry > 4 &&","entry >= 4 &&"),136,b"REACHED-CALLEE"),
])
directive_probe = f"""%%start
    bool(i8) admitted = fn(entry) {{ if (entry < 4) {{ true }} else {{ false }} }};
    unit(i8) check = fn(entry) {{
        core.narrow(admitted(entry), {report(145,"DIRECT-NARROW")});
        core.require(entry <= 3, {report(146,"PROBE-REFINEMENT")});
    }};
    %%end"""
directive_preserve = f"""%%start
    bool(i8) admitted = fn(entry) {{ if (entry < 4) {{ true }} else {{ false }} }};
    unit(i8) check = fn(entry) {{
        core.require(admitted(entry), {report(147,"DIRECT-REQUIRE")});
        if (entry == 5) {{ core.require(false, {report(148,"REQUIRE-KEEPS-PATH")}); }};
    }};
    %%end"""
directive_fixed = f"""%%start
    bool(bool,i8) admitted = fn(flag, entry) {{
        if (flag) {{ if (entry < 4) {{ true }} else {{ false }} }} else {{ false }}
    }};
    unit(i8) check = fn(entry) {{
        core.narrow(admitted(true,entry), {report(149,"DIRECT-FIXED")});
        core.require(entry <= 3, {report(150,"DIRECT-FIXED-OUT")});
    }};
    %%end"""
directive_actual = directive_probe.replace("%%end","check(5);\n    %%end")
directive_impure = directive_probe.replace("if (entry < 4)", f'core.require(entry >= 0, {report(151,"IMPURE")}); if (entry < 4)')
cases.extend([
    ("bare-directive-predicate-narrows-symbolic-input",directive_probe,0,b""),
    ("bare-directive-predicate-narrow-witness",directive_probe.replace("entry <= 3","entry <= 2"),146,b"PROBE-REFINEMENT"),
    ("bare-directive-require-keeps-original-continuation",directive_preserve,147,b"DIRECT-REQUIRE" b"REQUIRE-KEEPS-PATH"),
    ("bare-directive-predicate-fixed-bool",directive_fixed,0,b""),
    ("exact-startup-bare-directive-reports-empty-narrow",directive_actual,145,b"DIRECT-NARROW"),
    ("hypothetical-directive-callee-require-refused",directive_impure,2,b"unsupported analysis"),
])
context_unit = f"""%%start
    unit(i8) keep = fn(entry) {{ core.narrow(entry >= 0, {report(152,"CONTEXT-UNIT-CUT")}); }};
    unit(i8) check = fn(input) {{
        i8 core.context.count = input;
        keep(core.context.count);
        core.require(input >= 0, {report(153,"CONTEXT-ALIAS-CUT")});
        if (input == 2) {{
            i8 core.context.count = core.context.count;
            i8 saved = core.context.count;
            core.context.count = 7;
            core.require(saved == 2, {report(154,"CONTEXT-SNAPSHOT")});
            core.require(core.context.count == 7, {report(155,"CONTEXT-INNER")});
        }};
        if (input == 2) {{ core.require(core.context.count == 2, {report(156,"CONTEXT-RESTORED")}); }};
    }};
    keep(0);
    %%end"""
context_integer = make(f"""
    i8 core.context.count = input;
    i8 other = affine(core.context.count);
    if (other == 1) {{ core.require(core.context.count == 3, {report(157,"CONTEXT-INT-RESULT")}); }};
""").replace("i8 value = affine(input);", "")
context_affine_write = f"""%%start
    unit(i8) check = fn(input) {{
        {bounds("input",0,4)}
        i8 mapped = 4 - input;
        i8 core.context.count = mapped;
        i8 saved = core.context.count;
        core.context.count = 7;
        if (input == 3) {{ core.require(saved == 1, {report(158,"CONTEXT-AFFINE-SNAPSHOT")}); }};
        core.require(core.context.count == 7, {report(159,"CONTEXT-AFFINE-WRITE")});
    }};
    %%end"""
context_bool_actual = f"""%%start
    bool(i8) exactly = fn(entry) {{ if (entry == 3) {{ true }} else {{ false }} }};
    unit(i8) check = fn(input) {{
        i8 core.context.count = input;
        bool matches = exactly(core.context.count);
        core.require(matches, {report(160,"CONTEXT-BOOL-RESULT")});
    }};
    check(4);
    %%end"""
for name,program,correct,incorrect,code,label in [
    ("context-symbolic-unit-projects-and-restores",context_unit,"core.context.count == 2,","core.context.count == 7,",156,b"CONTEXT-RESTORED"),
    ("context-symbolic-integer-return",context_integer,"core.context.count == 3","core.context.count == 2",157,b"CONTEXT-INT-RESULT"),
    ("context-affine-write-detaches-snapshot",context_affine_write,"saved == 1","saved == 7",158,b"CONTEXT-AFFINE-SNAPSHOT"),
]:
    cases.extend([(name,program,0,b""),(name+"-witness",program.replace(correct,incorrect),code,label)])
cases.append(("exact-startup-contextual-input-to-bool-result",context_bool_actual,160,b"CONTEXT-BOOL-RESULT"))
context_declaration_only = f"""%%start
    unit(i8) check = fn(input) {{
        i8 core.context.count = input;
        if (input == 2) {{ core.require(input == 2, {report(161,"CONTEXT-DECLARATION-ONLY")}); }};
    }};
    %%end"""
cases.extend([
    ("ordinary-context-declaration-only-keeps-context",context_declaration_only,0,b""),
    ("ordinary-context-declaration-only-still-reports",
     context_declaration_only.replace("require(input == 2","require(input == 7"),161,b"CONTEXT-DECLARATION-ONLY"),
])
def state_report(code, label, selected, extra=""):
    return ('{ .stdout = ""; .stderr = "'+label+'"; .code = '+str(code)
            +'; .files = [{ .path = "state.json"; .state = '+selected+'; }'+extra+']; }')

real_boolean_initializer = f"""%%start
    bool(bool,i8) choose = fn(flag, entry) {{
        core.require(entry != 3, {state_report(201,"BOOL-CALLEE","entry")});
        flag
    }};
    bool initialized = false;
    bool selected = choose(initialized, 3);
    if (selected) {{ core.require(false, {report(202,"TRUE-STARTUP-ARM")}); }}
    else {{ core.require(false, {report(203,"FALSE-STARTUP-ARM")}); }};
    %%end"""
real_boolean_initializer_nullary = f"""%%start
    bool() pick = fn {{ true }};
    bool selected = pick();
    if (selected) {{ core.require(false, {report(204,"NULLARY-BOOL-TRUE")}); }};
    %%end"""
cases.extend([
    ("exact-startup-bool-return-initializes-false-module-value",real_boolean_initializer,201,b"BOOL-CALLEEFALSE-STARTUP-ARM"),
    ("exact-startup-bool-return-initializes-true-module-value",real_boolean_initializer.replace("initialized = false","initialized = true"),201,b"BOOL-CALLEETRUE-STARTUP-ARM"),
    ("exact-startup-nullary-bool-return-selects-arm",real_boolean_initializer_nullary,204,b"NULLARY-BOOL-TRUE"),
    ("exact-startup-bool-integer-narrow-prunes-initializer",
     real_boolean_initializer.replace("core.require(entry != 3,","core.narrow(entry != 3,"),201,b"BOOL-CALLEE"),
])
real_boolean_narrow_helper = f"""
    bool(bool) gate = fn(flag) {{
        core.narrow(flag, {state_report(205,"BOOL-NARROW-PRESTATE","flag")});
        flag
    }};
"""
real_boolean_narrow_initializer = f"""%%start
    {real_boolean_narrow_helper}
    bool selected = gate(false);
    if (selected) {{ core.require(false, {report(206,"BOOL-NARROW-TRUE")}); }}
    else {{ core.require(false, {report(207,"BOOL-NARROW-FALSE")}); }};
    %%end"""
real_boolean_narrow_local = f"""%%start
    {real_boolean_narrow_helper}
    unit() startup = fn {{
        bool selected = gate(false);
        if (selected) {{ core.require(false, {report(206,"BOOL-NARROW-TRUE")}); }}
        else {{ core.require(false, {report(207,"BOOL-NARROW-FALSE")}); }};
    }};
    startup();
    %%end"""
real_boolean_narrow_nested = real_boolean_narrow_local.replace(
    "unit() startup", "bool(bool) forward = fn(flag) { bool value = gate(flag); value };\n    unit() startup",
).replace("bool selected = gate(false);", "bool selected = forward(false);")
hypothetical_boolean_narrow = f"""%%start
    {real_boolean_narrow_helper}
    unit() startup = fn {{ core.require(gate(true), {report(208,"HYPOTHETICAL-NARROW")}); }};
    startup();
    %%end"""
cases.extend([
    ("exact-startup-lexical-bool-narrow-empties-initializer",real_boolean_narrow_initializer,205,b"BOOL-NARROW-PRESTATE"),
    ("exact-startup-lexical-bool-narrow-empties-bound-local",real_boolean_narrow_local,205,b"BOOL-NARROW-PRESTATE"),
    ("exact-startup-lexical-bool-narrow-empties-bound-chain",real_boolean_narrow_nested,205,b"BOOL-NARROW-PRESTATE"),
    ("exact-startup-lexical-bool-narrow-survives-bound-chain",real_boolean_narrow_nested.replace("forward(false)","forward(true)"),206,b"BOOL-NARROW-TRUE"),
    ("exact-startup-lexical-bool-narrow-survives",real_boolean_narrow_initializer.replace("gate(false)","gate(true)"),206,b"BOOL-NARROW-TRUE"),
    ("exact-startup-hypothetical-bool-narrow-remains-refused",hypothetical_boolean_narrow,2,b"unsupported analysis"),
])
real_nullary_initializer = f"""%%start
    i8() pick = fn {{
        core.require(false, {report(234,"NULLARY-CALLEE")});
        7
    }};
    i8 saved = pick();
    core.require(saved != 7, {report(235,"NULLARY-CALLER")});
    %%end"""
real_nullary_local = f"""%%start
    i8() pick = fn {{
        core.require(false, {report(234,"NULLARY-CALLEE")});
        7
    }};
    unit() startup = fn {{
        i8 local = pick();
        core.require(local != 7, {report(235,"NULLARY-LOCAL-CALLER")});
    }};
    startup();
    %%end"""
real_bool_initializer = f"""%%start
    i8(bool,i8) choose = fn(flag, entry) {{
        if (flag) {{ entry }} else {{ 5 }}
    }};
    bool selected = false;
    i8 saved = choose(selected, 8);
    core.require(saved != 5, {report(236,"BOOL-ARG-FALSE")});
    %%end"""
real_bool_nested_local = f"""%%start
    i8(bool,i8) choose = fn(flag, entry) {{
        if (flag) {{ entry }} else {{ 5 }}
    }};
    i8(i8) forward = fn(entry) {{
        bool selected = true;
        i8 local = choose(selected, entry);
        local
    }};
    i8 saved = forward(8);
    core.require(saved != 8, {report(237,"BOOL-ARG-TRUE")});
    %%end"""
cases.extend([
    ("exact-startup-nullary-int-return",real_nullary_initializer,234,b"NULLARY-CALLEENULLARY-CALLER"),
    ("exact-startup-nullary-int-narrow-does-not-return",real_nullary_initializer.replace("core.require(false","core.narrow(false"),234,b"NULLARY-CALLEE"),
    ("exact-startup-nullary-int-local",real_nullary_local,234,b"NULLARY-CALLEENULLARY-LOCAL-CALLER"),
    ("exact-startup-nullary-int-local-narrow-prunes",real_nullary_local.replace("core.require(false","core.narrow(false"),234,b"NULLARY-CALLEE"),
    ("exact-startup-bool-module-argument-int-return",real_bool_initializer,236,b"BOOL-ARG-FALSE"),
    ("exact-startup-bool-nested-argument-int-local",real_bool_nested_local,237,b"BOOL-ARG-TRUE"),
    ("exact-startup-bool-nested-argument-int-tail",real_bool_nested_local.replace(
        "i8 local = choose(selected, entry);\n        local","choose(selected, entry)"
    ),237,b"BOOL-ARG-TRUE"),
])
real_integer_tail = f"""%%start
    i8(i8) pick = fn(entry) {{
        core.require(entry != 3, {state_report(231,"TAIL-CHILD","entry")});
        entry
    }};
    i8(i8) forward = fn(entry) {{
        core.require(entry != 3, {report(230,"BEFORE-TAIL")});
        if (entry > 0) {{ pick(entry) }} else {{ 9 }}
    }};
    i8 saved = forward(3);
    core.require(saved != 3, {report(232,"AFTER-TAIL")});
    %%end"""
real_integer_tail_empty = real_integer_tail.replace(
    "core.require(entry != 3, " + state_report(231,"TAIL-CHILD","entry"),
    "core.narrow(entry < 3, " + state_report(233,"TAIL-EMPTY","entry"),
)
real_integer_tail_inactive = real_integer_tail.replace("forward(3)","forward(0)").replace("saved != 3","saved != 9").replace(
    "core.require(entry != 3, " + state_report(231,"TAIL-CHILD","entry") + ");", "",
)
cases.extend([
    ("exact-startup-real-integer-tail-transfers-result",real_integer_tail,230,b"BEFORE-TAILTAIL-CHILDAFTER-TAIL"),
    ("exact-startup-real-integer-tail-propagates-empty",real_integer_tail_empty,230,b"BEFORE-TAILTAIL-EMPTY"),
    ("exact-startup-real-integer-tail-ignores-unused-arm",real_integer_tail_inactive,232,b"AFTER-TAIL"),
    ("exact-startup-real-integer-tail-rejects-reentry",real_integer_tail.replace("pick(entry)","forward(entry)"),2,b"unsupported analysis"),
])
contextual_bool_scopes = f"""%%start
    unit() startup = fn {{
        bool initial = true;
        bool core.context.ready = initial;
        if (core.context.ready) {{
            bool core.context.ready = core.context.ready;
            core.context.ready = false;
            core.require(core.context.ready, {state_report(220,"INNER-FALSE","core.context.ready").replace("state.json","inner.json")});
        }};
        core.require(!core.context.ready, {state_report(221,"OUTER-TRUE","core.context.ready").replace("state.json","outer.json")});
    }};
    startup();
    %%end"""
symbolic_contextual_bool = f"""%%start
    unit(bool) inspect = fn(input) {{
        bool core.context.ready = input;
        bool prior = core.context.ready;
        if (prior) {{
            bool core.context.ready = core.context.ready;
            core.context.ready = false;
            core.require(core.context.ready, {state_report(220,"INNER-FALSE","core.context.ready").replace("state.json","inner.json")});
            core.require(prior, {report(223,"LOST-ALIAS")});
            core.context.ready = prior;
            core.require(core.context.ready, {report(224,"LOST-WRITE")});
            core.context.ready = false;
        }} else {{
            core.require(!core.context.ready, {report(222,"WRONG-PREIMAGE")});
        }};
        core.require(!core.context.ready, {state_report(221,"OUTER-TRUE","core.context.ready").replace("state.json","outer.json")});
    }};
    %%end"""
cases.extend([
    ("exact-startup-contextual-bool-restores-outer",contextual_bool_scopes,220,b"INNER-FALSEOUTER-TRUE"),
    ("symbolic-contextual-bool-restores-outer",symbolic_contextual_bool,220,b"INNER-FALSEOUTER-TRUE"),
    ("exact-startup-contextual-bool-restores-symbolic-shape",symbolic_contextual_bool.replace(
        "%%end","inspect(false); inspect(true); %%end"),220,b"INNER-FALSEOUTER-TRUE"),
])
contextual_counter = f"""%%start
    unit(u8) debit = fn(initial) {{
        core.narrow(initial == 1 || initial == 3, {report(1,"PRUNE")});
        u8 core.context.remaining = initial;
        u8 previous = core.context.remaining;
        core.context.remaining = core.context.remaining - 1;
        u8 current = core.context.remaining;
        if (current == 2) {{
            core.require(previous == 3 && initial == 3, {report(61,"LOST-PREIMAGE")});
            core.require(false, {{ .stdout = ""; .stderr = "COUNTER-DEBIT"; .code = 63; .files = [
                {{ .path = "old.json"; .state = previous; }},
                {{ .path = "new.json"; .state = core.context.remaining; }}
            ]; }});
        }} else {{
            core.require(current == 0 && previous == 1 && initial == 1, {report(59,"WRONG-DEBIT")});
        }};
    }};
    %%end"""
returning_contextual_counter = f"""%%start
    u8(u8) debit = fn(initial) {{
        u8 core.context.remaining = initial;
        core.context.remaining = core.context.remaining - 1;
        u8 current = core.context.remaining;
        core.require(false, {report(65,"RETURNING-COUNTER-WRITE")});
        current
    }};
    u8 saved = debit(3);
    core.require(saved != 2, {report(67,"RETURNING-COUNTER-RESULT")});
    %%end"""
cases.extend([
    ("ordinary-contextual-counter-debit",contextual_counter,63,b"COUNTER-DEBIT"),
    ("exact-startup-contextual-counter-debit",contextual_counter.replace("%%end","debit(1); debit(3); %%end"),63,b"COUNTER-DEBIT"),
    ("exact-startup-contextual-counter-returning-refused",returning_contextual_counter,2,b"unsupported analysis"),
    ("ordinary-contextual-counter-underflow",contextual_counter.replace("initial == 1","initial == 0"),2,b"unsupported analysis"),
    ("ordinary-contextual-counter-domain-cap",contextual_counter.replace("initial == 1 || initial == 3","initial <= 64").replace(
        "remaining - 1","remaining + 1"),2,b"unsupported analysis"),
])
namespace_scalar_key = "exact-startup-completed-namespace-scalar-return"
namespace_scalar_main = f"""%%start
    core.require(false, {report(210,"ROOT")});
    lib = namespace app "src/calc.w";
    alias = namespace app "src/calc.w";
    i8 saved = lib.choose(7);
    core.require(saved != 42, {report(213,"RETURN")});
    unit() startup = fn {{
        i8 unavailable = alias.retain(1);
        core.require(unavailable > 127, {report(215,"DEAD-CALLER")});
    }};
    startup();
    core.require(false, {report(216,"DEAD-STARTUP")});
    %%end"""
namespace_scalar_child = f"""%%start
    core.require(false, {report(211,"INIT")});
    i8(i8) choose = fn(entry) {{
        core.require(entry != 7, {state_report(212,"CALLEE","entry").replace("state.json","call.json")});
        if (entry == 7) {{ 42 }} else {{ 9 }}
    }};
    i8(i8) retain = fn(entry) {{
        core.narrow(entry >= 2, {state_report(214,"EMPTY","entry")});
        entry
    }};
    %%end"""
cases.append((namespace_scalar_key,namespace_scalar_main,210,b"ROOTINITCALLEERETURNEMPTY"))
namespace_bool_key = "exact-startup-completed-namespace-bool-return"
namespace_bool_main = f"""%%start
    lib = namespace app "src/calc.w";
    alias = namespace app "src/calc.w";
    if (lib.choose(true)) {{
        core.require(false, {report(53,"BOOL-BRANCH")});
    }};
    bool unavailable = alias.retain(false);
    core.require(unavailable, {report(59,"BOOL-UNREACHABLE")});
    %%end"""
namespace_bool_child = f"""%%start
    core.require(false, {report(47,"BOOL-INIT")});
    bool(bool) choose = fn(entry) {{
        core.require(false, {state_report(51,"BOOL-CHOOSE","entry").replace("state.json","chosen.json")});
        entry
    }};
    bool(bool) retain = fn(entry) {{
        core.narrow(entry, {state_report(57,"BOOL-PRUNED","entry").replace("state.json","rejected.json")});
        entry
    }};
    %%end"""
def namespace_bool_context_child(declaration, name):
    return namespace_bool_child.replace("choose = fn(entry) {",
        "choose = fn(entry) { " + declaration).replace(
        state_report(51,"BOOL-CHOOSE","entry").replace("state.json","chosen.json"),
        state_report(51,"BOOL-CHOOSE",name).replace("state.json","chosen.json"))
namespace_bool_context_children = {
    "exact-startup-completed-namespace-bool-rejects-context-i8":
        namespace_bool_context_child("i8 core.context.count = 7;", "core.context.count"),
    "exact-startup-completed-namespace-bool-rejects-context-bool":
        namespace_bool_context_child("bool core.context.ready = entry;", "core.context.ready"),
}
namespace_children = {
    namespace_scalar_key: namespace_scalar_child,
    namespace_bool_key: namespace_bool_child,
    **namespace_bool_context_children,
}
cases.append((namespace_bool_key,namespace_bool_main,47,b"BOOL-INITBOOL-CHOOSEBOOL-BRANCHBOOL-PRUNED"))
cases.extend((name,namespace_bool_main,2,b"unsupported analysis") for name in namespace_bool_context_children)
finite_scalar_stride = f"""%%start
    unit(i8) check = fn(entry) {{
        core.narrow(entry >= 0, {report(245,"FINITE-CUT")});
        core.narrow(entry <= 3, {report(245,"FINITE-CUT")});
        i8 product = entry * 3;
        if (product == 6) {{
            core.require(entry != 2, {state_report(246,"MULTIPLY-PREIMAGE","entry").replace("state.json","preimage.json")});
        }};
        core.require(product < 0, {state_report(247,"EXACT-STRIDE","product")});
    }};
    %%end"""
finite_scalar_signed = f"""%%start
    unit(i8) check = fn(entry) {{
        i8 quotient = entry / 3;
        i8 remainder = entry % 3;
        core.require(quotient == -2, {report(248,"WRONG-QUOTIENT")});
        core.require(remainder != -1, {state_report(249,"SIGNED-REMAINDER","remainder")});
    }};
    check(-7);
    %%end"""
finite_scalar_zero = f"""%%start
    unit(i8) check = fn(entry) {{
        core.require(false, {report(250,"FINITE-PRIOR-UNPUBLISHED")});
        i8 quotient = 42 / entry;
        core.require(quotient < 0, {report(251,"UNDEFINED-OUTCOME")});
    }};
    check(0);
    %%end"""
finite_scalar_width = f"""%%start
    unit(u8) check = fn(entry) {{
        core.narrow(entry <= 63, {report(245,"FINITE-CUT")});
        u8 output = entry / 1;
        core.require(output <= 255, {report(245,"FINITE-BOUND")});
    }};
    %%end"""
cases.extend([
    ("finite-ordinary-multiplication-has-exact-stride",finite_scalar_stride,246,b"MULTIPLY-PREIMAGEEXACT-STRIDE"),
    ("exact-startup-finite-ordinary-signed-quotient-remainder",finite_scalar_signed,249,b"SIGNED-REMAINDER"),
    ("exact-startup-finite-ordinary-zero-divisor-refused",finite_scalar_zero,2,b"unsupported analysis"),
    ("exact-startup-finite-ordinary-overflow-refused",finite_scalar_zero.replace("42 / entry","entry * 2").replace("check(0)","check(100)"),2,b"unsupported analysis"),
    ("finite-ordinary-accepts-sixty-four-atoms",finite_scalar_width,0,b""),
    ("finite-ordinary-refuses-sixty-five-atoms",finite_scalar_width.replace("entry <= 63","entry <= 64"),2,b"unsupported analysis"),
])
conditional_binding_symbolic = f"""%%start
    unit(i8) check = fn(input) {{
        i8 selected = if (input < 0) {{ 13 }} else {{ input }};
        if (selected == 0) {{
            core.require(input == 0, {report(238,"BIND-UNREASONED")});
            core.require(input != 0, {state_report(239,"BIND-IDENTITY","input")});
        }};
        if (selected == 13) {{
            core.require(input >= 0, {state_report(240,"BIND-CONSTANT","input").replace("state.json","constant.json")});
        }};
    }};
    %%end"""
conditional_binding_bool = f"""%%start
    unit(i8) startup = fn(input) {{
        bool selected = if (input > 0) {{ true }} else {{ false }};
        if (selected) {{ core.require(false, {report(241,"WRONG-BOOL-BIND")}); }};
        core.require(selected, {state_report(242,"FALSE-BIND","selected")});
    }};
    startup(0);
    %%end"""
conditional_binding_narrowed = f"""%%start
    unit(i8) check = fn(input) {{
        core.narrow(input >= 0, {report(243,"BIND-PRIOR-UNPUBLISHED")});
        i8 selected = if (input < 0) {{ input + 1 }} else {{ input }};
        core.require(selected < 0, {state_report(244,"BIND-ONLY-REACHED","selected")});
    }};
    %%end"""
cases.extend([
    ("conditional-scalar-binding-symbolic-preimages",conditional_binding_symbolic,239,b"BIND-IDENTITYBIND-CONSTANT"),
    ("exact-startup-conditional-bool-binding",conditional_binding_bool,242,b"FALSE-BIND"),
    ("conditional-scalar-binding-skips-unreached-operand",conditional_binding_narrowed,244,b"BIND-ONLY-REACHED"),
    ("conditional-scalar-binding-refuses-reached-operand",conditional_binding_narrowed.replace("core.narrow","core.require"),2,b"unsupported analysis"),
])
symbolic_integer_root_tail = f"""%%start
    i8(i8) pick = fn(entry) {{
        if (entry < 0) {{ 9 }} else {{ entry }}
    }};
    i8(i8) forward = fn(input) {{
        core.require(input >= 0, {state_report(33,"SYMBOLIC-TAIL-INPUT","input")});
        pick(input)
    }};
    %%end"""
cases.extend([
    ("symbolic-integer-root-tail-projects-conditional-callee",symbolic_integer_root_tail,33,b"SYMBOLIC-TAIL-INPUT"),
    ("symbolic-integer-root-tail-accepts-lexical-singleton",symbolic_integer_root_tail.replace(
        "core.require(input >= 0,", "core.narrow(input == 3,"),0,b""),
    ("symbolic-integer-root-tail-refuses-literal",symbolic_integer_root_tail.replace("pick(input)", "pick(3)"),
        2,b"unsupported analysis"),
    ("symbolic-integer-root-tail-refuses-leaf-obligation",symbolic_integer_root_tail.replace(
        "if (entry < 0)",f"core.require(entry >= 0, {report(34,'SYMBOLIC-TAIL-LEAF')}); if (entry < 0)"),
        2,b"unsupported analysis"),
])

integer_symbolic_root_call = f"""%%start
    i8(i8) transfer = fn(entry) {{
        if (entry < 0) {{ 13 }} else {{ entry }}
    }};
    i8(i8) check = fn(input) {{
        i8 output = transfer(input);
        if (output == 0) {{ core.require(input == 0, {report(31,"ROOT-PREIMAGE-WIDENED")}); }};
        if (output == 13) {{ core.require(input >= 0, {state_report(32,"ROOT-NEGATIVE-INPUT","input")}); }};
        output
    }};
    %%end"""
cases.append(("integer-symbolic-root-call-preimages",integer_symbolic_root_call,32,b"ROOT-NEGATIVE-INPUT"))

projected_conditional_write = f"""%%start
    i8(i8) transfer = fn(entry) {{
        i8 changed = entry;
        i8 previous = changed;
        changed = if (changed < 0) {{ 13 }} else {{ previous }};
        changed
    }};
    unit(i8) check = fn(input) {{
        i8 output = transfer(input);
        if (input < 0) {{
            core.require(output == 13, {report(204,"NEG-IRRELEVANT")});
        }};
        if (output == 0) {{
            core.require(input == 0, {report(205,"IDENTITY-WIDENED")});
            core.require(input != 0, {state_report(206,"IDENTITY-INPUT","input")});
        }};
        if (output == 13) {{
            core.require(input >= 0, {state_report(207,"CONSTANT-PREIMAGE","input").replace("state.json","preimage.json")});
        }};
    }};
    %%end"""
projected_conditional_unreached = f"""%%start
    i8(i8) transfer = fn(entry) {{
        i8 changed = 2;
        changed = if (entry < 0) {{ entry + 1 }} else {{ entry }};
        changed
    }};
    unit(i8) check = fn(input) {{
        core.narrow(input >= 0, {report(208,"PROJECTED-PRIOR-UNPUBLISHED")});
        i8 output = transfer(input);
        core.require(output < 0, {state_report(209,"ONLY-REACHED-ARM","output")});
    }};
    %%end"""
cases.extend([
    ("projected-conditional-local-write-keeps-input-pieces",projected_conditional_write,206,b"IDENTITY-INPUTCONSTANT-PREIMAGE"),
    ("projected-conditional-local-write-skips-unreached-operand",projected_conditional_unreached,209,b"ONLY-REACHED-ARM"),
    ("projected-conditional-local-write-rejects-reached-operand",projected_conditional_unreached.replace("core.narrow","core.require"),2,b"unsupported analysis"),
])
projected_local_copy = f"""%%start
    i8(i8) transfer = fn(entry) {{
        i8 changed = 2;
        i8 previous = changed;
        changed = entry;
        if (previous == 2) {{ changed }} else {{ 0 }}
    }};
    unit(i8) check = fn(input) {{
        i8 output = transfer(input);
        if (output == 3) {{
            core.require(input == 3, {report(200,"WRONG-INPUT")});
            core.require(input != 3, {state_report(201,"COPY-IDENTITY","input",', { .path = "result.json"; .state = output; }')});
        }};
    }};
    %%end"""
projected_local_copy_formal = f"""%%start
    i8(i8) transfer = fn(entry) {{
        i8 ordinary = 2;
        entry = ordinary;
        entry
    }};
    unit(i8) check = fn(input) {{
        core.require(input > 127, {report(202,"COPY-PRIOR-UNPUBLISHED")});
        i8 output = transfer(input);
        core.require(output != 2, {report(203,"COPY-FORMAL-UNPUBLISHED")});
    }};
    %%end"""
cases.extend([
    ("projected-returning-local-copy-identity",projected_local_copy,201,b"COPY-IDENTITY"),
    ("projected-returning-local-copy-refuses-formal",projected_local_copy_formal,2,b"unsupported analysis"),
])
projected_scalar_narrow = f"""%%start
    i8(i8) retain = fn(entry) {{
        core.narrow(entry >= 2, {report(190,"NARROW-UNEXPECTED")});
        entry
    }};
    unit(i8) check = fn(input) {{
        i8 output = retain(input);
        core.require(input >= 2, {report(191,"CALLER-UNPRUNED")});
        core.require(output < 2, {state_report(192,"SURVIVORS","output")});
    }};
    %%end"""
projected_scalar_empty = f"""%%start
    i8(i8) retain = fn(entry) {{
        core.narrow(entry <= 0, {state_report(193,"REACHED-NARROW","entry")});
        entry
    }};
    unit(i8) check = fn(input) {{
        if (input > 0) {{
            i8 output = retain(input);
            core.require(output > 127, {report(194,"DEAD-RETURN")});
        }} else {{
            core.require(input > 0, {state_report(195,"SIBLING","input").replace("state.json","branch.json")});
        }};
    }};
    %%end"""
cases.extend([
    ("projected-narrowed-integer-return-prunes-parent",projected_scalar_narrow,192,b"SURVIVORS"),
    ("projected-empty-integer-return-keeps-sibling",projected_scalar_empty,193,b"REACHED-NARROWSIBLING"),
])
conditional_local_write = f"""%%start
    i8(i8) revise = fn(entry) {{
        i8 current = 2;
        i8 previous = current;
        core.require(previous != 2, {report(196,"PRIOR")});
        current = if (entry > 0) {{ entry }} else {{
            core.require(false, {report(197,"ARM-EFFECT")});
            4
        }};
        core.require(previous != 2, {state_report(198,"UNCHANGED-OLD","previous")});
        current
    }};
    i8 saved = revise(7);
    core.require(saved != 7, {report(199,"RETURNED-SELECTED")});
    %%end"""
conditional_local_write_bool = f"""%%start
    i8(i8) revise = fn(entry) {{
        bool choosing = false;
        bool flag = true;
        flag = if (choosing) {{ true }} else {{ false }};
        if (flag) {{ 9 }} else {{ entry }}
    }};
    i8 saved = revise(7);
    core.require(saved != 7, {report(199,"RETURNED-ELSE")});
    %%end"""
cases.extend([
    ("exact-startup-conditional-local-write-ignores-unreached-arm",conditional_local_write,196,b"PRIORUNCHANGED-OLDRETURNED-SELECTED"),
    ("exact-startup-conditional-local-write-rejects-reached-effect",conditional_local_write.replace("revise(7)","revise(0)"),2,b"unsupported analysis"),
    ("exact-startup-conditional-local-write-selects-bool-else",conditional_local_write_bool,199,b"RETURNED-ELSE"),
])
scalar_local_write = f"""%%start
    i8(i8) revise = fn(entry) {{
        i8 current = 2;
        i8 old = current;
        current = entry;
        core.require(old != 2, {state_report(188,"REVISED","old",', { .path = "current.json"; .state = current; }')});
        current
    }};
    i8(i8) forward = fn(entry) {{
        i8 result = revise(entry);
        result
    }};
    i8 saved = forward(7);
    core.require(saved != 7, {report(189,"AFTER-WRITE")});
    %%end"""
scalar_local_write_direct = scalar_local_write[:scalar_local_write.index("    i8(i8) forward")] + scalar_local_write[
    scalar_local_write.index("    i8 saved"):].replace("forward(7)","revise(7)")
scalar_local_write_formal = scalar_local_write.replace(
    "current = entry;", "core.require(old > 127, " + report(188,"NEVER-SCALAR-WRITE") + "); entry = 7;",
).replace("i8 current = 2;", "i8 current = entry;")
scalar_local_write_context = scalar_local_write.replace(
    "current = entry;", "i8 core.context.count = entry; core.require(old > 127, "
    + report(188,"NEVER-SCALAR-WRITE") + "); core.context.count = 7;",
).replace("        current\n", "        core.context.count\n")
scalar_local_write_bool = f"""%%start
    i8(i8) revise = fn(entry) {{
        bool enabled = true;
        enabled = false;
        if (enabled) {{ 9 }} else {{ entry }}
    }};
    i8 saved = revise(7);
    core.require(saved != 7, {report(189,"AFTER-BOOL-WRITE")});
    %%end"""
cases.extend([
    ("exact-startup-integer-return-direct-local-write",scalar_local_write_direct,188,b"REVISEDAFTER-WRITE"),
    ("exact-startup-integer-return-nested-local-write",scalar_local_write,188,b"REVISEDAFTER-WRITE"),
    ("exact-startup-integer-return-formal-write-refused",scalar_local_write_formal,2,b"unsupported analysis"),
    ("exact-startup-integer-return-bool-local-write",scalar_local_write_bool,189,b"AFTER-BOOL-WRITE"),
    ("exact-startup-integer-return-context-write-refused",scalar_local_write_context,2,b"unsupported analysis"),
])
scalar_narrow = f"""%%start
    i8(i8) restrict = fn(entry) {{
        core.narrow(entry >= 2, {state_report(185,"NARROW-PRESTATE","entry")});
        entry
    }};
    i8(i8) forward = fn(entry) {{
        i8 result = restrict(entry);
        core.require(result > 127, {state_report(186,"AFTER-RETURN","result")});
        result
    }};
    i8 saved = forward(1);
    core.require(saved > 127, {report(187,"AFTER-INIT")});
    %%end"""
scalar_narrow_direct = scalar_narrow[0:scalar_narrow.index("    i8(i8) forward")] + scalar_narrow[
    scalar_narrow.index("    i8 saved"):].replace("forward(1)","restrict(1)")
cases.extend([
    ("exact-startup-scalar-narrow-leaf-empties-init",scalar_narrow_direct,185,b"NARROW-PRESTATE"),
    ("exact-startup-scalar-narrow-prunes-return-chain",scalar_narrow,185,b"NARROW-PRESTATE"),
    ("exact-startup-scalar-narrow-surviving-return",scalar_narrow.replace("forward(1)","forward(3)"),186,b"AFTER-RETURNAFTER-INIT"),
])
public_contextual_proof = """%%start
    unit() consume = fn { core.context.credit = core.context.credit - 1; };
    unit() relay = fn { consume(); };
    unit() entry = fn {
        u8 core.context.credit = 3;
        relay();
        core.require(core.context.credit == 2, """ + report(76,"WRONG-CONTEXT-EXIT") + """);
    };
    proof debit for consume() -> (null)
        ensures (core.context.credit == core.old(core.context.credit) - 1) {}
    entry();
%%end"""
public_contextual_refuted = public_contextual_proof.replace(
    "credit = core.context.credit - 1", "credit = core.context.credit - 0").replace(
    "relay();\n", "relay();\n        core.require(false, " + report(77,"PRIVATE-CONTEXT-OUTPUT") + ");\n")
cases.extend([
    ("exact-startup-public-contextual-proof", public_contextual_proof, 0, b""),
    ("exact-startup-public-contextual-proof-refuted", public_contextual_refuted, 1, b"proof obligation failed"),
    ("exact-startup-public-contextual-proof-overflow", public_contextual_proof.replace("credit = 3;", "credit = 0;"),
        2, b"unsupported analysis"),
])

unused_contextual_proof = """%%start
    unit() type_provider = fn {
        u8 core.context.credit = 3;
        if (core.context.credit == 3) {};
    };
    unit() unchanged = fn {
        if (core.context.credit == 3) { core.context.credit = 3; };
    };
    proof preserved for unchanged() -> (null)
        ensures (core.context.credit == core.old(core.context.credit) + 0) {}
%%end"""
unused_contextual_refuted = unused_contextual_proof.replace(
    "credit == 3) { core.context.credit = 3", "credit == 11) { core.context.credit = 2")
cases.extend([
    ("ordinary-unused-contextual-verified", unused_contextual_proof, 0, b""),
    ("ordinary-unused-contextual-refuted", unused_contextual_refuted, 1, b"proof obligation failed"),
    ("ordinary-unused-contextual-proof-overflow", unused_contextual_refuted.replace("+ 0", "+ 1"), 2, b"unsupported analysis"),
    ("ordinary-unused-contextual-proof-underflow", unused_contextual_refuted.replace("+ 0", "- 1"), 2, b"unsupported analysis"),
])
public_namespace_contextual_root = """%%start
    lib = namespace app "src/calc.w";
    unit() entry = fn {
        u8 core.context.credit = 3;
        lib.relay();
        core.require(core.context.credit == 2, """ + report(76,"WRONG-NAMESPACE-CONTEXT-EXIT") + """);
    };
    entry();
%%end"""
public_namespace_contextual_child = """%%start
    unit() consume = fn { core.context.credit = core.context.credit - 1; };
    unit() relay = fn { consume(); };
    proof debit for consume() -> (null)
        ensures (core.context.credit == core.old(core.context.credit) - 1) {}
%%end"""
for suffix, child, code, stderr in [
    ("verified", public_namespace_contextual_child, 0, b""),
    ("refuted", public_namespace_contextual_child.replace("credit = core.context.credit - 1", "credit = core.context.credit - 0"),
        1, b"proof obligation failed"),
    ("initializer-refused", public_namespace_contextual_child.replace("%%end", "relay(); %%end"),
        2, b"unsupported analysis"),
]:
    name = "exact-startup-public-namespace-contextual-" + suffix
    namespace_children[name] = child
    cases.append((name, public_namespace_contextual_root, code, stderr))

two_edge_ranked = """%%start
    u16(i8, u8) descend = fn(page, slot) {
        if (slot == 0) {
            if (page == 0) { 7 } else { descend(page - 1, 3) }
        } else { descend(page, slot - 1) }
    };
    proof rank for descend(p, s) -> (out)
        where (p >= 0 && p <= 2 && s <= 3) ensures (out == 7) decreases (p, s) {
            if (s == 0) {
                if (p == 0) { core.show(out == 7); } else { core.use(rank(p - 1, 3)); }
            } else { core.use(rank(p, s - 1)); }
        }
%%end"""
two_edge_ranked_reached = two_edge_ranked.replace("%%end",
    "u16 result = descend(2, 2); core.require(result == 7, " + report(233,"TWO-EDGE-RANK") + "); %%end")
cases.extend([
    ("exact-startup-two-edge-ranked-verified", two_edge_ranked_reached, 0, b""),
    ("exact-startup-two-edge-ranked-missing-transition",
        two_edge_ranked_reached.replace("descend(page, slot - 1)", "7"), 2, b"unsupported analysis"),
    ("exact-startup-two-edge-ranked-reset-outside-guard",
        two_edge_ranked_reached.replace("s <= 3", "s <= 2"), 2, b"unsupported analysis"),
    ("ordinary-ranked-two-edge-symbolic", two_edge_ranked, 0, b""),
    ("ordinary-ranked-two-edge-base-refuted", two_edge_ranked.replace("{ 7 } else", "{ 8 } else"),
        1, b"proof obligation failed"),
])

# Three different recursive leaves; the positive suffix offset is legal only
# under the checked prefix decrease and its local upper cut.
ranked_tree = """%%start
    u16(i8,u8) descend = fn(page,slot) {
        if (page > 0) {
            if (slot < 3) { descend(page - 1,slot + 1) } else { descend(page - 1,0) }
        } else {
            if (slot > 1) { descend(0,slot - 2) } else {
                if (slot == 1) { 7 } else { 7 }
            }
        }
    };
    proof rank for descend(p,s) -> (out)
        where (p >= 0 && p <= 4 && s <= 3) ensures (out == 7) decreases (p,s) {
            if (p > 0) {
                if (s < 3) { core.use(rank(p - 1,s + 1)); } else { core.use(rank(p - 1,0)); }
            } else {
                if (s > 1) { core.use(rank(0,s - 2)); } else { core.show(out == 7); }
            }
        }
%%end"""
ranked_tree_reached = ranked_tree.replace("%%end", "u16 result = descend(4,2); core.require(result == 7, "
    + report(233,"TREE-RETURN") + "); %%end")
cases.extend([
    ("exact-startup-ranked-tree-verified", ranked_tree_reached, 0, b""),
    ("exact-startup-ranked-tree-actual-result", ranked_tree_reached.replace("result == 7", "result == 8"), 233, b"TREE-RETURN"),
    ("exact-startup-ranked-tree-citation-mismatch", ranked_tree_reached.replace("rank(p - 1,s + 1)", "rank(p - 1,s)"),
        2, b"unsupported analysis"),
    ("exact-startup-ranked-tree-hidden-wrong-base", ranked_tree_reached.replace("if (slot == 1) { 7 }", "if (slot == 1) { 9 }"),
        2, b"unsupported analysis"),
    ("ordinary-ranked-tree-symbolic", ranked_tree, 0, b""),
    ("ordinary-ranked-tree-base-refuted", ranked_tree.replace("if (slot == 1) { 7 }", "if (slot == 1) { 9 }"),
        1, b"proof obligation failed"),
    ("ordinary-ranked-tree-suffix-not-closed", ranked_tree.replace(" < 3)", " <= 3)"), 2, b"unsupported analysis"),
    ("ordinary-ranked-tree-increasing-prefix", ranked_tree.replace("descend(0,slot", "descend(1,slot")
        .replace("rank(0,s", "rank(1,s"), 2, b"unsupported analysis"),
    ("ordinary-ranked-tree-equal-self-use", ranked_tree.replace(" - 2)", " - 0)"), 2, b"unsupported analysis"),
])

ranked_return = f"""%%start
    u8(u8) descend = fn(value) {{
        if (value > 0) {{
            u8 result = descend(value - 1);
            core.require(result == 7, {report(180,"BAD-INNER-RETURN")});
            core.require(value >= 3, {report(181,"FRAME")});
            result
        }} else {{
            core.require(value != 0, {report(182,"BASE")});
            7
        }}
    }};
    unit() startup = fn {{
        u8 final = descend(2);
        core.require(final == 7, {report(183,"BAD-FINAL-RETURN")});
        core.require(final != 7, {state_report(184,"FINAL","final")});
    }};
    startup();
    %%end"""
ranked_return_same = ranked_return.replace("descend(value - 1)", "descend(value)")
ranked_return_double = ranked_return.replace(
    'core.require(result == 7,',
    'u8 repeated = descend(0); core.require(repeated == 7, '
    + report(180,"BAD-INNER-RETURN") + '); core.require(result == 7,',
)
cases.extend([
    ("exact-startup-ranked-integer-return",ranked_return,182,b"BASEFRAMEFRAMEFINAL"),
    ("exact-startup-ranked-integer-return-same-entry",ranked_return_same,2,b"unsupported analysis"),
    ("exact-startup-ranked-integer-return-second-edge",ranked_return_double,2,b"unsupported analysis"),
])
parallel_mixed = f"""%%start
    unit() check = fn {{
        i8 first = 2;
        i8 second = 8;
        bool enabled = true;
        bool disabled = false;
        first, second, enabled, disabled = second, first, disabled, enabled;
        core.require(first == 8, {report(174,"FIRST")});
        core.require(second == 2, {report(175,"SECOND")});
        core.require(disabled, {report(176,"DISABLED")});
        core.require(enabled, {state_report(177,"PARALLEL-MIXED","enabled",', { .path = "first.json"; .state = first; }')});
    }};
    %%end"""
parallel_literals = f"""%%start
    unit() check = fn {{
        i8 count = 2;
        bool enabled = true;
        count, enabled = 7, false;
        core.require(count == 7, {report(174,"COUNT")});
        core.require(enabled, {state_report(178,"PARALLEL-LITERALS","enabled")});
    }};
    %%end"""
parallel_formal_unsupported = f"""%%start
    unit(i8) check = fn(formal) {{
        i8 local = 2;
        core.require(local != 2, {state_report(179,"FORMAL-NEVER-PUBLISHED","local")});
        formal, local = local, formal;
        core.require(formal != 2, {state_report(179,"FORMAL-NEVER-PUBLISHED","local")});
    }};
    %%end"""
cases.extend([
    ("parallel-whole-local-mixed-swap",parallel_mixed,177,b"PARALLEL-MIXED"),
    ("parallel-whole-local-mixed-literals",parallel_literals,178,b"PARALLEL-LITERALS"),
    ("parallel-formal-target-fails-closed",parallel_formal_unsupported,2,b"unsupported analysis"),
])
state_range = f"""%%start
    unit(i8) check = fn(value) {{
        core.require(value <= 3, {state_report(162,"AUTHORED-RANGE","value",', { .path = "note.txt"; .text = "authored"; }')});
    }};
    %%end"""
state_bool = f"""%%start
    unit(bool) check = fn(flag) {{ core.require(flag, {state_report(163,"AUTHORED-BOOL","flag")}); }};
    check(false);
    %%end"""
# A Bool return selects the ordinary Boolean root evaluator. A unit root
# without ordinary operations takes the intentionally integer-only root path.
state_full_bool = f"""%%start
    bool(bool) check = fn(flag) {{
        core.require(false, {state_report(163,"AUTHORED-BOOL","flag")});
        flag
    }};
    %%end"""
state_narrow = f"""%%start
    unit(i8) check = fn(value) {{ core.narrow(value >= 2, {state_report(164,"PRESTATE","value")}); }};
    check(1);
    %%end"""
state_i128 = f"""%%start
    unit(i128) check = fn(value) {{
        core.require(value != 170141183460469231731687303715884105727, {state_report(165,"I128","value")});
    }};
    %%end"""
state_contextual = f"""%%start
    unit(i8) check = fn(value) {{
        i8 core.context.count = value;
        core.require(core.context.count != 3, {state_report(166,"CONTEXT-STATE","core.context.count")});
    }};
    %%end"""
state_holes = f"""%%start
    unit(i8) check = fn(value) {{
        core.narrow(value == 1 || value == 3, {report(167,"HOLES")});
        core.require(false, {state_report(168,"ALTERNATIVES","value")});
    }};
    %%end"""
ordinary_pure_affine_proof = """%%start
    i8(i8) advance = fn(value) {
        i8 shifted = value + 2;
        i8 copied = shifted;
        copied
    };
    proof bounded for advance(entry) -> (out) where (entry <= 100) ensures (out <= 102) {}
    %%end"""
ordinary_pure_affine_proof_reached = ordinary_pure_affine_proof.replace("%%end", f"""
    i8 first = advance(99);
    i8 second = advance(100);
    core.require(first == 101, {report(57,"BAD-FIRST")});
    core.require(second == 102, {report(59,"BAD-SECOND")});
    %%end""")
grouped_affine_proof = ordinary_pure_affine_proof.replace("%%end",
    "proof lower for advance(entry) -> (out) where (entry <= 100) ensures (out >= -126) {} %%end")
grouped_affine_proof_reached = ordinary_pure_affine_proof_reached.replace("%%end",
    "proof lower for advance(entry) -> (out) where (entry <= 100) ensures (out >= -126) {} %%end")
cases.extend([
    ("ordinary-pure-affine-proof-shared-guard",grouped_affine_proof,0,b""),
    ("exact-startup-ordinary-pure-affine-proof-shared-guard",grouped_affine_proof_reached,0,b""),
])
relational_named_calling_producer = """%%start
    i8(i8) leaf = fn(n) { n };
    i8(i8) producer = fn(n) { i8 observed = leaf(n); observed };
    proof bounded for producer(entry) -> (out) where (entry >= 0) ensures (out == 7) {}
%%end"""
reflected_named_calling_producer = relational_named_calling_producer.replace("{ n }", "{ -1 - n }").replace("entry >= 0", "entry < 0")
cases.extend([
    ("ordinary-named-relational-alias-refuted",relational_named_calling_producer,1,b"proof obligation failed"),
    ("ordinary-named-relational-affine-refuted",reflected_named_calling_producer,1,b"proof obligation failed"),
    ("ordinary-named-relational-affine-proved",reflected_named_calling_producer.replace("entry < 0","entry == -8"),0,b""),
    ("ordinary-named-relational-entry-equality-affine-refuted",
        reflected_named_calling_producer.replace("out == 7","out == core.old(entry)"),
        1,b"proof obligation failed"),
    ("ordinary-named-relational-entry-equality-affine-outside-guard",
        reflected_named_calling_producer.replace("entry < 0","entry < -128").replace("out == 7","out == core.old(entry)"),
        0,b""),
    ("ordinary-named-relational-entry-equality-affine-overflow-outside-guard",
        reflected_named_calling_producer.replace("entry < 0","entry < -128").replace("out == 7","out == core.old(entry)")
            .replace("-1 - n","1 - n"),2,b"unsupported analysis"),
    ("ordinary-named-relational-affine-overflow",reflected_named_calling_producer.replace("-1 - n","1 - n"),2,b"unsupported analysis"),
])

authored_named_calling_producer = """%%start
    i8(i8) leaf = fn(n) { if (n < 0) { -3 } else { 7 } };
    i8(i8) producer = fn(n) { i8 observed = leaf(n); observed };
    proof bounded for producer(entry) -> (out) ensures (out == 7) { core.show(out < 7); }
%%end"""
cases.extend([
    ("ordinary-authored-named-calling-step-refuted",authored_named_calling_producer,1,b"proof obligation failed"),
    ("ordinary-authored-named-calling-goal-refuted",authored_named_calling_producer.replace("out < 7","out <= 7"),
        1,b"proof obligation failed"),
    ("ordinary-authored-named-calling-proved",authored_named_calling_producer.replace("out == 7","out <= 7")
        .replace("out < 7","out <= 7"),0,b""),
])

reached_named_calling_producer = f"""%%start
    i8(i8) leaf = fn(input) {{ if (input < 0) {{ -3 }} else {{ 7 }} }};
    i8(i8) producer = fn(input) {{ i8 result = leaf(input); result }};
    proof bounded for producer(entry) -> (out) where (entry >= 0) ensures (out <= 6) {{}}
    i8 inactive = producer(-4);
    i8 saved = producer(3);
    core.require(inactive == -3, {report(38,"WRONG-INACTIVE-OUTPUT")});
    core.require(saved == 7, {report(39,"WRONG-ACTIVE-OUTPUT")});
%%end"""
cases.extend([
    ("exact-startup-named-calling-producer-refuted",reached_named_calling_producer,1,b"proof obligation failed"),
    ("exact-startup-named-calling-producer-proved",reached_named_calling_producer.replace("out <= 6","out <= 7"),0,b""),
    ("exact-startup-named-calling-producer-observes-nested-watch",reached_named_calling_producer.replace("out <= 6","out <= 7",1).replace(
        "i8 inactive =", "proof leaf_checked for leaf(entry) -> (out) where (entry >= 0) ensures (out <= 6) {}\n    i8 inactive ="),
        1,b"proof obligation failed"),
])

symbolic_named_exact_pair = """%%start
    i16(i8, i16) leaf = fn(tag, value) { if (tag < 0) { 300 } else { value } };
    i16(i8, i16) producer = fn(tag, value) { i16 result = leaf(tag, value); result };
    proof bounded for producer(selector, before) -> (out)
        where (selector >= 0 && before >= 10) ensures (out <= 11) {}
%%end"""
cases.extend([
    ("ordinary-named-symbolic-pair-alias-refuted",symbolic_named_exact_pair,1,b"proof obligation failed"),
    ("ordinary-named-symbolic-pair-alias-proved",symbolic_named_exact_pair.replace("before >= 10","before == 10"),
        0,b""),
    ("ordinary-named-symbolic-pair-constant-refuted",symbolic_named_exact_pair.replace("selector >= 0","selector < 0"),
        1,b"proof obligation failed"),
])

rectangular_authored_show = """%%start
    u8(u8, u8) leaf = fn(tag, value) { if (tag < 1) {}; 255 - value };
    u8(u8, u8) producer = fn(tag, value) { u8 result = leaf(tag, value); result };
    proof checked for producer(selector, before) -> (out)
        where (selector >= 1 && before >= 10) ensures (out == 255 - core.old(before)) {
            core.show(out <= 245);
            core.show(out == 255 - core.old(before));
            core.show(out <= 250);
        }
%%end"""
rectangular_authored_translation = """%%start
    i16(i16, i16) leaf = fn(tag, value) { if (tag < 0) {}; value + 1 };
    i16(i16, i16) producer = fn(tag, value) {
        if (value == 32767) { value } else { i16 result = leaf(tag, value); result }
    };
    proof checked for producer(selector, before) -> (out)
        where (selector >= 0 && before < 32767) ensures (out == core.old(before) + 1) {
            core.show(out > -32768);
            core.show(out == core.old(before) + 1);
        }
%%end"""
cases.extend([
    ("ordinary-rectangular-authored-show-signed-translation", rectangular_authored_translation, 0, b""),
    ("ordinary-rectangular-authored-show-unsigned-reflection", rectangular_authored_show, 0, b""),
    ("ordinary-rectangular-authored-show-goal-failure", rectangular_authored_show.replace(
        "ensures (out == 255 - core.old(before))","ensures (out == 246)"), 1, b"proof obligation failed"),
    ("ordinary-rectangular-authored-show-first-step-failure", rectangular_authored_show.replace(
        "core.show(out == 255 - core.old(before))", "core.show(out == core.old(before) - 1)").replace(
        "core.show(out <= 250)", "core.show(out < 1)"), 1, b"proof obligation failed"),
])

verified_rectangular_affine = f"""%%start
    i16(i16, i16) leaf = fn(tag, value) {{ if (tag < 0) {{}}; -1 - value }};
    i16(i16, i16) producer = fn(tag, value) {{ i16 result = leaf(tag, value); result }};
    proof canonical for producer(selector, before) -> (out)
        where (selector >= 0 && before >= 10) ensures (out == -1 - core.old(before)) {{}}
    unit(i16, i16) consumer = fn(tag, value) {{
        core.narrow(tag >= 0, {report(46,"AFFINE-FIRST-NARROW")});
        core.narrow(value >= 10, {report(47,"AFFINE-SECOND-NARROW")});
        i16 observed = producer(tag, value);
        if (value == 12) {{ core.require(observed == -13, {report(48,"VERIFIED-AFFINE-OUTPUT")}); }};
    }};
%%end"""
cases.extend([
    ("ordinary-verified-rectangular-affine-consumed",verified_rectangular_affine,0,b""),
    ("ordinary-verified-rectangular-affine-exact-refutation",verified_rectangular_affine.replace("observed == -13","observed != -13"),
        48,b"VERIFIED-AFFINE-OUTPUT"),
    ("ordinary-verified-rectangular-affine-outside-guard",verified_rectangular_affine.replace("core.narrow(value >= 10","core.narrow(value >= 0"),
        2,b"unsupported analysis"),
    ("ordinary-verified-rectangular-affine-false-equation",verified_rectangular_affine.replace("-1 - core.old(before)","0 - core.old(before)"),
        2,b"unsupported analysis"),
])

verified_rectangular_alias = f"""%%start
    i16(i16, i16) leaf = fn(tag, value) {{ if (tag >= 0) {{ value }} else {{ 4 }} }};
    i16(i16, i16) producer = fn(tag, value) {{ i16 result = leaf(tag, value); result }};
    proof canonical for producer(selector, before) -> (out)
        where (selector >= 0 && before >= 10) ensures (out == core.old(before)) {{}}
    unit(i16, i16) consumer = fn(tag, value) {{
        core.narrow(tag >= 0, {report(46,"ALIAS-FIRST-NARROW")});
        core.narrow(value >= 10, {report(47,"ALIAS-SECOND-NARROW")});
        i16 observed = producer(tag, value);
        if (value == 12) {{ core.require(observed == 12, {report(48,"VERIFIED-ALIAS-OUTPUT")}); }};
    }};
%%end"""
cases.extend([
    ("ordinary-verified-rectangular-alias-consumed",verified_rectangular_alias,0,b""),
    ("ordinary-verified-rectangular-alias-exact-refutation",verified_rectangular_alias.replace("observed == 12", "observed != 12"),
        48,b"VERIFIED-ALIAS-OUTPUT"),
    ("ordinary-verified-rectangular-alias-first-guard",verified_rectangular_alias.replace("core.narrow(tag >= 0", "core.narrow(tag < 0"),
        2,b"unsupported analysis"),
    ("ordinary-verified-rectangular-alias-second-guard",verified_rectangular_alias.replace("core.narrow(value >= 10", "core.narrow(value >= 0"),
        2,b"unsupported analysis"),
    ("ordinary-verified-rectangular-alias-unproved-formal",verified_rectangular_alias.replace("core.old(before)", "core.old(selector)"),
        2,b"unsupported analysis"),
])

verified_rectangular_summary = f"""%%start
    i16(i16, i16) leaf = fn(tag, value) {{ if (tag >= 0) {{ value }} else {{ 4 }} }};
    i16(i16, i16) producer = fn(tag, value) {{ i16 result = leaf(tag, value); result }};
    proof canonical for producer(selector, before) -> (out)
        where (selector >= 0 && before == 7) ensures (out == 7) {{}}
    unit(i16, i16) consumer = fn(tag, value) {{
        core.narrow(tag >= 0, {report(46,"PAIR-FIRST-NARROW")});
        core.narrow(value == 7, {report(47,"PAIR-SECOND-NARROW")});
        i16 observed = producer(tag, value);
        core.require(observed != 7, {report(48,"VERIFIED-PAIR-OUTPUT")});
    }};
%%end"""
cases.extend([
    ("ordinary-verified-rectangular-summary-consumed",verified_rectangular_summary,48,b"VERIFIED-PAIR-OUTPUT"),
    ("ordinary-verified-rectangular-summary-first-guard",verified_rectangular_summary.replace("core.narrow(tag >= 0", "core.narrow(tag < 0"),
        2,b"unsupported analysis"),
    ("ordinary-verified-rectangular-summary-second-guard",verified_rectangular_summary.replace("core.narrow(value == 7", "core.narrow(value == 8"),
        2,b"unsupported analysis"),
    ("ordinary-verified-rectangular-summary-refuted-proof",verified_rectangular_summary.replace("ensures (out == 7)", "ensures (out == 8)"),
        2,b"unsupported analysis"),
])

unary_entry_relation = """%%start
    u8(u8) increment = fn(value) {
        u8 advanced = value + 1;
        advanced
    };
    proof advance for increment(entry) -> (out)
        where (entry >= 1 && entry <= 8) ensures (out == core.old(entry) + 1) {}
%%end"""
unary_summary_call = unary_entry_relation.replace("%%end", f"""
    unit(u8) consumer = fn(value) {{
        core.narrow(value >= 1, {report(0,"")});
        core.narrow(value <= 8, {report(0,"")});
        u8 observed = increment(value);
        core.require(observed != 9, {report(48,"UNARY-VERIFIED-ENTRY")});
    }};
%%end""")
unary_bad_premise = unary_entry_relation.replace("core.old(entry) + 1", "core.old(entry) + 2")
cases.extend([
    ("ordinary-verified-unary-entry-affine", unary_entry_relation, 0, b""),
    ("ordinary-verified-unary-entry-refuted", unary_bad_premise, 1, b"proof obligation failed"),
    ("ordinary-verified-unary-entry-caller-refuted", unary_summary_call, 48, b"UNARY-VERIFIED-ENTRY"),
    ("ordinary-verified-unary-entry-caller-passed", unary_summary_call.replace("observed != 9", "observed <= 9"), 0, b""),
    ("ordinary-verified-unary-entry-caller-outside", unary_summary_call.replace("core.narrow(value <= 8", "core.narrow(value <= 9"),
        2, b"unsupported analysis"),
    ("ordinary-verified-unary-entry-caller-no-unproved-summary", unary_summary_call.replace("core.old(entry) + 1", "core.old(entry) + 2"),
        2, b"unsupported analysis"),
    ("ordinary-verified-unary-entry-identity", unary_entry_relation.replace("value + 1", "value + 0")
        .replace("core.old(entry) + 1", "core.old(entry)"), 0, b""),
    ("exact-startup-verified-unary-entry-proved", unary_entry_relation.replace("%%end", "u8 saved = increment(4); %%end"), 0, b""),
    ("exact-startup-verified-unary-entry-refuted", unary_bad_premise.replace("%%end", "u8 saved = increment(4); %%end"),
        1, b"proof obligation failed"),
])
symbolic_named_pair_affine = """%%start
    i16(i16, i16) leaf = fn(tag, value) { if (tag < 0) {}; -1 - value };
    i16(i16, i16) producer = fn(tag, value) { i16 result = leaf(tag, value); result };
    proof bounded for producer(selector, before) -> (out)
        where (selector >= 0 && before >= 0) ensures (out >= -3) {}
%%end"""
cases.extend([
    ("ordinary-named-symbolic-pair-affine-refuted",symbolic_named_pair_affine,1,b"proof obligation failed"),
    ("ordinary-named-symbolic-pair-affine-proved",symbolic_named_pair_affine.replace("before >= 0","before == 2"),0,b""),
    ("ordinary-named-symbolic-pair-affine-own-old",symbolic_named_pair_affine.replace("out >= -3","out == core.old(before)"),
        1,b"proof obligation failed"),
    ("ordinary-named-symbolic-pair-affine-cross-old",symbolic_named_pair_affine.replace("out >= -3","out == core.old(selector)"),
        1,b"proof obligation failed"),
    ("ordinary-named-symbolic-pair-affine-refuses-masked-overflow",symbolic_named_pair_affine.replace("-1 - value","1 - value"),
        2,b"unsupported analysis"),
])

symbolic_named_pair_old_slot = """%%start
    i16(i16, i16) leaf = fn(first, second) { if (second < 0) { 7 } else { first } };
    i16(i16, i16) producer = fn(first, second) { i16 result = leaf(first, second); result };
    proof bounded for producer(selected, before) -> (out)
        where (selected == 3 && before >= 0) ensures (out == core.old(before)) {}
%%end"""
cases.extend([
    ("ordinary-named-symbolic-pair-old-cross-refuted",symbolic_named_pair_old_slot,1,b"proof obligation failed"),
    ("ordinary-named-symbolic-pair-old-cross-proved",symbolic_named_pair_old_slot.replace("before >= 0","before == 3"),
        0,b""),
    ("ordinary-named-symbolic-pair-old-own-proved",symbolic_named_pair_old_slot.replace("core.old(before)","core.old(selected)"),
        0,b""),
    ("ordinary-named-symbolic-pair-old-constant-refuted",symbolic_named_pair_old_slot.replace("before >= 0","before < 0"),
        1,b"proof obligation failed"),
])

symbolic_named_mixed_pair = """%%start
    i16(bool, i16) leaf = fn(enabled, value) { if (enabled) { 42 } else { value } };
    i16(bool, i16) producer = fn(enabled, value) { i16 result = leaf(enabled, value); result };
    proof bounded for producer(enabled, before) -> (out)
        where (!enabled && before >= 10) ensures (out <= 11) {}
%%end"""
cases.extend([
    ("ordinary-named-symbolic-mixed-false-refuted",symbolic_named_mixed_pair,1,b"proof obligation failed"),
    ("ordinary-named-symbolic-mixed-false-proved",symbolic_named_mixed_pair.replace("before >= 10","before == 10"),0,b""),
    ("ordinary-named-symbolic-mixed-true-old",symbolic_named_mixed_pair.replace("!enabled","enabled")
        .replace("out <= 11","out == core.old(before)"),1,b"proof obligation failed"),
    ("ordinary-named-symbolic-mixed-refuses-masked-return",symbolic_named_mixed_pair.replace("{ 42 }","{ value * value }"),
        2,b"unsupported analysis"),
])

stacked_named_exact_mixed = f"""%%start
    i16(bool, i16) leaf = fn(enabled, value) {{
        if (enabled) {{ 42 }} else {{ if (value == 300) {{ 9 }} else {{ value }} }}
    }};
    i16(bool, i16) producer = fn(enabled, value) {{ i16 result = leaf(enabled, value); result }};
    proof old_when_enabled for leaf(enabled, before) -> (out)
        where (enabled && before >= 0) ensures (out == core.old(before)) {{}}
    proof bounded for producer(enabled, before) -> (out)
        where (enabled && before >= 0) ensures (out <= 42) {{}}
    i16 inactive = producer(false, 300);
    i16 saved = producer(true, 300);
    core.require(inactive == 9, {report(38,"WRONG-INACTIVE-MIXED")});
    core.require(saved == 42, {report(39,"WRONG-ACTIVE-MIXED")});
%%end"""
cases.extend([
    ("exact-startup-named-stacked-mixed-old-refuted",stacked_named_exact_mixed,1,b"proof obligation failed"),
    ("exact-startup-named-stacked-mixed-old-proved",stacked_named_exact_mixed.replace("producer(true, 300)","producer(true, 42)"),
        0,b""),
    ("exact-startup-named-stacked-mixed-false-old",stacked_named_exact_mixed.replace("where (enabled","where (!enabled",1),
        1,b"proof obligation failed"),
])

stacked_named_exact_pair = f"""%%start
    i16(i8, i16) leaf = fn(tag, value) {{ if (tag > 0) {{ 42 }} else {{ if (value == 300) {{ 9 }} else {{ value }} }} }};
    i16(i8, i16) producer = fn(tag, value) {{ i16 result = leaf(tag, value); result }};
    proof old_second for leaf(selector, before) -> (out)
        where (selector > 0) ensures (out == core.old(before)) {{}}
    proof bounded for producer(selector, before) -> (out)
        where (selector > 0) ensures (out <= 42) {{}}
    i16 inactive = producer(-1, 300);
    i16 saved = producer(1, 300);
    core.require(inactive == 9, {report(38,"WRONG-INACTIVE-PAIR")});
    core.require(saved == 42, {report(39,"WRONG-ACTIVE-PAIR")});
%%end"""
cases.extend([
    ("exact-startup-named-stacked-pair-old-second-refuted",stacked_named_exact_pair,1,b"proof obligation failed"),
    ("exact-startup-named-stacked-pair-old-second-proved",stacked_named_exact_pair.replace("producer(1, 300)","producer(1, 42)"),
        0,b""),
])

stacked_named_calling_producer = f"""%%start
    i8(i8) leaf = fn(input) {{ if (input < 0) {{ -3 }} else {{ 7 }} }};
    i8(i8) bridge = fn(input) {{ i8 result = leaf(input); result }};
    i8(i8) producer = fn(input) {{ i8 result = bridge(input); result }};
    proof inner for leaf(entry) -> (out) where (entry >= 0) ensures (out <= 6) {{}}
    proof outer for producer(entry) -> (out) ensures (out <= 7) {{}}
    i8 saved = producer(3);
    core.require(saved == 7, {report(39,"WRONG-STACKED-OUTPUT")});
%%end"""
cases.append(("exact-startup-named-stacked-calling-producer-refuted",stacked_named_calling_producer,
    1,b"proof obligation failed"))

ordered_calling_citation = """%%start
    i8(i8) execute = fn(input) { if (input < 0) { -3 } else { 7 } };
    i8(i8) producer = fn(input) { i8 result = execute(input); result };
    proof uses_issued for producer(arg) -> (out) where (arg >= 0) ensures (out <= 7) { core.use(issued(arg)); }
    proof issued for producer(arg) -> (out) where (arg >= 0) ensures (out == 7) {}
%%end"""
cases.extend([
    ("ordered-calling-citation-verified-peer",ordered_calling_citation,0,b""),
    ("ordered-calling-citation-refuses-refuted-peer",ordered_calling_citation.replace("out == 7","out == 8"),
        2,b"unsupported analysis"),
    ("ordered-calling-citation-refuses-vacuous-peer",ordered_calling_citation.replace("arg >= 0","false"),
        2,b"unsupported analysis"),
    ("ordered-calling-citation-refuses-empty-entry",ordered_calling_citation.replace("arg >= 0","arg < -128"),
        2,b"unsupported analysis"),
    ("ordered-calling-citation-refuses-stronger-goal",ordered_calling_citation.replace("out <= 7","out <= 6"),
        2,b"unsupported analysis"),
])

ordered_rectangular_citation = """%%start
    i16(i16,i16) leaf = fn(tag,value) { if (tag >= 0) { value } else { 4 } };
    i16(i16,i16) producer = fn(tag,value) { i16 result = leaf(tag,value); result };
    proof top for producer(p,q) -> (out) where (p >= 2 && q == 7) ensures (out <= 9) {
        core.use(middle(p,q));
    }
    proof middle for producer(a,b) -> (out) where (a >= 1 && b >= 7 && b <= 8) ensures (out <= 8) {
        core.use(issued(a,b));
    }
    proof issued for producer(t,v) -> (out) where (t >= 0 && v >= 6 && v <= 8) ensures (out <= 8) {}
%%end"""
startup_rectangular_citation = ordered_rectangular_citation.replace("%%end", f"""
    i16 startup_value = producer(3,7);
    core.require(startup_value == 7, {report(48,"PAIR-CITATION-STARTUP")});
%%end""")
cases.extend([
    ("exact-startup-rectangular-citation-control", startup_rectangular_citation
        .replace("core.use(middle(p,q));", "").replace("core.use(issued(a,b));", ""), 0, b""),
    ("exact-startup-rectangular-citation-proved", startup_rectangular_citation, 0, b""),
    ("exact-startup-rectangular-citation-guard-at-actual-point",
        startup_rectangular_citation.replace("p >= 2", "p >= -1"), 0, b""),
    ("exact-startup-rectangular-citation-inapplicable-peer",
        startup_rectangular_citation.replace("p >= 2", "p >= -1").replace("producer(3,7)", "producer(0,7)"),
        2, b"unsupported analysis"),
    ("exact-startup-rectangular-citation-refuted-premise",
        startup_rectangular_citation.replace("ensures (out <= 8) {}", "ensures (out <= 6) {}"),
        1, b"proof obligation failed"),
    ("exact-startup-rectangular-citation-refuted-show",
        startup_rectangular_citation.replace("core.use(middle(p,q));", "core.use(middle(p,q)); core.show(out > 7);"),
        1, b"proof obligation failed"),
    ("exact-startup-rectangular-citation-no-implication",
        startup_rectangular_citation.replace("out <= 9", "out == 7"), 2, b"unsupported analysis"),
])
stacked_startup_citation = stacked_named_calling_producer.replace(
    "proof inner for leaf(entry) -> (out) where (entry >= 0) ensures (out <= 6) {}",
    """proof premise for bridge(entry) -> (out) ensures (out == 7) {}
    proof conclusion for bridge(entry) -> (out) ensures (out <= 8) { core.use(premise(entry)); }""")
cases.extend([
    ("exact-startup-citation-stacked-call-proved", stacked_startup_citation, 0, b""),
    ("exact-startup-citation-stacked-call-refuted",
        stacked_startup_citation.replace("ensures (out == 7) {}", "ensures (out == 6) {}"),
        1, b"proof obligation failed"),
])
cases.extend([
    ("ordered-calling-citation-unary-stronger-guard", ordered_calling_citation.replace("arg >= 0", "arg >= 1 && arg <= 5", 1),
        0, b""),
    ("ordered-calling-citation-pair-guarded", ordered_rectangular_citation, 0, b""),
    ("ordered-calling-citation-pair-weaker-first", ordered_rectangular_citation.replace("p >= 2", "p >= -1"),
        2, b"unsupported analysis"),
    ("ordered-calling-citation-pair-wrong-second", ordered_rectangular_citation.replace("q == 7", "q == 6"),
        2, b"unsupported analysis"),
    ("ordered-calling-citation-pair-arguments-swapped", ordered_rectangular_citation.replace("middle(p,q)", "middle(q,p)"),
        2, b"unsupported analysis"),
    ("ordered-calling-citation-pair-show-refuted",
        ordered_rectangular_citation.replace("core.use(middle(p,q));", "core.use(middle(p,q)); core.show(out > 7);"),
        1, b"proof obligation failed"),
    ("ordered-calling-citation-pair-refuses-failed-peer",
        ordered_rectangular_citation.replace("ensures (out <= 8) {}", "ensures (out <= 7) {}"),
        2, b"unsupported analysis"),
])
ordered_calling_citation_chain = ordered_calling_citation.replace(
    "proof uses_issued", "proof top for producer(arg) -> (out) where (arg >= 0) ensures (out <= 8) { core.use(uses_issued(arg)); }\n    proof uses_issued")
cases.extend([
    ("ordered-calling-citation-chain-verified",ordered_calling_citation_chain,0,b""),
    ("ordered-calling-citation-chain-refuses-middle-without-implication",
        ordered_calling_citation_chain.replace("out <= 8","out == 7"),2,b"unsupported analysis"),
    ("ordered-calling-citation-chain-refuses-cycle",
        ordered_calling_citation_chain.replace("core.use(issued(arg))","core.use(top(arg))"),2,b"unsupported analysis"),
])

ordered_calling_citation_then_show = ordered_calling_citation.replace(
    "ensures (out <= 7) { core.use(issued(arg)); }",
    "ensures (out == 7) { core.use(issued(arg)); core.show(out >= 7); }").replace(
    "ensures (out == 7) {}", "ensures (out <= 7) {}")
cases.extend([
    ("ordered-calling-citation-then-show-verified",ordered_calling_citation_then_show,0,b""),
    ("ordered-calling-citation-then-show-refuted",
        ordered_calling_citation_then_show.replace("out >= 7","out > 7"),1,b"proof obligation failed"),
    ("ordered-calling-citation-then-show-refuses-unverified-peer",
        ordered_calling_citation_then_show.replace("out <= 7","out <= 6"),2,b"unsupported analysis"),
])

ordered_shows_calling_producer = authored_named_calling_producer.replace(
    "core.show(out < 7);", "core.show(out <= 7); core.show(out < 7); core.show(out >= 0);")
cases.extend([
    ("ordinary-named-calling-ordered-shows-earliest",ordered_shows_calling_producer,1,b"proof obligation failed"),
    ("ordinary-named-calling-ordered-shows-later",ordered_shows_calling_producer.replace("out < 7","out <= 7"),
        1,b"proof obligation failed"),
    ("ordinary-named-calling-ordered-shows-proved",ordered_shows_calling_producer.replace("out < 7","out <= 7")
        .replace("out >= 0","out >= -3").replace("out == 7","out <= 7"),0,b""),
])

batched_named_calling_producer = """%%start
    i8(i8) leaf = fn(n) { if (n < 0) { -3 } else { 7 } };
    i8(i8) producer = fn(n) { i8 produced = leaf(n); produced };
    proof positive for producer(entry) -> (out) where (entry >= 0) ensures (out == 7) {}
    proof negative for producer(entry) -> (out) where (entry < 0) ensures (out == -3) {}
%%end"""
cases.extend([
    ("ordinary-batched-named-calling-producer-proved",batched_named_calling_producer,0,b""),
    ("ordinary-batched-named-calling-producer-refuted",batched_named_calling_producer.replace("out == -3","out == -4"),
        1,b"proof obligation failed"),
])

branching_affine_proof = """%%start
    i8(i8) normalize = fn(value) {
        i8 shifted = value + 2;
        i8 joined = if (value < 0) { shifted } else { value };
        if (joined <= 3) { i8 changed = joined + 1; changed }
        else { i8 reflected = 9 - joined; reflected }
    };
    proof bounded for normalize(entry) -> (out) where (entry <= 10) ensures (out <= 5) {}
    %%end"""
branching_affine_reached = branching_affine_proof.replace("%%end", f"""
    i8 first = normalize(-1);
    i8 second = normalize(4);
    core.require(first == 2, {report(9,"WRONG-FIRST-BRANCH")});
    core.require(second == 5, {report(11,"WRONG-SECOND-BRANCH")});
    %%end""")
cases.extend([
    ("ordinary-pure-affine-proof-conditional-proved",branching_affine_proof,0,b""),
    ("ordinary-pure-affine-proof-conditional-refuted",branching_affine_proof.replace("out <= 5","out <= 4"),1,
        b"proof obligation failed"),
    ("exact-startup-ordinary-pure-affine-proof-conditional-proved",branching_affine_reached,0,b""),
    ("exact-startup-ordinary-pure-affine-proof-conditional-refuted",branching_affine_reached.replace("out <= 5","out <= 4"),1,
        b"proof obligation failed"),
])
quantified_array = """%%start
    u8[4]() make4 = fn { [0, 7, 0, 0] };
    proof cells for make4() -> (items)
        forall (u64 i where i == 0 || i >= 2 && i < 4)
        ensures (items[i] == 0)
    { core.show(items[i] == 0); }
%%end"""
witnessed_array = """%%start
    u8[4]() make4 = fn { [0, 7, 0, 0] };
    proof cells for make4() -> (items)
        ensures (exists (u64 i where i < 4) { items[i] == 7 })
    { core.witness(1); }
%%end"""
cases.extend([
    ("ordinary-fixed-array-forall-verified", quantified_array, 0, b""),
    ("ordinary-fixed-array-forall-refuted", quantified_array.replace("i == 0", "i == 1"), 1, b"proof obligation failed"),
    ("ordinary-fixed-array-witness-verified", witnessed_array, 0, b""),
    ("ordinary-fixed-array-witness-refuted", witnessed_array.replace("witness(1)", "witness(2)"), 1, b"proof obligation failed"),
])
symbolic_array_cells = """%%start
    u8[3](u8) make = fn(value) {
        if (value < 7) {
            [value + 1,99,value + 1]
        } else {
            u8 adjusted = value - 0;
            u8[3] constructed = [adjusted,99,adjusted];
            u8[3] copied = constructed;
            copied
        }
    };
    proof cells for make(entry) -> (items)
        where (entry == 6 || entry == 7)
        forall (u64 index where index != 1 && index < 3)
        ensures (items[index] == 7)
    {}
%%end"""
cases.extend([
    ("ordinary-fixed-array-symbolic-cells-verified", symbolic_array_cells, 0, b""),
    ("ordinary-fixed-array-symbolic-cells-interval", symbolic_array_cells
        .replace("entry == 6 || entry == 7", "entry >= 5 && entry <= 7")
        .replace("items[index] == 7", "items[index] <= 7"), 0, b""),
    ("ordinary-fixed-array-symbolic-cells-affine-hole", symbolic_array_cells
        .replace("entry == 6 || entry == 7", "entry == 4 || entry == 6")
        .replace("items[index] == 7", "items[index] != 6"), 0, b""),
    ("ordinary-fixed-array-symbolic-cells-copy-refuted", symbolic_array_cells.replace("value - 0", "value + 1"),
        1, b"proof obligation failed"),
    ("ordinary-fixed-array-symbolic-cells-branch-refuted", symbolic_array_cells.replace("entry == 6", "entry == 5"),
        1, b"proof obligation failed"),
    ("ordinary-fixed-array-symbolic-cells-unsafe-unselected",
        symbolic_array_cells.replace(",99,value + 1", ",value + 250,value + 1"), 2, b"unsupported analysis"),
])
correlated_array_witness = """%%start
    u8[1](u8,u8) reflect = fn(left,right) {
        if (left < right) {
            u8 shifted = 10 - right;
            u8[1] constructed = [shifted];
            u8[1] copied = constructed;
            copied
        } else { [9] }
    };
    proof cells for reflect(a,b) -> (items)
        where (a >= 2 && a <= 4 && b >= 2 && b <= 4)
        forall (u64 index where index == 0)
        ensures (items[index] != 6)
    {}
%%end"""
cases.append(("ordinary-fixed-array-symbolic-cells-correlated", correlated_array_witness, 1, b"proof obligation failed"))
qualified_scalar_library = """%%start
    i8(i8) value = fn(x) { if (x < 0) { -3 } else { 7 } };
%%end"""
qualified_scalar_proof = """%%start
    imported = namespace app "src/calc.w";
    i8(i8) value = fn(local_input) { local_input };
    proof qualified for imported.value(entry) -> (out) where (entry >= 0) ensures (out == 7) {}
%%end"""
qualified_scalar_reached = qualified_scalar_proof.replace("%%end", f"""
    i8 selected = imported.value(3);
    i8 outside = imported.value(-2);
    i8 decoy = value(3);
    core.require(selected == 7, {report(39,"WRONG-QUALIFIED-RESULT")});
    core.require(outside == -3, {report(40,"WRONG-OUTSIDE-GUARD")});
    core.require(decoy == 3, {report(41,"WRONG-LOCAL-DECOY")});
%%end""")
for name, text, child, code, why in [
    ("exact-startup-qualified-scalar-proof-proved", qualified_scalar_reached, qualified_scalar_library, 0, b""),
    ("exact-startup-qualified-scalar-proof-refuted", qualified_scalar_reached.replace("out == 7", "out == 6"),
        qualified_scalar_library, 1, b"proof obligation failed"),
    ("exact-startup-qualified-scalar-proof-require", qualified_scalar_reached.replace("selected == 7", "selected == 6"),
        qualified_scalar_library, 39, b"WRONG-QUALIFIED-RESULT"),
    ("exact-startup-qualified-scalar-proof-effect-refused", qualified_scalar_reached,
        qualified_scalar_library.replace("if (x < 0)", f"core.require(false, {report(46,'UNPUBLISHED-LEAF')}); if (x < 0)"),
        2, b"unsupported analysis"),
    ("ordinary-qualified-scalar-proof-unused-refuted", qualified_scalar_proof.replace("out == 7", "out == 6"),
        qualified_scalar_library, 1, b"proof obligation failed"),
]:
    namespace_children[name] = child
    cases.append((name, text, code, why))

authored_case_proof = """%%start
    u8(u8) clamp = fn(value) { if (value > 10) { 10 } else { value } };
    proof clamped for clamp(x) -> (out) ensures (out <= 10) {
        if (x > 10) { core.show(out == 10); }
        else { core.show(out == x); }
    }
%%end"""
authored_case_startup = authored_case_proof.replace("%%end", f"""
    u8 uncapped = clamp(6);
    u8 capped = clamp(11);
    core.require(uncapped == 6, {report(42,"WRONG-UNCAPPED-RETURN")});
    core.require(capped == 10, {report(43,"WRONG-CAPPED-RETURN")});
%%end""")
authored_case_nested = authored_case_startup.replace(
    "if (x > 10) { core.show(out == 10); }",
    """if (x > 10) {
            if (x > 12) { core.show(out == 10); }
            else { core.show(out <= 10); core.show(out == 10); }
        }""").replace("core.show(out == x);", "core.show(out == core.old(x));")
cases.extend([
    ("ordinary-authored-case-proof-documented-clamp", authored_case_proof, 0, b""),
    ("ordinary-authored-case-proof-refuted-show", authored_case_proof.replace("if (x > 10)", "if (x > 11)"),
        1, b"proof obligation failed"),
    ("ordinary-authored-case-proof-refuted-goal", authored_case_proof.replace("ensures (out <= 10)", "ensures (out <= 9)"),
        1, b"proof obligation failed"),
    ("exact-startup-authored-case-proof-documented-clamp", authored_case_startup, 0, b""),
    ("exact-startup-authored-case-proof-wrong-arm", authored_case_startup.replace("if (x > 10)", "if (x > 11)"),
        1, b"proof obligation failed"),
    ("exact-startup-authored-case-proof-nested", authored_case_nested, 0, b""),
    ("exact-startup-authored-case-proof-nested-first-show",
        authored_case_nested.replace("core.show(out <= 10); core.show(out == 10);",
            "core.show(out < 10); core.show(out == 8);"),
        1, b"proof obligation failed"),
])

authored_case_rewrite = """%%start
    u8(u8) clamp = fn(value) { if (value > 10) { 10 } else { value } };
    proof clamped for clamp(x) -> (out) ensures (out <= 10) {
        if (x > 10) {
            core.show(out == 10);
            core.rewrite(out == 10, _ <= 10);
        } else {
            core.show(out == core.old(x));
            core.rewrite(core.old(x) == out, _ <= 10);
        }
    }
%%end"""
authored_case_rewrite_reached = authored_case_rewrite.replace("%%end", "u8 first = clamp(6); u8 second = clamp(11); %%end")
cases.extend([
    ("ordinary-authored-case-rewrite-proved", authored_case_rewrite, 0, b""),
    ("ordinary-authored-case-rewrite-refutes-premise",
        authored_case_rewrite.replace("if (value > 10) { 10 }", "if (value > 10) { 9 }"),
        1, b"proof obligation failed"),
    ("exact-startup-authored-case-rewrite-proved", authored_case_rewrite_reached, 0, b""),
    ("exact-startup-authored-case-rewrite-refutes-premise",
        authored_case_rewrite_reached.replace("if (value > 10) { 10 }", "if (value > 10) { 9 }"),
        1, b"proof obligation failed"),
    ("exact-startup-authored-case-rewrite-rejects-sibling-premise",
        authored_case_rewrite_reached.replace("core.show(out == core.old(x));", "")
            .replace("core.old(x) == out, _ <= 10", "out == 10, _ <= 10"),
        2, b"unsupported analysis"),
])

authored_case_citation = """%%start
    u8(u8) clamp = fn(value) { if (value > 10) { 10 } else { value } };
    proof clamped for clamp(x) -> (out) ensures (out <= 10) {
        if (x > 10) {
            core.use(high(x));
            core.rewrite(out == 10, _ <= 10);
            core.show(out >= 10);
        } else {
            core.show(out == x);
        }
    }
    proof high for clamp(input) -> (out) where (input > 10) ensures (out == 10) {}
%%end"""
authored_case_citation_reached = authored_case_citation.replace("%%end", "u8 selected = clamp(11); u8 uncapped = clamp(6); %%end")
authored_case_citation_ordered = authored_case_citation.replace("if (value > 10) { 10 }", "if (value > 10) { 9 }").replace(
    "core.use(high(x));", "core.show(out == 8); core.use(high(x));").replace(
    "proof high", "proof separate for clamp(input) -> (out) where (input > 10) ensures (out == 8) {}\n    proof high")
cases.extend([
    ("ordinary-authored-case-citation-ordered-diagnostics", authored_case_citation_ordered, 1, b"proof obligation failed"),
    ("exact-startup-authored-case-citation-ordered-diagnostics",
        authored_case_citation_ordered.replace("%%end", "u8 selected = clamp(11); %%end"), 1, b"proof obligation failed"),
    ("ordinary-authored-case-citation-proved", authored_case_citation, 0, b""),
    ("ordinary-authored-case-citation-inapplicable-peer", authored_case_citation.replace("input > 10", "input > 12"),
        2, b"unsupported analysis"),
    ("ordinary-authored-case-citation-refuted-premise",
        authored_case_citation.replace("if (value > 10) { 10 }", "if (value > 10) { 9 }"),
        1, b"proof obligation failed"),
    ("exact-startup-authored-case-citation-proved", authored_case_citation_reached, 0, b""),
    ("exact-startup-authored-case-citation-refuted-premise",
        authored_case_citation_reached.replace("if (value > 10) { 10 }", "if (value > 10) { 9 }"),
        1, b"proof obligation failed"),
    ("exact-startup-authored-case-citation-inapplicable-peer",
        authored_case_citation_reached.replace("input > 10", "input > 12"), 2, b"unsupported analysis"),
    ("exact-startup-authored-case-citation-sibling-equality",
        authored_case_citation_reached.replace("core.show(out == x);", "core.rewrite(out == 10, _ <= 10);"),
        2, b"unsupported analysis"),
])

bounded_summary_producer = """%%start
    u8(u8) select_value = fn(value) { if (value < 5) { value } else { 7 } };
    u8(u8) producer = fn(value) { u8 result = select_value(value); result };
    proof bounded for producer(entry) -> (out) where (entry >= 1 && entry <= 10) ensures (out <= 7) {}
%%end"""
bounded_summary_consumer = bounded_summary_producer.replace("%%end", f"""
    unit(u8) consume = fn(input) {{
        core.narrow(input >= 1, {report(50,"INPUT-LOW")});
        core.narrow(input <= 10, {report(51,"INPUT-HIGH")});
        u8 observed = producer(input);
        u8 fixed_result = producer(8);
        core.require(fixed_result == 7, {report(52,"WRONG-FIXED-RETURN")});
        core.require(observed != 5, {report(53,"UNREACHABLE-RETURN")});
        if (input == 3) {{ core.require(observed == 3, {report(54,"REAL-DEPENDENT-RETURN")}); }};
        if (input >= 5) {{ core.require(observed == 7, {report(55,"WRONG-HIGH-RETURN")}); }};
    }};
%%end""")
cases.extend([
    ("ordinary-unary-bounded-summary-proved", bounded_summary_consumer, 0, b""),
    ("ordinary-unary-bounded-summary-refutes-consumer", bounded_summary_consumer.replace("observed == 3", "observed != 3"),
        54, b"REAL-DEPENDENT-RETURN"),
    ("ordinary-unary-bounded-summary-refuses-guard", bounded_summary_consumer.replace("input <= 10", "input <= 11"),
        2, b"unsupported analysis"),
    ("ordinary-unary-bounded-summary-refuses-literal", bounded_summary_consumer.replace("producer(8)", "producer(11)"),
        2, b"unsupported analysis"),
    ("ordinary-unary-bounded-summary-refutes-proof", bounded_summary_producer.replace("out <= 7", "out < 7"),
        1, b"proof obligation failed"),
    ("exact-startup-unary-bounded-summary-refutes-proof", bounded_summary_producer.replace("out <= 7", "out < 7")
        .replace("%%end", "u8 startup = producer(8); %%end"), 1, b"proof obligation failed"),
])

pair_bound_producer = """%%start
    i16(i8, i16) leaf = fn(tag, value) { if (tag > 0) { value } else { 7 } };
    i16(i8, i16) producer = fn(tag, value) { i16 result = leaf(tag, value); result };
    proof bounded for producer(selector, before) -> (out)
        where (selector <= 4 && before <= 3) ensures (out <= 7) {}
%%end"""
pair_bound_consumer = pair_bound_producer.replace("%%end",f"""
    unit(i8, i16) consume = fn(tag, value) {{
        core.narrow(tag <= 4, {report(56,"PAIR-TAG")});
        core.narrow(value <= 3, {report(57,"PAIR-VALUE")});
        i16 observed = producer(tag, value);
        i16 same_arg = producer(1, value);
        i16 fixed_value = producer(tag, 3);
        i16 fixed_both = producer(0, 3);
        core.require(fixed_both == 7, {report(58,"PAIR-FIXED-BOTH")});
        if (tag <= 0) {{
            core.require(observed == 7, {report(59,"PAIR-NONPOSITIVE-TAG")});
            core.require(fixed_value == 7, {report(60,"PAIR-NONPOSITIVE-FIXED")});
        }} else {{
            core.require(observed <= 3, {report(61,"PAIR-DEPENDENT-BOUND")});
            core.require(fixed_value == 3, {report(62,"PAIR-DEPENDENT-FIXED")});
            if (value == 2) {{
                core.require(observed == 2, {report(63,"PAIR-DEPENDENT-ALIAS")});
                core.require(same_arg == 2, {report(64,"PAIR-DEPENDENT-FIXED-TAG")});
            }};
        }};
    }};
%%end""")
pair_bound_affine = verified_rectangular_affine.replace(
    "out == -1 - core.old(before)", "out <= -11").replace(
    "i16 observed = producer(tag, value);",
    f"""i16 observed = producer(tag, value);
        i16 fixed = producer(tag, 12);
        i16 selected = producer(1, value);
        core.require(fixed == -13, {report(65,"PAIR-AFFINE-FIXED")});
        if (value == 12) {{ core.require(selected == -13, {report(66,"PAIR-AFFINE-SELECTED")}); }};""")
cases.extend([
    ("ordinary-pair-bounded-summary-proved", pair_bound_consumer, 0, b""),
    ("ordinary-pair-bounded-summary-refutes-consumer", pair_bound_consumer.replace("observed == 2", "observed != 2"),
        63, b"PAIR-DEPENDENT-ALIAS"),
    ("ordinary-pair-bounded-summary-refuses-first-guard", pair_bound_consumer.replace("core.narrow(tag <= 4", "core.narrow(tag <= 5"),
        2, b"unsupported analysis"),
    ("ordinary-pair-bounded-summary-refuses-second-guard", pair_bound_consumer.replace("core.narrow(value <= 3", "core.narrow(value <= 4"),
        2, b"unsupported analysis"),
    ("ordinary-pair-bounded-summary-refuses-literal", pair_bound_consumer.replace("producer(0, 3)", "producer(5, 3)"),
        2, b"unsupported analysis"),
    ("ordinary-pair-bounded-summary-refuses-effectful-dependency", pair_bound_consumer.replace(
        "if (tag > 0)", f"core.require(false, {report(67,'PAIR-NO-ELISION')}); if (tag > 0)"),
        2, b"unsupported analysis"),
    ("ordinary-pair-bounded-summary-refutes-proof", pair_bound_producer.replace("out <= 7", "out < 7"),
        1, b"proof obligation failed"),
    ("ordinary-pair-bounded-summary-affine", pair_bound_affine, 0, b""),
    ("ordinary-pair-bounded-summary-refutes-affine", pair_bound_affine.replace("observed == -13", "observed != -13"),
        48, b"VERIFIED-AFFINE-OUTPUT"),
    ("ordinary-pair-bounded-summary-refuses-linked-arguments", pair_bound_affine.replace("producer(tag, value)", "producer(value, value)"),
        2, b"unsupported analysis"),
    ("exact-startup-pair-bounded-summary-refutes-proof", pair_bound_producer.replace("out <= 7", "out < 7")
        .replace("%%end", "i16 startup = producer(0, 3); %%end"), 1, b"proof obligation failed"),
])

inferred_array_exists = """%%start
    i8[3](i8) reflect = fn(value) { [value, 127, -1 - value] };
    proof covered for reflect(entry) -> (items)
        ensures (exists (u64 i where i == 0 || i == 2) { items[i] >= 0 }) {}
%%end"""
inferred_array_exists_captured = inferred_array_exists.replace("[value, 127, -1 - value]", """
    i8 inverse = -1 - value;
    i8[3] saved = [value, 127, inverse];
    i8[3] copied = saved;
    copied
""")
inferred_array_exists_branch = """%%start
    i8[2](i8) choose = fn(value) {
        if (value < 0) { [value, 0] } else { [0, -1 - value] }
    };
    proof covered for choose(entry) -> (items)
        ensures (exists (u64 i where i < 2) { items[i] >= 0 }) {}
%%end"""
cases.extend([
    ("ordinary-inferred-array-exists-direct", inferred_array_exists, 0, b""),
    ("ordinary-inferred-array-exists-captured", inferred_array_exists_captured, 0, b""),
    ("ordinary-inferred-array-exists-hole-refuted", inferred_array_exists_captured.replace("items[i] >= 0", "items[i] > 0"),
        1, b"proof obligation failed"),
    ("ordinary-inferred-array-exists-empty-refuted", inferred_array_exists.replace("i == 0 || i == 2", "i < 0"),
        1, b"proof obligation failed"),
    ("ordinary-inferred-array-exists-vacuous", inferred_array_exists.replace("i == 0 || i == 2", "i < 0")
        .replace("ensures (exists", "where (false) ensures (exists"), 0, b""),
    ("ordinary-inferred-array-exists-branches", inferred_array_exists_branch, 0, b""),
    ("ordinary-inferred-array-exists-branch-refuted", inferred_array_exists_branch.replace("[0, -1 - value]", "[-1, -1 - value]"),
        1, b"proof obligation failed"),
    ("ordinary-inferred-array-exists-refuses-outside-array", inferred_array_exists.replace("i == 0 || i == 2", "i < 4"),
        2, b"unsupported analysis"),
    ("ordinary-inferred-array-exists-refuses-masked-arithmetic", inferred_array_exists.replace(
        "[value, 127, -1 - value]", "[7, 127, value + 1]").replace("i == 0 || i == 2", "i == 0"),
        2, b"unsupported analysis"),
])

reached_fixed_array_exists = f"""%%start
    i8[3](i8) reflect = fn(value) {{
        i8 inverse = -1 - value;
        i8[3] made = [value, 127, inverse];
        i8[3] copied = made;
        copied
    }};
    proof covered for reflect(entry) -> (items)
        where (entry >= 0 && entry <= 6)
        ensures (exists (u64 i where i == 0 || i == 2) {{ items[i] >= 0 }}) {{}}
    i8[3] first = reflect(3);
    i8[3] second = reflect(5);
    i8[3] duplicate = reflect(3);
    i8[3] outside = reflect(9);
    u64 index = 2;
    i8 first_cell = first[0];
    i8 second_cell = second[0];
    i8 duplicate_cell = duplicate[0];
    i8 outside_cell = outside[index];
    core.require(first_cell == 3, {report(41,"WRONG-FIRST-ARRAY-CELL")});
    core.require(second_cell == 5, {report(42,"WRONG-SECOND-ARRAY-CELL")});
    core.require(duplicate_cell == 3, {report(43,"WRONG-DUPLICATE-ARRAY-CELL")});
    core.require(outside_cell == -10, {report(44,"WRONG-OUTSIDE-ARRAY-CELL")});
%%end"""
reached_fixed_array_pair = f"""%%start
    u8[2](u8,u8) make = fn(tag, value) {{
        if (tag == 1) {{ [value, 7] }} else {{ [9, value] }}
    }};
    proof covered for make(selector, before) -> (items)
        where (selector == 1 && before <= 5) forall (u64 i where i == 0)
        ensures (items[i] <= 5) {{}}
    u8[2] chosen = make(1, 3);
    u8[2] outside = make(2, 9);
    u8 observed = chosen[0];
    u8 outside_cell = outside[0];
    core.require(observed == 3, {report(41,"WRONG-PAIR-ARRAY-CELL")});
    core.require(outside_cell == 9, {report(42,"WRONG-OUTSIDE-PAIR-CELL")});
%%end"""
reached_fixed_array_witness = """%%start
    u8[2]() make = fn { [7, 8] };
    proof covered for make() -> (items)
        ensures (exists (u64 i where i < 2) { items[i] == 7 })
    { core.witness(0); }
    u8[2] picked = make();
    u8 observed = picked[0];
%%end"""
cases.extend([
    ("exact-startup-fixed-array-exists-proved", reached_fixed_array_exists, 0, b""),
    ("exact-startup-fixed-array-exists-refuted", reached_fixed_array_exists.replace("items[i] >= 0", "items[i] > 3"),
        1, b"proof obligation failed"),
    ("exact-startup-fixed-array-exists-output", reached_fixed_array_exists.replace("outside_cell == -10", "outside_cell == 0"),
        44, b"WRONG-OUTSIDE-ARRAY-CELL"),
    ("exact-startup-fixed-array-refuses-outside-index", reached_fixed_array_exists.replace("u64 index = 2", "u64 index = 3"),
        2, b"unsupported analysis"),
    ("exact-startup-fixed-array-refuses-unverified-target", reached_fixed_array_exists.replace(
        "%%end", "i8[1]() absent = fn { [4] }; i8[1] hidden = absent(); %%end"),
        2, b"unsupported analysis"),
    ("exact-startup-fixed-array-pair-proved", reached_fixed_array_pair, 0, b""),
    ("exact-startup-fixed-array-pair-refuted", reached_fixed_array_pair.replace("items[i] <= 5", "items[i] < 3"),
        1, b"proof obligation failed"),
    ("exact-startup-fixed-array-nullary-witness-proved", reached_fixed_array_witness, 0, b""),
    ("exact-startup-fixed-array-nullary-witness-refuted", reached_fixed_array_witness.replace("core.witness(0)", "core.witness(1)"),
        1, b"proof obligation failed"),
])

proof_diagnostic_notes = {
    "exact-startup-fixed-array-exists-refuted": b"call returned an array with no matching index",
    "exact-startup-fixed-array-pair-refuted": b"call returned cell [0] = 3",
    "exact-startup-fixed-array-nullary-witness-refuted": b"call returned cell [1] = 8",
    "ordinary-inferred-array-exists-hole-refuted": b"symbolic call entry: value = -1",
    "ordinary-inferred-array-exists-empty-refuted": b"symbolic call can return an array with no matching index",
    "ordinary-inferred-array-exists-branch-refuted": b"symbolic call entry: value = 0",
    "ordinary-pair-bounded-summary-refutes-proof": b"symbolic call can return 7",
    "exact-startup-pair-bounded-summary-refutes-proof": b"call returned 7",
    "ordinary-unary-bounded-summary-refutes-proof": b"symbolic call can return 7",
    "exact-startup-unary-bounded-summary-refutes-proof": b"call returned 7",
    "ordinary-authored-case-citation-ordered-diagnostics": b"core.show(out == 8)",
    "exact-startup-authored-case-citation-ordered-diagnostics": b"core.show(out == 8)",
    "ordinary-authored-case-citation-refuted-premise": b"symbolic call can return 9",
    "exact-startup-authored-case-citation-refuted-premise": b"call returned 9",
    "ordinary-authored-case-rewrite-refutes-premise": b"symbolic call can return 9",
    "exact-startup-authored-case-rewrite-refutes-premise": b"call returned 9",
    "ordinary-authored-case-proof-refuted-show": b"symbolic call entry: value = 11",
    "ordinary-authored-case-proof-refuted-goal": b"symbolic call can return 10",
    "exact-startup-authored-case-proof-wrong-arm": b"call returned 10",
    "exact-startup-authored-case-proof-nested-first-show": b"call returned 10",
    "exact-startup-qualified-scalar-proof-refuted": b"call returned 7",
    "ordinary-qualified-scalar-proof-unused-refuted": b"symbolic call can return 7",
    "ordered-calling-citation-pair-show-refuted": b"symbolic call can return 7",
    "exact-startup-rectangular-citation-refuted-premise": b"call returned 7",
    "exact-startup-rectangular-citation-refuted-show": b"core.show(out > 7)",
    "exact-startup-citation-stacked-call-refuted": b"reached from this call",
    "ordinary-verified-unary-entry-refuted": b"symbolic call can return 2",
    "exact-startup-verified-unary-entry-refuted": b"call returned 5",
    "ordinary-fixed-array-symbolic-cells-correlated": b"symbolic call can return cell [0] = 6",
    "ordinary-fixed-array-symbolic-cells-copy-refuted": b"symbolic call can return cell [0] = 8",
    "ordinary-fixed-array-symbolic-cells-branch-refuted": b"symbolic call can return cell [0] = 6",
    "ordinary-fixed-array-forall-refuted": b"symbolic call can return cell [1] = 7",
    "ordinary-fixed-array-witness-refuted": b"symbolic call can return cell [2] = 0",
    "ordinary-unused-contextual-refuted": b"contextual value at entry: 11; at exit: 2",
    "exact-startup-public-namespace-contextual-refuted": b"contextual value at entry: 3; at exit: 3",
    "exact-startup-public-contextual-proof-refuted": b"contextual value at entry: 3; at exit: 3",
    "ordinary-ranked-tree-base-refuted": b"symbolic call can return 9",
    "ordinary-ranked-two-edge-base-refuted": b"symbolic call can return 8",
    "ordinary-rectangular-authored-show-goal-failure": b"symbolic call can return 245",
    "ordinary-rectangular-authored-show-first-step-failure": b"symbolic call can return 245",
    "ordinary-named-symbolic-mixed-false-refuted": b"symbolic call can return 12",
    "ordinary-named-symbolic-mixed-true-old": b"symbolic call can return 42",
    "exact-startup-named-stacked-mixed-old-refuted": b"call returned 42",
    "exact-startup-named-stacked-mixed-false-old": b"call returned 9",
    "ordinary-named-symbolic-pair-affine-refuted": b"symbolic call can return -4",
    "ordinary-named-symbolic-pair-affine-own-old": b"symbolic call can return -1",
    "ordinary-named-symbolic-pair-affine-cross-old": b"symbolic call can return -1",
    "ordinary-named-symbolic-pair-old-cross-refuted": b"symbolic call can return 3",
    "ordinary-named-symbolic-pair-old-constant-refuted": b"symbolic call can return 7",
    "ordinary-named-symbolic-pair-alias-refuted": b"symbolic call can return 12",
    "ordinary-named-symbolic-pair-constant-refuted": b"symbolic call can return 300",
    "exact-startup-named-stacked-pair-old-second-refuted": b"call returned 42",
    "exact-startup-named-calling-producer-observes-nested-watch": b"reached from this call",
    "exact-startup-named-stacked-calling-producer-refuted": b"through this call",
    "exact-startup-named-calling-producer-refuted": b"call returned 7",
    "ordered-calling-citation-then-show-refuted": b"symbolic call can return 7",
    "ordinary-named-calling-ordered-shows-earliest": b"symbolic call can return 7",
    "ordinary-named-calling-ordered-shows-later": b"symbolic call can return -3",
    "ordinary-authored-named-calling-step-refuted": b"symbolic call can return 7",
    "ordinary-authored-named-calling-goal-refuted": b"symbolic call can return -3",
    "ordinary-named-relational-entry-equality-affine-refuted": b"symbolic call can return 127",
    "ordinary-named-relational-alias-refuted": b"symbolic call can return 0",
    "ordinary-named-relational-affine-refuted": b"symbolic call can return 127",
    "ordinary-batched-named-calling-producer-refuted": b"symbolic call can return -3",
    "ordinary-pure-affine-proof-conditional-refuted": b"symbolic call can return 5",
    "exact-startup-ordinary-pure-affine-proof-conditional-refuted": b"call returned 5",
    "ordinary-pure-affine-proof-symbolic-refuted": b"symbolic call can return 102",
    "exact-startup-ordinary-pure-affine-proof-refuted": b"call returned 102",
}
cases.extend([
    ("ordinary-pure-affine-proof-symbolic-proved",ordinary_pure_affine_proof,0,b""),
    ("ordinary-pure-affine-proof-symbolic-refuted",ordinary_pure_affine_proof.replace("out <= 102","out <= 101"),1,
        b"proof obligation failed"),
    ("exact-startup-ordinary-pure-affine-proof-proved",ordinary_pure_affine_proof_reached,0,b""),
    ("exact-startup-ordinary-pure-affine-proof-no-authority-outside-where",
        ordinary_pure_affine_proof_reached.replace("advance(100)","advance(101)"),2,b"unsupported analysis"),
    ("exact-startup-ordinary-pure-affine-proof-no-entry-equality-authority",
        ordinary_pure_affine_proof_reached.replace("out <= 102","out == core.old(entry)"),2,b"unsupported analysis"),
    ("exact-startup-ordinary-pure-affine-proof-refuted",ordinary_pure_affine_proof_reached.replace("out <= 102","out <= 101"),1,
        b"proof obligation failed"),
])
authored_unary = f"""%%start
    unit(i8) observe = fn(value) {{
        core.narrow(value == -7 || value == 2, {report(1,"UNREACHED-CUT")});
        core.require(false, {{ .stdout = ""; .stderr = "UNARY"; .code = 71;
            .files = [{{ .path = "before.json"; .state = value; }},
                      {{ .path = "reflected.json"; .state = -value; }}]; }});
    }};
    %%end"""
startup_unary = """%%start
    i8 status = -37;
    bool ready = true;
    core.narrow(false, { .stdout = ""; .stderr = "UNARY-STARTUP"; .code = -status;
        .files = [{ .path = "code.json"; .state = -status; },
                  { .path = "inverted.json"; .state = !ready; }]; });
    %%end"""
cases.extend([
    ("authored-unary-state-preserves-holes",authored_unary,71,b"UNARY"),
    ("authored-unary-code-is-failure-singleton",
     authored_unary.replace("require(false,","require(value != -7,").replace(".code = 71",".code = -value"),7,b"UNARY"),
    ("authored-unary-code-refuses-non-singleton",authored_unary.replace(".code = 71",".code = -value"),2,b"unsupported analysis"),
    ("authored-unary-state-refuses-overflow",authored_unary.replace("value == -7","value == -128"),2,b"unsupported analysis"),
    ("exact-startup-authored-unary-state-and-code",startup_unary,37,b"UNARY-STARTUP"),
])
module_authored_output = f"""%%start
    i8(i8) identity = fn(value) {{ value }};
    bool(bool) truth = fn(value) {{ value }};
    i8 status = identity(37);
    bool ready = truth(true);
    core.require(status != 37, {{
        .stdout = ""; .stderr = "MODULE-STATE"; .code = status;
        .files = [
            {{ .path = "status.json"; .state = status; }},
            {{ .path = "note.txt"; .text = "selected at startup"; }},
            {{ .path = "ready.json"; .state = ready; }},
            {{ .path = "status-copy.json"; .state = status; }}
        ];
    }});
    core.require(false, {report(200,"AFTER-MODULE-STATE")});
    %%end"""
cases.extend([
    ("exact-startup-module-typed-code-state",module_authored_output,37,b"MODULE-STATEAFTER-MODULE-STATE"),
    ("exact-startup-module-typed-narrow-prestate",module_authored_output.replace("core.require(status","core.narrow(status"),37,b"MODULE-STATE"),
    ("exact-startup-module-success-skips-state",module_authored_output.replace("core.require(status != 37","core.require(status == 37"),200,b"AFTER-MODULE-STATE"),
])
startup_conditional_scalar = f"""%%start
    i8(i8) step = fn(value) {{
        core.require(value != 37, {report(45,"VALUE-PRODUCER")});
        value
    }};
    i8 warm = step(0);
    bool ready = warm == 0;
    bool chosen = if (ready) {{ ready }} else {{ false }};
    i8 status = if (chosen) {{
        core.require(false, {report(43,"VALUE-ARM")});
        if (ready) {{ step(37) }} else {{ 8 }}
    }} else {{
        i8 held = 19;
        held
    }};
    core.require(false, {{ .stdout = ""; .stderr = "VALUE-STORED"; .code = status;
        .files = [{{ .path = "status.json"; .state = status; }}]; }});
    %%end"""
cases.extend([
    ("exact-startup-conditional-scalar-binding",startup_conditional_scalar,43,b"VALUE-ARMVALUE-PRODUCERVALUE-STORED"),
    ("exact-startup-conditional-scalar-binding-empty",startup_conditional_scalar.replace("core.require(value","core.narrow(value"),43,b"VALUE-ARMVALUE-PRODUCER"),
    ("exact-startup-conditional-scalar-binding-arm-empty",startup_conditional_scalar.replace("core.require(false, " + report(43,"VALUE-ARM"),"core.narrow(false, " + report(43,"VALUE-ARM")),43,b"VALUE-ARM"),
    ("exact-startup-conditional-scalar-binding-opaque",startup_conditional_scalar.replace("warm == 0","warm != 0"),2,b"unsupported analysis"),
])
mixed_startup_unit_storage = f"""%%start
    unit(bool, i8) release = fn(enabled, code) {{
        core.narrow(enabled, {{ .stdout = ""; .stderr = "UNIT-EMPTY"; .code = code;
            .files = [{{ .path = "gate.json"; .state = enabled; }}]; }});
    }};
    i8 status = 37;
    bool ready = status == 38;
    release(ready, status);
    core.require(false, {report(55,"UNIT-AFTER")});
    %%end"""
cases.extend([
    ("exact-startup-unit-mixed-storage-empty",mixed_startup_unit_storage,37,b"UNIT-EMPTY"),
    ("exact-startup-unit-mixed-storage-live",mixed_startup_unit_storage.replace("status == 38","status == 37"),55,b"UNIT-AFTER"),
])
startup_condition_producer = f"""%%start
    bool(bool) gate = fn(value) {{
        core.require(false, {report(47,"CALL")});
        value
    }};
    bool input = true;
    if (gate(input)) {{
        core.require(false, {report(51,"THEN")});
    }} else {{
        core.require(false, {report(53,"ELSE")});
    }};
    core.require(false, {report(55,"AFTER")});
    %%end"""
startup_condition_empty = startup_condition_producer.replace("bool input = true","bool input = false").replace(
    "core.require(false, " + report(47,"CALL") + ")",
    "core.narrow(value, " + state_report(49,"EMPTY","value") + ")")
cases.extend([
    ("exact-startup-bool-condition-call-then",startup_condition_producer,47,b"CALLTHENAFTER"),
    ("exact-startup-bool-condition-call-else",startup_condition_producer.replace("bool input = true","bool input = false"),47,b"CALLELSEAFTER"),
    ("exact-startup-bool-condition-call-empty",startup_condition_empty,49,b"EMPTY"),
])
startup_bool_binding = f"""%%start
    i8(i8) identity = fn(value) {{ value }};
    bool(bool) truth = fn(value) {{ value }};
    i8 status = identity(37);
    bool ready = truth(true);
    bool eligible = (ready || truth(ready)) && status == 37;
    bool saved = eligible;
    bool answer = truth(saved);
    core.narrow(!answer, {{
        .stdout = ""; .stderr = "STARTUP-BOOL-BINDING"; .code = status;
        .files = [{{ .path = "answer.json"; .state = answer; }}];
    }});
    core.require(false, {report(9,"UNREACHABLE")});
    %%end"""
cases.extend([
    ("exact-startup-computed-bool-binding-true",startup_bool_binding,37,b"STARTUP-BOOL-BINDING"),
    ("exact-startup-computed-bool-binding-false",startup_bool_binding.replace("status == 37","status == 38").replace("narrow(!answer","narrow(answer"),37,b"STARTUP-BOOL-BINDING"),
    ("exact-startup-computed-bool-binding-live-unsupported",startup_bool_binding.replace("(ready || truth","(!ready || truth"),2,b"unsupported analysis"),
])
startup_logic = f"""%%start
    i8(i8) identity = fn(value) {{ value }};
    bool(bool) truth = fn(value) {{ value }};
    i8 status = identity(37);
    bool ready = truth(true);
    if (ready && !(status != 37) && (ready || truth(ready))) {{
        core.require(!ready || status == 37, {report(11,"WRONG-REQUIRE")});
        core.narrow(!ready && truth(ready), {{
            .stdout = ""; .stderr = "STARTUP-LOGIC"; .code = status;
            .files = [{{ .path = "startup-logic.json"; .state = ready; }}];
        }});
    }} else {{
        core.require(false, {report(13,"WRONG-BRANCH")});
    }};
    core.require(false, {report(17,"UNREACHABLE")});
    %%end"""
cases.extend([
    ("exact-startup-composed-logic",startup_logic,37,b"STARTUP-LOGIC"),
    ("exact-startup-composed-logic-live-unsupported",startup_logic.replace("(ready || truth(ready)","(!ready || truth(ready)"),2,b"unsupported analysis"),
])
lexical_code_report = state_report(0,"LEXICAL-CODE","status").replace(".code = 0",".code = status")
lexical_code = f"""%%start
    unit(i8) check = fn(status) {{
        core.require(status != 37, {lexical_code_report});
    }};
    %%end"""
lexical_narrow_code = f"""%%start
    unit(i8) check = fn(status) {{
        core.narrow(status >= 45, {lexical_code_report});
    }};
    check(43);
    %%end"""
lexical_code_non_singleton = lexical_code.replace(
    f"core.require(status != 37, {lexical_code_report});",
    "core.require(false, " + report(44,"LEXICAL-CODE-PRIOR") + "); "
    + f"core.require(false, {lexical_code_report});",
)
cases.extend([
    ("authored-lexical-code-resolves-failure-witness",lexical_code,37,b"LEXICAL-CODE"),
    ("exact-startup-lexical-code-resolves-failed-narrow",lexical_narrow_code,43,b"LEXICAL-CODE"),
    ("authored-lexical-code-nonsingleton-fails-atomically",lexical_code_non_singleton,2,b"unsupported analysis"),
    ("authored-lexical-code-success-does-not-resolve",lexical_code.replace("status != 37","status <= 127"),0,b""),
])
state_fixed_array = f"""%%start
    unit(i8) check = fn(value) {{
        core.narrow(value == 1 || value == 3, {report(167,"HOLES")});
        i8[2] snapshot = [value, 7];
        value = 9;
        core.require(value != 9, {state_report(217,"FIXED-ARRAY","snapshot",', { .path = "copy.json"; .state = snapshot; }')});
    }};
    %%end"""
state_nested_array = f"""%%start
    struct Snapshot {{
        i8[2][2] matrix;
        bool[2] flags;
    }}
    unit(i8) check = fn(value) {{
        Snapshot snapshot = {{ .flags = [true, false]; .matrix = [[1, value], [3, 4]]; }};
        core.require(value != 2, {state_report(218,"NESTED-ARRAY","snapshot")});
    }};
    %%end"""
state_array_too_wide = f"""%%start
    unit() check = fn {{
        i8[65] snapshot = [{', '.join(str(item) for item in range(65))}];
        core.require(false, {state_report(219,"WIDE-ARRAY","snapshot")});
    }};
    %%end"""
cases.extend([
    ("authored-state-fixed-array-precision",state_fixed_array,217,b"FIXED-ARRAY"),
    ("authored-state-array-inside-nominal",state_nested_array,218,b"NESTED-ARRAY"),
    ("authored-state-array-unsupported-element",state_fixed_array.replace("[value, 7]","[value * 2, 7]"),2,b"unsupported analysis"),
    ("authored-state-array-shape-cap",state_array_too_wide,2,b"unsupported analysis"),
])
state_nominal = f"""%%start
    struct Snapshot {{
        i8 saved;
        bool ready;
    }}
    unit(i8) check = fn(value) {{
        core.narrow(value == 1 || value == 3, {report(167,"HOLES")});
        Snapshot snapshot = {{ .ready = true; .saved = value; }};
        value = 7;
        core.require(value != 7, {state_report(170,"SNAPSHOT","snapshot",', { .path = "copy.json"; .state = snapshot; }')});
    }};
    %%end"""
def copied_state_program(declarations, ty, initializer, extra="", second=True):
    report_copy = state_report(57,"COPIED-STATE","copy_two",
        ', { .path = "copy.json"; .state = copy_one; }' if second else "")
    return f"""%%start
    {declarations}
    unit(i8) check = fn(value) {{
        core.narrow(value == 1 || value == 3, {report(167,"HOLES")});
        {ty} original = {initializer};
        {ty} copy_one = original;
        {extra}
        value = 7;
        {ty} copy_two = copy_one;
        core.require(value != 7, {report_copy});
    }};
    %%end"""
nominal_copy = copied_state_program("struct copy Snapshot { i8 saved; }","Snapshot","{ .saved = value; }")
fixed_array_copy = copied_state_program("","i8[2]","[value, 4]")
nominal_move = copied_state_program("struct Snapshot { i8 saved; }","Snapshot","{ .saved = value; }",second=False)
nominal_mutated_copy = copied_state_program("struct copy Snapshot { i8 saved; }","Snapshot","{ .saved = value; }","copy_one.saved = 9;")
cases.extend([
    ("authored-state-nominal-copy-chain",nominal_copy,57,b"COPIED-STATE"),
    ("authored-state-array-copy-chain",fixed_array_copy,57,b"COPIED-STATE"),
    ("authored-state-nominal-move-not-copy",nominal_move,2,b"unsupported analysis"),
    ("authored-state-nominal-copy-mutation-unsupported",nominal_mutated_copy,2,b"unsupported analysis"),
])
affine_snapshot = f"""%%start
    struct Snapshot {{ i8 raised; i8 reflected; i8 plain; }}
    unit(i8) inspect = fn(value) {{
        core.narrow(value == 1 || value == 3, {report(1,"PRUNE")});
        Snapshot snapshot = {{ .raised = value + 2; .reflected = 7 - value; .plain = value; }};
        if (value == 1) {{
            i8 read = snapshot.raised;
            core.require(read == 3, {report(58,"LOST-PREIMAGE")});
        }};
        value = 7;
        core.require(value != 7, {state_report(59,"AFFINE-SNAPSHOT","snapshot")});
    }};
    %%end"""
affine_snapshot_actual = affine_snapshot.replace("%%end","inspect(1); %%end")
affine_snapshot_overflow = affine_snapshot.replace("value == 1 || value == 3","value >= 125").replace("value + 2","value + 3")
affine_array_snapshot = state_fixed_array.replace("[value, 7]","[value + 1, 7]")
cases.extend([
    ("authored-state-affine-nominal-construction",affine_snapshot,59,b"AFFINE-SNAPSHOT"),
    ("exact-startup-authored-state-affine-nominal-construction",affine_snapshot_actual,59,b"AFFINE-SNAPSHOT"),
    ("authored-state-affine-nominal-overflow",affine_snapshot_overflow,2,b"unsupported analysis"),
    ("authored-state-affine-array-construction",affine_array_snapshot,217,b"FIXED-ARRAY"),
])
ordinary_array_read = f"""%%start
    unit(u64, i8) inspect = fn(index, value) {{
        core.narrow(value == 1 || value == 3, {report(1,"PRUNE")});
        i8[2] original = [value, 9];
        i8[2] copied = original;
        value = 7;
        if (index == 0) {{
            i8 observed = copied[index];
            i8 fixed = copied[1];
            core.require(fixed == 9, {report(59,"WRONG-LITERAL-INDEX")});
            if (observed == 3) {{
                core.require(index == 0, {report(60,"WRONG-PREIMAGE")});
                core.require(value != 7, {state_report(61,"SELECTED-ELEMENT","observed")});
            }} else {{
                core.require(observed == 1, {report(63,"LOST-CORRELATION")});
            }};
        }};
    }};
    %%end"""
ordinary_array_read_actual = ordinary_array_read.replace("%%end","inspect(0, 3); %%end")
ordinary_array_alternatives = ordinary_array_read.replace("if (index == 0)","if (index == 0 || index == 2)").replace(
    "i8[2]","i8[3]").replace("[value, 9]","[value, 13, 9]").replace("copied[1]","copied[2]").replace(
    "observed == 1,","observed == 1 || (index == 2 && observed == 9),")
cases.extend([
    ("ordinary-array-read-control",ordinary_array_read,61,b"SELECTED-ELEMENT"),
    ("exact-startup-ordinary-array-read-control",ordinary_array_read_actual,61,b"SELECTED-ELEMENT"),
    ("ordinary-array-read-partitions-holed-indices",ordinary_array_alternatives,61,b"SELECTED-ELEMENT"),
    ("ordinary-array-read-refuses-mixed-invalid-indices",
        ordinary_array_alternatives.replace("index == 2)","index == 3)"),2,b"unsupported analysis"),
    ("exact-startup-ordinary-array-read-rejects-out-of-bounds",ordinary_array_read_actual.replace(
        "if (index == 0)","if (index == 2)").replace("inspect(0, 3)","inspect(2, 3)"),2,b"unsupported analysis"),
])
ordinary_nested_array = f"""%%start
    unit(u64, i8) inspect = fn(index, value) {{
        core.narrow(value == 1 || value == 3, {report(1,"PRUNE")});
        i8[2][2] matrix = [[value, 29], [49, 7]];
        i8[2][2] copied = matrix;
        value = 9;
        if (index <= 1) {{
            i8 observed = copied[index][index];
            i8 constant = copied[0][1];
            core.require(constant == 29, {report(58,"WRONG-CONSTANT")});
            if (observed == 3) {{
                core.require(index == 0, {report(60,"WRONG-DIAGONAL-INDEX")});
                core.require(value != 9, {state_report(61,"SELECTED-DIAGONAL","observed")});
            }} else {{
                core.require(observed == 1 || (index == 1 && observed == 7), {report(63,"OFF-DIAGONAL")});
            }};
        }};
    }};
    %%end"""
cases.extend([
    ("ordinary-nested-array-exact-diagonal",ordinary_nested_array,61,b"SELECTED-DIAGONAL"),
    ("exact-startup-ordinary-nested-array-exact-diagonal",ordinary_nested_array.replace("%%end","inspect(0, 3); %%end"),
        61,b"SELECTED-DIAGONAL"),
    ("ordinary-nested-array-inner-out-of-bounds",ordinary_nested_array.replace("index <= 1","index <= 2").replace(
        "copied[index][index]","copied[0][index]"),2,b"unsupported analysis"),
])
ordinary_nominal_read = f"""%%start
    struct copy Snapshot {{ i8 saved; bool enabled; }}
    unit(i8) inspect = fn(value) {{
        core.narrow(value == 1 || value == 3, {report(1,"PRUNE")});
        Snapshot original = {{ .saved = value; .enabled = true; }};
        Snapshot copied = original;
        if (value == 3) {{
            i8 earlier = copied.saved;
            core.require(earlier == 3, {report(59,"LOST-RELATION")});
        }};
        value = 7;
        i8 observed = copied.saved;
        bool enabled = copied.enabled;
        if (enabled && observed == 3) {{
            core.require(value != 7, {state_report(61,"SELECTED-MEMBER","observed")});
        }} else {{
            core.require(observed == 1, {report(63,"WRONG-LANE")});
        }};
    }};
    %%end"""
ordinary_nominal_actual = ordinary_nominal_read.replace("%%end","inspect(3); %%end")
cases.extend([
    ("ordinary-nominal-member-read-control",ordinary_nominal_read,61,b"SELECTED-MEMBER"),
    ("exact-startup-ordinary-nominal-member-read-control",ordinary_nominal_actual,61,b"SELECTED-MEMBER"),
    ("exact-startup-ordinary-nominal-member-rejects-write",ordinary_nominal_actual.replace(
        "value = 7;","copied.saved = 2; value = 7;"),2,b"unsupported analysis"),
])
authored_array_projection = f"""%%start
    unit(i8) check = fn(value) {{
        core.narrow(value == 1 || value == 3, {report(167,"HOLES")});
        i8[3] original = [value, 57, 99];
        i8[3] copied = original;
        value = 7;
        core.require(value != 7, {{ .stdout = ""; .stderr = "ARRAY-PROJECTION"; .code = copied[1];
            .files = [{{ .path = "selected.json"; .state = copied[0]; }}]; }});
    }};
    %%end"""
authored_projection_code_only = """%%start
    struct Snapshot { i8 status; }
    unit(i8) check = fn(value) {
        Snapshot saved = { .status = value; };
        core.require(value != 7, { .stdout = ""; .stderr = "PROJECTED-CODE"; .code = saved.status; .files = []; });
    };
    %%end"""
cases.extend([
    ("authored-state-array-projection-code-and-file",authored_array_projection,57,b"ARRAY-PROJECTION"),
    ("authored-state-array-projection-nonsingleton-code",authored_array_projection.replace(".code = copied[1]",".code = copied[0]"),2,b"unsupported analysis"),
    ("authored-state-nominal-projection-code-only",authored_projection_code_only,7,b"PROJECTED-CODE"),
])
authored_nominal_field = f"""%%start
    struct copy Snapshot {{ i8 saved; bool ready; i8 omitted; }}
    unit(i8) check = fn(value) {{
        core.narrow(value == 1 || value == 3, {report(167,"HOLES")});
        Snapshot original = {{ .omitted = 44; .ready = true; .saved = value; }};
        Snapshot copied = original;
        value = 7;
        core.require(value != 7, {{ .stdout = ""; .stderr = "FIELD-ONLY"; .code = 58; .files = [
            {{ .path = "saved.json"; .state = copied.saved; }},
            {{ .path = "inverted.json"; .state = !copied.ready; }},
            {{ .path = "same.json"; .state = copied.saved; }}
        ]; }});
    }};
    %%end"""
cases.append(("authored-state-nominal-field-only",authored_nominal_field,58,b"FIELD-ONLY"))
state_nominal_nested_scope = f"""%%start
    struct Snapshot {{ i8 saved; }}
    unit(i8) check = fn(value) {{
        Snapshot snapshot = {{ .saved = value; }};
        if (value == 1) {{
            i8 inner_value = 7;
            Snapshot inner_snapshot = {{ .saved = inner_value; }};
            core.require(inner_value != 7, {state_report(171,"NESTED-SNAPSHOT","snapshot",', { .path = "inner.json"; .state = inner_snapshot; }')});
        }};
    }};
    %%end"""
state_nominal_recursive = f"""%%start
    struct Child {{ i8 saved; bool ready; }}
    struct Snapshot {{ Child earlier; Child later; }}
    unit(i8) check = fn(value) {{
        core.narrow(value == 1 || value == 3, {report(167,"HOLES")});
        Snapshot snapshot = {{
            .later = {{ .ready = false; .saved = 7; }};
            .earlier = {{ .ready = true; .saved = value; }};
        }};
        value = 9;
        core.require(value != 9, {state_report(172,"NESTED-NOMINAL","snapshot",', { .path = "copy.json"; .state = snapshot; }')});
    }};
    %%end"""
depth_declarations = "struct Level0 { i8 saved; }\n"
depth_value = "{ .saved = 7; }"
for level in range(1, 9):
    depth_declarations += f"struct Level{level} {{ Level{level - 1} child; }}\n"
    depth_value = "{ .child = " + depth_value + "; }"
state_nominal_depth_limit = f"""%%start
    {depth_declarations}
    unit() check = fn {{
        Level8 snapshot = {depth_value};
        core.require(false, {state_report(173,"TOO-DEEP","snapshot")});
    }};
    %%end"""
state_nominal_visible_alias = state_nominal.replace(
    'value = 7;', 'i8 ordinary = value; value = 7;',
).replace('core.require(value != 7,', 'core.require(ordinary != 1,')
state_nominal_unsupported_operand = state_nominal.replace('.saved = value;', '.saved = value * 2;')
state_repeated = state_range.replace(
    '.path = "note.txt"; .text = "authored"; }',
    '.path = "note.txt"; .text = "authored"; }, { .path = "state-copy.json"; .state = value; }',
)
state_atomic_report = state_report(
    169, "MULTI-FILE", "stable",
    ', { .path = "note.txt"; .text = "never published"; }, { .path = "later.json"; .state = value; }',
)
state_atomic = f"""%%start
    unit(i8) check = fn(value) {{
        core.narrow(value == 1 || value == 3, {report(167,"HOLES")});
        i8 stable = 2;
        core.require(stable != 2, {state_atomic_report});
    }};
    %%end"""
cases.extend([
    ("authored-state-nominal-construction-snapshot",state_nominal,170,b"SNAPSHOT"),
    ("authored-state-nominal-recursive-literals",state_nominal_recursive,172,b"NESTED-NOMINAL"),
    ("authored-state-nominal-depth-fails-closed",state_nominal_depth_limit,2,b"unsupported analysis"),
    ("authored-state-nominal-nested-scope",state_nominal_nested_scope,171,b"NESTED-SNAPSHOT"),
    ("authored-state-nominal-mixed-visible-alias-still-unsupported",state_nominal_visible_alias,2,b"unsupported analysis"),
    ("authored-state-nominal-unsupported-operand",state_nominal_unsupported_operand,2,b"unsupported analysis"),
    ("authored-state-repeated-binding",state_repeated,162,b"AUTHORED-RANGE"),
    ("authored-state-multi-file",state_atomic,169,b"MULTI-FILE"),
    ("authored-state-range-and-text",state_range,162,b"AUTHORED-RANGE"),
    ("exact-startup-authored-state-bool",state_bool,163,b"AUTHORED-BOOL"),
    ("exact-startup-authored-state-failed-narrow-prestate",state_narrow,164,b"PRESTATE"),
    ("authored-state-i128-exact",state_i128,165,b"I128"),
    ("authored-state-active-context",state_contextual,166,b"CONTEXT-STATE"),
    ("authored-state-mixed",state_holes.replace("value == 1 || value == 3","value == 1 || value >= 3"),168,b"ALTERNATIVES"),
    ("authored-state-full-bool",state_full_bool,163,b"AUTHORED-BOOL"),
    ("authored-state-holes",state_holes,168,b"ALTERNATIVES"),
    ("authored-state-accepted-condition-writes-nothing",state_range.replace("value <= 3","value <= 127"),0,b""),
])
holes_bytes = b'{"type":{"kind":"i8"},"value":{"kind":"alternatives","values":[1,3]}}\n'
def typed_state(kind, value):
    return {"type": {"kind": kind}, "value": value}
def array_state(element_type, elements):
    return {"type": {"kind": "array", "length": len(elements), "element": element_type}, "value": elements}
def state_bytes(value):
    import json
    return (json.dumps(value, separators=(",", ":")) + "\n").encode()
fixed_array_bytes = state_bytes(array_state({"kind": "i8"}, [
    typed_state("i8", {"kind": "alternatives", "values": [1, 3]}), typed_state("i8", 7),
]))
snapshot_rows = [
    array_state({"kind": "i8"}, [typed_state("i8", first), typed_state("i8", second)])
    for first, second in [(1, 2), (3, 4)]
]
nested_array_bytes = state_bytes({
    "type": {"kind": "struct", "name": "Snapshot"},
    "value": {
        "matrix": array_state(snapshot_rows[0]["type"], snapshot_rows),
        "flags": array_state({"kind": "bool"}, [typed_state("bool", True), typed_state("bool", False)]),
    },
})
copied_alternatives = typed_state("i8", {"kind": "alternatives", "values": [1,3]})
copied_nominal_bytes = state_bytes({
    "type": {"kind": "struct", "name": "Snapshot"}, "value": {"saved": copied_alternatives}
})
copied_array_bytes = state_bytes(array_state({"kind":"i8"}, [copied_alternatives, typed_state("i8", 4)]))
def affine_nominal_bytes(raised, reflected, plain):
    return state_bytes({"type": {"kind": "struct", "name": "Snapshot"}, "value": {
        "raised": typed_state("i8", raised), "reflected": typed_state("i8", reflected),
        "plain": typed_state("i8", plain),
    }})
def alternatives(*values):
    return {"kind": "alternatives", "values": list(values)}
affine_array_bytes = state_bytes(array_state({"kind": "i8"}, [
    typed_state("i8", alternatives(2,4)), typed_state("i8", 7),
]))
authored_files = {
    **{name: {"old.json": state_bytes(typed_state("u8", 3)), "new.json": state_bytes(typed_state("u8", 2))}
       for name in ("ordinary-contextual-counter-debit", "exact-startup-contextual-counter-debit")},
    "authored-state-affine-nominal-construction": {
        "state.json": affine_nominal_bytes(alternatives(3,5), alternatives(4,6), alternatives(1,3)),
    },
    "exact-startup-authored-state-affine-nominal-construction": {"state.json": affine_nominal_bytes(3,6,1)},
    "authored-state-affine-array-construction": {"state.json": affine_array_bytes, "copy.json": affine_array_bytes},
    **{name: {"state.json": state_bytes(typed_state("i8", 3))} for name in (
        "ordinary-nominal-member-read-control", "exact-startup-ordinary-nominal-member-read-control",
        "ordinary-array-read-control", "exact-startup-ordinary-array-read-control",
        "ordinary-array-read-partitions-holed-indices",
        "ordinary-nested-array-exact-diagonal", "exact-startup-ordinary-nested-array-exact-diagonal",
    )},
    "authored-state-array-projection-code-and-file": {"selected.json": state_bytes(copied_alternatives)},
    "authored-state-nominal-field-only": {
        "saved.json": state_bytes(copied_alternatives),
        "inverted.json": state_bytes(typed_state("bool", False)),
        "same.json": state_bytes(copied_alternatives),
    },
    "authored-state-nominal-copy-chain": {"state.json": copied_nominal_bytes, "copy.json": copied_nominal_bytes},
    "authored-state-array-copy-chain": {"state.json": copied_array_bytes, "copy.json": copied_array_bytes},
    "authored-unary-state-preserves-holes": {
        "before.json": b'{"type":{"kind":"i8"},"value":{"kind":"alternatives","values":[-7,2]}}\n',
        "reflected.json": b'{"type":{"kind":"i8"},"value":{"kind":"alternatives","values":[-2,7]}}\n',
    },
    "authored-unary-code-is-failure-singleton": {
        "before.json": b'{"type":{"kind":"i8"},"value":-7}\n',
        "reflected.json": b'{"type":{"kind":"i8"},"value":7}\n',
    },
    "exact-startup-authored-unary-state-and-code": {
        "code.json": b'{"type":{"kind":"i8"},"value":37}\n',
        "inverted.json": b'{"type":{"kind":"bool"},"value":false}\n',
    },
    namespace_bool_key: {
        "chosen.json": b'{"type":{"kind":"bool"},"value":true}\n',
        "rejected.json": b'{"type":{"kind":"bool"},"value":false}\n',
    },
    "exact-startup-conditional-scalar-binding": {"status.json": b'{"type":{"kind":"i8"},"value":37}\n'},
    "exact-startup-unit-mixed-storage-empty": {"gate.json": b'{"type":{"kind":"bool"},"value":false}\n'},
    "exact-startup-bool-condition-call-empty": {"state.json": b'{"type":{"kind":"bool"},"value":false}\n'},
    "exact-startup-computed-bool-binding-true": {"answer.json": b'{"type":{"kind":"bool"},"value":true}\n'},
    "exact-startup-computed-bool-binding-false": {"answer.json": b'{"type":{"kind":"bool"},"value":false}\n'},
    "exact-startup-composed-logic": {"startup-logic.json": b'{"type":{"kind":"bool"},"value":true}\n'},
    **{name: {
        "status.json": b'{"type":{"kind":"i8"},"value":37}\n',
        "status-copy.json": b'{"type":{"kind":"i8"},"value":37}\n',
        "ready.json": b'{"type":{"kind":"bool"},"value":true}\n',
        "note.txt": b"selected at startup",
    } for name in ("exact-startup-module-typed-code-state", "exact-startup-module-typed-narrow-prestate")},
    **{name: {"state.json": b'{"type":{"kind":"bool"},"value":false}\n'} for name in (
        "exact-startup-lexical-bool-narrow-empties-initializer",
        "exact-startup-lexical-bool-narrow-empties-bound-local",
        "exact-startup-lexical-bool-narrow-empties-bound-chain",
    )},
    "exact-startup-bool-integer-narrow-prunes-initializer": {"state.json": b'{"type":{"kind":"i8"},"value":3}\n'},
    **{name: {"state.json": b'{"type":{"kind":"i8"},"value":3}\n'} for name in (
        "exact-startup-bool-return-initializes-false-module-value",
        "exact-startup-bool-return-initializes-true-module-value",
    )},
    "inline-finite-multiplication-refines-integer-root": {"state.json": b'{"type":{"kind":"i8"},"value":2}\n'},
    "finite-ordinary-multiplication-has-exact-stride": {
        "state.json": b'{"type":{"kind":"i8"},"value":{"kind":"alternatives","values":[0,3,6,9]}}\n',
        "preimage.json": b'{"type":{"kind":"i8"},"value":2}\n',
    },
    "exact-startup-finite-ordinary-signed-quotient-remainder": {
        "state.json": b'{"type":{"kind":"i8"},"value":-1}\n',
    },
    "conditional-scalar-binding-symbolic-preimages": {
        "state.json": b'{"type":{"kind":"i8"},"value":0}\n',
        "constant.json": b'{"type":{"kind":"i8"},"value":{"kind":"range","minimum":-128,"maximum":-1}}\n',
    },
    "exact-startup-conditional-bool-binding": {"state.json": b'{"type":{"kind":"bool"},"value":false}\n'},
    "conditional-scalar-binding-skips-unreached-operand": {
        "state.json": b'{"type":{"kind":"i8"},"value":{"kind":"range","minimum":0,"maximum":127}}\n',
    },
    **{name: {"state.json": b'{"type":{"kind":"i8"},"value":3}\n'} for name in (
        "exact-startup-real-integer-tail-transfers-result", "exact-startup-real-integer-tail-propagates-empty",
    )},
    "authored-lexical-code-resolves-failure-witness": {"state.json": b'{"type":{"kind":"i8"},"value":37}\n'},
    "exact-startup-lexical-code-resolves-failed-narrow": {"state.json": b'{"type":{"kind":"i8"},"value":43}\n'},
    **{key: {
        "inner.json": b'{"type":{"kind":"bool"},"value":false}\n',
        "outer.json": b'{"type":{"kind":"bool"},"value":true}\n',
    } for key in ("exact-startup-contextual-bool-restores-outer",
                   "symbolic-contextual-bool-restores-outer",
                   "exact-startup-contextual-bool-restores-symbolic-shape")},
    "authored-state-fixed-array-precision": {"state.json": fixed_array_bytes, "copy.json": fixed_array_bytes},
    "authored-state-array-inside-nominal": {"state.json": nested_array_bytes},
    namespace_scalar_key: {
        "call.json": b'{"type":{"kind":"i8"},"value":7}\n',
        "state.json": b'{"type":{"kind":"i8"},"value":1}\n',
    },
    "symbolic-integer-root-tail-projects-conditional-callee": {
        "state.json": b'{"type":{"kind":"i8"},"value":{"kind":"range","minimum":-128,"maximum":-1}}\n',
    },
    "integer-symbolic-root-call-preimages": {
        "state.json": b'{"type":{"kind":"i8"},"value":{"kind":"range","minimum":-128,"maximum":-1}}\n',
    },
    "projected-conditional-local-write-keeps-input-pieces": {
        "state.json": b'{"type":{"kind":"i8"},"value":0}\n',
        "preimage.json": b'{"type":{"kind":"i8"},"value":{"kind":"range","minimum":-128,"maximum":-1}}\n',
    },
    "projected-conditional-local-write-skips-unreached-operand": {
        "state.json": b'{"type":{"kind":"i8"},"value":{"kind":"range","minimum":0,"maximum":127}}\n',
    },
    "projected-returning-local-copy-identity": {
        "state.json": b'{"type":{"kind":"i8"},"value":3}\n',
        "result.json": b'{"type":{"kind":"i8"},"value":3}\n',
    },
    "exact-startup-conditional-local-write-ignores-unreached-arm": {
        "state.json": b'{"type":{"kind":"i8"},"value":2}\n',
    },
    "projected-narrowed-integer-return-prunes-parent": {
        "state.json": b'{"type":{"kind":"i8"},"value":{"kind":"range","minimum":2,"maximum":127}}\n',
    },
    "projected-empty-integer-return-keeps-sibling": {
        "state.json": b'{"type":{"kind":"i8"},"value":{"kind":"range","minimum":1,"maximum":127}}\n',
        "branch.json": b'{"type":{"kind":"i8"},"value":{"kind":"range","minimum":-128,"maximum":0}}\n',
    },
    **{name: {
        "state.json": b'{"type":{"kind":"i8"},"value":2}\n',
        "current.json": b'{"type":{"kind":"i8"},"value":7}\n',
    } for name in ("exact-startup-integer-return-direct-local-write", "exact-startup-integer-return-nested-local-write")},
    "exact-startup-scalar-narrow-leaf-empties-init": {
        "state.json": b'{"type":{"kind":"i8"},"value":1}\n',
    },
    "exact-startup-scalar-narrow-prunes-return-chain": {
        "state.json": b'{"type":{"kind":"i8"},"value":1}\n',
    },
    "exact-startup-scalar-narrow-surviving-return": {
        "state.json": b'{"type":{"kind":"i8"},"value":3}\n',
    },
    "exact-startup-ranked-integer-return": {
        "state.json": b'{"type":{"kind":"u8"},"value":7}\n',
    },
    "parallel-whole-local-mixed-swap": {
        "state.json": b'{"type":{"kind":"bool"},"value":false}\n',
        "first.json": b'{"type":{"kind":"i8"},"value":8}\n',
    },
    "parallel-whole-local-mixed-literals": {
        "state.json": b'{"type":{"kind":"bool"},"value":false}\n',
    },
    "authored-state-holes": {"state.json": holes_bytes},
    "authored-state-full-bool": {"state.json": b'{"type":{"kind":"bool"},"value":{"kind":"alternatives","values":[false,true]}}\n'},
    "authored-state-mixed": {"state.json": b'{"type":{"kind":"i8"},"value":{"kind":"alternatives","values":[1,{"kind":"range","minimum":3,"maximum":127}]}}\n'},
    "authored-state-multi-file": {
        "state.json": b'{"type":{"kind":"i8"},"value":2}\n',
        "note.txt": b"never published", "later.json": holes_bytes,
    },
    "authored-state-range-and-text": {
        "state.json": b'{"type":{"kind":"i8"},"value":{"kind":"range","minimum":4,"maximum":127}}\n',
        "note.txt": b"authored",
    },
    "exact-startup-authored-state-bool": {"state.json": b'{"type":{"kind":"bool"},"value":false}\n'},
    "exact-startup-authored-state-failed-narrow-prestate": {"state.json": b'{"type":{"kind":"i8"},"value":1}\n'},
    "authored-state-i128-exact": {
        "state.json": b'{"type":{"kind":"i128"},"value":170141183460469231731687303715884105727}\n',
    },
    "authored-state-active-context": {"state.json": b'{"type":{"kind":"i8"},"value":3}\n'},
}
nominal_bytes = (b'{"type":{"kind":"struct","name":"Snapshot"},"value":{"saved":{"type":{"kind":"i8"},'
    b'"value":{"kind":"alternatives","values":[1,3]}},"ready":{"type":{"kind":"bool"},"value":true}}}\n')
authored_files["authored-state-nominal-construction-snapshot"] = {
    "state.json": nominal_bytes, "copy.json": nominal_bytes,
}
recursive_nominal_bytes = (
    b'{"type":{"kind":"struct","name":"Snapshot"},"value":{'
    b'"earlier":{"type":{"kind":"struct","name":"Child"},"value":{'
    b'"saved":{"type":{"kind":"i8"},"value":{"kind":"alternatives","values":[1,3]}},'
    b'"ready":{"type":{"kind":"bool"},"value":true}}},'
    b'"later":{"type":{"kind":"struct","name":"Child"},"value":{'
    b'"saved":{"type":{"kind":"i8"},"value":7},'
    b'"ready":{"type":{"kind":"bool"},"value":false}}}}}\n'
)
authored_files["authored-state-nominal-recursive-literals"] = {
    "state.json": recursive_nominal_bytes, "copy.json": recursive_nominal_bytes,
}
authored_files["authored-state-nominal-nested-scope"] = {
    "state.json": b'{"type":{"kind":"struct","name":"Snapshot"},"value":{"saved":{"type":{"kind":"i8"},"value":1}}}\n',
    "inner.json": b'{"type":{"kind":"struct","name":"Snapshot"},"value":{"saved":{"type":{"kind":"i8"},"value":7}}}\n',
}
authored_files["authored-state-repeated-binding"] = {
    **authored_files["authored-state-range-and-text"],
    "state-copy.json": authored_files["authored-state-range-and-text"]["state.json"],
}
with tempfile.TemporaryDirectory(prefix="affine-calls-") as path:
    root=Path(path)
    (root/"src").mkdir()
    for target in ("native","wasm-wasip1"):
        (root/"wosy.toml").write_text(manifest.format(output=target))
        for scope in ("--main-flow-only","--declared-package"):
            for name,text,expected,stderr in cases:
                included = scope == "--declared-package" or name.startswith("exact-startup-")
                # Root scope skips unused valid citation declarations. Static
                # plan/shape refusal remains independent of authored reach.
                main_flow_citation_refusal = scope == "--main-flow-only" and name == "ordered-calling-citation-pair-arguments-swapped"
                code = 2 if main_flow_citation_refusal else (expected if included else 0)
                expected_stderr = b"unsupported analysis" if main_flow_citation_refusal else (stderr if included else b"")
                (root/"src/main.w").write_text(text.strip()+"\n")
                if name in namespace_children:
                    (root/"src/calc.w").write_text(namespace_children[name].strip()+"\n")
                res=subprocess.run([cli,"check",scope],cwd=root,capture_output=True,timeout=20)
                print(target,scope,name,res.returncode,res.stdout,res.stderr,flush=True)
                assert res.returncode==code,res
                assert not res.stdout,res
                if code==2:
                    assert expected_stderr in res.stderr,res
                    if name == "exact-startup-qualified-scalar-proof-effect-refused":
                        assert b"UNPUBLISHED-LEAF" not in res.stderr.splitlines(), res
                    if name == "exact-startup-real-integer-tail-rejects-reentry":
                        assert b"BEFORE-TAIL" not in res.stderr and b"AFTER-TAIL" not in res.stderr,res
                    if name == "authored-lexical-code-nonsingleton-fails-atomically":
                        assert b"LEXICAL-CODE" not in res.stderr,res
                    if name == "authored-state-array-projection-nonsingleton-code":
                        assert b"ARRAY-PROJECTION" not in res.stderr,res
                    if name == "parallel-formal-target-fails-closed":
                        assert b"FORMAL-NEVER-PUBLISHED" not in res.stderr,res
                    if name.startswith("exact-startup-ranked-integer-return-"):
                        assert b"BASE" not in res.stderr and b"FRAME" not in res.stderr,res
                    if name.startswith("exact-startup-integer-return-"):
                        assert b"NEVER-SCALAR-WRITE" not in res.stderr,res
                    if name == "exact-startup-conditional-local-write-rejects-reached-effect":
                        assert b"PRIOR" not in res.stderr and b"ARM-EFFECT" not in res.stderr,res
                    if name.startswith("exact-startup-finite-ordinary-"):
                        assert b"FINITE-PRIOR-UNPUBLISHED" not in res.stderr,res
                    if name == "exact-startup-contextual-counter-returning-refused":
                        assert b"RETURNING-COUNTER-WRITE" not in res.stderr and b"RETURNING-COUNTER-RESULT" not in res.stderr,res
                    if name in namespace_bool_context_children:
                        assert all(label not in res.stderr for label in (b"BOOL-INIT", b"BOOL-CHOOSE", b"BOOL-BRANCH")),res
                    if name == "conditional-scalar-binding-refuses-reached-operand":
                        assert b"BIND-PRIOR-UNPUBLISHED" not in res.stderr,res
                    if name == "projected-conditional-local-write-rejects-reached-operand":
                        assert b"PROJECTED-PRIOR-UNPUBLISHED" not in res.stderr,res
                    if name == "projected-returning-local-copy-refuses-formal":
                        assert b"COPY-PRIOR-UNPUBLISHED" not in res.stderr and b"COPY-FORMAL-UNPUBLISHED" not in res.stderr,res
                elif included and name in proof_diagnostic_notes:
                    assert expected_stderr in res.stderr and proof_diagnostic_notes[name] in res.stderr,res
                    if name in ("ordinary-authored-case-citation-ordered-diagnostics", "exact-startup-authored-case-citation-ordered-diagnostics"):
                        failures = res.stderr.split(b"error: proof obligation failed")[1:]
                        assert len(failures) == 3 and b"core.show(out == 8)" in failures[0], res
                        assert b"proof separate" in failures[1] and b"proof high" in failures[2], res
                    if name in ("ordinary-authored-case-citation-refuted-premise", "exact-startup-authored-case-citation-refuted-premise"):
                        assert res.stderr.count(b"proof obligation failed") == 1, res
                        assert b"proof high" in res.stderr and b"proof goal" not in res.stderr, res
                    if name in ("ordinary-authored-case-rewrite-refutes-premise", "exact-startup-authored-case-rewrite-refutes-premise"):
                        assert b"core.show(out == 10)" in res.stderr and b"proof goal" in res.stderr, res
                    if name in ("ordinary-authored-case-proof-refuted-show", "exact-startup-authored-case-proof-wrong-arm"):
                        assert b"core.show(out == x)" in res.stderr and b"proof goal" in res.stderr, res
                    if name == "exact-startup-authored-case-proof-nested-first-show":
                        primary = res.stderr.split(b"not satisfied by this call", 1)[0]
                        assert b"core.show(out < 10)" in primary, res
                    if name == "ordinary-authored-case-proof-refuted-goal":
                        assert b"ensures (out <= 9)" in res.stderr and b"proof goal" not in res.stderr, res
                    if name == "exact-startup-qualified-scalar-proof-refuted":
                        assert res.stderr.count(b"proof obligation failed") == 1, res
                        assert b"src/main.w" in res.stderr and b"imported.value(3)" in res.stderr, res
                    if name == "ordinary-qualified-scalar-proof-unused-refuted":
                        assert b"src/main.w" in res.stderr and b"src/calc.w" in res.stderr, res
                        assert b"symbolic call entry: x = 0" in res.stderr, res
                    if name == "exact-startup-rectangular-citation-refuted-premise":
                        assert res.stderr.count(b"proof obligation failed") == 1, res
                        assert b"proof issued" in res.stderr and b"producer(3,7)" in res.stderr, res
                    if name == "ordinary-verified-unary-entry-refuted":
                        assert b"symbolic call entry: value = 1" in res.stderr,res
                    if name == "exact-startup-fixed-array-exists-refuted":
                        assert res.stderr.count(b"error: proof obligation failed") == 2, res
                        assert res.stderr.count(b"applicable call") == 2 and res.stderr.count(b"call entry: value = 3") == 2, res
                        assert b"symbolic call" not in res.stderr, res
                    if name == "exact-startup-fixed-array-pair-refuted":
                        assert b"applicable call" in res.stderr and b"call entry: tag = 1, value = 3" in res.stderr, res
                    if name.startswith("ordinary-inferred-array-exists-"):
                        assert b"returned array" in res.stderr and b"no matching index" in res.stderr, res
                        assert b"symbolic call can return cell" not in res.stderr, res
                    if name == "ordinary-fixed-array-symbolic-cells-correlated":
                        assert b"symbolic call entry: left = 2, right = 4" in res.stderr,res
                    if name == "ordinary-fixed-array-symbolic-cells-copy-refuted":
                        assert b"symbolic call entry: value = 7" in res.stderr,res
                    if name == "ordinary-fixed-array-symbolic-cells-branch-refuted":
                        assert b"symbolic call entry: value = 5" in res.stderr,res
                    if name == "ordinary-unused-contextual-refuted":
                        assert b"symbolic contextual call" in res.stderr and b"call supplying this context" not in res.stderr,res
                    if name == "exact-startup-public-namespace-contextual-refuted":
                        assert b"src/main.w" in res.stderr and b"src/calc.w" in res.stderr,res
                        assert b"WRONG-NAMESPACE-CONTEXT-EXIT" not in res.stderr.splitlines(),res
                    if name == "exact-startup-public-contextual-proof-refuted":
                        assert b"PRIVATE-CONTEXT-OUTPUT" not in res.stderr.splitlines() and b"active contextual binding" in res.stderr,res
                    if name.startswith("ordinary-rectangular-authored-show-"):
                        primary, separator, secondary = res.stderr.partition(b"not satisfied by this symbolic call")
                        assert separator,res
                        if name.endswith("-goal-failure"):
                            assert b"ensures (out == 246)" in primary and b"proof goal" not in secondary,res
                        elif name.endswith("-first-step-failure"):
                            assert b"core.show(out == core.old(before) - 1);" in primary and b"proof goal" in secondary,res
                            assert b"core.show(out < 1);" not in primary,res
                else:
                    assert res.stderr==expected_stderr,res
                files = authored_files.get(name, {}) if included else {}
                check_outputs = Path(".wosy/outputs/check")
                expected_paths = {"src", "src/main.w", "wosy.toml"}
                if name in namespace_children:
                    expected_paths.add("src/calc.w")
                if files:
                    expected_paths.update(str(path) for path in (check_outputs, *check_outputs.parents) if path != Path("."))
                    expected_paths.update(str(check_outputs / filename) for filename in files)
                observed_paths = {str(p.relative_to(root)) for p in root.rglob("*")}
                assert observed_paths == expected_paths, {
                    "case": name, "missing": sorted(expected_paths - observed_paths),
                    "unexpected": sorted(observed_paths - expected_paths),
                }
                for filename, expected_bytes in files.items():
                    artifact = root / check_outputs / filename
                    assert artifact.read_bytes() == expected_bytes, artifact
                    artifact.unlink()
                if files:
                    for path in (check_outputs, *check_outputs.parents):
                        if path != Path("."):
                            (root / path).rmdir()
                if name in namespace_children:
                    (root/"src/calc.w").unlink()
