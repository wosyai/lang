//! Local fixture adapter; it never invokes configured project tools.
use std::path::Path;
use wosy_cli::comptime_fragment::plan_closed_comptime_fragment;
use wosy_compiler::{core_runtime, ScalarTargetLayout};

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
    let fragment = plan_closed_comptime_fragment(Path::new(&root), &target, &profile, &id, layout)
        .map_err(|error| format!("{error:?}"))?;
    let answer = serde_json::json!({
        "id": fragment.id(), "site": fragment.site(), "result_type": fragment.result_type(),
        "llvm": fragment.llvm().to_text(), "runtime": core_runtime("llvm", output)?,
    });
    println!("{answer}");
    Ok(())
}
