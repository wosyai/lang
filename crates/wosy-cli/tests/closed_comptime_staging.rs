use std::{
    ffi::OsStr, fs,
    io::{self, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use wosy_cli::comptime_fragment::{plan_closed_comptime_staging, ClosedComptimePaths, ClosedComptimePlanError};
use wosy_compiler::{ComptimeParameterOrigin, ComptimeParameterValue, ScalarTargetLayout, ScalarType};

const SOURCE: &str = r#"%%start
# WHY: parâmetros com café ☕
bool answer = comptime prepare {
    i32 left = core.comp_param("shared", 5);
    u32 right = core.comp_param("shared", 6);
    i32 left_expected = 5;
    u32 right_expected = 6;
    bool enabled = core.comp_param("enabled", true);
    (left == left_expected && right == right_expected) && enabled
};
%%end
"#;

const MANIFEST: &str = r#"
[package]
name = "app"
default_target = "app"

[comments]
categories = ["WHY"]

[builders.ordinary]
command = ["must-not-start-runtime-builder"]
toolchain_identity = "ordinary"

[runners.ordinary]
command = ["must-not-start-runtime-runner"]

[builders.compile]
command = ["path to declared comptime builder", "literal *", "$WORD"]
args = ["--label=two words"]
toolchain_identity = "selected evaluator toolchain"

[runners.evaluate]
command = ["path to declared comptime runner", "literal runner"]
args = ["--literal=${VAR}"]

[targets.app]
root = "src/main.w"

[targets.app.artifacts.main]
artifact = "Main"

[targets.app.artifacts.main.profiles.defaults]
backend = "llvm"
output = "native"
builder = "ordinary"
run_runner = "ordinary"

[targets.app.artifacts.main.profiles.override]
backend = "llvm"
output = "native"
builder = "ordinary"
run_runner = "ordinary"

[targets.app.artifacts.main.profiles.legacy]
backend = "llvm"
output = "native"
builder = "ordinary"
run_runner = "ordinary"

[targets.app.profiles.defaults]
main_artifact = "main"
comptime_builder = "compile"
comptime_runner = "evaluate"

[targets.app.profiles.override]
main_artifact = "main"
comptime_builder = "compile"
comptime_runner = "evaluate"
build_permissions = []
build_mounts = []

[targets.app.profiles.override.params]
shared = 9
enabled = false
"unused.value" = "not read by the fragment"

[targets.app.profiles.legacy]
main_artifact = "main"

[comptime.unrelated]
watch = ["directory-not-opened/**"]
"#;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("wosy-closed-staging-{}-{id}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        fs::create_dir_all(fixture.project().join("src")).unwrap();
        fs::write(fixture.project().join("src/main.w"), SOURCE).unwrap();
        fixture.manifest(MANIFEST);
        fixture
    }

    fn project(&self) -> PathBuf { self.0.join("project") }

    fn manifest(&self, body: &str) {
        fs::write(self.project().join("wosy.toml"), body).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn bytes_in(span: wosy_syntax::ByteSpan) -> &'static str {
    &SOURCE[span.start as usize..span.end as usize]
}

#[test]
fn actual_selected_profile_stages_exact_ordinary_ir_and_literal_invocations() {
    let fixture = Fixture::new();
    let project = fixture.project();
    let plan = |profile| plan_closed_comptime_staging(&project, "app", profile, "prepare", ScalarTargetLayout::WASM32);
    let defaults = plan("defaults").unwrap();
    let overridden = plan("override").unwrap();
    assert_eq!(defaults.fragment().layout(), ScalarTargetLayout::WASM32);
    assert_eq!(defaults.fragment().result_type(), &ScalarType::Bool);
    assert_eq!(defaults.fragment().site().source.path, "src/main.w");
    assert!(bytes_in(defaults.fragment().site().range).starts_with("comptime prepare"));
    assert_eq!(defaults.configuration_identity(), overridden.configuration_identity());

    let defaults_observed = defaults.fragment().parameter_uses();
    let overridden_observed = overridden.fragment().parameter_uses();
    assert_eq!(defaults_observed.len(), 3);
    assert_eq!(overridden_observed.len(), 3);
    for (index, (name, context, value, declared, expected_literal)) in [
        ("shared", ScalarType::I32, ComptimeParameterValue::Integer(5), ComptimeParameterValue::Integer(9), "5"),
        ("shared", ScalarType::U32, ComptimeParameterValue::Integer(6), ComptimeParameterValue::Integer(9), "6"),
        ("enabled", ScalarType::Bool, ComptimeParameterValue::Boolean(true), ComptimeParameterValue::Boolean(false), "true"),
    ].into_iter().enumerate() {
        let default = &defaults_observed[index];
        let override_ = &overridden_observed[index];
        assert_eq!((default.name.as_str(), &default.context, default.resolved), (name, &context, value));
        assert_eq!((override_.name.as_str(), &override_.context, override_.resolved), (name, &context, declared));
        assert_eq!(default.call, override_.call);
        assert_eq!(bytes_in(default.call), format!(r#"core.comp_param("{name}", {expected_literal})"#));
        let ComptimeParameterOrigin::Default { literal } = default.origin else { panic!("expected missing profile value"); };
        assert_eq!(bytes_in(literal), expected_literal);
        assert_eq!(override_.origin, ComptimeParameterOrigin::Profile);
    }
    assert!(defaults.tools().build_permissions().is_none());
    assert!(overridden.tools().build_mounts().is_some_and(<[_]>::is_empty));
    assert_eq!(overridden.tools().build_permissions(), Some([].as_slice()));
    assert_eq!(overridden.tools().builder().0, "compile");
    assert_eq!(overridden.tools().runner().0, "evaluate");

    let stage = fixture.0.join("explicit scratch");
    let paths = stage.join("evaluator.ll");
    let output_dir = stage.join("output directory");
    let manifest = stage.join("comptime-manifest");
    fs::create_dir(&stage).unwrap();
    let builder = overridden.builder(ClosedComptimePaths { evaluator_ir: &paths, output_dir: &output_dir, manifest: &manifest });
    assert_eq!(builder.executable, OsStr::new("path to declared comptime builder"));
    assert_eq!(builder.working_directory, project.canonicalize().unwrap());
    assert_eq!(builder.arguments, [
        OsStr::new("literal *"), OsStr::new("$WORD"), OsStr::new("--label=two words"),
        paths.as_os_str(), output_dir.as_os_str(), manifest.as_os_str(),
    ]);
    let runner = overridden.runner(&manifest);
    assert_eq!(runner.executable, OsStr::new("path to declared comptime runner"));
    assert_eq!(runner.working_directory, builder.working_directory);
    assert_eq!(runner.arguments, [OsStr::new("literal runner"), OsStr::new("--literal=${VAR}"), manifest.as_os_str()]);
    let mut staged = std::fs::File::create_new(&paths).unwrap();
    let written = overridden.write_evaluator_ir(&mut staged, 1 << 16).unwrap();
    drop(staged);
    let actual = fs::read(&paths).unwrap();
    assert_eq!(actual, overridden.fragment().llvm().to_text().as_bytes());
    assert_eq!(written.bytes, actual.len());
    assert_eq!(written.blake3, blake3::hash(&actual));
    assert_ne!(written.blake3, defaults.write_evaluator_ir(&mut io::sink(), 1 << 16).unwrap().blake3);
    assert!(!manifest.exists() && !output_dir.exists() && !project.join(".wosy").exists());

    // Explicitly observed configuration is preserved; only a new plan gets
    // a new manifest identity, not an execution permission for either plan.
    fixture.manifest(&format!("{MANIFEST}\n# exact authored revision changed\n"));
    let later = plan("override").unwrap();
    assert_ne!(later.configuration_identity(), overridden.configuration_identity());
    assert_eq!(later.write_evaluator_ir(&mut io::sink(), 1 << 16).unwrap(), written);
}

#[test]
fn no_unresolved_build_capabilities_or_watches_can_stage() {
    let fixture = Fixture::new();
    let project = fixture.project();
    let refusal = |profile, expected: &str| {
        let error = plan_closed_comptime_staging(&project, "app", profile, "prepare", ScalarTargetLayout::NATIVE64)
            .err().expect("staging must not claim resolution for this project");
        let ClosedComptimePlanError::Project(message) = error else { panic!("{error:?}"); };
        assert_eq!(message, expected);
    };
    refusal("legacy", "closed comptime staging needs an explicit comptime builder and runner");
    for declaration in ["build_permissions = [\"network\"]", "build_mounts = [{ host_path = \"data\", guest_path = \"ro\", access = \"read\" }]"] {
        fixture.manifest(&MANIFEST.replace("[targets.app.profiles.defaults]\n", &format!("[targets.app.profiles.defaults]\n{declaration}\n")));
        refusal("defaults", "closed comptime staging does not resolve build permissions or mounts");
    }
    for declaration in ["[comptime.prepare]\nwatch = [\"not-opened/**\"]", "[comptime.\"app:prepare\"]\nwatch_add = [\"not-opened/**\"]"] {
        fixture.manifest(&format!("{MANIFEST}\n{declaration}\n"));
        refusal("defaults", "closed comptime staging does not resolve watches");
    }
    assert!(!project.join(".wosy").exists());
}

#[test]
fn egress_digest_counts_partial_writes_and_never_returns_a_partial_receipt() {
    struct Short(Vec<u8>);
    impl Write for Short {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let written = bytes.len().min(7);
            self.0.extend_from_slice(&bytes[..written]);
            Ok(written)
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    let fixture = Fixture::new();
    let plan = plan_closed_comptime_staging(&fixture.project(), "app", "override", "prepare", ScalarTargetLayout::NATIVE64).unwrap();
    let expected = plan.fragment().llvm().to_text();
    let mut writer = Short(Vec::new());
    let receipt = plan.write_evaluator_ir(&mut writer, expected.len()).unwrap();
    assert_eq!(writer.0, expected.as_bytes());
    assert_eq!(receipt.blake3, blake3::hash(expected.as_bytes()));
    assert_eq!(receipt.bytes, expected.len());
    let mut writer = Short(Vec::new());
    let error = plan.write_evaluator_ir(&mut writer, expected.len() - 1).unwrap_err();
    assert_eq!(error.to_string(), "closed comptime evaluator IR exceeds byte budget");
    assert!(writer.0.len() < expected.len());
}

#[cfg(target_os = "linux")]
#[test]
fn compiler_provisions_the_paths_and_binds_only_its_own_typed_result_slot() {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    let fixture = Fixture::new();
    let scratch = fixture.0.join("private staging");
    fs::DirBuilder::new().mode(0o700).create(&scratch).unwrap();
    let select = |profile| plan_closed_comptime_staging(
        &fixture.project(), "app", profile, "prepare", ScalarTargetLayout::WASM32,
    ).unwrap();
    let pending = select("override").provision(&scratch, 1 << 16).unwrap();
    let other = select("defaults").provision(&scratch, 1 << 16).unwrap();
    let builder = pending.builder_invocation();
    let runner = pending.runner_invocation();
    let ir_path = std::path::Path::new(builder.arguments[3]);
    let output = std::path::Path::new(builder.arguments[4]);
    assert_eq!(*builder.arguments.last().unwrap(), pending.manifest_path().as_os_str());
    assert_eq!(*runner.arguments.last().unwrap(), pending.manifest_path().as_os_str());
    assert_eq!(output.parent(), Some(scratch.as_path()));
    assert_eq!(ir_path, output.join("evaluator.ll"));
    assert_eq!(pending.expected_module_path(), output.join("evaluator.module"));
    assert_ne!(pending.expected_module_path(), other.expected_module_path());
    assert_eq!(fs::metadata(output).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(fs::metadata(ir_path).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(!pending.manifest_path().exists());
    let ir = fs::read(ir_path).unwrap();
    assert_eq!(ir, pending.staging().fragment().llvm().to_text().as_bytes());
    assert_eq!(pending.evaluator_ir().blake3, blake3::hash(&ir));
    assert_eq!(pending.observe_module(64).err().unwrap().kind(), io::ErrorKind::NotFound);

    // Only the bytes in the exact provision's reserved slot are observed;
    // these benign test bytes aren't executed or admitted as a compiler result.
    let module = b"non-executable observation only";
    fs::write(pending.expected_module_path(), []).unwrap();
    assert!(pending.observe_module(64).is_err());
    fs::write(pending.expected_module_path(), module).unwrap();
    assert!(pending.observe_module(module.len() as u64 - 1).is_err());
    assert!(other.observe_module(64).is_err());
    let scoped = pending.observe_module(module.len() as u64).unwrap();
    assert_eq!(scoped.bytes(), module.len() as u64);
    assert_eq!(scoped.digest(), blake3::hash(module));
    assert_eq!(scoped.result_type(), &ScalarType::Bool);
    let observed = scoped.provision().staging();
    assert_eq!(observed.tools().target(), "app");
    assert_eq!(observed.tools().profile(), "override");
    assert_eq!(observed.fragment().site().source.path, "src/main.w");
    assert_eq!(observed.fragment().parameter_uses()[0].resolved, ComptimeParameterValue::Integer(9));
    fixture.manifest(&MANIFEST.replace("shared = 9", "shared = 10"));
    assert_ne!(select("override").configuration_identity(), observed.configuration_identity());
    assert_eq!(observed.fragment().parameter_uses()[0].resolved, ComptimeParameterValue::Integer(9));

    assert!(!fixture.project().join(".wosy").exists());
    drop(pending);
    drop(other);
    assert!(fs::read_dir(&scratch).unwrap().next().is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn declined_provision_has_no_staging_files() {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    let fixture = Fixture::new();
    let scratch = fixture.0.join("private");
    let non_private = fixture.0.join("not private");
    fs::DirBuilder::new().mode(0o700).create(&scratch).unwrap();
    fs::DirBuilder::new().mode(0o755).create(&non_private).unwrap();
    fs::set_permissions(&non_private, fs::Permissions::from_mode(0o755)).unwrap();
    let select = || plan_closed_comptime_staging(
        &fixture.project(), "app", "defaults", "prepare", ScalarTargetLayout::NATIVE64,
    ).unwrap();
    let error = select().provision(&non_private, 1 << 16).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(fs::read_dir(&non_private).unwrap().next().is_none());
    let error = select().provision(&scratch, 0).err().unwrap();
    assert_eq!(error.to_string(), "closed comptime evaluator IR exceeds byte budget");
    assert!(fs::read_dir(&scratch).unwrap().next().is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn selected_comptime_stdout_materializes_resolved_structs_arrays_and_utf8() {
    use std::{os::unix::process::ExitStatusExt, process::ExitStatus};
    use wosy_cli::comptime_fragment::{prepare_project_comptime_stdout, ComptimeReadbackError as Error};
    use wosy_compiler::{ComptimeStdoutError as Format, ScalarExpression as Expr};

    let fixture = Fixture::new();
    let project = fixture.project();
    let standard = fixture.0.join("std");
    fs::create_dir_all(standard.join("src")).unwrap();
    fs::write(standard.join("wosy.toml"), "[package]\nname = \"std\"\nsource_root = \"src\"\n[comments]\ncategories = []\n").unwrap();
    fs::write(standard.join("src/utf8.w"), "%%start\nstruct utf8 { *?u8 data; u64 length; }\n%%end\n").unwrap();
    fixture.manifest(&format!("{MANIFEST}\n[dependencies.std]\npath = \"../std\"\n"));
    let source = project.join("src/main.w");
    let authored = r#"%%start
# WHY: inspeção de retorno com café ☕
std = namespace std "utf8.w";
struct Cell { bool enabled; i32 offset; }
struct Report { std.utf8 label; u32[2] numbers; Cell[2] cells; }
Report result = comptime prepare {
    { .cells = [{ .offset = -7; .enabled = true; }, { .offset = 0; .enabled = false; }];
      .numbers = [10, 20]; .label = "fonte"; }
};
%%end
"#;
    let ok = ExitStatus::from_raw(0);
    let exit_9 = ExitStatus::from_raw(9 << 8);
    let signal = ExitStatus::from_raw(15);
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        fs::write(&source, authored).unwrap();
        let prepared = prepare_project_comptime_stdout(&project, "app", "override", "prepare", layout)
            .unwrap_or_else(|error| panic!("ordinary aggregate result on {layout:?}: {error:?}"));
        let example = "[\"olá\\n\\\"ok\\\\x\",[4294967295,20],[[true,-2147483648],[false,0]]]\n".as_bytes();
        let Expr::StructLiteral { fields, span } = prepared.read_stdout(ok, example, example.len()).unwrap()
        else { panic!("one ordinary struct value"); };
        assert_eq!(span, prepared.site().range);
        assert_eq!(fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(), ["label", "numbers", "cells"]);
        assert!(matches!(&fields[0].value, Expr::Utf8 { value, .. } if value == "olá\n\"ok\\x".as_bytes()));
        let check_int = |actual: &Expr, expected: i64| {
            assert!(matches!(actual, Expr::Integer { value, .. } if i64::try_from(value) == Ok(expected)));
        };
        let Expr::ArrayLiteral { elements, .. } = &fields[1].value else { panic!("u32 array"); };
        assert_eq!(elements.len(), 2);
        check_int(&elements[0], u32::MAX.into());
        check_int(&elements[1], 20);
        let Expr::ArrayLiteral { elements, .. } = &fields[2].value else { panic!("Cell array"); };
        assert_eq!(elements.len(), 2);
        for (index, record) in elements.iter().enumerate() {
            let Expr::StructLiteral { fields, .. } = record else { panic!("Cell"); };
            assert_eq!(fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(), ["enabled", "offset"]);
            assert!(matches!(&fields[0].value, Expr::Boolean { value, .. } if *value == (index == 0)));
            check_int(&fields[1].value, if index == 0 { i32::MIN.into() } else { 0 });
        }
        assert_eq!(prepared.read_stdout(exit_9, example, example.len()).err(), Some(Error::Exit(Some(9))));
        assert_eq!(prepared.read_stdout(signal, example, example.len()).err(), Some(Error::Exit(None)));
        assert_eq!(prepared.read_stdout(ok, example, example.len() - 1).err(), Some(Error::Output(Format::ByteLimit)));
        for (framing, reason) in [
            (b"[\"x\",[1,2],[[true,0],[false,0]]]\n\n".as_slice(), Format::NonCanonical),
            (b"[\"x\",[1,2],[[true,0],[false,0]]]".as_slice(), Format::NonCanonical),
            (b"[\"x\",[1],[[true,0],[false,0]]]\n".as_slice(), Format::NonCanonical),
            (b"[\"x\",[1,2,3],[[true,0],[false,0]]]\n".as_slice(), Format::NonCanonical),
            (b"[\"x\",[1, 2],[[true,0],[false,0]]]\n".as_slice(), Format::NonCanonical),
            (b"[\"x\",[1,2],[[true,0],[false,0]],0]\n".as_slice(), Format::NonCanonical),
            (b"[\"x\",[1,2],[[true,00],[false,0]]]\n".as_slice(), Format::NonCanonical),
            (b"[\"x\",[1,2],[[true,-0],[false,0]]]\n".as_slice(), Format::NonCanonical),
            (b"[\"x\",[1,2],[[true,+1],[false,0]]]\n".as_slice(), Format::NonCanonical),
            (b"[\"x\",[4294967296,2],[[true,0],[false,0]]]\n".as_slice(), Format::OutOfRange),
            (b"[\"x\",[1,2],[[true,-2147483649],[false,0]]]\n".as_slice(), Format::OutOfRange),
            (b"[\"\\u000a\",[1,2],[[true,0],[false,0]]]\n".as_slice(), Format::NonCanonical),
            (b"[\"x\n\",[1,2],[[true,0],[false,0]]]\n".as_slice(), Format::NonCanonical),
            (b"[\"\xff\",[1,2],[[true,0],[false,0]]]\n".as_slice(), Format::NonCanonical),
        ] {
            assert_eq!(prepared.read_stdout(ok, framing, 256).err(), Some(Error::Output(reason)), "{framing:?}");
        }

        // Independent result, same id but a different source revision and
        // type. The earlier prepared result owns its original shape.
        fs::write(&source, "%%start\nbool answer = comptime prepare { true };\n%%end\n").unwrap();
        let scalar = prepare_project_comptime_stdout(&project, "app", "override", "prepare", layout).unwrap();
        assert_ne!(scalar.site().source.revision, prepared.site().source.revision);
        assert!(matches!(scalar.read_stdout(ok, b"true\n", 5).unwrap(), Expr::Boolean { value: true, .. }));
        assert_eq!(scalar.read_stdout(ok, example, 256).err(), Some(Error::Output(Format::NonCanonical)));
        assert!(prepared.read_stdout(ok, example, 256).is_ok());
    }
    assert!(!project.join(".wosy").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn provisioned_scalar_stdout_uses_the_same_typed_format_without_a_runner() {
    use std::{os::unix::{fs::DirBuilderExt, process::ExitStatusExt}, process::ExitStatus};
    use wosy_cli::comptime_fragment::ComptimeReadbackError as Error;
    use wosy_compiler::{ComptimeStdoutError as Format, ScalarExpression as Expr};

    let fixture = Fixture::new();
    let scratch = fixture.0.join("private scalar stdout");
    fs::DirBuilder::new().mode(0o700).create(&scratch).unwrap();
    let success = ExitStatus::from_raw(0);
    let project = fixture.project();
    for (ty, accepted, expected) in [
        ("i32", b"-2147483648\n".as_slice(), i64::from(i32::MIN)),
        ("u32", b"4294967295\n".as_slice(), i64::from(u32::MAX)),
    ] {
        fs::write(project.join("src/main.w"), format!("%%start\n{ty} answer = comptime prepare {{ 7 }};\n%%end\n")).unwrap();
        let pending = plan_closed_comptime_staging(&project, "app", "override", "prepare", ScalarTargetLayout::NATIVE64)
            .unwrap().provision(&scratch, 1 << 16).unwrap();
        fs::write(pending.expected_module_path(), b"inert test bytes, no runner").unwrap();
        let module = pending.observe_module(64).unwrap();
        assert!(matches!(module.read_stdout(success, accepted, accepted.len()).unwrap(),
            Expr::Integer { value, .. } if i64::try_from(&value) == Ok(expected)));
        assert_eq!(module.read_stdout(success, b"007\n", 12).err(), Some(Error::Output(Format::NonCanonical)));
        assert_eq!(module.read_stdout(success, b"{\"value\":7}\n", 12).err(), Some(Error::Output(Format::NonCanonical)));
        assert!(!pending.manifest_path().exists());
    }
    assert!(fs::read_dir(scratch).unwrap().next().is_none());
}
