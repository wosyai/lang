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
