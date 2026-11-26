//! A typed single-result/root fixture. No identifier here is a wire or
//! storage format, an executable attestation, or production cache authority.
use super::{Fixture, KNOWN_WASIP1_REPORT, MANIFEST};
use std::{fs, os::unix::{fs::DirBuilderExt, process::ExitStatusExt}, process::ExitStatus};
use wosy_cli::{
    comptime_fragment::{ComptimeReadbackError, StagedWasip1ToolPair},
    source_preparation::{prepare_wasip1_comptime_inputs, Wasip1ComptimeInputSelection},
};
use wosy_compiler::{ComptimeStdoutError, ScalarExpression};
use wosy_project::packages::{ComptimeWatchLimits, WatchRoot};

const STDOUT: &[u8] = "[\"olá\\n\\t\\r\\\"ok\\\\\\u0000\",[7,14]]\n".as_bytes();
const MAX_STDOUT: usize = 256;

/// Private synthetic witness. No Rust default, path, tool version label
/// or caller-supplied compiler AST can make a real production completion.
#[derive(Clone, Copy)]
struct CompletedTools {
    builder: blake3::Hash,
    runner: blake3::Hash,
    image: blake3::Hash,
}

impl CompletedTools {
    fn baseline() -> Self {
        Self { builder: blake3::hash(b"fixture compiled-image builder"),
            runner: blake3::hash(b"fixture isolated-image runner"),
            image: blake3::hash(b"fixture immutable closed image; WASIp1; no capabilities") }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FixtureInputId(blake3::Hash);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FixtureResultId(blake3::Hash);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FixtureRootId(blake3::Hash);

/// The only leaf admitted here is a closed root initializer: there is no
/// call tuple, parameter read or build capability. The exact site+IR and
/// selected scope come from the held compiler output, not the witness.
struct ResultRootFixture {
    stage: StagedWasip1ToolPair,
    input: FixtureInputId,
    result: FixtureResultId,
    root: FixtureRootId,
    value: ScalarExpression,
}

impl ResultRootFixture {
    fn complete(
        stage: StagedWasip1ToolPair, tools: CompletedTools, status: ExitStatus, stdout: &[u8],
    ) -> Result<Self, ComptimeReadbackError> {
        if !status.success() { return Err(ComptimeReadbackError::Exit(status.code())); }
        let provision = stage.provision();
        let retained = provision.preparation();
        let prepared = retained.continuation().evaluator().output();
        let value = prepared.parse_stdout(stdout, MAX_STDOUT).map_err(ComptimeReadbackError::Output)?;
        let site = prepared.site();
        let observed = retained.watches();
        let mut input = blake3::Hasher::new_derive_key("wosy host fixture closed comptime input");
        for field in [
            site.source.project.as_bytes(), site.source.package.as_bytes(), site.source.path.as_bytes(),
            site.source.revision.as_bytes(), observed.target.as_bytes(), observed.profile.as_bytes(),
            observed.id.as_bytes(),
        ] { frame(&mut input, field); }
        input.update(&site.range.start.to_be_bytes()).update(&site.range.end.to_be_bytes());
        input.update(provision.evaluator_ir().blake3.as_bytes());
        input.update(tools.builder.as_bytes()).update(tools.runner.as_bytes()).update(tools.image.as_bytes());
        input.update(&(observed.files.len() as u64).to_be_bytes());
        // The existing source watcher already returns exact sorted,
        // role-qualified content identities. Never re-read or re-sort here.
        for watched in &observed.files {
            let role = match watched.root { WatchRoot::OwningPackage => b"owner", WatchRoot::ConsumingProject => b"input" };
            input.update(role);
            frame(&mut input, watched.path.as_path().as_os_str().as_encoded_bytes());
            frame(&mut input, watched.revision.0.as_bytes());
        }
        let input = input.finalize();
        // Values have one existing expected-type-directed parse; identity
        // uses that same canonical byte stream without re-encoding the AST.
        let result = blake3::Hasher::new_derive_key("wosy host fixture comptime result")
            .update(input.as_bytes()).update(blake3::hash(stdout).as_bytes()).finalize();
        let root = blake3::Hasher::new_derive_key("wosy host fixture comptime root with one result")
            .update(&retained.source_root().0).update(result.as_bytes()).finalize();
        Ok(Self { stage, input: FixtureInputId(input), result: FixtureResultId(result), root: FixtureRootId(root), value })
    }
}

fn frame(hasher: &mut blake3::Hasher, field: &[u8]) {
    hasher.update(&(field.len() as u64).to_be_bytes()).update(field);
}

struct ProjectFixture(Fixture);

impl ProjectFixture {
    fn new() -> Self {
        let fixture = Fixture::new();
        let root = fixture.project();
        let std = fixture.0.join("std");
        fs::create_dir_all(std.join("src/wasi")).unwrap();
        for (path, content) in [
            ("wosy.toml", include_str!("../../../../stdlib/wosy.toml")),
            ("src/bootstrap.w", include_str!("../../../../stdlib/src/bootstrap.w")),
            ("src/wasi/preview1.w", include_str!("../../../../stdlib/src/wasi/preview1.w")),
        ] { fs::write(std.join(path), content).unwrap(); }
        fs::DirBuilder::new().mode(0o700).create(fixture.0.join("scratch")).unwrap();
        fs::create_dir(root.join("assets")).unwrap();
        fs::write(root.join("src/main.w"), KNOWN_WASIP1_REPORT).unwrap();
        fixture.manifest(&format!("{}{}", MANIFEST.replace("output = \"native\"", "output = \"wasm-wasip1\""), r#"
[dependencies.std]
path = "../std"
[comptime.prepare]
watch = ["assets/*.txt"]
[targets.app.artifacts.main.profiles.second]
backend = "llvm"
output = "wasm-wasip1"
builder = "ordinary"
run_runner = "ordinary"
[targets.app.profiles.second]
main_artifact = "main"
comptime_builder = "compile"
comptime_runner = "evaluate"
[targets.second]
root = "src/main.w"
[targets.second.artifacts.main]
artifact = "Main"
[targets.second.artifacts.main.profiles.defaults]
backend = "llvm"
output = "wasm-wasip1"
builder = "ordinary"
run_runner = "ordinary"
[targets.second.profiles.defaults]
main_artifact = "main"
comptime_builder = "compile"
comptime_runner = "evaluate"
"#));
        let fixture = Self(fixture);
        fixture.watch(b"first");
        fixture
    }

    fn watch(&self, bytes: &[u8]) {
        fs::write(self.0.project().join("assets/selected.txt"), bytes).unwrap();
    }

    fn stage(&self, target: &str, profile: &str) -> StagedWasip1ToolPair {
        let (sources, prep) = prepare_wasip1_comptime_inputs(&self.0.project(), Wasip1ComptimeInputSelection {
            target, profile, id: "prepare", max_authored_source_bytes: 1 << 17,
            max_stdout_bytes: MAX_STDOUT, watch_limits: ComptimeWatchLimits::default(),
        }).unwrap();
        assert_eq!(sources.root, prep.source_root());
        assert_eq!(prep.watches().files.len(), 1);
        prep.provision(&self.0.0.join("scratch"), 1 << 16).unwrap().stage_tool_pair().unwrap()
    }

    fn complete(&self, target: &str, profile: &str) -> ResultRootFixture {
        complete(self.stage(target, profile), CompletedTools::baseline(), STDOUT)
    }
}

fn complete(stage: StagedWasip1ToolPair, tools: CompletedTools, bytes: &[u8]) -> ResultRootFixture {
    ResultRootFixture::complete(stage, tools, ExitStatus::from_raw(0), bytes).unwrap()
}

#[test]
fn result_root_fixture_identifies_equal_inputs_and_discriminates_real_watch_edits() {
    let fixture = ProjectFixture::new();
    let cold = fixture.complete("app", "defaults");
    let warm = fixture.complete("app", "defaults");
    assert_eq!((warm.input, warm.result, warm.root), (cold.input, cold.result, cold.root));
    fixture.watch(b"changed content");
    let changed = fixture.complete("app", "defaults");
    assert_eq!(changed.stage.provision().preparation().source_root(), cold.stage.provision().preparation().source_root());
    assert_eq!(changed.stage.provision().evaluator_ir(), cold.stage.provision().evaluator_ir());
    assert_ne!(changed.input, cold.input);
    assert_ne!(changed.result, cold.result);
    assert_ne!(changed.root, cold.root);
    fixture.watch(b"first");
    let restored = fixture.complete("app", "defaults");
    assert_eq!((restored.input, restored.result, restored.root), (cold.input, cold.result, cold.root));
}

#[test]
fn result_root_fixture_binds_actual_target_profile_and_each_completed_tool_role() {
    let fixture = ProjectFixture::new();
    let expected = fixture.complete("app", "defaults");
    let target = fixture.complete("second", "defaults");
    let profile = fixture.complete("app", "second");
    for alternative in [&target,&profile] {
        assert_eq!(alternative.stage.provision().evaluator_ir(), expected.stage.provision().evaluator_ir());
        assert_eq!(alternative.stage.provision().preparation().watches().files,
            expected.stage.provision().preparation().watches().files);
        assert_ne!(alternative.input, expected.input);
        assert_ne!(alternative.result, expected.result);
        assert_ne!(alternative.root, expected.root);
    }
    assert_eq!(profile.stage.provision().preparation().source_root(), expected.stage.provision().preparation().source_root());
    let tools = CompletedTools::baseline();
    for changed_tools in [
        CompletedTools { builder: blake3::hash(b"changed fixture builder"), ..tools },
        CompletedTools { runner: blake3::hash(b"changed fixture runner"), ..tools },
        CompletedTools { image: blake3::hash(b"changed fixture image"), ..tools },
    ] {
        let alternative = complete(fixture.stage("app", "defaults"), changed_tools, STDOUT);
        assert_ne!(alternative.input, expected.input);
        assert_ne!(alternative.result, expected.result);
    }
}

#[test]
fn result_root_fixture_only_accepts_the_retained_type_and_distinguishes_different_values() {
    let fixture = ProjectFixture::new();
    let tools = CompletedTools::baseline();
    let expected = fixture.complete("app", "defaults");
    let ScalarExpression::StructLiteral { fields, .. } = &expected.value else { panic!("expected typed Report") };
    let [label, buckets] = &fields[..] else { panic!("expected two typed fields") };
    assert_eq!((label.name.as_str(),buckets.name.as_str()), ("label","buckets"));
    assert!(matches!(&label.value, ScalarExpression::Utf8{value,..} if value == "olá\n\t\r\"ok\\\0".as_bytes()));
    assert!(matches!(&buckets.value, ScalarExpression::ArrayLiteral { elements, .. }
        if matches!(&elements[..], [ScalarExpression::Integer { value: a, .. }, ScalarExpression::Integer { value: b, .. }]
            if i64::try_from(a) == Ok(7) && i64::try_from(b) == Ok(14))));
    // Changed typed output is an explicit nondeterminism control, not
    // a claim that the known evaluator produces a different result.
    let alternate = complete(fixture.stage("app", "defaults"), tools,
        "[\"olá\\n\\t\\r\\\"ok\\\\\\u0000\",[7,15]]\n".as_bytes());
    assert_eq!(alternate.input, expected.input);
    assert_ne!(alternate.value, expected.value);
    assert_ne!(alternate.result, expected.result);
    assert_ne!(alternate.root, expected.root);
    assert!(matches!(ResultRootFixture::complete(fixture.stage("app", "defaults"), tools,
        ExitStatus::from_raw(0), b"14\n"), Err(ComptimeReadbackError::Output(ComptimeStdoutError::NonCanonical))));
    assert!(matches!(ResultRootFixture::complete(fixture.stage("app", "defaults"), tools,
        ExitStatus::from_raw(1 << 8), STDOUT), Err(ComptimeReadbackError::Exit(Some(1)))));
}
