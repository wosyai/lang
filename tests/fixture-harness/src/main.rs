use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use toml_edit::{DocumentMut, Item};

fn main() -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixtures = root.join("../fixtures");
    let mut cases = Vec::new();
    collect_cases(&fixtures, &mut cases)?;
    if let Ok(selected) = env::var("FIXTURE") {
        cases.retain(|case| {
            case.file_name()
                .is_some_and(|name| name == std::ffi::OsStr::new(&selected))
        });
    }
    cases.sort();
    for case in cases {
        run_case(&case)?;
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
        if let Some(path) = step
            .get("remove")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
        {
            fs::remove_file(temporary.join(path)).map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(path) = step
            .get("remove_manifest_field")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
        {
            let manifest_path = temporary.join(path);
            let bytes = fs::read(&manifest_path).map_err(|error| error.to_string())?;
            let mut manifest = serde_json::from_slice::<serde_json::Value>(&bytes)
                .map_err(|error| error.to_string())?;
            manifest
                .as_object_mut()
                .ok_or_else(|| "manifest must be a JSON object".to_owned())?
                .remove("source_observations");
            fs::write(
                manifest_path,
                serde_json::to_vec(&manifest).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(path) = step
            .get("write")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
        {
            let contents = step
                .get("contents")
                .and_then(Item::as_value)
                .and_then(|value| value.as_str())
                .ok_or_else(|| "write step contents must be a string".to_owned())?;
            let destination = temporary.join(path);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::write(destination, contents).map_err(|error| error.to_string())?;
            continue;
        }
        select_source(step, &temporary, &case)?;
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
        let has_source_selector = assertion
            .get("source")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .is_some();
        select_source(assertion, &temporary, &case)?;
        let command = command_values(assertion)?;
        if command.first().map(String::as_str) == Some("wosy")
            && command.get(1).map(String::as_str) == Some("run")
            && has_source_selector
        {
            let status = Command::new(&binary)
                .args(["build"])
                .current_dir(&temporary)
                .status()
                .map_err(|error| error.to_string())?;
            if !status.success() {
                return Err(format!("{}: selected source build failed", case.display()));
            }
        }
        let assertion_program = if command[0] == "wosy" {
            binary.clone()
        } else {
            PathBuf::from(&command[0])
        };
        let output = Command::new(assertion_program)
            .args(command.iter().skip(1))
            .current_dir(&temporary)
            .output()
            .map_err(|error| error.to_string())?;
        let expected_status = assertion
            .get("exit")
            .and_then(Item::as_value)
            .and_then(|value| value.as_integer())
            .ok_or_else(|| "assert exit must be an integer".to_owned())?;
        if i64::from(
            output
                .status
                .code()
                .ok_or_else(|| "assertion was terminated".to_owned())?,
        ) != expected_status
        {
            return Err(format!(
                "{}: unexpected assertion exit status",
                case.display()
            ));
        }
        let selectors = assertion
            .get("select")
            .and_then(Item::as_value)
            .and_then(|value| value.as_array());
        if let Some(selectors) = selectors {
            let expected_path = assertion
                .get("stdout")
                .and_then(Item::as_value)
                .and_then(|value| value.as_str())
                .ok_or_else(|| "JSON assert stdout must be a string".to_owned())?;
            let observed =
                serde_json::from_slice::<serde_json::Value>(&output.stdout).map_err(|error| {
                    format!("{}: observed stdout is not JSON: {error}", case.display())
                })?;
            let expected_bytes =
                fs::read(case.join(expected_path)).map_err(|error| error.to_string())?;
            let expected =
                serde_json::from_slice::<serde_json::Value>(&expected_bytes).map_err(|error| {
                    format!("{}: expected file is not JSON: {error}", case.display())
                })?;
            for selector in selectors {
                let selector = selector
                    .as_str()
                    .ok_or_else(|| "assert select values must be strings".to_owned())?;
                let observed = observed.pointer(selector).ok_or_else(|| {
                    format!("{}: observed JSON is missing {selector}", case.display())
                })?;
                assert_json_subset(&expected, observed, &case.display().to_string())?;
            }
        } else {
            assert_stream(assertion, "stdout", &output.stdout, &case)?;
            assert_stream(assertion, "stderr", &output.stderr, &case)?;
            assert_stream_contains(assertion, "stderr_contains", &output.stderr, &case)?;
            if let Some(path) = assertion
                .get("file")
                .and_then(Item::as_value)
                .and_then(|value| value.as_str())
            {
                let expected_path = assertion
                    .get("contents")
                    .and_then(Item::as_value)
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| "file assert contents must be a string".to_owned())?;
                let observed = fs::read(temporary.join(path)).map_err(|error| error.to_string())?;
                let expected =
                    fs::read(case.join(expected_path)).map_err(|error| error.to_string())?;
                let terminal_newline = observed.len() == expected.len() + 1
                    && observed[..expected.len()] == expected
                    && observed[expected.len()] == b'\n';
                if observed != expected && !terminal_newline {
                    return Err(format!(
                        "{}: file assertion differs for {path}",
                        case.display()
                    ));
                }
            }
        }
    }
    fs::remove_dir_all(&temporary).map_err(|error| error.to_string())?;
    println!("passed {}", case.display());
    Ok(())
}

fn assert_stream_contains(
    assertion: &toml_edit::Table,
    key: &str,
    observed: &[u8],
    case: &Path,
) -> Result<(), String> {
    let Some(values) = assertion
        .get(key)
        .and_then(Item::as_value)
        .and_then(|value| value.as_array())
    else {
        return Ok(());
    };
    let text = String::from_utf8_lossy(observed);
    for value in values {
        let expected = value
            .as_str()
            .ok_or_else(|| format!("{key} values must be strings"))?;
        if !text.contains(expected) {
            return Err(format!(
                "{}: stderr is missing {expected:?}",
                case.display()
            ));
        }
    }
    Ok(())
}

fn assert_stream(
    assertion: &toml_edit::Table,
    stream: &str,
    observed: &[u8],
    case: &Path,
) -> Result<(), String> {
    let Some(expected_path) = assertion
        .get(stream)
        .and_then(Item::as_value)
        .and_then(|value| value.as_str())
    else {
        return Ok(());
    };
    let expected = fs::read(case.join(expected_path)).map_err(|error| error.to_string())?;
    let exact = match assertion.get("exact") {
        Some(item) => item
            .as_value()
            .and_then(|value| value.as_bool())
            .ok_or_else(|| "assert exact must be a boolean".to_owned())?,
        None => false,
    };
    if !exact && stream == "stdout" {
        if let (Ok(expected_json), Ok(observed_json)) = (
            serde_json::from_slice::<serde_json::Value>(&expected),
            serde_json::from_slice::<serde_json::Value>(observed),
        ) {
            return assert_json_subset(&expected_json, &observed_json, &case.display().to_string());
        }
    }
    if observed != expected {
        return Err(format!("{}: {stream} assertion differs", case.display()));
    }
    Ok(())
}

fn select_source(table: &toml_edit::Table, temporary: &Path, case: &Path) -> Result<(), String> {
    let Some(source) = table
        .get("source")
        .and_then(Item::as_value)
        .and_then(|value| value.as_str())
    else {
        return Ok(());
    };
    fs::copy(case.join(source), temporary.join("src/main.w")).map_err(|error| {
        format!(
            "{}: failed to select source {source}: {error}",
            case.display()
        )
    })?;
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

fn assert_json_subset(
    expected: &serde_json::Value,
    observed: &serde_json::Value,
    case: &str,
) -> Result<(), String> {
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

fn collect_cases(directory: &Path, cases: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.join("case.toml").is_file() {
            cases.push(path);
        } else if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            collect_cases(&path, cases)?;
        }
    }
    Ok(())
}
