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
    ("inline-nonlinear-remains-unsupported",inline_return.replace("entry - 1","entry * 1"),2,b"unsupported analysis"),
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
with tempfile.TemporaryDirectory(prefix="affine-calls-") as path:
    root=Path(path)
    (root/"src").mkdir()
    for target in ("native","wasm-wasip1"):
        (root/"wosy.toml").write_text(manifest.format(output=target))
        for scope in ("--main-flow-only","--declared-package"):
            for name,text,expected,stderr in cases:
                included = scope == "--declared-package" or name in ("exact-startup-lexical-copy", "exact-startup-inline-arithmetic")
                code = expected if included else 0
                expected_stderr = stderr if included else b""
                (root/"src/main.w").write_text(text.strip()+"\n")
                res=subprocess.run([cli,"check",scope],cwd=root,capture_output=True,timeout=20)
                print(target,scope,name,res.returncode,res.stdout,res.stderr,flush=True)
                assert res.returncode==code,res
                assert not res.stdout,res
                if code==2:
                    assert expected_stderr in res.stderr,res
                else:
                    assert res.stderr==expected_stderr,res
                assert sorted(str(p.relative_to(root)) for p in root.rglob("*")) == ["src","src/main.w","wosy.toml"]
