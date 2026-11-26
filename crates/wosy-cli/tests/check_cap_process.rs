#![cfg(all(target_os = "linux", feature = "internal-checker"))]

#[cfg(feature = "private-check-cap-process")]
use std::fs;
use std::path::Path;
#[cfg(feature = "private-check-cap-process")]
use std::path::PathBuf;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_wosy");

fn wosy(root: &Path, args: &[&str]) -> Output {
    Command::new(BIN).current_dir(root).env("NO_COLOR", "1").args(args).output().unwrap()
}

#[test]
fn check_cap_is_not_advertised() {
    let help = wosy(Path::new("."), &["check", "--help"]);
    assert!(help.status.success());
    assert!(!String::from_utf8_lossy(&help.stdout).contains("--max-errors"));
    #[cfg(not(feature = "private-check-cap-process"))]
    {
        let absent = wosy(Path::new("."), &["check", "--max-errors", "1"]);
        assert_eq!(absent.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&absent.stderr).contains("unexpected argument '--max-errors'"));
    }
}

#[cfg(feature = "private-check-cap-process")]
struct Fixture(PathBuf);

#[cfg(feature = "private-check-cap-process")]
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

#[cfg(feature = "private-check-cap-process")]
impl Drop for Fixture {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

#[cfg(feature = "private-check-cap-process")]
fn require(label: &str, code: u8, path: &str) -> String {
    format!(r#"core.require(false, {{ .stdout = "{label}"; .stderr = "[{label}]"; .code = {code}; .files = [{{ .path = "{path}"; .text = "{label}"; }}]; }});"#)
}

#[cfg(feature = "private-check-cap-process")]
fn assert_terminal(output: &Output, code: i32, stdout: &[u8], stderr: &[u8]) {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    assert_eq!(output.stdout, stdout, "{output:?}");
    assert_eq!(output.stderr, stderr, "{output:?}");
}

#[cfg(feature = "private-check-cap-process")]
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
            let incumbent = project.run(&[scope]);
            assert_terminal(&incumbent, 19, b"firstsecond", b"[first][second]");
            assert_eq!(fs::read(project.output().join("first.txt")).unwrap(), b"first");
            assert_eq!(fs::read(project.output().join("second.txt")).unwrap(), b"second");
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

#[cfg(feature = "private-check-cap-process")]
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

#[cfg(feature = "private-check-cap-process")]
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
