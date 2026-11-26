use super::{BuilderConfig, ProjectConfiguration, RunnerConfig};
use std::path::Path;
use toml_edit::Item;

/// The consuming profile's two declared evaluator tools, observed from
/// one ordinary project manifest read. Not an executable identity, a
/// capability grant, or a full build configuration.
#[derive(Debug)]
pub struct SelectedComptimeTools {
    target: String,
    profile: String,
    builder_name: String,
    builder: BuilderConfig,
    runner_name: String,
    runner: RunnerConfig,
}

impl SelectedComptimeTools {
    /// Resolve only an explicitly selected profile. Legacy profiles with
    /// neither declaration have no evaluator tools. An artifact's runner
    /// and any dependency's tools are not candidates.
    pub fn load(root: &Path, target: &str, profile: &str) -> Result<Option<Self>, String> {
        let (mut configuration, document) = ProjectConfiguration::load_with_manifest(root)?;
        configuration.target_profile(target, profile)?;
        let authored = document.get("targets").and_then(Item::as_table)
            .and_then(|targets| targets.get(target)).and_then(Item::as_table)
            .and_then(|target| target.get("profiles")).and_then(Item::as_table)
            .and_then(|profiles| profiles.get(profile)).and_then(Item::as_table)
            .ok_or_else(|| format!("target {target} profile {profile} is missing from its manifest"))?;
        let (builder, runner) = match (authored.get("comptime_builder"), authored.get("comptime_runner")) {
            (None, None) => return Ok(None),
            (Some(builder), Some(runner)) => (builder, runner),
            _ => return Err(format!(
                "target {target} profile {profile} must declare both comptime_builder and comptime_runner"
            )),
        };
        let builder_name = builder.as_str().ok_or_else(|| format!(
            "target {target} profile {profile} comptime_builder must be a string"
        ))?;
        let runner_name = runner.as_str().ok_or_else(|| format!(
            "target {target} profile {profile} comptime_runner must be a string"
        ))?;
        let builder = configuration.builders.remove(builder_name)
            .ok_or_else(|| format!("comptime builder {builder_name} is missing"))?;
        let runner = configuration.runners.remove(runner_name)
            .ok_or_else(|| format!("comptime runner {runner_name} is missing"))?;
        for (kind, name, command) in [
            ("comptime builder", builder_name, &builder.command),
            ("comptime runner", runner_name, &runner.command),
        ] {
            if command.first().is_none_or(String::is_empty) {
                return Err(format!("{kind} {name} has no executable command"));
            }
        }
        Ok(Some(Self {
            target: target.into(),
            profile: profile.into(),
            builder_name: builder_name.into(),
            builder,
            runner_name: runner_name.into(),
            runner,
        }))
    }

    pub fn target(&self) -> &str { &self.target }
    pub fn profile(&self) -> &str { &self.profile }
    pub fn builder(&self) -> (&str, &BuilderConfig) { (&self.builder_name, &self.builder) }
    pub fn runner(&self) -> (&str, &RunnerConfig) { (&self.runner_name, &self.runner) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    const MANIFEST: &str = r#"
        [package]
        name = "consumer"
        default_target = "app"

        [dependencies.provider]
        path = "../provider"

        [builders.artifact]
        command = ["scripts/artifact"]
        toolchain_identity = "output-toolchain"

        [builders.eval]
        command = ["scripts/eval", "two words"]
        args = ["literal *", "--a=b"]
        toolchain_identity = "declared-eval-toolchain"

        [builders.special]
        command = ["scripts/special"]
        toolchain_identity = "declared-special-toolchain"

        [runners.runtime]
        command = ["scripts/runtime"]
        args = ["runtime-only"]

        [runners.evaluate]
        command = ["scripts/evaluate", "runner prefix"]
        args = ["literal ${input}"]

        [runners.alt]
        command = ["scripts/alternative"]

        [targets.app]
        root = "src/main.w"

        [targets.app.artifacts.main]
        artifact = "Main"

        [targets.app.artifacts.main.profiles.dev]
        backend = "llvm"
        output = "native"
        builder = "artifact"
        run_runner = "runtime"

        [targets.app.artifacts.main.profiles.wasm]
        backend = "llvm"
        output = "wasm-wasip1"
        builder = "artifact"
        run_runner = "runtime"

        [targets.app.profiles.dev]
        main_artifact = "main"
        comptime_builder = "eval"
        comptime_runner = "evaluate"

        [targets.app.profiles.wasm]
        main_artifact = "main"
        comptime_builder = "special"
        comptime_runner = "alt"

        [targets.helper]
        root = "src/helper.w"
        [targets.helper.artifacts]
        [targets.helper.profiles.dev]
        main_artifact = "ignored"
    "#;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let base = std::env::temp_dir().join(format!("wosy-comptime-tools-{}", uuid::Uuid::now_v7()));
            fs::create_dir(&base).expect("new fixture only");
            let fixture = Self(base);
            fs::create_dir(fixture.project()).unwrap();
            let provider = fixture.0.join("provider");
            fs::create_dir(&provider).unwrap();
            fs::create_dir(provider.join("src")).unwrap();
            fs::write(provider.join("wosy.toml"), r#"
                [package]
                name = "provider"
                source_root = "src"

                [builders.eval]
                command = ["scripts/provider-eval"]
                toolchain_identity = "other-toolchain"

                [builders.provider_only]
                command = ["scripts/provider-only"]
                toolchain_identity = "other-toolchain"

                [runners.evaluate]
                command = ["scripts/provider-run"]

                [targets.app.profiles.dev]
                comptime_builder = "provider_only"
                comptime_runner = "evaluate"
            "#).unwrap();
            fixture.write(MANIFEST);
            fixture
        }

        fn project(&self) -> PathBuf { self.0.join("project") }

        fn write(&self, manifest: &str) { fs::write(self.project().join("wosy.toml"), manifest).unwrap(); }

        fn load(&self, target: &str, profile: &str) -> Result<Option<SelectedComptimeTools>, String> {
            SelectedComptimeTools::load(&self.project(), target, profile)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn selects_consumer_pair_and_keeps_command_arguments_literal() {
        let fixture = Fixture::new();
        let ordinary = ProjectConfiguration::load(&fixture.project()).unwrap();
        assert!(ordinary.resolved_packages.contains_key("provider"));
        let (_, _, runtime, _) = ordinary.artifact_profile("app", "dev").unwrap();
        assert_eq!(runtime.run_runner, "runtime");
        let dev = fixture.load("app", "dev").unwrap().unwrap();
        assert_eq!((dev.target(), dev.profile()), ("app", "dev"));
        let (builder, declaration) = dev.builder();
        assert_eq!(builder, "eval");
        assert_eq!(declaration.command, ["scripts/eval", "two words"]);
        assert_eq!(declaration.args, ["literal *", "--a=b"]);
        assert_eq!(declaration.toolchain_identity, "declared-eval-toolchain");
        let (runner, declaration) = dev.runner();
        assert_eq!(runner, "evaluate");
        assert_eq!(declaration.command, ["scripts/evaluate", "runner prefix"]);
        assert_eq!(declaration.args, ["literal ${input}"]);
        let wasm = fixture.load("app", "wasm").unwrap().unwrap();
        assert_eq!((wasm.profile(), wasm.builder().0, wasm.runner().0), ("wasm", "special", "alt"));
        assert!(fixture.load("helper", "dev").unwrap().is_none());
        assert_eq!(fixture.load("missing", "dev").unwrap_err(), "target missing is missing from wosy.toml");
        assert_eq!(fixture.load("app", "missing").unwrap_err(), "target app profile missing is missing");
    }

    #[test]
    fn legacy_and_evaluator_fields_leave_ordinary_configuration_serialization_equal() {
        let fixture = Fixture::new();
        let configured = serde_json::to_vec(&ProjectConfiguration::load(&fixture.project()).unwrap()).unwrap();
        let previous = fixture.load("app", "dev").unwrap().unwrap();
        let mut legacy: toml_edit::DocumentMut = MANIFEST.parse().unwrap();
        for profile in ["dev", "wasm"] {
            let selected = legacy["targets"]["app"]["profiles"][profile].as_table_mut().unwrap();
            selected.remove("comptime_builder").unwrap();
            selected.remove("comptime_runner").unwrap();
            let artifact = &mut legacy["targets"]["app"]["artifacts"]["main"]["profiles"][profile];
            artifact["comptime_builder"] = toml_edit::value("not-a-target-profile");
            artifact["comptime_runner"] = toml_edit::value("not-a-target-profile");
        }
        fixture.write(&legacy.to_string());
        assert_eq!(configured, serde_json::to_vec(&ProjectConfiguration::load(&fixture.project()).unwrap()).unwrap());
        assert!(fixture.load("app", "dev").unwrap().is_none());
        assert!(fixture.load("app", "wasm").unwrap().is_none());
        assert_eq!((previous.builder().0, previous.runner().0), ("eval", "evaluate"));
    }

    #[test]
    fn refuses_partial_bad_names_and_empty_registered_commands_only_when_opted_in() {
        let fixture = Fixture::new();
        for (original, replacement, error) in [
            ("comptime_runner = \"evaluate\"", "", "target app profile dev must declare both comptime_builder and comptime_runner"),
            ("comptime_builder = \"eval\"", "", "target app profile dev must declare both comptime_builder and comptime_runner"),
            ("comptime_builder = \"eval\"", "comptime_builder = 1", "target app profile dev comptime_builder must be a string"),
            ("comptime_runner = \"evaluate\"", "comptime_runner = []", "target app profile dev comptime_runner must be a string"),
            ("comptime_builder = \"eval\"", "comptime_builder = \"provider_only\"", "comptime builder provider_only is missing"),
            ("comptime_runner = \"evaluate\"", "comptime_runner = \"missing\"", "comptime runner missing is missing"),
            ("command = [\"scripts/eval\", \"two words\"]", "command = []", "comptime builder eval has no executable command"),
            ("command = [\"scripts/evaluate\", \"runner prefix\"]", "command = [\"\"]", "comptime runner evaluate has no executable command"),
        ] {
            fixture.write(&MANIFEST.replace(original, replacement));
            let ordinary = ProjectConfiguration::load(&fixture.project()).unwrap();
            assert_eq!(ordinary.artifact_profile("app", "dev").unwrap().2.run_runner, "runtime");
            assert!(fixture.load("app", "wasm").unwrap().is_some());
            assert_eq!(fixture.load("app", "dev").unwrap_err(), error);
        }
    }
}
