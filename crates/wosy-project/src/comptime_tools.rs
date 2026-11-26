use super::{string_array, BuilderConfig, ProjectConfiguration, RunnerConfig};
use std::{collections::BTreeMap, path::Path};
use toml_edit::{ImDocument, Item, TableLike};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeclaredComptimeParameter {
    Integer(i64),
    Boolean(bool),
    /// An authored value needing a compiler expansion not supported by
    /// the first closed scalar evaluator. Unused values stay irrelevant.
    Other,
}

pub fn declared_comptime_parameters(
    configuration: &ProjectConfiguration, document: &ImDocument<String>, target: &str, profile: &str,
) -> Result<BTreeMap<String, DeclaredComptimeParameter>, String> {
    configuration.target_profile(target, profile)?;
    parameters_in(authored_profile(document, target, profile)?)
}

fn parameters_in(authored: &toml_edit::Table) -> Result<BTreeMap<String, DeclaredComptimeParameter>, String> {
    let Some(parameters) = authored.get("params") else {
        return Ok(BTreeMap::new());
    };
    let parameters = parameters.as_table_like().ok_or("compile-time params must be a table")?;
    Ok(parameters.iter().map(|(name, value)| {
        let value = if let Some(value) = value.as_integer() {
            DeclaredComptimeParameter::Integer(value)
        } else if let Some(value) = value.as_bool() {
            DeclaredComptimeParameter::Boolean(value)
        } else {
            DeclaredComptimeParameter::Other
        };
        (name.to_owned(), value)
    }).collect())
}

/// A real profile/pair observed in a single loaded project configuration
/// with no build mounts or permissions. No command authority.
pub struct ObservedClosedComptimeProfile {
    tools: SelectedComptimeTools,
    parameters: BTreeMap<String, DeclaredComptimeParameter>,
}

impl ObservedClosedComptimeProfile {
    pub fn select(
        configuration: &ProjectConfiguration, document: &ImDocument<String>,
        target: &str, profile: &str, package: &str, id: &str,
    ) -> Result<Self, String> {
        Self::select_with(configuration, document, target, profile, || reject_applicable_watches(document, package, id))
    }

    /// Select declarations for read-only input evidence. This does not
    /// resolve watches: the source assembler resolves them for the exact
    /// authored site separately. It is not a launch or staging permission.
    pub fn observe_input_declarations(
        configuration: &ProjectConfiguration, document: &ImDocument<String>,
        target: &str, profile: &str,
    ) -> Result<Self, String> {
        Self::select_with(configuration, document, target, profile, || Ok(()))
    }

    fn select_with(
        configuration: &ProjectConfiguration, document: &ImDocument<String>, target: &str, profile: &str,
        watches: impl FnOnce() -> Result<(), String>,
    ) -> Result<Self, String> {
        configuration.target_profile(target, profile)?;
        let authored = authored_profile(document, target, profile)?;
        let tools = SelectedComptimeTools::from_authored(configuration, authored, target, profile)?
            .ok_or("closed comptime staging needs an explicit comptime builder and runner")?;
        if tools.build_permissions().is_some_and(|permissions| !permissions.is_empty())
            || tools.build_mounts().is_some_and(|mounts| !mounts.is_empty())
        {
            return Err("closed comptime staging does not resolve build permissions or mounts".into());
        }
        watches()?;
        let parameters = parameters_in(authored)?;
        Ok(Self { tools, parameters })
    }

    pub fn parameters(&self) -> &BTreeMap<String, DeclaredComptimeParameter> { &self.parameters }

    pub fn tools(&self) -> &SelectedComptimeTools { &self.tools }

    pub fn into_tools(self) -> SelectedComptimeTools { self.tools }
}

fn reject_applicable_watches(document: &ImDocument<String>, package: &str, id: &str) -> Result<(), String> {
    let Some(comptime) = document.get("comptime") else { return Ok(()); };
    let declarations = comptime.as_table_like().ok_or("comptime watches must be a table")?;
    // This fragment has no imports. Inspect the exact local site and refuse
    // any qualified declaration that could otherwise be mistaken for its input.
    for key in [id, &format!("{package}:{id}")] {
        let Some(item) = declarations.get(key) else { continue; };
        let site = item.as_table_like().ok_or("comptime site watches must be a table")?;
        for field in ["watch", "watch_add"] {
            if let Some(paths) = site.get(field) {
                let paths = paths.as_array().ok_or("comptime watch paths must be an array")?;
                if !paths.is_empty() { return Err("closed comptime staging does not resolve watches".into()); }
            }
        }
    }
    Ok(())
}

fn authored_profile<'a>(
    document: &'a ImDocument<String>, target: &str, profile: &str,
) -> Result<&'a toml_edit::Table, String> {
    document.get("targets").and_then(Item::as_table)
        .and_then(|targets| targets.get(target)).and_then(Item::as_table)
        .and_then(|target| target.get("profiles")).and_then(Item::as_table)
        .and_then(|profiles| profiles.get(profile)).and_then(Item::as_table)
        .ok_or_else(|| format!("target {target} profile {profile} is missing from its manifest"))
}

/// The consuming profile's evaluator tools and build declarations from one
/// ordinary manifest read. Not an executable identity, a capability grant,
/// or a full build configuration.
#[derive(Debug, Eq, PartialEq)]
pub struct SelectedComptimeTools {
    target: String,
    profile: String,
    builder_name: String,
    builder: BuilderConfig,
    runner_name: String,
    runner: RunnerConfig,
    build_permissions: Option<Vec<String>>,
    build_mounts: Option<Vec<ComptimeBuildMountDeclaration>>,
}

/// An authored mount declaration, not a resolved or authorized host path.
#[derive(Debug, Eq, PartialEq)]
pub struct ComptimeBuildMountDeclaration {
    host_path: String,
    guest_path: String,
    access: ComptimeMountAccess,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComptimeMountAccess {
    Read,
    ReadWrite,
}

impl ComptimeBuildMountDeclaration {
    pub fn host_path(&self) -> &str { &self.host_path }
    pub fn guest_path(&self) -> &str { &self.guest_path }
    pub fn access(&self) -> ComptimeMountAccess { self.access }
}

impl SelectedComptimeTools {
    /// Resolve only an explicitly selected profile. Legacy profiles with
    /// neither declaration have no evaluator tools. An artifact's runner
    /// and any dependency's tools are not candidates.
    pub fn load(root: &Path, target: &str, profile: &str) -> Result<Option<Self>, String> {
        let (configuration, document) = ProjectConfiguration::load_with_manifest(root)?;
        configuration.target_profile(target, profile)?;
        let authored = authored_profile(&document, target, profile)?;
        Self::from_authored(&configuration, authored, target, profile)
    }

    fn from_authored(
        configuration: &ProjectConfiguration, authored: &toml_edit::Table,
        target: &str, profile: &str,
    ) -> Result<Option<Self>, String> {
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
        let builder = configuration.builders.get(builder_name)
            .ok_or_else(|| format!("comptime builder {builder_name} is missing"))?;
        let runner = configuration.runners.get(runner_name)
            .ok_or_else(|| format!("comptime runner {runner_name} is missing"))?;
        for (kind, name, command) in [
            ("comptime builder", builder_name, &builder.command),
            ("comptime runner", runner_name, &runner.command),
        ] {
            if command.first().is_none_or(String::is_empty) {
                return Err(format!("{kind} {name} has no executable command"));
            }
        }
        let build_permissions = authored.get("build_permissions")
            .map(|_| string_array(authored, "build_permissions")).transpose()?;
        let build_mounts = authored.get("build_mounts").map(build_mount_declarations).transpose()?;
        Ok(Some(Self {
            target: target.into(),
            profile: profile.into(),
            builder_name: builder_name.into(),
            builder: builder.clone(),
            runner_name: runner_name.into(),
            runner: runner.clone(),
            build_permissions,
            build_mounts,
        }))
    }

    pub fn target(&self) -> &str { &self.target }
    pub fn profile(&self) -> &str { &self.profile }
    pub fn builder(&self) -> (&str, &BuilderConfig) { (&self.builder_name, &self.builder) }
    pub fn runner(&self) -> (&str, &RunnerConfig) { (&self.runner_name, &self.runner) }
    pub fn build_permissions(&self) -> Option<&[String]> { self.build_permissions.as_deref() }
    pub fn build_mounts(&self) -> Option<&[ComptimeBuildMountDeclaration]> { self.build_mounts.as_deref() }
}

fn build_mount_declarations(item: &Item) -> Result<Vec<ComptimeBuildMountDeclaration>, String> {
    if let Some(array) = item.as_array() {
        return array.iter().map(|value| {
            let row = value.as_inline_table().ok_or("build_mounts entries must be tables")?;
            build_mount(row)
        }).collect();
    }
    if let Some(tables) = item.as_array_of_tables() {
        return tables.iter().map(|row| build_mount(row)).collect();
    }
    Err("build_mounts must be an array".into())
}

fn build_mount(row: &dyn TableLike) -> Result<ComptimeBuildMountDeclaration, String> {
    let field = |key: &str| row.get(key).and_then(Item::as_str)
        .ok_or_else(|| format!("build_mounts.{key} must be a string"));
    let host_path = field("host_path")?;
    let guest_path = field("guest_path")?;
    let access = match field("access")? {
        "read" => ComptimeMountAccess::Read,
        "read_write" => ComptimeMountAccess::ReadWrite,
        _ => return Err("build_mounts.access must be read or read_write".into()),
    };
    Ok(ComptimeBuildMountDeclaration { host_path: host_path.into(), guest_path: guest_path.into(), access })
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
        run_permissions = ["network"]
        run_mounts = [{ host_path = "runtime", guest_path = "/var", access = "read_write" }]

        [targets.app.artifacts.main.profiles.wasm]
        backend = "llvm"
        output = "wasm-wasip1"
        builder = "artifact"
        run_runner = "runtime"

        [targets.app.profiles.dev]
        main_artifact = "main"
        comptime_builder = "eval"
        comptime_runner = "evaluate"
        build_permissions = ["filesystem:schemas_read", "filesystem:scratch"]
        build_mounts = [
            { host_path = "schemas", guest_path = "/schemas", access = "read" },
            { host_path = "./scratch", guest_path = "/tmp", access = "read_write" },
        ]

        [targets.app.profiles.wasm]
        main_artifact = "main"
        comptime_builder = "special"
        comptime_runner = "alt"
        build_permissions = []
        build_mounts = []

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
                build_permissions = ["network"]
                build_mounts = [{ host_path = ".", guest_path = "/provider", access = "read" }]
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

    fn mount_values(tools: &SelectedComptimeTools) -> Vec<(&str, &str, ComptimeMountAccess)> {
        tools.build_mounts().unwrap().iter()
            .map(|row| (row.host_path(), row.guest_path(), row.access())).collect()
    }

    #[test]
    fn build_declarations_are_selected_and_preserve_order_empty_and_absent() {
        use ComptimeMountAccess::{Read, ReadWrite};
        let fixture = Fixture::new();
        let dev = fixture.load("app", "dev").unwrap().unwrap();
        assert_eq!(dev.build_permissions().unwrap(), ["filesystem:schemas_read", "filesystem:scratch"]);
        assert_eq!(mount_values(&dev), [("schemas", "/schemas", Read), ("./scratch", "/tmp", ReadWrite)]);
        let wasm = fixture.load("app", "wasm").unwrap().unwrap();
        assert!(wasm.build_permissions().unwrap().is_empty());
        assert!(wasm.build_mounts().unwrap().is_empty());

        let ordinary = serde_json::to_vec(&ProjectConfiguration::load(&fixture.project()).unwrap()).unwrap();
        let mut without: toml_edit::DocumentMut = MANIFEST.parse().unwrap();
        let profile = without["targets"]["app"]["profiles"]["dev"].as_table_mut().unwrap();
        profile.remove("build_permissions").unwrap();
        profile.remove("build_mounts").unwrap();
        fixture.write(&without.to_string());
        assert_eq!(serde_json::to_vec(&ProjectConfiguration::load(&fixture.project()).unwrap()).unwrap(), ordinary);
        let absent = fixture.load("app", "dev").unwrap().unwrap();
        assert!(absent.build_permissions().is_none() && absent.build_mounts().is_none());
        assert_eq!(mount_values(&dev), [("schemas", "/schemas", Read), ("./scratch", "/tmp", ReadWrite)]);

        fixture.write(&format!("{without}\n{}", r#"
            [[targets.app.profiles.dev.build_mounts]]
            host_path = "schemas"
            guest_path = "/schemas"
            access = "read"
            [[targets.app.profiles.dev.build_mounts]]
            host_path = "./scratch"
            guest_path = "/tmp"
            access = "read_write"
        "#));
        assert_eq!(mount_values(&fixture.load("app", "dev").unwrap().unwrap()), mount_values(&dev));
    }

    #[test]
    fn malformed_build_declarations_fail_only_for_the_opted_in_profile() {
        let fixture = Fixture::new();
        for (field, value, error) in [
            ("build_permissions", "true", "build_permissions must be an array"),
            ("build_permissions", "[1]", "build_permissions values must be strings"),
            ("build_mounts", "\"not a list\"", "build_mounts must be an array"),
            ("build_mounts", "[123]", "build_mounts entries must be tables"),
            ("build_mounts", "[{ guest_path = \"/data\", access = \"read\" }]", "build_mounts.host_path must be a string"),
            ("build_mounts", "[{ host_path = \"data\", guest_path = 5, access = \"read\" }]", "build_mounts.guest_path must be a string"),
            ("build_mounts", "[{ host_path = \"data\", guest_path = \"/data\" }]", "build_mounts.access must be a string"),
            ("build_mounts", "[{ host_path = \"data\", guest_path = \"/data\", access = \"write\" }]",
                "build_mounts.access must be read or read_write"),
        ] {
            let mut manifest: toml_edit::DocumentMut = MANIFEST.parse().unwrap();
            let candidate = format!("value = {value}").parse::<toml_edit::DocumentMut>().unwrap();
            let selected = manifest["targets"]["app"]["profiles"]["dev"].as_table_mut().unwrap();
            selected[field] = candidate["value"].clone();
            fixture.write(&manifest.to_string());
            assert!(ProjectConfiguration::load(&fixture.project()).unwrap().artifact_profile("app", "dev").is_ok());
            assert!(fixture.load("app", "wasm").unwrap().is_some());
            assert_eq!(fixture.load("app", "dev").unwrap_err(), error);
        }
    }
}
