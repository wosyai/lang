use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use toml_edit::{DocumentMut, Item};
use wasmtime::{Caller, Engine, Linker, Memory, Module, Store};
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::WasiCtx;

const ARTIFACT_MANIFEST: &str = ".wosy/artifacts/app/dev/main/artifact-manifest.json";

struct HostState {
    wasi: WasiP1Ctx,
    writes: Vec<WriteRecord>,
    reads: Vec<ReadRecord>,
    read_bytes: Vec<u8>,
    read_errno: i32,
    read_capacity: u32,
}

#[derive(Debug, PartialEq, Eq)]
struct WriteRecord {
    descriptor: i32,
    text: String,
    reported: u32,
}

#[derive(Debug, PartialEq, Eq)]
struct ReadRecord {
    descriptor: i32,
    capacity: u32,
    bytes: Vec<u8>,
    reported: u32,
    errno: i32,
}

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
        run_case(&case, &root)?;
    }
    Ok(())
}

fn run_case(case: &Path, root: &Path) -> Result<(), String> {
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
    if document
        .get("shared_stdlib")
        .and_then(Item::as_value)
        .and_then(|value| value.as_bool())
        .is_some_and(|enabled| enabled)
    {
        let repository_root = root.join("../..");
        let stdlib =
            fs::canonicalize(repository_root.join("stdlib")).map_err(|error| error.to_string())?;
        link_shared_stdlib(&stdlib, &temporary)?;
    }
    let binary = env::var("WOSY_BIN")
        .map(PathBuf::from)
        .map_err(|_| "WOSY_BIN must point to the wosy executable".to_owned())?;
    let binary = if binary.is_absolute() {
        binary
    } else {
        env::current_dir()
            .map_err(|error| error.to_string())?
            .join(binary)
    };
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
            let field = step
                .get("field")
                .and_then(Item::as_value)
                .and_then(|value| value.as_str())
                .ok_or_else(|| "remove_manifest_field step field must be a string".to_owned())?;
            let bytes = fs::read(&manifest_path).map_err(|error| error.to_string())?;
            let mut manifest = serde_json::from_slice::<serde_json::Value>(&bytes)
                .map_err(|error| error.to_string())?;
            remove_manifest_field(&mut manifest, field)?;
            fs::write(
                manifest_path,
                serde_json::to_vec(&manifest).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(path) = step
            .get("replace_manifest_field")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
        {
            let field = step
                .get("field")
                .and_then(Item::as_value)
                .and_then(|value| value.as_str())
                .ok_or_else(|| "replace_manifest_field step field must be a string".to_owned())?;
            let replacement = step
                .get("value")
                .and_then(Item::as_value)
                .and_then(|value| value.as_str())
                .ok_or_else(|| "replace_manifest_field step value must be a string".to_owned())?;
            let manifest_path = temporary.join(path);
            let bytes = fs::read(&manifest_path).map_err(|error| error.to_string())?;
            let mut manifest = serde_json::from_slice::<serde_json::Value>(&bytes)
                .map_err(|error| error.to_string())?;
            replace_manifest_field(&mut manifest, field, replacement)?;
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
        if let Some(read_assertion) = fd_read_assertion(assertion)? {
            run_fd_read_assertion(assertion, read_assertion, &temporary, &case, &binary)?;
            assert_source_observations(assertion, &temporary, &case)?;
            continue;
        }
        if let Some(partial_fd_write_count) = partial_fd_write_count(assertion)? {
            run_partial_fd_write_assertion(
                assertion,
                partial_fd_write_count,
                &temporary,
                &case,
                &binary,
            )?;
            assert_source_observations(assertion, &temporary, &case)?;
            continue;
        }
        let has_source_selector = assertion
            .get("source")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .is_some();
        select_source(assertion, &temporary, &case)?;
        let command = command_values(assertion)?;
        if requires_selected_source_build(has_source_selector, &command) {
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
        assert_source_observations(assertion, &temporary, &case)?;
    }
    fs::remove_dir_all(&temporary).map_err(|error| error.to_string())?;
    println!("passed {}", case.display());
    Ok(())
}

struct FdReadAssertion {
    bytes: Vec<u8>,
    errno: i32,
    capacity: u32,
    calls: usize,
}

fn fd_read_assertion(assertion: &toml_edit::Table) -> Result<Option<FdReadAssertion>, String> {
    let Some(item) = assertion.get("fd_read_capacity") else {
        return Ok(None);
    };
    let capacity = item
        .as_value()
        .and_then(|value| value.as_integer())
        .ok_or_else(|| "fd_read_capacity must be an integer".to_owned())?;
    let capacity = u32::try_from(capacity)
        .map_err(|_| "fd_read_capacity must fit in u32".to_owned())?;
    let bytes = assertion
        .get("fd_read_bytes")
        .and_then(Item::as_value)
        .and_then(|value| value.as_array())
        .ok_or_else(|| "fd_read_bytes must be an array".to_owned())?
        .iter()
        .map(|value| {
            value
                .as_integer()
                .ok_or_else(|| "fd_read_bytes values must be integers".to_owned())
                .and_then(|value| {
                    u8::try_from(value)
                        .map_err(|_| "fd_read_bytes values must fit in u8".to_owned())
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let errno = assertion
        .get("fd_read_errno")
        .and_then(Item::as_value)
        .and_then(|value| value.as_integer())
        .ok_or_else(|| "fd_read_errno must be an integer".to_owned())
        .and_then(|value| {
            i32::try_from(value).map_err(|_| "fd_read_errno must fit in i32".to_owned())
        })?;
    let calls = assertion
        .get("fd_read_calls")
        .and_then(Item::as_value)
        .and_then(|value| value.as_integer())
        .ok_or_else(|| "fd_read_calls must be an integer".to_owned())
        .and_then(|value| {
            usize::try_from(value).map_err(|_| "fd_read_calls must be nonnegative".to_owned())
        })?;
    Ok(Some(FdReadAssertion {
        bytes,
        errno,
        capacity,
        calls,
    }))
}

fn run_fd_read_assertion(
    assertion: &toml_edit::Table,
    read_assertion: FdReadAssertion,
    temporary: &Path,
    case: &Path,
    binary: &Path,
) -> Result<(), String> {
    if assertion.get("command").is_some() {
        return Err(format!(
            "{}: fd_read_capacity assertion accepts no command",
            case.display()
        ));
    }
    select_source(assertion, temporary, case)?;
    let status = Command::new(binary)
        .arg("build")
        .current_dir(temporary)
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("{}: fd_read source build failed", case.display()));
    }
    let executable = artifact_executable(temporary, case)?;
    let writes = run_fd_read_host(&executable, &read_assertion, case)?;
    assert_stream(assertion, "stdout", &writes, case)
}

fn run_fd_read_host(
    executable: &Path,
    read_assertion: &FdReadAssertion,
    case: &Path,
) -> Result<Vec<u8>, String> {
    let engine = Engine::default();
    let module = Module::from_file(&engine, executable).map_err(|error| error.to_string())?;
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |state: &mut HostState| &mut state.wasi)
        .map_err(|error| error.to_string())?;
    linker.allow_shadowing(true);
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_read",
            controlled_fd_read,
        )
        .map_err(|error| error.to_string())?;
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_write",
            recording_fd_write,
        )
        .map_err(|error| error.to_string())?;
    let wasi = WasiCtx::builder().build_p1();
    let mut store = Store::new(
        &engine,
        HostState {
            wasi,
            writes: Vec::new(),
            reads: Vec::new(),
            read_bytes: read_assertion.bytes.clone(),
            read_errno: read_assertion.errno,
            read_capacity: read_assertion.capacity,
        },
    );
    let instance = linker.instantiate(&mut store, &module).map_err(|error| {
        format!(
            "{}: controlled fd_read instantiation failed: {error}",
            case.display()
        )
    })?;
    instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .map_err(|error| format!("{}: _start export is missing: {error}", case.display()))?
        .call(&mut store, ())
        .map_err(|error| format!("{}: _start failed: {error:?}", case.display()))?;
    assert_fd_read_result(&store.data().reads, read_assertion, case)?;
    Ok(store
        .data()
        .writes
        .iter()
        .filter(|write| write.descriptor == 1)
        .flat_map(|write| write.text.bytes())
        .collect())
}

fn controlled_fd_read(
    mut caller: Caller<'_, HostState>,
    descriptor: i32,
    iovs: i32,
    iovs_len: i32,
    nread: i32,
) -> Result<i32, wasmtime::Error> {
    if descriptor != 0 {
        return Err(wasmtime::Error::msg(format!(
            "fd_read expected descriptor 0, observed {descriptor}"
        )));
    }
    if iovs_len != 1 {
        return Err(wasmtime::Error::msg(format!(
            "fd_read expected one iovec, observed {iovs_len}"
        )));
    }
    let memory = caller
        .get_export("memory")
        .and_then(|export| export.into_memory())
        .ok_or_else(|| wasmtime::Error::msg("fd_read guest memory is missing"))?;
    let iovec = read_guest_memory(&memory, &mut caller, iovs as u32, 8)?;
    let data = u32::from_le_bytes(iovec[..4].try_into().expect("iovec address"));
    let capacity = u32::from_le_bytes(iovec[4..].try_into().expect("iovec length"));
    if capacity != caller.data().read_capacity {
        return Err(wasmtime::Error::msg(format!(
            "fd_read expected capacity {}, observed {capacity}",
            caller.data().read_capacity
        )));
    }
    let errno = caller.data().read_errno;
    let bytes = if errno == 0 && caller.data().reads.is_empty() {
        caller.data().read_bytes.clone()
    } else {
        Vec::new()
    };
    if bytes.len() > capacity as usize {
        return Err(wasmtime::Error::msg("fd_read bytes exceed guest capacity"));
    }
    let reported = if errno == 0 { bytes.len() as u32 } else { 0 };
    if errno == 0 {
        memory
            .write(&mut caller, data as usize, &bytes)
            .map_err(|error| wasmtime::Error::msg(format!("fd_read data write failed: {error}")))?;
    }
    memory
        .write(&mut caller, nread as usize, &reported.to_le_bytes())
        .map_err(|error| wasmtime::Error::msg(format!("fd_read nread write failed: {error}")))?;
    caller.data_mut().reads.push(ReadRecord {
        descriptor,
        capacity,
        bytes,
        reported,
        errno,
    });
    Ok(errno)
}

fn recording_fd_write(
    mut caller: Caller<'_, HostState>,
    descriptor: i32,
    iovs: i32,
    iovs_len: i32,
    nwritten: i32,
) -> Result<i32, wasmtime::Error> {
    if iovs_len != 1 {
        return Err(wasmtime::Error::msg(format!(
            "fd_write expected one iovec, observed {iovs_len}"
        )));
    }
    let memory = caller
        .get_export("memory")
        .and_then(|export| export.into_memory())
        .ok_or_else(|| wasmtime::Error::msg("fd_write guest memory is missing"))?;
    let iovec = read_guest_memory(&memory, &mut caller, iovs as u32, 8)?;
    let data = u32::from_le_bytes(iovec[..4].try_into().expect("iovec address"));
    let length = u32::from_le_bytes(iovec[4..].try_into().expect("iovec length"));
    let bytes = read_guest_memory(&memory, &mut caller, data, length as usize)?;
    memory
        .write(&mut caller, nwritten as usize, &length.to_le_bytes())
        .map_err(|error| {
            wasmtime::Error::msg(format!("fd_write nwritten write failed: {error}"))
        })?;
    caller.data_mut().writes.push(WriteRecord {
        descriptor,
        text: String::from_utf8(bytes)
            .map_err(|error| wasmtime::Error::msg(format!("fd_write text is not UTF-8: {error}")))?,
        reported: length,
    });
    Ok(0)
}

fn assert_fd_read_result(
    reads: &[ReadRecord],
    read_assertion: &FdReadAssertion,
    case: &Path,
) -> Result<(), String> {
    if reads.len() != read_assertion.calls {
        return Err(format!(
            "{}: fd_read call count expected {}, observed {}",
            case.display(),
            read_assertion.calls,
            reads.len()
        ));
    }
    for (index, read) in reads.iter().enumerate() {
        let expected_bytes: &[u8] = if index == 0 && read_assertion.errno == 0 {
            read_assertion.bytes.as_slice()
        } else {
            &[]
        };
        let expected_reported = expected_bytes.len() as u32;
        if read.descriptor != 0
            || read.capacity != read_assertion.capacity
            || read.bytes != expected_bytes
            || read.reported != expected_reported
            || read.errno != read_assertion.errno
        {
            return Err(format!(
                "{}: fd_read record {index} has unexpected descriptor, capacity, bytes, reported count, or errno",
                case.display()
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn link_shared_stdlib(stdlib: &Path, temporary: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(stdlib, temporary.join("stdlib"))
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn link_shared_stdlib(stdlib: &Path, temporary: &Path) -> Result<(), String> {
    std::os::windows::fs::symlink_dir(stdlib, temporary.join("stdlib"))
        .map_err(|error| error.to_string())
}

fn remove_manifest_field(manifest: &mut serde_json::Value, path: &str) -> Result<(), String> {
    let fields = path.split('.').collect::<Vec<_>>();
    let (field, parent) = fields
        .split_last()
        .ok_or_else(|| "manifest field path must not be empty".to_owned())?;
    manifest_field_parent(manifest, parent)?
        .remove(*field)
        .ok_or_else(|| format!("manifest field {path} is missing"))?;
    Ok(())
}

fn replace_manifest_field(
    manifest: &mut serde_json::Value,
    path: &str,
    replacement: &str,
) -> Result<(), String> {
    let fields = path.split('.').collect::<Vec<_>>();
    let (field, parent) = fields
        .split_last()
        .ok_or_else(|| "manifest field path must not be empty".to_owned())?;
    let entry = manifest_field_parent(manifest, parent)?
        .get_mut(*field)
        .ok_or_else(|| format!("manifest field {path} is missing"))?;
    *entry = serde_json::Value::String(replacement.to_owned());
    Ok(())
}

fn manifest_field_parent<'a>(
    manifest: &'a mut serde_json::Value,
    fields: &[&str],
) -> Result<&'a mut serde_json::Map<String, serde_json::Value>, String> {
    if let Some((field, remaining)) = fields.split_first() {
        let object = manifest
            .as_object_mut()
            .ok_or_else(|| format!("manifest field {field} has no object parent"))?;
        let value = object
            .get_mut(*field)
            .ok_or_else(|| format!("manifest field {field} is missing"))?;
        manifest_field_parent(value, remaining)
    } else {
        manifest
            .as_object_mut()
            .ok_or_else(|| "manifest field has no object parent".to_owned())
    }
}

fn partial_fd_write_count(assertion: &toml_edit::Table) -> Result<Option<u32>, String> {
    let Some(item) = assertion.get("partial_fd_write_count") else {
        return Ok(None);
    };
    let count = item
        .as_value()
        .and_then(|value| value.as_integer())
        .ok_or_else(|| "partial_fd_write_count must be an integer".to_owned())?;
    u32::try_from(count)
        .map(Some)
        .map_err(|_| "partial_fd_write_count must fit in u32".to_owned())
}

fn run_partial_fd_write_assertion(
    assertion: &toml_edit::Table,
    partial_count: u32,
    temporary: &Path,
    case: &Path,
    binary: &Path,
) -> Result<(), String> {
    if assertion.get("command").is_some() {
        return Err(format!(
            "{}: partial_fd_write_count assertion accepts no command",
            case.display()
        ));
    }
    if assertion
        .get("source")
        .and_then(Item::as_value)
        .and_then(|value| value.as_str())
        .is_none()
    {
        return Err(format!(
            "{}: partial_fd_write_count assertion requires source",
            case.display()
        ));
    }
    select_source(assertion, temporary, case)?;
    let status = Command::new(binary)
        .arg("build")
        .current_dir(temporary)
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!(
            "{}: partial_fd_write_count source build failed",
            case.display()
        ));
    }
    let executable = artifact_executable(temporary, case)?;
    run_partial_fd_write_host(&executable, partial_count, case)
}

fn artifact_executable(temporary: &Path, case: &Path) -> Result<PathBuf, String> {
    let manifest_path = temporary.join(ARTIFACT_MANIFEST);
    let manifest = fs::read(&manifest_path).map_err(|error| error.to_string())?;
    let manifest = serde_json::from_slice::<serde_json::Value>(&manifest)
        .map_err(|error| format!("{}: artifact manifest is not JSON: {error}", case.display()))?;
    let executable = manifest
        .get("executable")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            format!(
                "{}: artifact manifest executable is missing",
                case.display()
            )
        })?;
    Ok(temporary.join(executable))
}

fn run_partial_fd_write_host(
    executable: &Path,
    partial_count: u32,
    case: &Path,
) -> Result<(), String> {
    let engine = Engine::default();
    let module = Module::from_file(&engine, executable).map_err(|error| error.to_string())?;
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |state: &mut HostState| &mut state.wasi)
        .map_err(|error| error.to_string())?;
    linker.allow_shadowing(true);
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_write",
            move |mut caller: Caller<'_, HostState>,
                  fd: i32,
                  iovs: i32,
                  iovs_len: i32,
                  nwritten: i32| {
                controlled_fd_write(&mut caller, fd, iovs, iovs_len, nwritten, partial_count)
            },
        )
        .map_err(|error| error.to_string())?;
    let wasi = WasiCtx::builder().build_p1();
    let mut store = Store::new(
        &engine,
        HostState {
            wasi,
            writes: Vec::new(),
            reads: Vec::new(),
            read_bytes: Vec::new(),
            read_errno: 0,
            read_capacity: 0,
        },
    );
    let instance = linker.instantiate(&mut store, &module).map_err(|error| {
        format!(
            "{}: embedded P1 instantiation failed: {error}",
            case.display()
        )
    })?;
    instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .map_err(|error| format!("{}: _start export is missing: {error}", case.display()))?
        .call(&mut store, ())
        .map_err(|error| format!("{}: _start failed: {error}", case.display()))?;
    assert_partial_fd_write_result(&store.data().writes, partial_count, case)
}

fn controlled_fd_write(
    caller: &mut Caller<'_, HostState>,
    descriptor: i32,
    iovs: i32,
    iovs_len: i32,
    nwritten: i32,
    partial_count: u32,
) -> Result<i32, wasmtime::Error> {
    if iovs_len != 1 {
        return Err(wasmtime::Error::msg(format!(
            "fd_write expected one iovec, observed {iovs_len}"
        )));
    }
    let memory = caller
        .get_export("memory")
        .and_then(|export| export.into_memory())
        .ok_or_else(|| wasmtime::Error::msg("fd_write guest memory is missing"))?;
    let iovec = read_guest_memory(&memory, caller, iovs as u32, 8)?;
    let data = u32::from_le_bytes(iovec[..4].try_into().expect("iovec address"));
    let length = u32::from_le_bytes(iovec[4..].try_into().expect("iovec length"));
    let bytes = read_guest_memory(&memory, caller, data, length as usize)?;
    let reported = match descriptor {
        1 => partial_count,
        2 => length,
        _ => {
            return Err(wasmtime::Error::msg(format!(
                "fd_write expected descriptor 1 or 2, observed {descriptor}"
            )));
        }
    };
    memory
        .write(&mut *caller, nwritten as usize, &reported.to_le_bytes())
        .map_err(|error| {
            wasmtime::Error::msg(format!("fd_write nwritten write failed: {error}"))
        })?;
    caller.data_mut().writes.push(WriteRecord {
        descriptor,
        text: String::from_utf8(bytes).map_err(|error| {
            wasmtime::Error::msg(format!("fd_write text is not UTF-8: {error}"))
        })?,
        reported,
    });
    Ok(0)
}

fn read_guest_memory(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    offset: u32,
    length: usize,
) -> Result<Vec<u8>, wasmtime::Error> {
    let mut bytes = vec![0; length];
    memory
        .read(caller, offset as usize, &mut bytes)
        .map_err(|error| {
            wasmtime::Error::msg(format!("fd_write guest memory read failed: {error}"))
        })?;
    Ok(bytes)
}

fn assert_partial_fd_write_result(
    writes: &[WriteRecord],
    partial_count: u32,
    case: &Path,
) -> Result<(), String> {
    let expected = [
        WriteRecord {
            descriptor: 1,
            text: "partial\n".to_owned(),
            reported: partial_count,
        },
        WriteRecord {
            descriptor: 2,
            text: "partial result\n".to_owned(),
            reported: "partial result\n".len() as u32,
        },
    ];
    if writes.len() != expected.len() {
        return Err(format!(
            "{}: fd_write record count expected {}, observed {}",
            case.display(),
            expected.len(),
            writes.len()
        ));
    }
    for (index, (actual, expected)) in writes.iter().zip(expected.iter()).enumerate() {
        if actual != expected {
            return Err(format!(
                "{}: fd_write record {index} expected descriptor {}, text {:?}, reported {}; observed descriptor {}, text {:?}, reported {}",
                case.display(),
                expected.descriptor,
                expected.text,
                expected.reported,
                actual.descriptor,
                actual.text,
                actual.reported
            ));
        }
    }
    Ok(())
}

fn requires_selected_source_build(has_source_selector: bool, command: &[String]) -> bool {
    has_source_selector
        && command.first().map(String::as_str) == Some("wosy")
        && command.get(1).map(String::as_str) == Some("run")
}

fn assert_source_observations(
    assertion: &toml_edit::Table,
    temporary: &Path,
    case: &Path,
) -> Result<(), String> {
    let Some(expected) = assertion
        .get("source_observations")
        .and_then(Item::as_array_of_tables)
    else {
        return Ok(());
    };
    let manifest = fs::read(temporary.join(".wosy/artifacts/app/dev/main/artifact-manifest.json"))
        .map_err(|error| error.to_string())?;
    let manifest = serde_json::from_slice::<serde_json::Value>(&manifest)
        .map_err(|error| format!("{}: manifest is not JSON: {error}", case.display()))?;
    let observations = manifest
        .get("source_observations")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("{}: source observations are missing", case.display()))?;
    if observations.len() != expected.len() {
        return Err(format!(
            "{}: source observation count differs",
            case.display()
        ));
    }
    for (index, expected) in expected.iter().enumerate() {
        let observed = observations
            .get(index)
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| format!("{}: source observation is not an object", case.display()))?;
        let source = observed
            .get("source")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| format!("{}: source identity is missing", case.display()))?;
        for (field, item) in expected {
            let value = item
                .as_value()
                .and_then(|value| value.as_str())
                .ok_or_else(|| "source observation fields must be strings".to_owned())?;
            let observed_value = source
                .get(field)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| format!("{}: source field {field} is missing", case.display()))?;
            if observed_value != value {
                return Err(format!(
                    "{}: source observation {index} has unexpected {field}",
                    case.display()
                ));
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::{
        assert_partial_fd_write_result, link_shared_stdlib, partial_fd_write_count,
        requires_selected_source_build, WriteRecord,
    };
    use std::{env, fs, path::Path};
    use toml_edit::DocumentMut;

    #[test]
    fn selected_source_negative_build_assertion_runs_directly() {
        let command = vec!["wosy".to_owned(), "build".to_owned()];

        assert!(!requires_selected_source_build(true, &command));
    }

    #[test]
    fn selected_source_runtime_assertion_builds_first() {
        let command = vec!["wosy".to_owned(), "run".to_owned()];

        assert!(requires_selected_source_build(true, &command));
    }

    #[test]
    fn recognizes_partial_fd_write_count() {
        let document =
            "[[assert]]\nsource = \"project/src/public_partial.w\"\npartial_fd_write_count = 3\n"
                .parse::<DocumentMut>()
                .expect("assertion document");
        let assertion = document
            .get("assert")
            .and_then(toml_edit::Item::as_array_of_tables)
            .and_then(|assertions| assertions.iter().next())
            .expect("partial assertion");

        assert_eq!(partial_fd_write_count(assertion), Ok(Some(3)));
    }

    #[test]
    fn ignores_assertions_without_partial_fd_write_count() {
        let assertion = toml_edit::Table::new();

        assert_eq!(partial_fd_write_count(&assertion), Ok(None));
    }

    #[test]
    fn validates_controlled_partial_write_records() {
        let writes = [
            WriteRecord {
                descriptor: 1,
                text: "partial\n".to_owned(),
                reported: 3,
            },
            WriteRecord {
                descriptor: 2,
                text: "partial result\n".to_owned(),
                reported: 15,
            },
        ];

        assert_eq!(
            assert_partial_fd_write_result(&writes, 3, Path::new("runtime_stdio")),
            Ok(())
        );
    }

    #[test]
    fn reports_controlled_partial_write_record_mismatches() {
        let writes = [WriteRecord {
            descriptor: 1,
            text: "partial\n".to_owned(),
            reported: 3,
        }];

        assert_eq!(
            assert_partial_fd_write_result(&writes, 3, Path::new("runtime_stdio")),
            Err("runtime_stdio: fd_write record count expected 2, observed 1".to_owned())
        );
    }

    #[test]
    fn shared_stdlib_link_exposes_its_manifest() {
        let temporary = env::temp_dir().join(format!(
            "wosy-fixture-harness-stdlib-link-{}",
            std::process::id()
        ));
        fs::create_dir_all(&temporary).expect("temporary directory");
        let result = (|| -> Result<(), String> {
            let stdlib = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stdlib");
            link_shared_stdlib(&stdlib, &temporary)?;
            fs::read_to_string(temporary.join("stdlib/wosy.toml"))
                .map(|_| ())
                .map_err(|error| error.to_string())
        })();

        fs::remove_dir_all(&temporary).expect("temporary directory cleanup");
        assert!(!temporary.exists(), "temporary directory was not removed");
        result.expect("shared stdlib manifest is readable");
    }
}
