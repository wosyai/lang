use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use toml_edit::{DocumentMut, Item};

fn main() -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixtures = root.join("../fixtures/parse");
    for entry in fs::read_dir(&fixtures).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            run_case(&entry.path())?;
        }
    }
    Ok(())
}

fn run_case(case: &Path) -> Result<(), String> {
    let document = fs::read_to_string(case.join("case.toml"))
        .map_err(|error| format!("{}: {error}", case.display()))?
        .parse::<DocumentMut>()
        .map_err(|error| error.to_string())?;
    let temporary = env::temp_dir().join(format!(
        "wosy-parse-fixture-{}-{}",
        std::process::id(),
        case.file_name().expect("fixture name").to_string_lossy()
    ));
    copy_tree(&case.join("project"), &temporary)?;
    let binary = env::var("WOSY_BIN")
        .map(PathBuf::from)
        .map_err(|_| "WOSY_BIN must point to the wosy executable".to_owned())?;
    let steps = document
        .get("steps")
        .and_then(Item::as_array_of_tables)
        .ok_or_else(|| format!("{} has no [[steps]]", case.display()))?;
    for step in steps.iter() {
        let command = command_values(step)?;
        let expected = step
            .get("exit")
            .and_then(Item::as_value)
            .and_then(|value| value.as_integer())
            .ok_or_else(|| "step exit must be an integer".to_owned())?;
        let status = Command::new(&binary)
            .args(command.iter().skip(1))
            .current_dir(&temporary)
            .status()
            .map_err(|error| error.to_string())?;
        if i64::from(
            status
                .code()
                .ok_or_else(|| "step was terminated".to_owned())?,
        ) != expected
        {
            return Err(format!("{}: unexpected exit status", case.display()));
        }
    }
    let assertions = document
        .get("assert")
        .and_then(Item::as_array_of_tables)
        .ok_or_else(|| format!("{} has no [[assert]]", case.display()))?;
    for assertion in assertions.iter() {
        let command = command_values(assertion)?;
        let expected_path = assertion
            .get("stdout")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .ok_or_else(|| "assert stdout must be a string".to_owned())?;
        let output = Command::new(&command[0])
            .args(command.iter().skip(1))
            .current_dir(&temporary)
            .output()
            .map_err(|error| error.to_string())?;
        let expected_status = assertion
            .get("exit")
            .and_then(Item::as_value)
            .and_then(|value| value.as_integer())
            .ok_or_else(|| "assert exit must be an integer".to_owned())?;
        if i64::from(output.status.code().ok_or_else(|| "assertion was terminated".to_owned())?)
            != expected_status
        {
            return Err(format!("{}: unexpected assertion exit status", case.display()));
        }
        let observed = serde_json::from_slice::<serde_json::Value>(&output.stdout)
            .map_err(|error| format!("{}: observed stdout is not JSON: {error}", case.display()))?;
        let expected_bytes = fs::read(case.join(expected_path)).map_err(|error| error.to_string())?;
        let expected = serde_json::from_slice::<serde_json::Value>(&expected_bytes)
            .map_err(|error| format!("{}: expected file is not JSON: {error}", case.display()))?;
        let selectors = assertion
            .get("select")
            .and_then(Item::as_value)
            .and_then(|value| value.as_array());
        if let Some(selectors) = selectors {
            for selector in selectors {
                let selector = selector
                    .as_str()
                    .ok_or_else(|| "assert select values must be strings".to_owned())?;
                let observed = observed
                    .pointer(selector)
                    .ok_or_else(|| format!("{}: observed JSON is missing {selector}", case.display()))?;
                assert_json_subset(&expected, observed, &case.display().to_string())?;
            }
        } else {
            assert_json_subset(&expected, &observed, &case.display().to_string())?;
        }
    }
    fs::remove_dir_all(&temporary).map_err(|error| error.to_string())?;
    println!("passed {}", case.display());
    Ok(())
}

fn command_values(table: &toml_edit::Table) -> Result<Vec<String>, String> {
    let command = table
        .get("command")
        .and_then(Item::as_value)
        .and_then(|value| value.as_array())
        .ok_or_else(|| "command must be an array".to_owned())?;
    command
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "command values must be strings".to_owned())
        })
        .collect()
}

fn assert_json_subset(expected: &serde_json::Value, observed: &serde_json::Value, case: &str) -> Result<(), String> {
    match (expected, observed) {
        (serde_json::Value::Object(expected), serde_json::Value::Object(observed)) => {
            for (key, expected_value) in expected {
                let observed_value = observed
                    .get(key)
                    .ok_or_else(|| format!("{case}: observed JSON is missing {key:?}"))?;
                assert_json_subset(expected_value, observed_value, case)?;
            }
            Ok(())
        }
        (serde_json::Value::Array(expected), serde_json::Value::Array(observed)) => {
            if expected.len() != observed.len() {
                return Err(format!("{case}: JSON array lengths differ"));
            }
            for (expected_value, observed_value) in expected.iter().zip(observed) {
                assert_json_subset(expected_value, observed_value, case)?;
            }
            Ok(())
        }
        (expected, observed) if expected == observed => Ok(()),
        (expected, observed) => Err(format!("{case}: expected {expected}, observed {observed}")),
    }
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let target = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}
