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
      .numbers = [10, 20]; .label = "olá\n\"ok\\x\0"; }
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
        let from_source = "[\"olá\\n\\\"ok\\\\x\\u0000\",[10,20],[[true,-7],[false,0]]]\n".as_bytes();
        let mut produced = Vec::new();
        assert_eq!(prepared.write_literal_stdout(&mut produced, from_source.len()).unwrap(), from_source.len());
        assert_eq!(produced, from_source);
        let Expr::StructLiteral { fields: produced_fields, .. } =
            prepared.read_stdout(ok, &produced, produced.len()).unwrap()
        else { panic!("the actual produced result is one struct"); };
        assert!(matches!(&produced_fields[0].value, Expr::Utf8 { value, .. }
            if value == "olá\n\"ok\\x\0".as_bytes()));
        let Expr::ArrayLiteral { elements: produced_cells, .. } = &produced_fields[2].value
        else { panic!("actual produced Cell array"); };
        let Expr::StructLiteral { fields: produced_first, .. } = &produced_cells[0]
        else { panic!("actual produced Cell"); };
        assert!(matches!(&produced_first[1].value, Expr::Integer { value, .. }
            if i64::try_from(value) == Ok(-7)));
        let mut partial = Vec::new();
        assert_eq!(prepared.write_literal_stdout(&mut partial, from_source.len() - 1).unwrap_err().to_string(),
            "comptime stdout exceeds its byte budget");
        assert_eq!(partial, &from_source[..from_source.len() - 1]);

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
        let mut scalar_line = Vec::new();
        assert_eq!(scalar.write_literal_stdout(&mut scalar_line, 5).unwrap(), 5);
        assert_eq!(scalar_line, b"true\n");
        assert!(matches!(scalar.read_stdout(ok, &scalar_line, 5).unwrap(), Expr::Boolean { value: true, .. }));
        assert_eq!(scalar.read_stdout(ok, example, 256).err(), Some(Error::Output(Format::NonCanonical)));
        assert!(prepared.read_stdout(ok, example, 256).is_ok());
    }
    assert!(!project.join(".wosy").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn checked_literal_producer_does_not_guess_computed_results() {
    use wosy_cli::comptime_fragment::prepare_project_comptime_stdout;
    let fixture = Fixture::new();
    let project = fixture.project();
    let source = project.join("src/main.w");
    for body in ["7 ^ 9", "u32 selected = 43; selected"] {
        fs::write(&source, format!("%%start\nu32 answer = comptime prepare {{ {body} }};\n%%end\n")).unwrap();
        let candidate = prepare_project_comptime_stdout(
            &project, "app", "override", "prepare", ScalarTargetLayout::NATIVE64,
        );
        let candidate = candidate.unwrap();
        let mut bytes = Vec::new();
        let refusal = candidate.write_literal_stdout(&mut bytes, 32).unwrap_err();
        assert_eq!(refusal.kind(), io::ErrorKind::Unsupported);
        assert!(bytes.is_empty(), "a computed result must not emit a partial literal");
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

#[cfg(target_os = "linux")]
#[test]
fn emits_the_selected_computed_aggregate_using_ordinary_return_abi() {
    use std::{os::unix::process::ExitStatusExt, process::ExitStatus};
    use wosy_cli::comptime_fragment::prepare_project_comptime_stdout_evaluator;
    use wosy_compiler::{ClosedComptimeRejection, ScalarExpression as Expr};

    let fixture = Fixture::new();
    let project = fixture.project();
    let source = project.join("src/main.w");
    let text = r#"%%start
# WHY: computed authored carrier ☕
struct copy Pair { u32 first; u32 second; }
struct copy Report { Pair pair; u32[2] buckets; }
u32 ambient_value = 91;
u32() ambient_function = fn { ambient_value };
Report result = comptime prepare {
    u32 base = 7;
    u32 changed = base ^ 9;
    { .buckets = [changed, base]; .pair = { .second = changed; .first = base; }; }
};
%%end
"#;
    let plan = |layout| prepare_project_comptime_stdout_evaluator(&project, "app", "override", "prepare", layout);
    let ok = ExitStatus::from_raw(0);
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        fs::write(&source, text).unwrap();
        let evaluator = plan(layout).unwrap_or_else(|error| panic!("computed result {layout:?}: {error:?}"));
        assert_eq!(evaluator.layout(), layout);
        assert!(matches!(evaluator.result_type(), ScalarType::Struct(_)));
        assert!(text[evaluator.site().range.start as usize..].starts_with("comptime prepare"));
        let mut bytes = Vec::new();
        let receipt = evaluator.write_evaluator_ir(&mut bytes, 1 << 16).unwrap();
        assert_eq!(receipt.bytes, bytes.len());
        assert_eq!(receipt.blake3, blake3::hash(&bytes));
        let ir = std::str::from_utf8(&bytes).unwrap();
        let body = ir.split_once("define [16 x i8] @evaluate(").expect("ordinary copy aggregate result")
            .1.split_once("\n}").unwrap().0;
        assert!(body.contains("xor i32"), "{body}");
        assert!(body.contains("ret [16 x i8] "), "{body}");
        assert!(!ir.contains("@ambient_function") && !ir.contains("@ambient_value"), "{ir}");
        assert!(!ir.contains("@printf"));
        let (ir_path, modules, manifest) =
            (fixture.0.join("evaluator.ll"), fixture.0.join("modules"), fixture.0.join("manifest"));
        let builder = evaluator.builder(ClosedComptimePaths { evaluator_ir: &ir_path, output_dir: &modules, manifest: &manifest });
        let runner = evaluator.runner(&manifest);
        assert_eq!(builder.executable, OsStr::new("path to declared comptime builder"));
        assert_eq!(runner.executable, OsStr::new("path to declared comptime runner"));
        assert_eq!(&builder.arguments[builder.arguments.len() - 3..], [ir_path.as_os_str(), modules.as_os_str(), manifest.as_os_str()]);
        assert_eq!(runner.arguments.last().unwrap(), &manifest.as_os_str());
        assert!(!manifest.exists() && !ir_path.exists() && !modules.exists());
        // The bridge has not run. Exercise the same emitted evaluator's
        // prepared shape with a synthetic result rather than reading ABI bytes.
        let stdout = b"[[7,14],[14,7]]\n";
        let Expr::StructLiteral { fields, .. } = evaluator.read_stdout(ok, stdout, 32).unwrap()
        else { panic!("same nominal output used by readback"); };
        assert_eq!(fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(), ["pair", "buckets"]);
        assert!(matches!(&fields[1].value, Expr::ArrayLiteral { elements, .. }
            if matches!(&elements[..], [Expr::Integer { value: first, .. }, Expr::Integer { value: second, .. }]
                if i64::try_from(first) == Ok(14) && i64::try_from(second) == Ok(7))));
        assert!(evaluator.read_stdout(ok, b"[[7,14],true]\n", 32).is_err());
    }


    // Import a genuine declaration through the ordinary source graph.
    // Multi-module lowering uses the existing qualified LLVM name, not
    // a duplicate hard-coded external ABI or a locally invented type.
    let types = fixture.0.join("types");
    fs::create_dir_all(types.join("src")).unwrap();
    fs::write(types.join("wosy.toml"), "[package]\nname=\"types\"\nsource_root=\"src\"\n[comments]\ncategories=[]\n").unwrap();
    fs::write(types.join("src/shape.w"),
        "%%start\nstruct copy Pair { u32 first; u32 second; }\nu32() dependency_only = fn { 1 };\n%%end\n").unwrap();
    fixture.manifest(&format!("{MANIFEST}\n[dependencies.types]\npath=\"../types\"\n"));
    let imported = text.replace("struct copy Pair { u32 first; u32 second; }", "types = namespace types \"shape.w\";")
        .replace("Pair pair;", "types.Pair pair;");
    fs::write(&source, imported).unwrap();
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let staged = plan(layout).unwrap_or_else(|error| panic!("imported computed result {layout:?}: {error:?}"));
        let mut text = Vec::new();
        staged.write_evaluator_ir(&mut text, 1 << 16).unwrap();
        let text = std::str::from_utf8(&text).unwrap();
        let entry = text.lines().find(|line| line.starts_with("define [16 x i8] @wosy_fn__")
            && line.contains("__6576616c75617465(")).expect("ordinary source-qualified evaluator symbol");
        assert!(entry.ends_with(" {"), "{entry}");
        assert!(!text.contains("__616d6269656e745f66756e6374696f6e(")
            && !text.contains("__646570656e64656e63795f6f6e6c79("));
    }

    fs::write(&source, text.replace("u32 base = 7;", "u32 base = ambient_function();")).unwrap();
    assert!(matches!(plan(ScalarTargetLayout::NATIVE64), Err(ClosedComptimePlanError::Fragment(
        ClosedComptimeRejection::Unsupported { feature: "not a closed authored scalar/aggregate evaluator body", .. }
    ))));
    assert!(!project.join(".wosy").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn computed_wasip1_stdout_has_one_ordinary_evaluation_and_only_preview1_stdout() {
    use std::{os::unix::process::ExitStatusExt, process::ExitStatus};
    use wosy_cli::comptime_fragment::prepare_project_comptime_wasip1_stdout_evaluator;
    use wosy_compiler::{ClosedComptimeRejection, ScalarExpression as Expr};

    let fixture = Fixture::new();
    let project = fixture.project();
    let source = project.join("src/main.w");
    let text = r#"%%start
struct copy Child { i32 delta; bool active; }
struct copy Report { Child state; u32[2] buckets; }
u32 unrelated = 99;
Report result = comptime prepare {
    i32 baseline = -7;
    i32 adjusted = baseline ^ 1;
    u32 low = 9;
    u32 high = low | 2;
    { .buckets = [high, low]; .state = { .active = true; .delta = adjusted; }; }
};
%%end
"#;
    fs::write(&source, text).unwrap();
    let select = |budget| prepare_project_comptime_wasip1_stdout_evaluator(
        &project, "app", "override", "prepare", 1 << 16, budget,
    );
    for manifest in [MANIFEST.to_owned(), MANIFEST.replace("output = \"native\"", "output = \"wasm-wasip2\"")] {
        fixture.manifest(&manifest);
        assert!(matches!(select(256), Err(ClosedComptimePlanError::Project(message))
            if message == "computed stdout stage requires an explicit llvm/wasm-wasip1 profile"));
    }
    fixture.manifest(&MANIFEST.replace("output = \"native\"", "output = \"wasm-wasip1\""));
    assert!(matches!(select(1), Err(ClosedComptimePlanError::Fragment(ClosedComptimeRejection::Emitter(message)))
        if message == "comptime stdout shape exceeds the selected byte budget"));
    let staged = select(256).unwrap_or_else(|error| panic!("preview1 computed stdout: {error:?}"));
    let mut bytes = Vec::new();
    let receipt = staged.write_evaluator_ir(&mut bytes, 1 << 16).unwrap();
    assert_eq!(receipt.blake3, blake3::hash(&bytes));
    let ir = std::str::from_utf8(&bytes).unwrap();
    let evaluator = ir.split_once("define [16 x i8] @evaluate(").unwrap().1.split_once("\n}").unwrap().0;
    assert!(evaluator.contains("xor i32") && evaluator.contains("or i32"), "{evaluator}");
    let main = ir.split_once("define i32 @main(").unwrap().1.split_once("\n}").unwrap().0;
    assert_eq!(main.matches("call [16 x i8] @evaluate(").count(), 1);
    assert!(main.contains("icmp slt i32") && main.contains("wire.array.index"));
    assert!(main.contains("call i32 @fd_write(i32 1,"));
    assert!(main.contains("stdout.progress") && main.contains("stdout.within") && main.contains("stdout.next.write"));
    assert!(main.contains("ret i32 1") && main.contains("ret i32 0"));
    assert!(ir.contains("\"wasm-import-module\"=\"wasi_snapshot_preview1\""));
    assert!(ir.contains("\"wasm-import-name\"=\"fd_write\""));
    assert!(!ir.contains("@unrelated") && !ir.contains("@printf"));
    // No evaluator has run: these bytes are a host oracle for the result
    // shape kept by the same emitted staging object.
    let expected = b"[[-8,true],[11,9]]\n";
    let Expr::StructLiteral { fields, .. } = staged.read_stdout(ExitStatus::from_raw(0), expected, 256).unwrap()
    else { panic!("ordinary struct stdout result"); };
    assert_eq!(fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(), ["state", "buckets"]);

    // A separate scalar uses the same compiled stdout entry without an
    // aggregate-specific extension or a different client grammar.
    fs::write(&source, "%%start\nu32 answer = comptime prepare { u32 seed = 7; seed ^ 9 };\n%%end\n").unwrap();
    assert!(matches!(select(10), Err(ClosedComptimePlanError::Fragment(ClosedComptimeRejection::Emitter(_)))));
    let scalar = select(11).unwrap();
    let mut bytes = Vec::new();
    scalar.write_evaluator_ir(&mut bytes, 1 << 16).unwrap();
    let scalar_ir = std::str::from_utf8(&bytes).unwrap();
    assert!(scalar_ir.contains("call i32 @evaluate()") && scalar_ir.contains("@fd_write(i32 1,"));
    assert!(matches!(scalar.read_stdout(ExitStatus::from_raw(0), b"14\n", 11).unwrap(),
        Expr::Integer { value, .. } if i64::try_from(&value) == Ok(14)));
    assert!(!project.join(".wosy").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn wasip1_stdout_captures_authored_text_from_real_standard_library() {
    use std::{os::unix::process::ExitStatusExt, process::ExitStatus};
    use wosy_cli::comptime_fragment::prepare_project_comptime_wasip1_stdout_evaluator;
    use wosy_compiler::{ClosedComptimeRejection, ScalarExpression as Expr};
    let fixture = Fixture::new();
    let project = fixture.project();
    let source = project.join("src/main.w");
    let standard = fixture.0.join("std");
    fs::create_dir_all(standard.join("src/wasi")).unwrap();
    for (path, text) in [
        ("wosy.toml", include_str!("../../../stdlib/wosy.toml")),
        ("src/bootstrap.w", include_str!("../../../stdlib/src/bootstrap.w")),
        ("src/wasi/preview1.w", include_str!("../../../stdlib/src/wasi/preview1.w")),
    ] {
        fs::write(standard.join(path), text).unwrap();
    }
    fixture.manifest(&format!("{}\n[dependencies.std]\npath=\"../std\"\n",
        MANIFEST.replace("output = \"native\"", "output = \"wasm-wasip1\"")));
    let assemble = |maximum| prepare_project_comptime_wasip1_stdout_evaluator(
        &project, "app", "override", "prepare", 1 << 17, maximum,
    );
    let ok = ExitStatus::from_raw(0);
    let aggregate = r#"%%start
std = namespace std "bootstrap.w";
struct Report { std.utf8 label; u32[2] buckets; }
Report result = comptime prepare {
    std.utf8 text = "olá\n\t\r\"ok\\\0";
    u32 base = 7;
    { .buckets = [base, base ^ 9]; .label = text; }
};
%%end
"#;
    let direct = r#"%%start
std = namespace std "bootstrap.w";
std.utf8 result = comptime prepare {
    std.utf8 first = "olá\n\t\r\"ok\\\0";
    first
};
%%end
"#;
    let arrays = r#"%%start
std = namespace std "bootstrap.w";
std.utf8[2] result = comptime prepare {
    std.utf8 first = "olá\n\t\r\"ok\\\0";
    std.utf8 second = "";
    [first, second]
};
%%end
"#;
    for (authored, expected, scalar) in [
        (aggregate, "[\"olá\\n\\t\\r\\\"ok\\\\\\u0000\",[7,14]]\n", Some("xor i32")),
        (direct, "\"olá\\n\\t\\r\\\"ok\\\\\\u0000\"\n", None),
        (arrays, "[\"olá\\n\\t\\r\\\"ok\\\\\\u0000\",\"\"]\n", None),
    ] {
        fs::write(&source, authored).unwrap();
        assert!(matches!(assemble(3), Err(ClosedComptimePlanError::Fragment(ClosedComptimeRejection::Emitter(message)))
            if message == "comptime stdout shape exceeds the selected byte budget"));
        let staged = assemble(256).unwrap_or_else(|error| panic!("selected authored WASI text: {error:?}\n{authored}"));
        assert_eq!(staged.layout(), ScalarTargetLayout::WASM32);
        let mut bytes = Vec::new();
        staged.write_evaluator_ir(&mut bytes, 1 << 16).unwrap();
        let ir = std::str::from_utf8(&bytes).unwrap();
        let entry = ir.lines().find(|line| line.starts_with("define [") && line.contains("__6576616c75617465("))
            .unwrap_or_else(|| panic!("missing by-value selected evaluator:\n{ir}"));
        let region = ir.split_once(entry).unwrap().1.split_once("\n}").unwrap().0;
        assert!(region.contains("ret [") && !region.contains("ret ptr"));
        if let Some(operation) = scalar { assert!(region.contains(operation), "{region}"); }
        let root = ir.split_once("define i32 @main(").unwrap().1.split_once("\n}").unwrap().0;
        assert_eq!(root.matches("__6576616c75617465(").count(), 1);
        assert!(root.contains("load i64") && root.contains("inttoptr i32") && root.contains("call void @__wosy_ct_quote_utf8"));
        let quoted = ir.split_once("define internal void @__wosy_ct_quote_utf8(").unwrap().1.split_once("\n}").unwrap().0;
        assert!(quoted.contains("switch i8") && quoted.contains("utf8.control") && quoted.contains("utf8.nibble.low"));
        assert!(root.contains("call i32 @fd_write(i32 1,"));
        assert!(!ir.contains("@fd_read") && !ir.contains("@printf"));
        // The emitted assembly has not executed. Exercise its nominal
        // compiler-selected result shape with an independent host oracle.
        let value = staged.read_stdout(ok, expected.as_bytes(), 256).unwrap();
        let actual_text = match value {
            Expr::Utf8 { value, .. } => value,
            Expr::StructLiteral { fields, .. } => match &fields[0].value {
                Expr::Utf8 { value, .. } => value.clone(),
                _ => panic!("ordinary UTF-8 field"),
            },
            Expr::ArrayLiteral { elements, .. } => match &elements[..] {
                [Expr::Utf8 { value, .. }, Expr::Utf8 { value: empty, .. }] if empty.is_empty() => value.clone(),
                _ => panic!("ordinary fixed pair of UTF-8 values"),
            },
            _ => panic!("ordinary declared text result"),
        };
        assert_eq!(actual_text, "olá\n\t\r\"ok\\\0".as_bytes());
    }
    // The real std bootstrap and this valid regular module each fit the
    // caller's cap alone. Their combined bytes exceed it before parsing the
    // auxiliary module, which follows bootstrap in ordinary discovery order.
    fs::write(standard.join("src/extra.w"),
        format!("%%start\nstruct Extra {{ bool ok; }}\n{}\n%%end\n", " ".repeat(1 << 16))).unwrap();
    let namespace = "std = namespace std \"bootstrap.w\";\n";
    fs::write(&source, direct.replacen(namespace,
        &format!("{namespace}extra = namespace std \"extra.w\";\n"), 1)).unwrap();
    let oversized = assemble(256);
    assert!(matches!(&oversized, Err(ClosedComptimePlanError::Project(message))
        if message == "preview source exceeds the authored byte budget"),
        "aggregate source budget control returned {:?}", oversized.err());
    assert!(!project.join(".wosy").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn retained_wasip1_watch_inputs_distinguish_source_and_watch_changes() {
    use std::time::Duration;
    use tokio::time::Instant;
    use wosy_cli::source_preparation::{prepare, prepare_wasip1_comptime_inputs,
        ComptimeInputsFreshness::{SourceOrConfigurationChanged, Unchanged, WatchesChanged},
        Wasip1ComptimeInputSelection};
    use wosy_cli::comptime_fragment::prepare_project_comptime_wasip1_stdout_evaluator;
    use wosy_project::packages::ComptimeWatchLimits;
    let fixture = Fixture::new();
    let project = fixture.project();
    let manifest = format!("{}\n[comptime.prepare]\nwatch = [\"assets/*.txt\"]\n",
        MANIFEST.replace("output = \"native\"", "output = \"wasm-wasip1\""));
    fixture.manifest(&manifest);
    let src = project.join("src/main.w");
    let text = format!("%%start\nu32 value = comptime prepare {{ u32 seed = 7; seed ^ 9 }};\n{}\n%%end\n",
        " ".repeat(1 << 16));
    fs::write(&src, &text).unwrap();
    let assets = project.join("assets");
    fs::create_dir(&assets).unwrap();
    let watched = assets.join("input.txt");
    fs::write(&watched, "first").unwrap();
    let selection = |max_authored_source_bytes| Wasip1ComptimeInputSelection {
        target: "app", profile: "override", id: "prepare", max_authored_source_bytes,
        max_stdout_bytes: 256, watch_limits: ComptimeWatchLimits::default(),
    };
    let denied = prepare_wasip1_comptime_inputs(&project, selection(text.len() - 1)).err().unwrap();
    assert!(denied.to_string().contains("authored byte budget"));
    let (prepared, inputs) = prepare_wasip1_comptime_inputs(&project, selection(text.len())).unwrap();
    let ordinary = prepare(&project, Some("app"), None).unwrap();
    assert_eq!((prepared.parsed, prepared.reused), (1, 0));
    assert_eq!(prepared.root, inputs.source_root());
    assert_eq!(prepared.root, ordinary.root);
    assert_eq!(prepared.delta.nodes, ordinary.delta.nodes);
    assert_eq!(inputs.continuation().authored_sources().count(), 1);
    assert_eq!((inputs.declared_tools().target(), inputs.declared_tools().profile()), ("app", "override"));
    assert_eq!(inputs.watches().files.len(), 1);
    let mut captured = Vec::new();
    let observed = inputs.write_evaluator_ir(&mut captured, 1 << 16).unwrap();
    let mut direct = Vec::new();
    let observe = |limit| prepare_project_comptime_wasip1_stdout_evaluator(
        &project, "app", "override", "prepare", limit, 256);
    assert!(matches!(observe(text.len() - 1), Err(ClosedComptimePlanError::Project(message))
        if message.contains("authored byte budget")));
    let single = observe(text.len()).unwrap();
    assert_eq!(single.write_evaluator_ir(&mut direct, 1 << 16).unwrap(), observed);
    assert_eq!(captured, direct);

    let current = || inputs.observe_current(Instant::now()+Duration::from_secs(5)).unwrap();
    assert_eq!(current(), Unchanged);
    fs::write(assets.join("irrelevant.bin"), "irrelevant bytes").unwrap();
    assert_eq!(current(), Unchanged);
    fs::write(&watched, "second").unwrap();
    assert_eq!(current(), WatchesChanged);
    fs::write(&watched, "first").unwrap();
    assert_eq!(current(), Unchanged);
    let added = assets.join("added.txt");
    fs::write(&added, "additional match").unwrap();
    assert_eq!(current(), WatchesChanged);
    fs::remove_file(added).unwrap();
    assert_eq!(current(), Unchanged);
    fs::remove_file(&watched).unwrap();
    assert_eq!(current(), WatchesChanged);
    fs::write(&watched, "first").unwrap();
    assert_eq!(current(), Unchanged);

    // Do not attempt to parse a changed live revision to decide freshness.
    fs::write(&src, "not a Wosy module").unwrap();
    assert_eq!(current(), SourceOrConfigurationChanged);
    fs::write(&src, &text).unwrap();
    assert_eq!(current(), Unchanged);
    fixture.manifest(&manifest.replace("selected evaluator toolchain", "changed tool declaration"));
    assert_eq!(current(), SourceOrConfigurationChanged);
    assert!(!project.join(".wosy").exists());
}


#[cfg(target_os = "linux")]
#[test]
fn owned_retained_wasip1_handoff_uses_the_selected_preparation_and_tool_paths() {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use wosy_cli::source_preparation::{prepare_wasip1_comptime_inputs, Wasip1ComptimeInputSelection};
    use wosy_project::packages::ComptimeWatchLimits;

    let fixture = Fixture::new();
    let project = fixture.project().canonicalize().unwrap();
    let scratch = fixture.0.join("private evaluator handoff");
    fs::DirBuilder::new().mode(0o700).create(&scratch).unwrap();
    fixture.manifest(&MANIFEST.replace("output = \"native\"", "output = \"wasm-wasip1\""));
    fs::write(project.join("src/main.w"), "%%start\nu32 value = comptime prepare { u32 seed = 7; seed ^ 9 };\n%%end\n").unwrap();
    let selection = Wasip1ComptimeInputSelection {
        target: "app", profile: "override", id: "prepare", max_authored_source_bytes: 1 << 17,
        max_stdout_bytes: 256, watch_limits: ComptimeWatchLimits::default(),
    };
    let (_, insufficient) = prepare_wasip1_comptime_inputs(&project, selection).unwrap();
    assert!(insufficient.provision(&scratch, 1).is_err());
    assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);

    let (sources, inputs) = prepare_wasip1_comptime_inputs(&project, selection).unwrap();
    assert_eq!(sources.parsed, 1);
    let expected = inputs.continuation().evaluator().llvm().to_text();
    let staged = inputs.provision(&scratch, expected.len()).unwrap();
    assert_eq!(staged.preparation().source_root(), sources.root);
    assert_eq!(staged.evaluator_ir().blake3, blake3::hash(expected.as_bytes()));
    assert_eq!(staged.evaluator_ir().bytes, expected.len());
    assert_eq!(staged.preparation().declared_tools().build_permissions(), Some([].as_slice()));
    assert!(staged.preparation().declared_tools().build_mounts().is_some_and(<[_]>::is_empty));

    let module_path = staged.expected_module_path();
    let output = module_path.parent().unwrap();
    let ir_path = output.join("evaluator.ll");
    let manifest = staged.manifest_path();
    assert_eq!(fs::metadata(output).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(fs::read(&ir_path).unwrap(), expected.as_bytes());
    let builder = staged.builder_invocation();
    assert_eq!(builder.executable, OsStr::new("path to declared comptime builder"));
    assert_eq!(builder.working_directory, project);
    assert_eq!(builder.arguments, [
        OsStr::new("literal *"), OsStr::new("$WORD"), OsStr::new("--label=two words"),
        ir_path.as_os_str(), output.as_os_str(), manifest.as_os_str(),
    ]);
    let runner = staged.runner_invocation();
    assert_eq!(runner.executable, OsStr::new("path to declared comptime runner"));
    assert_eq!(runner.working_directory, project);
    assert_eq!(runner.arguments, [OsStr::new("literal runner"), OsStr::new("--literal=${VAR}"), manifest.as_os_str()]);
    assert!(!manifest.exists());
    assert!(matches!(staged.observe_module(48), Err(error) if error.kind() == io::ErrorKind::NotFound));

    // Dummy data only: a bounded observation of the exact slot, not a
    // builder/runner receipt, execution evidence, or authority to resume.
    let dummy = b"inert evaluator slot fixture";
    fs::write(module_path, dummy).unwrap();
    assert!(staged.observe_module(dummy.len() as u64 - 1).is_err());
    let module = staged.observe_module(dummy.len() as u64).unwrap();
    assert!(std::ptr::eq(module.provision(), &staged));
    assert_eq!(module.result_type(), &ScalarType::U32);
    assert_eq!(module.bytes(), dummy.len() as u64);
    assert_eq!(module.digest(), blake3::hash(dummy));

    let output = output.to_owned();
    drop(staged);
    assert!(!output.exists() && !project.join(".wosy").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn private_wasip1_pair_stages_the_actual_inspected_evaluator_and_resolved_manifest() {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use wosy_cli::source_preparation::{prepare_wasip1_comptime_inputs, Wasip1ComptimeInputSelection};
    use wosy_project::packages::ComptimeWatchLimits;

    let fixture = Fixture::new();
    let project = fixture.project();
    let scratch = fixture.0.join("compiler scratch");
    fs::DirBuilder::new().mode(0o700).create(&scratch).unwrap();
    let driver = fixture.0.join("known_wasip1_pair.py");
    fs::write(&driver, include_str!("fixtures/known_wasip1_pair.py")).unwrap();
    fs::write(fixture.0.join("wasip1_pair_template.py"), include_str!("fixtures/wasip1_pair_template.py")).unwrap();
    let toolchain = fixture.0.join("known host image");
    // A dry plan needs no external executables and does not run Python.
    assert!(!toolchain.exists());
    let std = fixture.0.join("std");
    fs::create_dir_all(std.join("src/wasi")).unwrap();
    for (path, content) in [
        ("wosy.toml", include_str!("../../../stdlib/wosy.toml")),
        ("src/bootstrap.w", include_str!("../../../stdlib/src/bootstrap.w")),
        ("src/wasi/preview1.w", include_str!("../../../stdlib/src/wasi/preview1.w")),
    ] { fs::write(std.join(path), content).unwrap(); }
    let source = project.join("src/main.w");
    fs::write(&source, KNOWN_WASIP1_REPORT).unwrap();
    let manifest = format!("{}\n[dependencies.std]\npath=\"../std\"\n",
        MANIFEST.replace("output = \"native\"", "output = \"wasm-wasip1\"")
            .replace("command = [\"path to declared comptime builder\", \"literal *\", \"$WORD\"]\nargs = [\"--label=two words\"]",
                &format!("command = {:?}", ["/usr/bin/python3", "-I", driver.to_str().unwrap(), "build", toolchain.to_str().unwrap()]))
            .replace("selected evaluator toolchain", "private pinned fixture toolchain")
            .replace("command = [\"path to declared comptime runner\", \"literal runner\"]\nargs = [\"--literal=${VAR}\"]",
                &format!("command = {:?}", ["/usr/bin/python3", "-I", driver.to_str().unwrap(), "run", toolchain.to_str().unwrap()])));
    fixture.manifest(&manifest);
    let selection = |profile| Wasip1ComptimeInputSelection {
        target: "app", profile, id: "prepare", max_authored_source_bytes: 1 << 17,
        max_stdout_bytes: 256, watch_limits: ComptimeWatchLimits::default(),
    };
    let stage = |profile| {
        let (_, inputs) = prepare_wasip1_comptime_inputs(&project, selection(profile)).unwrap();
        inputs.provision(&scratch, 1 << 16).unwrap()
    };
    let error = stage("override").stage_tool_pair().err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);

    let pending = stage("defaults");
    // The known artifact hash selects this test only. The library path
    // admits any supported preparation without a fixture allowlist.
    assert_eq!(pending.evaluator_ir().bytes, 13981);
    assert_eq!(pending.evaluator_ir().blake3.to_hex().as_str(),
        "3b6bb4bdd9a842a20ce21b205e3250c40150922b119bd9970328ecc041aa3d61");
    let staged = pending.stage_tool_pair().unwrap();
    let p = staged.provision();
    let manifest = p.manifest_path();
    let output = manifest.parent().unwrap();
    let ir = output.join("evaluator.ll");
    assert_eq!(fs::metadata(manifest).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&fs::read(manifest).unwrap()).unwrap(),
        serde_json::json!({"executable": p.expected_module_path(), "parameters": {}, "permissions": [], "mounts": []}));
    assert_eq!(staged.builder().arguments, [OsStr::new("-I"), driver.as_os_str(), OsStr::new("build"),
        toolchain.as_os_str(), ir.as_os_str(), output.as_os_str(), manifest.as_os_str()]);
    assert_eq!(staged.runner().arguments, [OsStr::new("-I"), driver.as_os_str(), OsStr::new("run"),
        toolchain.as_os_str(), manifest.as_os_str()]);
    assert!(!p.expected_module_path().exists() && !project.join(".wosy").exists());
    drop(staged);
    assert_eq!(fs::read_dir(scratch).unwrap().count(), 0);
}

#[cfg(target_os = "linux")]
const KNOWN_WASIP1_REPORT: &str = r#"%%start
std = namespace std "bootstrap.w";
struct Report { std.utf8 label; u32[2] buckets; }
Report result = comptime prepare {
    std.utf8 text = "olá\n\t\r\"ok\\\0";
    u32 base = 7;
    { .buckets = [base, base ^ 9]; .label = text; }
};
%%end
"#;

#[cfg(target_os = "linux")]
#[path = "closed_comptime_staging/result_root_fixture.rs"]
mod result_root_fixture;
