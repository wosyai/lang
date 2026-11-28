use super::{PreparedScalarProject, ScalarFunction, ScalarItem, ScalarModule, ScalarProject, ScalarTargetLayout, ScalarType};
use std::collections::BTreeMap;
use wosy_syntax::SourceIdentity;

/// A source declaration that the compiler is instructed to wrap in this
/// prepared LLVM partition. This conveys positive source identity only: a
/// downstream builder may expose additional symbols, or omit the wrapper.
pub(crate) struct PreparedWasmRootExport<'project, 'manifest> {
    label: &'manifest str,
    module: &'project ScalarModule,
    item: usize,
}

impl<'project, 'manifest> PreparedWasmRootExport<'project, 'manifest> {
    pub(crate) fn label(&self) -> &'manifest str {
        self.label
    }

    pub(crate) fn source(&self) -> &'project SourceIdentity {
        &self.module.source
    }

    #[cfg(any(test, feature = "internal-checker"))]
    pub(crate) fn declaration_item(&self) -> usize {
        self.item
    }

    pub(crate) fn function(&self) -> &'project ScalarFunction {
        let ScalarItem::Function(function) = &self.module.items[self.item] else {
            unreachable!("root export preparation retains an immutable function item");
        };
        function
    }
}

pub(crate) fn resolve_prepared_wasm_root_exports<'project, 'manifest>(
    prepared: &'project PreparedScalarProject,
    root: &SourceIdentity,
    requested: &'manifest BTreeMap<String, String>,
) -> Result<Vec<PreparedWasmRootExport<'project, 'manifest>>, String> {
    resolve_wasm_root_exports(prepared.project(), root, requested)
}

pub(crate) fn resolve_wasm_root_exports<'project, 'manifest>(
    project: &'project ScalarProject,
    root: &SourceIdentity,
    requested: &'manifest BTreeMap<String, String>,
) -> Result<Vec<PreparedWasmRootExport<'project, 'manifest>>, String> {
    let module = project.modules.iter().find(|module| &module.source == root)
        .ok_or_else(|| format!("missing selected Wasm export root {}", root.path))?;
    if module.target_layout != ScalarTargetLayout::WASM32 {
        return Err("Wasm root exports require a Wasm artifact".into());
    }
    let direct = |ty: &ScalarType| matches!(
        ty, ScalarType::I32 | ScalarType::U32 | ScalarType::I64 | ScalarType::U64
            | ScalarType::F32 | ScalarType::F64
    );
    let mut root_functions = requested
        .values()
        .map(|name| (name.as_str(), None))
        .collect::<BTreeMap<_, _>>();
    for (item, declaration) in module.items.iter().enumerate() {
        if let ScalarItem::Function(function) = declaration {
            if let Some(entry) = root_functions.get_mut(function.name.as_str()) {
                match entry {
                    Some((_, _, ambiguous)) => *ambiguous = true,
                    None => *entry = Some((item, function, false)),
                }
            }
        }
    }
    let mut exports = Vec::with_capacity(requested.len());
    for (label, authored) in requested {
        let mut bytes = label.bytes();
        if !bytes.next().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
            || !bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
            || matches!(label.as_str(), "main" | "_start" | "memory")
            || label.starts_with("__") || label.starts_with("wosy_")
        {
            return Err(format!("invalid or reserved Wasm root export label {label:?}"));
        }
        let (item, function, ambiguous) = root_functions[authored.as_str()].ok_or_else(|| {
            format!("Wasm root export {label:?} names no root function {authored:?}")
        })?;
        if ambiguous || !function.generic_parameters.is_empty()
            || !function.overload_arms.is_empty()
        {
            return Err(format!("Wasm root export {label:?} needs an unambiguous concrete declaration"));
        }
        let ScalarType::Callable { outputs, parameters } = &function.signature else {
            return Err(format!("Wasm root export {label:?} is not callable"));
        };
        if !parameters.iter().all(direct) || !match outputs.outputs.as_slice() {
            [] => true,
            [output] => output.ty == ScalarType::Unit || direct(&output.ty),
            _ => false,
        } {
            return Err(format!("Wasm root export {label:?} needs only direct scalar ABI values"));
        }
        exports.push(PreparedWasmRootExport { label, module, item });
    }
    Ok(exports)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scalar::{derive_scalar_declarations_from_cst, prepare_scalar_project, ScalarPreparationInputs};

    #[test]
    fn intended_wasm_labels_resolve_to_the_same_prepared_declaration() {
        let source = |revision: &str| SourceIdentity::new("example".into(), "app".into(),
            "src/main.w".into(), revision.into());
        let root = source("actual");
        let contents = r#"%%start
            i32 bias = 2;
            i32(i32) update = fn(value) { value+bias };
            bool(bool) bool_only = fn(value) { value };
            unit() dormant = fn {};
        %%end"#;
        let prepare = |target_layout| {
            let parsed = crate::parse_source(root.clone(), contents.into(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
            prepare_scalar_project(vec![(declarations, Vec::new())], vec![root.clone()],
                ScalarPreparationInputs { target_layout }).unwrap()
        };
        let wasm = prepare(ScalarTargetLayout::WASM32);
        let entries = BTreeMap::from([
            ("ui_first".to_owned(), "update".to_owned()),
            ("ui_second".to_owned(), "update".to_owned()),
        ]);
        let selected = resolve_prepared_wasm_root_exports(&wasm, &root, &entries).unwrap();
        let [first, second] = selected.as_slice() else { panic!("two labels for one source function"); };
        assert_eq!((first.label(), second.label()), ("ui_first", "ui_second"));
        assert_eq!(first.source(), &root);
        assert_eq!(first.declaration_item(), second.declaration_item());
        assert!(std::ptr::eq(first.function(), second.function()));
        assert_eq!(&contents[first.function().name_span.start as usize..first.function().name_span.end as usize], "update");
        assert!(resolve_prepared_wasm_root_exports(&wasm, &source("stale"), &entries).is_err());
        assert!(resolve_prepared_wasm_root_exports(&prepare(ScalarTargetLayout::NATIVE64), &root, &entries).is_err());
        let invalid = BTreeMap::from([("ui_flag".to_owned(), "bool_only".to_owned())]);
        assert!(resolve_prepared_wasm_root_exports(&wasm, &root, &invalid).is_err());
        assert!(resolve_prepared_wasm_root_exports(&wasm, &root, &BTreeMap::new()).unwrap().is_empty());
    }
}
