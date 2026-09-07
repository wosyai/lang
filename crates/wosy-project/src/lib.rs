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
