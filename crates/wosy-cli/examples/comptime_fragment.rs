//! Local fixture adapter; it never invokes configured project tools.
use std::{io::{self, Read}, path::Path};
use wosy_cli::comptime_fragment::{plan_closed_comptime_fragment, plan_closed_comptime_staging, ClosedComptimeStaging};
use wosy_compiler::{core_runtime, ComptimeParameterOrigin, ComptimeParameterValue, ScalarTargetLayout, ScalarType};

const MAX_FIXTURE_VALUE: usize = 12; // -2147483648\n

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = args.next().ok_or("expected fixture root")?;
    let target = args.next().ok_or("expected target")?;
    let profile = args.next().ok_or("expected profile")?;
    let id = args.next().ok_or("expected comptime id")?;
    let (layout, output) = match args.next().as_deref() {
        Some("native") => (ScalarTargetLayout::NATIVE64, "native"),
        Some("wasi") => (ScalarTargetLayout::WASM32, "wasm-wasip1"),
        _ => return Err("expected native or wasi evaluator layout".into()),
    };
    let mode = args.next();
    if let Some(mode) = mode.as_deref() {
        return fixture(&root, &target, &profile, &id, layout, output, mode, args);
    }
    let fragment = plan_closed_comptime_fragment(Path::new(&root), &target, &profile, &id, layout)
        .map_err(|error| format!("{error:?}"))?;
    let answer = serde_json::json!({
        "id": fragment.id(), "site": fragment.site(), "result_type": fragment.result_type(),
        "llvm": fragment.llvm().to_text(), "runtime": core_runtime("llvm", output)?,
    });
    println!("{answer}");
    Ok(())
}

// Private test shim. Its receipt only relates fixture stages; it is neither
// a public manifest nor executable authority or a cache/result key.
fn fixture(
    root: &str, target: &str, profile: &str, id: &str, layout: ScalarTargetLayout,
    output: &str, mode: &str, mut args: impl Iterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let expected = match mode {
        "fixture-stage" => None,
        "fixture-readback" => {
            let digest = args.next().ok_or("expected reviewed fixture observation")?;
            if digest.len() != 64 || !digest.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                return Err("invalid fixture observation digest".into());
            }
            Some(digest)
        }
        _ => return Err("unknown fixture mode".into()),
    };
    if args.next().is_some() { return Err("unexpected fixture argument".into()); }
    let stage = plan_closed_comptime_staging(Path::new(root), target, profile, id, layout)
        .map_err(|error| format!("{error:?}"))?;
    let runtime = core_runtime("llvm", output)?;
    let mut llvm = Vec::new();
    let ir = if expected.is_some() {
        stage.write_evaluator_ir(&mut io::sink(), 1 << 20)?
    } else {
        stage.write_evaluator_ir(&mut llvm, 1 << 20)?
    };
    let observation = fixture_observation(&stage, output, &ir.blake3, runtime)?;
    let fragment = stage.fragment();
    let result = if let Some(expected) = expected {
        if observation != expected { return Err("fixture source or selection changed".into()); }
        let bytes = read_fixture_value(io::stdin().lock())?;
        serde_json::json!({
            "observation": observation,
            "value": decode_fixture_word(fragment.result_type(), &bytes)?.fixture_report(),
            "result_type": fragment.result_type(),
        })
    } else {
        serde_json::json!({
            "observation": observation, "id": fragment.id(), "site": fragment.site(),
            "result_type": fragment.result_type(), "runtime": runtime, "llvm": String::from_utf8(llvm)?,
        })
    };
    println!("{result}");
    Ok(())
}

fn fixture_observation(
    stage: &ClosedComptimeStaging, output: &str, ir: &blake3::Hash, runtime: &str,
) -> Result<String, serde_json::Error> {
    let fragment = stage.fragment();
    let reads: Vec<_> = fragment.parameter_uses().iter().map(|read| {
        let origin = match read.origin {
            ComptimeParameterOrigin::Profile => serde_json::json!(["profile"]),
            ComptimeParameterOrigin::Default { literal } => serde_json::json!(["default", literal]),
        };
        let value = match read.resolved {
            ComptimeParameterValue::Boolean(value) => serde_json::json!(value),
            ComptimeParameterValue::Integer(value) => serde_json::json!(value),
        };
        serde_json::json!([read.name, read.call, read.context, value, origin])
    }).collect();
    let bytes = serde_json::to_vec(&serde_json::json!([
        "closed-comptime-owned-fixture", stage.configuration_identity(), stage.tools().target(),
        stage.tools().profile(), output, fragment.id(), fragment.site(), fragment.result_type(),
        reads, ir.to_hex().as_str(), blake3::hash(runtime.as_bytes()).to_hex().as_str()
    ]))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn read_fixture_value(input: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(MAX_FIXTURE_VALUE + 1);
    input.take((MAX_FIXTURE_VALUE + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FIXTURE_VALUE {
        return Err(io::Error::other("fixture stdout exceeds the word-line bound"));
    }
    Ok(bytes)
}

// Native and Wasmtime fixture shims expose a signed 32-bit word and LF.
// The originating compiler type interprets those bits, not stdout.
#[derive(Debug, Eq, PartialEq)]
enum FixtureValue {
    I32(i32),
    U32(u32),
    Bool(bool),
}

impl FixtureValue {
    fn fixture_report(self) -> serde_json::Value {
        match self {
            Self::I32(value) => serde_json::json!(value),
            Self::U32(value) => serde_json::json!(value),
            Self::Bool(value) => serde_json::json!(value),
        }
    }
}

fn decode_fixture_word(kind: &ScalarType, bytes: &[u8]) -> Result<FixtureValue, &'static str> {
    let line = std::str::from_utf8(bytes).map_err(|_| "fixture word is not UTF-8")?
        .strip_suffix('\n').ok_or("fixture word needs one terminal LF")?;
    let value: i32 = line.parse().map_err(|_| "invalid fixture word")?;
    if value.to_string() != line { return Err("noncanonical fixture word"); }
    match kind {
        ScalarType::I32 => Ok(FixtureValue::I32(value)),
        ScalarType::U32 => Ok(FixtureValue::U32(u32::from_ne_bytes(value.to_ne_bytes()))),
        ScalarType::Bool if value == 0 || value == 1 => Ok(FixtureValue::Bool(value == 1)),
        ScalarType::Bool => Err("not a fixture bool bit"),
        _ => Err("unsupported fixture result type"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_accepts_only_one_canonical_word_under_the_compiler_type() {
        for (kind, bytes, expected) in [
            (ScalarType::I32, &b"-2147483648\n"[..], FixtureValue::I32(i32::MIN)),
            (ScalarType::U32, &b"-1\n"[..], FixtureValue::U32(u32::MAX)),
            (ScalarType::Bool, &b"1\n"[..], FixtureValue::Bool(true)),
            (ScalarType::Bool, &b"0\n"[..], FixtureValue::Bool(false)),
        ] {
            assert_eq!(decode_fixture_word(&kind, &read_fixture_value(bytes).unwrap()).unwrap(), expected);
        }
        for bytes in [&b"+1\n"[..], b"01\n", b"-0\n", b" 1\n", b"1\r\n", b"1\n\n", b"1", b"", b"2147483648\n"] {
            assert!(decode_fixture_word(&ScalarType::I32, bytes).is_err());
        }
        assert!(decode_fixture_word(&ScalarType::Bool, b"2\n").is_err());
        assert!(read_fixture_value(&b"0000000000000"[..]).is_err());
    }
}
