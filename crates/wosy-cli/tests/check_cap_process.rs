#![cfg(all(target_os = "linux", feature = "internal-checker"))]

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_wosy");

fn wosy(root: &Path, args: &[&str]) -> Output {
    Command::new(BIN).current_dir(root).env("NO_COLOR", "1").args(args).output().unwrap()
}

#[test]
fn check_error_limit_policy_is_visible_and_exclusive() {
    let help = wosy(Path::new("."), &["check", "--help"]);
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("--max-errors") && help.contains("--all-errors") && help.contains("default: 1"));
    let conflict = wosy(Path::new("."), &["check", "--all-errors", "--max-errors", "1"]);
    assert_eq!(conflict.status.code(), Some(2));
    let diagnostic = String::from_utf8_lossy(&conflict.stderr);
    assert!(diagnostic.contains("--max-errors") && diagnostic.contains("--all-errors"));
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "wosy-check-cap-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        ));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }

    fn package(&self, relative: &str, layout: &str, profile: &str, extra: &str, source: &str) {
        let root = self.0.join(relative);
        let name = if relative.is_empty() { "app" } else { relative };
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.w"), source).unwrap();
        fs::write(root.join("wosy.toml"), format!(r#"
[package]
name = "{name}"
default_target = "app"
source_root = "src"
[comments]
categories = []
[builders]
[runners]
[targets.app]
root = "src/main.w"
default_profile = "{profile}"
[targets.app.artifacts.main]
artifact = "RuntimeStdio"
[targets.app.artifacts.main.profiles.{profile}]
backend = "llvm"
output = "{layout}"
builder = "never"
run_runner = "never"
[targets.app.profiles.{profile}]
main_artifact = "main"
{extra}
"#)).unwrap();
    }

    fn run(&self, flags: &[&str]) -> Output {
        let mut args = vec!["check"];
        args.extend_from_slice(flags);
        let result = wosy(&self.0, &args);
        assert!(!self.0.join(".wosy/artifacts").exists(), "Check must not build");
        result
    }

    fn output(&self) -> PathBuf { self.0.join(".wosy/outputs/check") }

    fn clear(&self) { fs::remove_dir_all(self.output()).unwrap(); }
}

impl Drop for Fixture {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn require(label: &str, code: u8, path: &str) -> String {
    format!(r#"core.require(false, {{ .stdout = "{label}"; .stderr = "[{label}]"; .code = {code}; .files = [{{ .path = "{path}"; .text = "{label}"; }}]; }});"#)
}

fn assert_terminal(output: &Output, code: i32, stdout: &[u8], stderr: &[u8]) {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    assert_eq!(output.stdout, stdout, "{output:?}");
    assert_eq!(output.stderr, stderr, "{output:?}");
}

#[test]
fn check_cap_process_publishes_only_selected_original_paths() {
    for layout in ["native", "wasm-wasip1"] {
        for scope in ["--main-flow-only", "--declared-package"] {
            let project = Fixture::new();
            let source = format!("%%start\n{}\n{}\n%%end", require("first", 0, "first.txt"), require("second", 19, "second.txt"));
            project.package("", layout, "dev", "", &source);

            for invalid in ["0", "99999999999999999999999999999999999999999"] {
                let output = project.run(&[scope, "--max-errors", invalid]);
                assert_eq!(output.status.code(), Some(2));
                assert!(output.stdout.is_empty() && !project.output().exists());
                assert!(String::from_utf8_lossy(&output.stderr).contains("invalid value"));
            }
            let incumbent = project.run(&[scope, "--all-errors"]);
            assert_terminal(&incumbent, 19, b"firstsecond", b"[first][second]");
            assert_eq!(fs::read(project.output().join("first.txt")).unwrap(), b"first");
            assert_eq!(fs::read(project.output().join("second.txt")).unwrap(), b"second");
            project.clear();

            let default = project.run(&[scope]);
            assert_terminal(&default, 0, b"first", b"[first]");
            assert_eq!(fs::read(project.output().join("first.txt")).unwrap(), b"first");
            assert!(!project.output().join("second.txt").exists());
            project.clear();

            let one = project.run(&[scope, "--max-errors", "1"]);
            assert_terminal(&one, 0, b"first", b"[first]");
            assert_eq!(fs::read(project.output().join("first.txt")).unwrap(), b"first");
            assert!(!project.output().join("second.txt").exists());
            project.clear();
            for cap in ["2", "3"] {
                assert_terminal(&project.run(&[scope, "--max-errors", cap]), 19, &incumbent.stdout, &incumbent.stderr);
                assert_eq!(fs::read(project.output().join("first.txt")).unwrap(), b"first");
                assert_eq!(fs::read(project.output().join("second.txt")).unwrap(), b"second");
                project.clear();
            }
        }
    }
}

#[test]
fn check_cap_process_preflights_later_packages_and_outputs_atomically() {
    for (layout, other) in [("native", "wasm-wasip1"), ("wasm-wasip1", "native")] {
        for scope in ["--main-flow-only", "--declared-package"] {
            let project = Fixture::new();
            project.package("", layout, "dev", "[dependencies.a]\npath = \"a\"\n[dependencies.b]\npath = \"b\"",
                "%%start core.require(true, { .stdout = \"\"; .stderr = \"\"; .code = 0; .files = []; }); %%end");
            project.package("a", other, "own", "", &format!("%%start {} %%end", require("a", 7, "same.txt")));
            project.package("b", layout, "own", "", &format!("%%start {} %%end", require("b", 8, "same.txt")));
            assert_terminal(&project.run(&[scope, "--max-errors", "2"]), 0, b"", b"");
            assert!(!project.output().exists());
            assert_terminal(&project.run(&[scope, "--dependencies", "--max-errors", "1"]), 7, b"a", b"[a]");
            assert_eq!(fs::read(project.output().join("same.txt")).unwrap(), b"a");
            project.clear();

            let conflict = project.run(&[scope, "--dependencies", "--max-errors", "2"]);
            assert_terminal(&conflict, 2, b"", b"wosy: check: result requires unsupported output; no result published\n");
            assert!(!project.output().exists());

            fs::write(project.0.join("b/src/main.w"), "%%start u8 bad = true; %%end").unwrap();
            let invalid = project.run(&[scope, "--dependencies", "--max-errors", "1"]);
            assert_eq!(invalid.status.code(), Some(2), "{invalid:?}");
            assert!(invalid.stdout.is_empty() && !project.output().exists());
            assert!(String::from_utf8_lossy(&invalid.stderr).contains("b::src/main.w"), "{invalid:?}");
        }
    }
}

#[test]
fn check_cap_process_keeps_late_formal_analysis_and_never_falls_back() {
    for layout in ["native", "wasm-wasip1"] {
        for scope in ["--main-flow-only", "--declared-package"] {
            let project = Fixture::new();
            let first = require("first", 0, "first.txt");
            let source = format!(r#"%%start
                unit(u8) consume = fn(charged) {{
                    if (charged == 1) {{ core.context.credit = core.context.credit - 1; }}
                    else {{ core.context.credit = core.context.credit - 1; }};
                }};
                unit() entry = fn {{
                    u8 core.context.credit = 3;
                    {first}
                    consume(1);
                }};
                proof debit for consume(cost) -> (null) where (cost <= 1)
                    ensures (core.context.credit == core.old(core.context.credit) - 1) {{}}
                entry();
            %%end"#);
            project.package("", layout, "dev", "", &source);
            assert_terminal(&project.run(&[scope, "--max-errors", "1"]), 0, b"first", b"[first]");
            assert_eq!(fs::read(project.output().join("first.txt")).unwrap(), b"first");
            project.clear();

            fs::write(project.0.join("src/main.w"), source.replace("credit = core.context.credit - 1", "credit = core.context.credit - 0")).unwrap();
            let refuted = project.run(&[scope, "--max-errors", "1"]);
            assert_eq!(refuted.status.code(), Some(1), "{refuted:?}");
            assert!(refuted.stdout.is_empty() && !project.output().exists());
            let diagnostic = String::from_utf8_lossy(&refuted.stderr);
            assert!(diagnostic.contains("proof obligation failed") && diagnostic.contains("explicit call entry: cost = 1")
                && diagnostic.contains("contextual value at entry: 3; at exit: 3") && !diagnostic.contains("[first]"), "{diagnostic}");

            // This legitimate Bool-returning initializer composes a call
            // result with OR, which the bounded startup corridor refuses.
            let unsupported = format!(r#"%%start
                bool(bool, i8) choose = fn(flag, entry) {{
                    core.require(entry != 3, {{ .stdout = "producer"; .stderr = ""; .code = 0; .files = []; }});
                    if (flag) {{ true }} else {{ false }}
                }};
                bool initialized = true;
                bool ready = choose(initialized, 3);
                i8 status = if (ready || false) {{
                    core.require(false, {{ .stdout = "yes"; .stderr = ""; .code = 5; .files = []; }}); 7
                }} else {{
                    core.require(false, {{ .stdout = "no"; .stderr = ""; .code = 6; .files = []; }}); 8
                }};
                core.require(status < 7, {{ .stdout = "after"; .stderr = ""; .code = status; .files = []; }});
            %%end"#);
            fs::write(project.0.join("src/main.w"), unsupported).unwrap();
            assert_terminal(&project.run(&[scope, "--max-errors", "8"]), 2, b"",
                b"wosy: check: unsupported analysis; no result published\n");
            assert!(!project.output().exists());
        }
    }
}

#[test]
fn check_cap_process_keeps_original_two_owner_contextual_proof_after_cutoff() {
    for layout in ["native", "wasm-wasip1"] {
        for scope in ["--main-flow-only", "--declared-package"] {
            let project = Fixture::new();
            let root = format!(
                r#"%%start
                    lib = namespace app "credit.w";
                    unit() entry = fn {{
                        u8 core.context.credit = 3;
                        {}
                        lib.relay();
                        {}
                    }};
                    entry();
                %%end"#,
                require("first", 0, "first.txt"),
                require("second", 76, "second.txt"),
            );
            let provider = r#"%%start
                unit() consume = fn { core.context.credit = core.context.credit - 1; };
                unit() relay = fn { consume(); };
                proof debit for consume() -> (null)
                    ensures (core.context.credit == core.old(core.context.credit) - 1) {}
            %%end"#;
            project.package("", layout, "dev", "", &root);
            let credit = project.0.join("src/credit.w");
            fs::write(&credit, provider).unwrap();
            let old = project.run(&[scope, "--all-errors"]);
            assert_terminal(&old, 76, b"firstsecond", b"[first][second]");
            let first = fs::read(project.output().join("first.txt")).unwrap();
            let second = fs::read(project.output().join("second.txt")).unwrap();
            project.clear();

            assert_terminal(&project.run(&[scope, "--max-errors", "1"]), 0, b"first", b"[first]");
            assert_eq!(fs::read(project.output().join("first.txt")).unwrap(), first);
            assert!(!project.output().join("second.txt").exists());
            project.clear();

            for cap in ["2", "3"] {
                assert_terminal(&project.run(&[scope, "--max-errors", cap]), 76, &old.stdout, &old.stderr);
                assert_eq!(fs::read(project.output().join("first.txt")).unwrap(), first);
                assert_eq!(fs::read(project.output().join("second.txt")).unwrap(), second);
                project.clear();
            }

            let wrong = provider.replace("credit = core.context.credit - 1", "credit = core.context.credit - 0");
            fs::write(&credit, wrong).unwrap();
            let old = project.run(&[scope, "--all-errors"]);
            let capped = project.run(&[scope, "--max-errors", "1"]);
            assert_terminal(&old, 1, b"", &capped.stderr);
            assert_terminal(&capped, 1, b"", &old.stderr);
            let diagnostic = String::from_utf8_lossy(&capped.stderr);
            assert!(diagnostic.contains("proof obligation failed") && diagnostic.contains("credit.w")
                && diagnostic.contains("lib.relay()") && diagnostic.contains("consume()"), "{diagnostic}");
            assert!(!project.output().exists());
        }
    }
}

#[test]
fn check_cap_process_finishes_independent_two_owner_contextual_dependency() {
    for (layout, other) in [("native", "wasm-wasip1"), ("wasm-wasip1", "native")] {
        for scope in ["--main-flow-only", "--declared-package"] {
            let project = Fixture::new();
            let root = format!("%%start {} %%end", require("app", 0, "app.txt"));
            project.package("", layout, "dev", "[dependencies.lib]\npath = \"lib\"", &root);
            let dependent = format!(
                r#"%%start
                    ledger = namespace lib "credit.w";
                    unit() entry = fn {{
                        u8 core.context.credit = 3;
                        {}
                        ledger.relay();
                        {}
                    }};
                    entry();
                %%end"#,
                require("one", 76, "one.txt"),
                require("two", 77, "two.txt"),
            );
            let provider = r#"%%start
                unit() consume = fn { core.context.credit = core.context.credit - 1; };
                unit() relay = fn { consume(); };
                proof debit for consume() -> (null)
                    ensures (core.context.credit == core.old(core.context.credit) - 1) {}
            %%end"#;
            project.package("lib", other, "own", "", &dependent);
            let credit = project.0.join("lib/src/credit.w");
            fs::write(&credit, provider).unwrap();
            let old = project.run(&[scope, "--dependencies", "--all-errors"]);
            assert_terminal(&old, 76, b"apponetwo", b"[app][one][two]");
            project.clear();
            for (cap, status, stdout, stderr, paths) in [
                ("1", 0, "app", "[app]", &["app"][..]),
                ("2", 76, "appone", "[app][one]", &["app", "one"][..]),
                ("4", 76, "apponetwo", "[app][one][two]", &["app", "one", "two"][..]),
            ] {
                let bounded = project.run(&[scope, "--dependencies", "--max-errors", cap]);
                assert_terminal(&bounded, status, stdout.as_bytes(), stderr.as_bytes());
                for name in ["app", "one", "two"] {
                    let file = project.output().join(format!("{name}.txt"));
                    if paths.contains(&name) {
                        assert_eq!(fs::read(file).unwrap(), name.as_bytes());
                    } else {
                        assert!(!file.exists(), "unselected output {name}");
                    }
                }
                assert!(!project.0.join("lib/.wosy/outputs/check").exists());
                project.clear();
            }

            // Proof refutation stays visible at zero dependency quota,
            // but must not erase the independent selected app path.
            let refuted_provider = provider.replace("credit = core.context.credit - 1", "credit = core.context.credit - 0");
            fs::write(&credit, refuted_provider).unwrap();
            let legacy_refuted = project.run(&[scope, "--dependencies", "--all-errors"]);
            assert_eq!(legacy_refuted.status.code(), Some(1), "{legacy_refuted:?}");
            assert_eq!(legacy_refuted.stdout, b"app");
            project.clear();
            for cap in ["1", "4"] {
                let bounded = project.run(&[scope, "--dependencies", "--max-errors", cap]);
                let expected_stderr = if cap == "1" { b"[app]".as_slice() } else { &legacy_refuted.stderr };
                assert_terminal(&bounded, 1, b"app", expected_stderr);
                let diagnostic = String::from_utf8_lossy(&legacy_refuted.stderr);
                assert!(diagnostic.contains("proof obligation failed") && diagnostic.contains("lib::src/credit.w")
                    && diagnostic.contains("ledger.relay()") && diagnostic.contains("consume()"), "{diagnostic}");
                assert_eq!(fs::read(project.output().join("app.txt")).unwrap(), b"app");
                assert!(!project.output().join("one.txt").exists() && !project.output().join("two.txt").exists());
                project.clear();
            }

            // Uncovered execution must veto the whole publication, not
            // disappear as a zero-budget dependency with no failures.
            fs::write(&credit, provider.replace("%%end", "relay(); %%end")).unwrap();
            let unsupported = project.run(&[scope, "--dependencies", "--max-errors", "1"]);
            assert_terminal(&unsupported, 2, b"", b"wosy: check: unsupported analysis; no result published\n");
            assert!(!project.output().exists());
        }
    }
}

#[test]
fn check_cap_process_keeps_mixed_contextual_header_encounter_order() {
    for layout in ["native", "wasm-wasip1"] {
        for scope in ["--main-flow-only", "--declared-package"] {
            for scalar_first in [false, true] {
                let project = Fixture::new();
                let calls = if scalar_first { "u8 scored = score(4); entry();" }
                    else { "entry(); u8 scored = score(4);" };
                let root = format!(r#"%%start
                    lib = namespace app "credit.w";
                    unit() entry = fn {{
                        u8 core.context.credit = 3;
                        lib.spend();
                    }};
                    u8(u8) score = fn(value) {{ value }};
                    proof scalar_value for score(input) -> (out) where (input == 4) ensures (out == 3) {{}}
                    {}
                    {calls}
                    core.require(scored == 3, {{ .stdout = "second"; .stderr = "[second]"; .code = 19;
                        .files = [{{ .path = "second.txt"; .text = "second"; }}]; }});
                    %%end"#, require("first", 0, "first.txt"));
                let provider = r#"%%start
                    unit() drain = fn { core.context.credit = core.context.credit - 1; };
                    unit() consume = fn { core.context.credit = core.context.credit - 1; };
                    unit() spend = fn { consume(); drain(); };
                    proof drain_goal for drain() -> (null)
                        ensures (core.context.credit == core.old(core.context.credit) - 0) {}
                    proof debit_goal for consume() -> (null)
                        ensures (core.context.credit == core.old(core.context.credit) - 0) {}
                %%end"#;
                project.package("", layout, "dev", "", &root);
                let credit = project.0.join("src/credit.w");
                fs::write(&credit, provider).unwrap();
                let old = project.run(&[scope, "--all-errors"]);
                assert_eq!(old.status.code(), Some(19), "{old:?}");
                assert_eq!(old.stdout, b"firstsecond");
                let first_file = fs::read(project.output().join("first.txt")).unwrap();
                let second_file = fs::read(project.output().join("second.txt")).unwrap();
                project.clear();

                let capped = project.run(&[scope, "--max-errors", "1"]);
                assert_eq!(capped.status.code(), Some(1), "{capped:?}");
                assert_eq!(capped.stdout, b"first");
                assert_eq!(capped.stderr, b"[first]");
                let diagnostic = String::from_utf8_lossy(&old.stderr);
                let first = diagnostic.find("contextual value at entry: 3; at exit: 2").unwrap();
                let second = diagnostic.find("contextual value at entry: 2; at exit: 1").unwrap();
                let scalar = diagnostic.find("call returned 4").unwrap();
                assert!(first < second && if scalar_first { scalar < first } else { second < scalar });
                assert_eq!(fs::read(project.output().join("first.txt")).unwrap(), first_file);
                assert!(!project.output().join("second.txt").exists());
                project.clear();

                assert_terminal(&project.run(&[scope, "--max-errors", "5"]), 19, &old.stdout, &old.stderr);
                assert_eq!(fs::read(project.output().join("first.txt")).unwrap(), first_file);
                assert_eq!(fs::read(project.output().join("second.txt")).unwrap(), second_file);
                project.clear();

                let proved = provider.replace("- 0) {}", "- 1) {}");
                assert_ne!(proved, provider);
                fs::write(&credit, proved).unwrap();
                let only_scalar = project.run(&[scope, "--max-errors", "3"]);
                assert_eq!(only_scalar.status.code(), Some(19), "{only_scalar:?}");
                assert_eq!(only_scalar.stdout, b"firstsecond");
                let scalar_diagnostic = String::from_utf8_lossy(&only_scalar.stderr);
                assert!(scalar_diagnostic.contains("call returned 4")
                    && !scalar_diagnostic.contains("contextual value at entry:"));
                project.clear();

                // Removing an edge cannot silently declare the second
                // requested theorem observed or substitute another call.
                fs::write(&credit, provider.replace("consume(); drain();", "consume(); consume();")).unwrap();
                assert_terminal(&project.run(&[scope, "--max-errors", "1"]), 2, b"",
                    b"wosy: check: unsupported analysis; no result published\n");
                assert!(!project.output().exists());
            }
        }
    }
}

#[test]
fn check_cap_process_preserves_exact_mixed_literal_inequality() {
    for layout in ["native", "wasm-wasip1"] {
        for scope in ["--main-flow-only", "--declared-package"] {
            for (predicate, refuted) in [("out > 3", false), ("out > 4", true), ("out != 4", true)] {
                let project = Fixture::new();
                let root = format!(r#"%%start
                    lib = namespace app "credit.w";
                    unit() entry = fn {{ u8 core.context.credit = 3; lib.spend(); }};
                    u8(u8) score = fn(value) {{ value }};
                    proof scalar_value for score(input) -> (out) where (input == 4) ensures ({predicate}) {{}}
                    {}
                    entry();
                    u8 scored = score(4);
                    core.require(scored == 5, {{ .stdout = "second"; .stderr = "[second]"; .code = 19;
                        .files = [{{ .path = "second.txt"; .text = "second"; }}]; }});
                %%end"#, require("first", 0, "first.txt"));
                let provider = r#"%%start
                    unit() consume = fn { core.context.credit = core.context.credit - 1; };
                    unit() spend = fn { consume(); };
                    proof debit_goal for consume() -> (null)
                        ensures (core.context.credit == core.old(core.context.credit) - 1) {}
                %%end"#;
                project.package("", layout, "dev", "", &root);
                fs::write(project.0.join("src/credit.w"), provider).unwrap();
                let old = project.run(&[scope, "--all-errors"]);
                assert_eq!(old.status.code(), Some(19), "{old:?}");
                assert_eq!(old.stdout, b"firstsecond");
                let expected = old.stderr.strip_prefix(b"[first][second]").unwrap();
                let text = String::from_utf8_lossy(expected);
                if refuted {
                    assert!(text.contains(predicate) && text.contains("call returned 4"), "{text}");
                } else {
                    assert!(text.is_empty());
                }
                project.clear();
                assert_terminal(&project.run(&[scope, "--max-errors", "1"]), i32::from(refuted), b"first", b"[first]");
                assert_eq!(fs::read(project.output().join("first.txt")).unwrap(), b"first");
                assert!(!project.output().join("second.txt").exists());
                project.clear();
                assert_terminal(&project.run(&[scope, "--max-errors", "3"]), 19, &old.stdout, &old.stderr);
                assert_eq!(fs::read(project.output().join("first.txt")).unwrap(), b"first");
                assert_eq!(fs::read(project.output().join("second.txt")).unwrap(), b"second");
            }
        }
    }
}

#[test]
fn check_cap_process_preserves_original_direct_qualified_contextual_leaf() {
    for layout in ["native", "wasm-wasip1"] {
        for scope in ["--main-flow-only", "--declared-package"] {
            let project = Fixture::new();
            let root = format!(r#"%%start
                lib = namespace app "credit.w";
                unit() entry = fn {{ u8 core.context.credit = 3; lib.consume(); }};
                u8(u8) score = fn(value) {{ value }};
                proof scalar_value for score(input) -> (out) where (input == 4) ensures (out > 3) {{}}
                {}
                entry();
                u8 scored = score(4);
                core.require(scored == 5, {{ .stdout = "second"; .stderr = "[second]"; .code = 19;
                    .files = [{{ .path = "second.txt"; .text = "second"; }}]; }});
                %%end"#, require("first", 0, "first.txt"));
            let provider = r#"%%start
                unit() consume = fn { core.context.credit = core.context.credit - 1; };
                proof debit_goal for consume() -> (null)
                    ensures (core.context.credit == core.old(core.context.credit) - 1) {}
            %%end"#;
            project.package("", layout, "dev", "", &root);
            let credit = project.0.join("src/credit.w");
            for refuted in [false, true] {
                let source = if refuted { provider.replace("- 1) {}", "- 0) {}") } else { provider.to_owned() };
                fs::write(&credit, source).unwrap();
                let old = project.run(&[scope, "--all-errors"]);
                assert_eq!(old.status.code(), Some(19), "{old:?}");
                assert_eq!(old.stdout, b"firstsecond");
                let expected_diagnostics = old.stderr.strip_prefix(b"[first][second]").unwrap();
                let diagnostic = String::from_utf8_lossy(expected_diagnostics);
                if refuted {
                    assert!(diagnostic.contains("src/credit.w") && diagnostic.contains("lib.consume()")
                        && diagnostic.contains("contextual value at entry: 3; at exit: 2"), "{diagnostic}");
                } else {
                    assert!(diagnostic.is_empty());
                }
                project.clear();

                assert_terminal(&project.run(&[scope, "--max-errors", "1"]), i32::from(refuted), b"first", b"[first]");
                assert_eq!(fs::read(project.output().join("first.txt")).unwrap(), b"first");
                assert!(!project.output().join("second.txt").exists());
                project.clear();

                assert_terminal(&project.run(&[scope, "--max-errors", "3"]), 19, &old.stdout, &old.stderr);
                assert_eq!(fs::read(project.output().join("second.txt")).unwrap(), b"second");
                project.clear();
            }
        }
    }
}

#[test]
fn check_cap_process_preserves_the_existing_binary_entry_comparison() {
    for layout in ["native", "wasm-wasip1"] {
        for scope in ["--main-flow-only", "--declared-package"] {
            for refuted in [false, true] {
                let project = Fixture::new();
                // This is the original signed binary Check proof from
                // check_main_flow.py, with an independent early authored path.
                let invocation = if refuted {
                    "i16 answer = choose(4, 8); i16 other = choose(3, 9);"
                } else { "i16 answer = choose(2, -3);" };
                let source = format!(r#"%%start
                    i16(i16, i16) choose = fn(left, right) {{ if (right < 0) {{ left }} else {{ right }} }};
                    proof identity for choose(entry, selector) -> (out)
                        where (entry >= -5) ensures (out == core.old(entry)) {{}}
                    {}
                    {invocation}
                    {}
                    %%end"#,
                    require("first", 0, "first.txt"),
                    require("second", 19, "second.txt"));
                project.package("", layout, "dev", "", &source);
                let old = project.run(&[scope, "--all-errors"]);
                assert_eq!(old.status.code(), Some(19), "{old:?}");
                assert_eq!(old.stdout, b"firstsecond");
                let original_diagnostics = old.stderr.strip_prefix(b"[first][second]").unwrap();
                let diagnostic = String::from_utf8_lossy(original_diagnostics);
                if refuted {
                    assert_eq!(diagnostic.matches("proof obligation failed").count(), 2);
                    assert!(diagnostic.contains("out == core.old(entry)")
                        && diagnostic.contains("choose(4, 8)") && diagnostic.contains("choose(3, 9)"));
                    assert!(diagnostic.find("call returned 8").unwrap() < diagnostic.find("call returned 9").unwrap());
                } else {
                    assert!(diagnostic.is_empty());
                }
                project.clear();

                assert_terminal(&project.run(&[scope, "--max-errors", "1"]), i32::from(refuted), b"first", b"[first]");
                assert_eq!(fs::read(project.output().join("first.txt")).unwrap(), b"first");
                assert!(!project.output().join("second.txt").exists());
                project.clear();

                let first_formal = diagnostic.match_indices("error: proof obligation failed").nth(1)
                    .map_or(original_diagnostics, |(second, _)| &original_diagnostics[..second]);
                let mut crossed = b"[first][second]".to_vec();
                crossed.extend(first_formal);
                assert_terminal(&project.run(&[scope, "--max-errors", "3"]), 19, &old.stdout, &crossed);
                assert_eq!(fs::read(project.output().join("second.txt")).unwrap(), b"second");
                project.clear();
                assert_terminal(&project.run(&[scope, "--max-errors", "4"]), 19, &old.stdout, &old.stderr);
                project.clear();

                let proof_only = source.replace(&require("first", 0, "first.txt"), "")
                    .replace(&require("second", 19, "second.txt"), "");
                fs::write(project.0.join("src/main.w"), proof_only).unwrap();
                let complete = project.run(&[scope, "--all-errors"]);
                assert_eq!(complete.status.code(), Some(i32::from(refuted)), "{complete:?}");
                assert_eq!(complete.stdout, b"");
                let proof_only_text = String::from_utf8_lossy(&complete.stderr);
                assert_eq!(proof_only_text.matches("error: proof obligation failed").count(), if refuted { 2 } else { 0 });
                let first = proof_only_text.match_indices("error: proof obligation failed").nth(1)
                    .map_or(complete.stderr.as_slice(), |(second, _)| &complete.stderr[..second]);
                assert_terminal(&project.run(&[scope]), i32::from(refuted), b"", first);
                assert_terminal(&project.run(&[scope, "--max-errors", "1"]), i32::from(refuted), b"", first);
                assert_terminal(&project.run(&[scope, "--max-errors", "2"]), i32::from(refuted), b"", &complete.stderr);
                assert!(!project.output().exists());
            }
        }
    }
}

#[test]
fn check_cap_process_preserves_the_existing_declared_dormant_fallback_proof() {
    for layout in ["native", "wasm-wasip1"] {
        for scope in ["--main-flow-only", "--declared-package"] {
            for proof in ["out != 7", "out >= 0"] {
                for early in [false, true] {
                    let project = Fixture::new();
                    let first = if early { require("first", 0, "first.txt") } else { String::new() };
                    let source = format!(r#"%%start
                        i16(i16, i16) choose = fn(left, right) {{ if (right < 0) {{ left }} else {{ right }} }};
                        proof identity for choose(entry, selector) -> (out)
                            where (entry >= -5) ensures (out == core.old(entry)) {{}}
                        {first}
                        i16 answer = choose(2, -3);
                        %%end"#);
                    let dormant = format!(r#"%%start
                        u8(u8) fallback = fn(value) {{ value }};
                        proof every_return for fallback(entry) -> (out) ensures ({proof}) {{}}
                        {}
                        %%end"#, require("must not emit", 71, "must-not-create.txt"));
                    project.package("", layout, "dev", "", &source);
                    let deferred_owner = project.0.join("src/fallback.w");
                    fs::write(&deferred_owner, &dormant).unwrap();
                    let old = project.run(&[scope, "--all-errors"]);
                    let refuted = scope == "--declared-package" && proof == "out != 7";
                    let expected = if early { b"first".as_slice() } else { b"".as_slice() };
                    assert_eq!(old.status.code(), Some(i32::from(refuted)), "{old:?}");
                    assert_eq!(old.stdout, expected);
                    let diagnostic = String::from_utf8_lossy(&old.stderr);
                    if refuted {
                        assert_eq!(diagnostic.matches("proof obligation failed").count(), 1);
                        for expected in ["src/fallback.w", "out != 7", "unused function checked symbolically", "symbolic call can return 7"] {
                            assert!(diagnostic.contains(expected), "{diagnostic}");
                        }
                    } else {
                        assert_eq!(old.stderr, if early { b"[first]".as_slice() } else { b"".as_slice() });
                    }
                    if early { project.clear(); }
                    let limited = project.run(&[scope, "--max-errors", "1"]);
                    let selected_stderr = if early { b"[first]".as_slice() } else { &old.stderr };
                    assert_terminal(&limited, i32::from(refuted), &old.stdout, selected_stderr);
                    if early {
                        assert_eq!(fs::read(project.output().join("first.txt")).unwrap(), b"first");
                        assert!(!project.output().join("must-not-create.txt").exists());
                    } else {
                        assert!(!project.output().exists());
                    }
                    if !early && refuted {
                        // A typed initializer in an inspected dormant
                        // source remains nonexecuting. Do not use its 8
                        // as the theorem witness instead of symbolic 7.
                        fs::write(&deferred_owner, dormant.replace("%%end", "u8 staged = fallback(8); %%end")).unwrap();
                        let old = project.run(&[scope, "--all-errors"]);
                        assert_eq!(old.status.code(), Some(1), "{old:?}");
                        assert!(String::from_utf8_lossy(&old.stderr).contains("symbolic call can return 7"));
                        assert_terminal(&project.run(&[scope, "--max-errors", "1"]), 1, b"", &old.stderr);
                        assert!(!project.output().exists());

                        // A lexical reference in the same inspected source
                        // does not execute without a selected caller root.
                        fs::write(&deferred_owner, dormant.replace("%%end", "u8() caller = fn { fallback(8) }; %%end")).unwrap();
                        let old = project.run(&[scope, "--all-errors"]);
                        assert_eq!(old.status.code(), Some(1), "{old:?}");
                        assert!(String::from_utf8_lossy(&old.stderr).contains("symbolic call can return 7"));
                        assert_terminal(&project.run(&[scope, "--max-errors", "1"]), 1, b"", &old.stderr);
                        assert_terminal(&project.run(&["--main-flow-only", "--max-errors", "1"]), 0, b"", b"");
                        assert!(!project.output().exists());

                        // A different dormant callable owns an authored
                        // directive and can reach the lexical caller.
                        // The inspection-only exception must now refuse.
                        let rooted = format!("u8() caller = fn {{ fallback(8) }}; u8() independent = fn {{ {} caller() }}; %%end",
                            require("independent", 73, "independent.txt"));
                        fs::write(&deferred_owner, dormant.replace("%%end", &rooted)).unwrap();
                        assert_terminal(&project.run(&[scope, "--max-errors", "1"]), 2, b"",
                            b"wosy: check: unsupported analysis; no result published
");
                        assert!(!project.output().exists());
                    }
                }
            }
        }
    }
}

#[test]
fn check_cap_process_walks_actual_nested_unit_frames_in_prepared_owners() {
    let a = require("a", 5, "a.txt");
    let b = require("b", 7, "b.txt");
    let library = format!("%%start unit() leaf = fn {{ {a}{b} }}; unit() relay = fn {{ leaf(); }}; %%end");
    for (layout, other) in [("native", "wasm-wasip1"), ("wasm-wasip1", "native")] {
        for scope in ["--declared-package", "--main-flow-only"] {
            for topology in ["local", "qualified", "dependency"] {
                let project = Fixture::new();
                let qualified = |package| format!(r#"%%start provider = namespace {package} "relay.w"; provider.relay(); %%end"#);
                match topology {
                    "local" => project.package("", layout, "dev", "", &library.replace("%%end", "relay(); %%end")),
                    "qualified" => {
                        project.package("", layout, "dev", "", &qualified("app"));
                        fs::write(project.0.join("src/relay.w"), &library).unwrap();
                    }
                    "dependency" => {
                        project.package("", layout, "dev", "[dependencies.lib]\npath = \"lib\"", "%%start u8 placeholder = 0; %%end");
                        project.package("lib", other, "own", "", &qualified("lib"));
                        fs::write(project.0.join("lib/src/relay.w"), &library).unwrap();
                    }
                    _ => unreachable!(),
                }
                let run = |choice: &[&str]| {
                    let mut flags = vec![scope];
                    if topology == "dependency" { flags.push("--dependencies"); }
                    flags.extend(choice);
                    project.run(&flags)
                };
                for (limit, full) in [(&["--all-errors"][..], true), (&[][..], false),
                    (&["--max-errors","1"][..], false), (&["--max-errors","2"][..], true),
                    (&["--max-errors","3"][..], true)] {
                    assert_terminal(&run(limit), 5,
                        if full { b"ab" } else { b"a" },
                        if full { b"[a][b]" } else { b"[a]" });
                    assert_eq!(fs::read(project.output().join("a.txt")).unwrap(), b"a");
                    if full { assert_eq!(fs::read(project.output().join("b.txt")).unwrap(), b"b"); }
                    else { assert!(!project.output().join("b.txt").exists()); }
                    assert!(!project.0.join("lib/.wosy/outputs/check").exists());
                    project.clear();
                }
            }
        }
    }
}

#[test]
fn check_cap_process_shares_original_bare_calls_without_multiplying_authored_paths() {
    for layout in ["native", "wasm-wasip1"] {
        for scope in ["--declared-package", "--main-flow-only"] {
            for (goal_a, entry_b, goal_b, returns) in [
                (2, 3, 3, None),
                (4, 3, 5, Some((2, 3))),
                (4, 2, 5, Some((2, 2))),
            ] {
                let project = Fixture::new();
                let source = format!(r#"%%start
                    u8(u8) choose = fn(value) {{ value }};
                    proof first_case for choose(entry) -> (out) where (entry == 2) ensures (out == {goal_a}) {{}}
                    proof second_case for choose(entry) -> (out) where (entry == {entry_b}) ensures (out == {goal_b}) {{}}
                    u8 first = choose(2);
                    u8 second = choose(3);
                    if (first < second) {{ {}{} }};
                    %%end"#, require("a", 5, "a.txt"), require("b", 7, "b.txt"));
                project.package("", layout, "dev", "", &source);
                let full = project.run(&[scope, "--all-errors"]);
                assert_eq!(full.status.code(), Some(5), "{full:?}");
                assert_eq!(full.stdout, b"ab");
                assert!(full.stderr.starts_with(b"[a][b]"));
                if let Some((first, second)) = returns {
                    let text = String::from_utf8_lossy(&full.stderr);
                    let offsets = text.match_indices("error: proof obligation failed").map(|(offset, _)| offset).collect::<Vec<_>>();
                    assert_eq!(offsets.len(), 2);
                    assert!(text[offsets[0]..offsets[1]].contains(&format!("call returned {first}")), "{text}");
                    assert!(text[offsets[1]..].contains(&format!("call returned {second}")), "{text}");
                    assert!(text[offsets[0]..offsets[1]].contains("first_case"), "{text}");
                    assert!(text[offsets[1]..].contains("second_case"), "{text}");
                } else {
                    assert_eq!(full.stderr, b"[a][b]");
                }
                assert_eq!(fs::read(project.output().join("a.txt")).unwrap(), b"a");
                assert_eq!(fs::read(project.output().join("b.txt")).unwrap(), b"b");
                project.clear();
                assert_terminal(&project.run(&[scope]), 5, b"a", b"[a]");
                assert_eq!(fs::read(project.output().join("a.txt")).unwrap(), b"a");
                assert!(!project.output().join("b.txt").exists());
                project.clear();
                let second_diag = String::from_utf8_lossy(&full.stderr)
                    .match_indices("error: proof obligation failed").nth(1).map(|(offset, _)| offset).unwrap_or(full.stderr.len());
                for (limit, expected) in [("2", b"[a][b]".as_slice()), ("3", &full.stderr[..second_diag]), ("4", &full.stderr)] {
                    assert_terminal(&project.run(&[scope, "--max-errors", limit]), 5, b"ab", expected);
                    assert_eq!(fs::read(project.output().join("a.txt")).unwrap(), b"a");
                    assert_eq!(fs::read(project.output().join("b.txt")).unwrap(), b"b");
                    project.clear();
                }
            }
        }
    }
}

#[test]
fn check_cap_process_preserves_prepared_zero_obligations_without_skipping_selected_work() {
    for layout in ["native", "wasm-wasip1"] {
        for scope in ["--main-flow-only", "--declared-package"] {
            let project = Fixture::new();
            // A valid typed startup initializer that doesn't fit the
            // bounded integer startup evaluator. It is not an obligation.
            let original = "%%start f32 base = 1.5; %%end";
            project.package("", layout, "dev", "", original);
            for limit in [&["--all-errors"][..], &[][..], &["--max-errors", "1"][..]] {
                let mut flags = vec![scope];
                flags.extend(limit);
                assert_terminal(&project.run(&flags), 0, b"", b"");
                assert!(!project.output().exists());
            }

            let source = project.0.join("src/main.w");
            let authored = format!("%%start unit() report = fn {{ {} }}; report(); %%end",
                require("reached", 13, "reached.txt"));
            fs::write(&source, &authored).unwrap();
            assert_terminal(&project.run(&[scope]), 13, b"reached", b"[reached]");
            assert_eq!(fs::read(project.output().join("reached.txt")).unwrap(), b"reached");
            project.clear();

            fs::write(&source, "%%start u8 broken = true; %%end").unwrap();
            let incumbent = project.run(&[scope, "--all-errors"]);
            assert_eq!(incumbent.status.code(), Some(2));
            assert!(incumbent.stdout.is_empty() && String::from_utf8_lossy(&incumbent.stderr).contains("src/main.w"));
            assert_terminal(&project.run(&[scope]), 2, b"", &incumbent.stderr);
            assert!(!project.output().exists());
        }
    }
}

#[test]
fn check_cap_process_initializes_prepared_passive_float_array_but_refuses_unknown_consumers() {
    for layout in ["native", "wasm-wasip1"] {
        let project = Fixture::new();
        let mut literals = vec!["0.6205"; 113];
        literals[1] = "-0.0";
        literals[112] = "0.42";
        let binding = format!("f64[113] profile = [{}];", literals.join(", "));
        let authored = require("reachable", 13, "selected.txt");
        let program = format!("%%start {binding} {authored} %%end");
        project.package("", layout, "dev", "", &program);
        for scope in ["--main-flow-only", "--declared-package"] {
            for mode in [&[][..], &["--all-errors"][..]] {
                let mut flags = vec![scope];
                flags.extend(mode);
                assert_terminal(&project.run(&flags), 13, b"reachable", b"[reachable]");
                assert_eq!(fs::read(project.output().join("selected.txt")).unwrap(), b"reachable");
                project.clear();
            }
        }
        let source = project.0.join("src/main.w");
        for change in [
            format!("{binding} f64 sample = profile[0];"),
            format!("{binding} f64[113] replica = profile;"),
            "f64[2] profile = [0.5 + 0.25, 0.75];".to_owned(),
        ] {
            fs::write(&source, format!("%%start {change} {authored} %%end")).unwrap();
            for mode in [&[][..], &["--all-errors"][..]] {
                let mut flags = vec!["--main-flow-only"];
                flags.extend(mode);
                assert_terminal(&project.run(&flags), 2, b"",
                    b"wosy: check: unsupported analysis; no result published\n");
                assert!(!project.output().exists());
            }
        }
    }
}

#[test]
fn check_cap_process_initializes_exact_passive_part_and_nested_literals() {
    let declaration = "struct copy Part { i32 section; i32 axis; i32 finish; f32 x; f32 y; f32 z; f32 l; f32 u; f32 v; f32 taper; }";
    let literal = "{.section=0;.axis=0;.finish=0;.x=0.0;.y=0.0;.z=0.0;.l=0.0;.u=0.0;.v=0.0;.taper=1.0;}";
    let authored = require("reachable", 13, "selected.txt");
    for layout in ["native", "wasm-wasip1"] {
        let project = Fixture::new();
        let bindings = format!("Part empty={literal}; Part[1][1] nested=[[{literal}]];");
        project.package("", layout, "dev", "", &format!("%%start {declaration} {bindings} {authored} %%end"));
        for scope in ["--main-flow-only", "--declared-package"] {
            for mode in [&[][..], &["--all-errors"][..]] {
                let mut flags = vec![scope];
                flags.extend(mode);
                assert_terminal(&project.run(&flags), 13, b"reachable", b"[reachable]");
                assert_eq!(fs::read(project.output().join("selected.txt")).unwrap(), b"reachable");
                project.clear();
            }
        }
        let source = project.0.join("src/main.w");
        for change in [
            format!("Part empty={literal}; Part[1] row=[empty];"),
            format!("Part empty={literal}; f32 sample=empty.x;"),
            format!("Part empty={};", literal.replace(".taper=1.0;", ".taper=0.75+0.25;")),
        ] {
            fs::write(&source, format!("%%start {declaration} {change} {authored} %%end")).unwrap();
            for mode in [&[][..], &["--all-errors"][..]] {
                let mut flags = vec!["--main-flow-only"];
                flags.extend(mode);
                assert_terminal(&project.run(&flags), 2, b"",
                    b"wosy: check: unsupported analysis; no result published\n");
                assert!(!project.output().exists());
            }
        }
    }
}
