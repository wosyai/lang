use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectContext {
    pub root: PathBuf,
    pub package: String,
    pub source_root: PathBuf,
    pub comment_categories: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BuilderConfig {
    pub command: Vec<String>,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RunnerConfig {
    pub command: Vec<String>,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactProfile {
    pub backend: String,
    pub output: String,
    pub builder: String,
    pub run_runner: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactConfig {
    pub artifact: String,
    pub profiles: std::collections::BTreeMap<String, ArtifactProfile>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TargetProfile {
    pub main_artifact: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TargetConfig {
    pub root: PathBuf,
    pub artifacts: std::collections::BTreeMap<String, ArtifactConfig>,
    pub profiles: std::collections::BTreeMap<String, TargetProfile>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectConfiguration {
    pub builders: std::collections::BTreeMap<String, BuilderConfig>,
    pub runners: std::collections::BTreeMap<String, RunnerConfig>,
    pub targets: std::collections::BTreeMap<String, TargetConfig>,
}

impl ProjectConfiguration {
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join("wosy.toml");
        let content = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let document = content
            .parse::<DocumentMut>()
            .map_err(|error| error.to_string())?;
        let builders = named_commands(document.get("builders"), "builder")?;
        let runners = named_runners(document.get("runners"))?;
        let targets_table = document
            .get("targets")
            .and_then(Item::as_table)
            .ok_or_else(|| "targets is missing".to_owned())?;
        let mut targets = std::collections::BTreeMap::new();
        for (name, item) in targets_table {
            let table = item
                .as_table()
                .ok_or_else(|| format!("target {name} is not a table"))?;
            let root = table
                .get("root")
                .and_then(Item::as_value)
                .and_then(|value| value.as_str())
                .ok_or_else(|| format!("target {name} has no root"))?;
            let artifacts = parse_artifacts(table.get("artifacts"))?;
            let profiles = parse_target_profiles(table.get("profiles"))?;
            targets.insert(
                name.to_owned(),
                TargetConfig {
                    root: PathBuf::from(root),
                    artifacts,
                    profiles,
                },
            );
        }
        Ok(Self {
            builders,
            runners,
            targets,
        })
    }
}

fn named_commands(
    item: Option<&Item>,
    kind: &str,
) -> Result<std::collections::BTreeMap<String, BuilderConfig>, String> {
    let table = item
        .and_then(Item::as_table)
        .ok_or_else(|| format!("{kind}s are missing"))?;
    let mut commands = std::collections::BTreeMap::new();
    for (name, item) in table {
        let table = item
            .as_table()
            .ok_or_else(|| format!("{kind} {name} is not a table"))?;
        commands.insert(
            name.to_owned(),
            BuilderConfig {
                command: string_array(table, "command")?,
                args: string_array_optional(table, "args")?,
            },
        );
    }
    Ok(commands)
}

fn named_runners(
    item: Option<&Item>,
) -> Result<std::collections::BTreeMap<String, RunnerConfig>, String> {
    let table = item
        .and_then(Item::as_table)
        .ok_or_else(|| "runners are missing".to_owned())?;
    let mut commands = std::collections::BTreeMap::new();
    for (name, item) in table {
        let table = item
            .as_table()
            .ok_or_else(|| format!("runner {name} is not a table"))?;
        commands.insert(
            name.to_owned(),
            RunnerConfig {
                command: string_array(table, "command")?,
                args: string_array_optional(table, "args")?,
            },
        );
    }
    Ok(commands)
}

fn string_array(table: &toml_edit::Table, key: &str) -> Result<Vec<String>, String> {
    table
        .get(key)
        .and_then(Item::as_value)
        .and_then(|value| value.as_array())
        .ok_or_else(|| format!("{key} must be an array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{key} values must be strings"))
        })
        .collect()
}

fn string_array_optional(table: &toml_edit::Table, key: &str) -> Result<Vec<String>, String> {
    table
        .get(key)
        .map_or(Ok(Vec::new()), |_item| string_array(table, key))
}

fn parse_artifacts(
    item: Option<&Item>,
) -> Result<std::collections::BTreeMap<String, ArtifactConfig>, String> {
    let table = item
        .and_then(Item::as_table)
        .ok_or_else(|| "artifacts are missing".to_owned())?;
    let mut artifacts = std::collections::BTreeMap::new();
    for (name, item) in table {
        let table = item
            .as_table()
            .ok_or_else(|| format!("artifact {name} is not a table"))?;
        let artifact = table
            .get("artifact")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .ok_or_else(|| format!("artifact {name} has no identity"))?;
        let profiles_table = table
            .get("profiles")
            .and_then(Item::as_table)
            .ok_or_else(|| format!("artifact {name} has no profiles"))?;
        let mut profiles = std::collections::BTreeMap::new();
        for (profile_name, profile_item) in profiles_table {
            let profile = profile_item
                .as_table()
                .ok_or_else(|| format!("artifact profile {profile_name} is not a table"))?;
            profiles.insert(
                profile_name.to_owned(),
                ArtifactProfile {
                    backend: scalar_string(profile, "backend")?,
                    output: scalar_string(profile, "output")?,
                    builder: scalar_string(profile, "builder")?,
                    run_runner: scalar_string(profile, "run_runner")?,
                },
            );
        }
        artifacts.insert(
            name.to_owned(),
            ArtifactConfig {
                artifact: artifact.to_owned(),
                profiles,
            },
        );
    }
    Ok(artifacts)
}

fn parse_target_profiles(
    item: Option<&Item>,
) -> Result<std::collections::BTreeMap<String, TargetProfile>, String> {
    let table = item
        .and_then(Item::as_table)
        .ok_or_else(|| "target profiles are missing".to_owned())?;
    let mut profiles = std::collections::BTreeMap::new();
    for (name, item) in table {
        let profile = item
            .as_table()
            .ok_or_else(|| format!("target profile {name} is not a table"))?;
        profiles.insert(
            name.to_owned(),
            TargetProfile {
                main_artifact: scalar_string(profile, "main_artifact")?,
            },
        );
    }
    Ok(profiles)
}

fn scalar_string(table: &toml_edit::Table, key: &str) -> Result<String, String> {
    table
        .get(key)
        .and_then(Item::as_value)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("{key} must be a string"))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactIdentity {
    pub source_identity: String,
    pub configuration_identity: String,
    pub artifact_identity: String,
    pub backend: String,
    pub output: String,
    pub builder_identity: String,
    pub runner_identity: String,
    pub llvm_input_identity: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactManifest {
    pub identity: ArtifactIdentity,
    pub executable: PathBuf,
}

pub fn publish_artifact(
    output_dir: &Path,
    executable: PathBuf,
    identity: ArtifactIdentity,
) -> Result<ArtifactManifest, String> {
    if !executable.is_file() {
        return Err(format!("builder did not produce {}", executable.display()));
    }
    fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;
    let manifest = ArtifactManifest {
        identity,
        executable,
    };
    let manifest_path = output_dir.join("artifact-manifest.json");
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    fs::write(manifest_path, bytes).map_err(|error| error.to_string())?;
    Ok(manifest)
}

impl ProjectContext {
    pub fn new(
        root: PathBuf,
        package: String,
        source_root: PathBuf,
        comment_categories: Vec<String>,
    ) -> Self {
        Self {
            root,
            package,
            source_root,
            comment_categories,
        }
    }

    pub fn load(root: &Path, target: &str) -> Result<Self, String> {
        let config_path = root.join("wosy.toml");
        let content = fs::read_to_string(&config_path)
            .map_err(|error| format!("failed to read {}: {error}", config_path.display()))?;
        let document = content
            .parse::<DocumentMut>()
            .map_err(|error| format!("failed to parse {}: {error}", config_path.display()))?;
        let target_table = document
            .get("targets")
            .and_then(Item::as_table)
            .and_then(|targets| targets.get(target))
            .and_then(Item::as_table)
            .ok_or_else(|| format!("target {target} is missing from wosy.toml"))?;
        let source = target_table
            .get("root")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .ok_or_else(|| format!("target {target} has no root source"))?;
        let package = document
            .get("package")
            .and_then(Item::as_table)
            .and_then(|table| table.get("name"))
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .ok_or_else(|| "package.name is missing from wosy.toml".to_owned())?
            .to_owned();
        let comment_categories = document
            .get("comments")
            .and_then(Item::as_table)
            .and_then(|table| table.get("categories"))
            .and_then(Item::as_value)
            .and_then(|value| value.as_array())
            .map(|array| {
                array
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .ok_or_else(|| "comment categories must be strings".to_owned())
                            .map(str::to_owned)
                    })
                    .collect::<Result<Vec<_>, String>>()
            })
            .transpose()?
            .ok_or_else(|| "comments.categories is missing from wosy.toml".to_owned())?;
        Ok(Self::new(
            root.to_path_buf(),
            package,
            PathBuf::from(source),
            comment_categories,
        ))
    }

    pub fn source_path(&self) -> PathBuf {
        self.root.join(&self.source_root)
    }
}

pub fn discover_root(start: &Path) -> Result<PathBuf, String> {
    let mut current = start.to_path_buf();
    loop {
        if current.join("wosy.toml").is_file() {
            return Ok(current);
        }
        if !current.pop() {
            return Err("could not find wosy.toml".to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProjectContext;
    use std::path::PathBuf;

    #[test]
    fn project_context_serializes_structured_configuration() {
        let context = ProjectContext::new(
            PathBuf::from("/workspace"),
            "app".to_owned(),
            PathBuf::from("src"),
            vec!["INTENT".to_owned(), "TYPE".to_owned()],
        );

        let value = serde_json::to_value(&context).expect("project context serialization");

        assert_eq!(value["package"], "app");
        assert_eq!(value["comment_categories"][0], "INTENT");
    }
}
