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
    ("second-affine-producing-call-remains-unsupported",make(f'i8 again = affine(value); core.require(again > 0, {report(41,"SECOND")});'),2,b"unsupported analysis"),
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
cases.append(("dependent-endpoints-not-independent",paired.replace("pick(value, side)", "pick(value, input)"),
              2,b"unsupported analysis"))
cases.append(("previously-unsupported-lexical-exact-assertion", make(f"""
    i8 fixed = echo(input);
    core.require(fixed == 2, {report(30,"EXACT")});
    if (value == 1) {{ core.require(input == 3, {report(32,"CORRELATION")}); }};
""", "i8(i8) echo = fn(n) { n };"), 30, b"EXACT"))
with tempfile.TemporaryDirectory(prefix="affine-calls-") as path:
    root=Path(path)
    (root/"src").mkdir()
    for target in ("native","wasm-wasip1"):
        (root/"wosy.toml").write_text(manifest.format(output=target))
        for scope in ("--main-flow-only","--declared-package"):
            for name,text,expected,stderr in cases:
                code = expected if scope == "--declared-package" else 0
                expected_stderr = stderr if scope == "--declared-package" else b""
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
