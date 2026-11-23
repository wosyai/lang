use super::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarFieldLayout {
    pub offset: Option<u64>,
    pub layout: Option<ScalarLayout>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarAggregateLayout {
    pub target: ScalarTargetLayout,
    pub layout: Option<ScalarLayout>,
    pub fields: Vec<ScalarFieldLayout>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalarTargetLayouts {
    pub target: ScalarTargetLayout,
    #[serde(with = "aggregate_entries")]
    aggregates: BTreeMap<ScalarStructId, ScalarAggregateLayout>,
}

mod aggregate_entries {
    use super::{ScalarAggregateLayout, ScalarStructId};
    use serde::de::{Error, SeqAccess, Visitor};
    use serde::ser::SerializeSeq;
    use serde::{Deserializer, Serializer};
    use std::collections::{btree_map::Entry, BTreeMap};
    use std::fmt;

    pub(super) fn serialize<S>(
        aggregates: &BTreeMap<ScalarStructId, ScalarAggregateLayout>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(aggregates.len()))?;
        for entry in aggregates {
            sequence.serialize_element(&entry)?;
        }
        sequence.end()
    }

    pub(super) fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<ScalarStructId, ScalarAggregateLayout>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(AggregateEntriesVisitor)
    }

    struct AggregateEntriesVisitor;

    impl<'de> Visitor<'de> for AggregateEntriesVisitor {
        type Value = BTreeMap<ScalarStructId, ScalarAggregateLayout>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a sequence of unique struct ID and aggregate layout pairs")
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut aggregates = BTreeMap::new();
            while let Some((id, layout)) =
                sequence.next_element::<(ScalarStructId, ScalarAggregateLayout)>()?
            {
                match aggregates.entry(id) {
                    Entry::Vacant(entry) => {
                        entry.insert(layout);
                    }
                    Entry::Occupied(_) => {
                        return Err(A::Error::custom("duplicate aggregate layout struct ID"))
                    }
                }
            }
            Ok(aggregates)
        }
    }
}

impl ScalarTargetLayouts {
    pub fn new(target: ScalarTargetLayout) -> Self {
        Self {
            target,
            aggregates: BTreeMap::new(),
        }
    }

    pub fn aggregate(&self, id: &ScalarStructId) -> Option<&ScalarAggregateLayout> {
        self.aggregates.get(id)
    }

    pub fn field(&self, id: &ScalarStructFieldId) -> Option<&ScalarFieldLayout> {
        self.aggregate(&id.structure)?.fields.get(id.index)
    }
}

pub(crate) struct ScalarLayoutIndex<'a> {
    aggregates: BTreeMap<&'a ScalarStructId, &'a ScalarAggregateLayout>,
}

impl<'a> ScalarLayoutIndex<'a> {
    pub(crate) fn program(layouts: &'a ScalarTargetLayouts) -> Self {
        #[cfg(test)]
        observations::record_lookup(layouts.aggregates.len());
        Self {
            aggregates: layouts.aggregates.iter().collect(),
        }
    }

    pub(crate) fn project(modules: &'a [ScalarModule]) -> Self {
        #[cfg(test)]
        observations::record_lookup(
            modules
                .iter()
                .map(|module| module.layouts.aggregates.len())
                .sum(),
        );
        Self {
            aggregates: modules
                .iter()
                .flat_map(|module| module.layouts.aggregates.iter())
                .collect(),
        }
    }

    pub(crate) fn aggregate(&self, id: &ScalarStructId) -> Option<&ScalarAggregateLayout> {
        self.aggregates.get(id).copied()
    }

    pub(crate) fn field(&self, id: &ScalarStructFieldId) -> Option<&ScalarFieldLayout> {
        self.aggregate(&id.structure)?.fields.get(id.index)
    }
}

pub(super) fn resolve_program_struct_layouts(
    program: &mut ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let declarations = program
        .structs
        .iter()
        .map(|structure| (structure, program.target_layout))
        .collect::<Vec<_>>();
    program.layouts = ScalarTargetLayouts {
        target: program.target_layout,
        aggregates: compute_layouts(&declarations, diagnostics),
    };
}

pub(super) fn resolve_project_struct_layouts(
    project: &mut ScalarProject,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    for module in &mut project.modules {
        module.layouts = ScalarTargetLayouts::new(module.target_layout);
    }
    let declarations = project
        .modules
        .iter()
        .flat_map(|module| {
            module
                .structs
                .iter()
                .map(move |structure| (structure, module.target_layout))
        })
        .collect::<Vec<_>>();
    let mut results = compute_layouts(&declarations, diagnostics);
    for module in &mut project.modules {
        for structure in &module.structs {
            let (id, result) = results
                .remove_entry(&structure.id)
                .expect("computed aggregate layout");
            module.layouts.aggregates.insert(id, result);
        }
    }
    assert!(results.is_empty());
}

fn compute_layouts(
    declarations: &[(&ScalarStruct, ScalarTargetLayout)],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> BTreeMap<ScalarStructId, ScalarAggregateLayout> {
    #[cfg(test)]
    observations::record_context(declarations);
    let (cycles, order) = aggregate_layout_order(declarations);
    let mut results = BTreeMap::new();
    for ((participant, field_index), cycle) in cycles {
        let field = &declarations[participant].0.fields[field_index];
        recursive_layout_diagnostic(
            &declarations[participant].0.id.source,
            field.name_span,
            diagnostics,
        );
        for index in cycle {
            let (structure, target) = declarations[index];
            results.insert(structure.id.clone(), absent_layout(structure, target));
        }
    }
    for index in order {
        let (structure, target) = declarations[index];
        let result = compute_struct_layout(structure, &results, target, diagnostics);
        results.insert(structure.id.clone(), result);
    }
    results
}

fn aggregate_layout_order(
    structures: &[(&ScalarStruct, ScalarTargetLayout)],
) -> (Vec<((usize, usize), Vec<usize>)>, Vec<usize>) {
    let indices = structures
        .iter()
        .enumerate()
        .map(|(index, (structure, _))| (&structure.id, index))
        .collect::<BTreeMap<_, _>>();
    let dependencies = structures
        .iter()
        .map(|(structure, _)| {
            let mut ids = Vec::new();
            for field in &structure.fields {
                by_value_struct_dependencies(&field.ty, &mut ids);
            }
            ids.into_iter()
                .filter_map(|id| indices.get(id).copied())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    #[cfg(test)]
    observations::record_edges(dependencies.iter().map(Vec::len).sum());
    let components = aggregate_layout_components(&dependencies);
    let mut cyclic = vec![false; structures.len()];
    let mut cycles = Vec::new();
    for component in components {
        let self_edge = component.len() == 1 && dependencies[component[0]].contains(&component[0]);
        if component.len() > 1 || self_edge {
            for &index in &component {
                cyclic[index] = true;
            }
            cycles.push((cycle_participant(structures, &component), component));
        }
    }
    cycles.sort_unstable_by_key(|(participant, _)| *participant);
    let mut visited = vec![false; structures.len()];
    let mut order = Vec::new();
    for index in 0..structures.len() {
        aggregate_layout_visit(index, &dependencies, &cyclic, &mut visited, &mut order);
    }
    (cycles, order)
}

fn by_value_struct_dependencies<'a>(
    ty: &'a ScalarType,
    dependencies: &mut Vec<&'a ScalarStructId>,
) {
    match ty {
        ScalarType::Struct(id) => dependencies.push(id),
        ScalarType::Array { element, .. } => by_value_struct_dependencies(element, dependencies),
        _ => {}
    }
}

fn aggregate_layout_components(dependencies: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut index = 0;
    let mut indices = vec![None; dependencies.len()];
    let mut lowlinks = vec![0; dependencies.len()];
    let mut stack = Vec::new();
    let mut on_stack = vec![false; dependencies.len()];
    let mut components = Vec::new();
    for node in 0..dependencies.len() {
        if indices[node].is_none() {
            aggregate_layout_component_visit(
                node,
                dependencies,
                &mut index,
                &mut indices,
                &mut lowlinks,
                &mut stack,
                &mut on_stack,
                &mut components,
            );
        }
    }
    components
}

fn aggregate_layout_component_visit(
    node: usize,
    dependencies: &[Vec<usize>],
    index: &mut usize,
    indices: &mut [Option<usize>],
    lowlinks: &mut [usize],
    stack: &mut Vec<usize>,
    on_stack: &mut [bool],
    components: &mut Vec<Vec<usize>>,
) {
    indices[node] = Some(*index);
    lowlinks[node] = *index;
    *index += 1;
    stack.push(node);
    on_stack[node] = true;
    for &dependency in &dependencies[node] {
        if indices[dependency].is_none() {
            aggregate_layout_component_visit(
                dependency,
                dependencies,
                index,
                indices,
                lowlinks,
                stack,
                on_stack,
                components,
            );
            lowlinks[node] = lowlinks[node].min(lowlinks[dependency]);
        } else if on_stack[dependency] {
            lowlinks[node] = lowlinks[node].min(indices[dependency].expect("component index"));
        }
    }
    if lowlinks[node] == indices[node].expect("component index") {
        let mut component = Vec::new();
        loop {
            let member = stack.pop().expect("component member");
            on_stack[member] = false;
            component.push(member);
            if member == node {
                break;
            }
        }
        component.sort_unstable();
        components.push(component);
    }
}

fn aggregate_layout_visit(
    node: usize,
    dependencies: &[Vec<usize>],
    cyclic: &[bool],
    visited: &mut [bool],
    order: &mut Vec<usize>,
) {
    if visited[node] || cyclic[node] {
        return;
    }
    visited[node] = true;
    for &dependency in &dependencies[node] {
        aggregate_layout_visit(dependency, dependencies, cyclic, visited, order);
    }
    order.push(node);
}

fn cycle_participant(
    structures: &[(&ScalarStruct, ScalarTargetLayout)],
    cycle: &[usize],
) -> (usize, usize) {
    let members = cycle
        .iter()
        .map(|&index| &structures[index].0.id)
        .collect::<BTreeSet<_>>();
    for &structure_index in cycle {
        for (field_index, field) in structures[structure_index].0.fields.iter().enumerate() {
            let mut dependencies = Vec::new();
            by_value_struct_dependencies(&field.ty, &mut dependencies);
            if dependencies
                .iter()
                .any(|dependency| members.contains(dependency))
            {
                return (structure_index, field_index);
            }
        }
    }
    panic!("recursive aggregate component has a participating field")
}

fn recursive_layout_diagnostic(
    source: &SourceIdentity,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    diagnostics.push(super::Diagnostic {
        code: "B0003".to_owned(),
        severity: super::DiagnosticSeverity::Error,
        message: "recursive by-value struct layout is unsupported".to_owned(),
        labels: vec![super::DiagnosticLabel {
            kind: super::DiagnosticLabelKind::Primary,
            span: SourceSpan::new(source.clone(), span),
            message: "field participates in a recursive struct layout".to_owned(),
        }],
        notes: Vec::new(),
    });
}

fn absent_layout(structure: &ScalarStruct, target: ScalarTargetLayout) -> ScalarAggregateLayout {
    #[cfg(test)]
    observations::record_result(structure.fields.len());
    ScalarAggregateLayout {
        target,
        layout: None,
        fields: structure
            .fields
            .iter()
            .map(|_| ScalarFieldLayout {
                offset: None,
                layout: None,
            })
            .collect(),
    }
}

fn compute_struct_layout(
    structure: &ScalarStruct,
    results: &BTreeMap<ScalarStructId, ScalarAggregateLayout>,
    target_layout: ScalarTargetLayout,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarAggregateLayout {
    #[cfg(test)]
    observations::record_completion();
    let source = &structure.id.source;
    let mut offset = 0;
    let mut alignment = 1;
    let mut result = absent_layout(structure, target_layout);
    for (field, observation) in structure.fields.iter().zip(&mut result.fields) {
        let layout = match layout_for_type_in_results(&field.ty, results, target_layout) {
            Ok(layout) => layout,
            Err(error) => {
                layout_error_source(source, error, diagnostics);
                return result;
            }
        };
        alignment = alignment.max(layout.alignment);
        let Some((field_offset, next_offset)) = checked_layout_offset(offset, &layout) else {
            layout_error_source(
                source,
                LayoutError::AggregateSizeOverflow(field.name_span),
                diagnostics,
            );
            clear_layout(&mut result);
            return result;
        };
        observation.offset = Some(field_offset);
        observation.layout = Some(layout);
        offset = next_offset;
    }
    let Some(size) = checked_align_offset(offset, alignment) else {
        layout_error_source(
            source,
            LayoutError::AggregateSizeOverflow(structure.name_span),
            diagnostics,
        );
        clear_layout(&mut result);
        return result;
    };
    result.layout = Some(ScalarLayout { size, alignment });
    result
}

fn clear_layout(result: &mut ScalarAggregateLayout) {
    result.layout = None;
    for field in &mut result.fields {
        field.layout = None;
        field.offset = None;
    }
}

fn layout_for_type_in_results(
    ty: &ScalarType,
    results: &BTreeMap<ScalarStructId, ScalarAggregateLayout>,
    target_layout: ScalarTargetLayout,
) -> Result<ScalarLayout, LayoutError> {
    match ty {
        ScalarType::Array {
            element,
            length,
            length_span,
            ..
        } => {
            let element = layout_for_type_in_results(element, results, target_layout)?;
            Ok(ScalarLayout {
                size: element
                    .size
                    .checked_mul(*length)
                    .ok_or(LayoutError::ArraySizeOverflow(*length_span))?,
                alignment: element.alignment,
            })
        }
        ScalarType::Struct(id) => results
            .get(id)
            .and_then(|structure| structure.layout.clone())
            .ok_or(LayoutError::InvalidType),
        _ => layout_for_type(ty, target_layout),
    }
}

#[cfg(test)]
pub(in crate::scalar) mod observations {
    use super::*;

    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    pub(in crate::scalar) struct Counts {
        pub contexts: usize,
        pub declarations: usize,
        pub fields: usize,
        pub edges: usize,
        pub completions: usize,
        pub aggregate_results: usize,
        pub field_results: usize,
        pub lookups: usize,
        pub lookup_entries: usize,
        pub program_copies: usize,
        pub module_constructions: usize,
        pub module_copies: usize,
        pub struct_copies: usize,
        pub cfg_copies: usize,
        pub copied_payload_units: usize,
    }

    thread_local! {
        static ACTIVE: RefCell<Option<Counts>> = const { RefCell::new(None) };
    }

    pub(in crate::scalar) struct Scope {
        previous: Option<Counts>,
    }

    impl Scope {
        pub(in crate::scalar) fn new() -> Self {
            Self {
                previous: ACTIVE.with(|active| active.replace(Some(Counts::default()))),
            }
        }

        pub(in crate::scalar) fn counts(&self) -> Counts {
            ACTIVE.with(|active| {
                active
                    .borrow()
                    .as_ref()
                    .expect("active layout observations")
                    .clone()
            })
        }
    }

    impl Drop for Scope {
        fn drop(&mut self) {
            ACTIVE.with(|active| active.replace(self.previous.take()));
        }
    }

    fn record(operation: impl FnOnce(&mut Counts)) {
        ACTIVE.with(|active| {
            if let Some(counts) = active.borrow_mut().as_mut() {
                operation(counts);
            }
        });
    }

    pub(super) fn record_context(declarations: &[(&ScalarStruct, ScalarTargetLayout)]) {
        record(|counts| {
            counts.contexts += 1;
            counts.declarations += declarations.len();
            counts.fields += declarations
                .iter()
                .map(|(structure, _)| structure.fields.len())
                .sum::<usize>();
        });
    }

    pub(super) fn record_edges(edges: usize) {
        record(|counts| counts.edges += edges);
    }
    pub(super) fn record_completion() {
        record(|counts| counts.completions += 1);
    }
    pub(super) fn record_result(fields: usize) {
        record(|counts| {
            counts.aggregate_results += 1;
            counts.field_results += fields;
        });
    }
    pub(super) fn record_lookup(entries: usize) {
        record(|counts| {
            counts.lookups += 1;
            counts.lookup_entries += entries;
        });
    }
    pub(super) fn record_program_copy(items: usize) {
        record(|counts| {
            counts.program_copies += 1;
            counts.copied_payload_units += items;
        });
    }
    pub(super) fn record_module_copy(items: usize) {
        record(|counts| {
            counts.module_copies += 1;
            counts.copied_payload_units += items;
        });
    }
    pub(in crate::scalar) fn record_module_construction() {
        record(|counts| counts.module_constructions += 1);
    }
    pub(super) fn record_struct_copy(fields: usize) {
        record(|counts| {
            counts.struct_copies += 1;
            counts.copied_payload_units += fields;
        });
    }
    pub(super) fn record_cfg_copy(points: usize) {
        record(|counts| {
            counts.cfg_copies += 1;
            counts.copied_payload_units += points;
        });
    }
}

#[cfg(test)]
impl Clone for ScalarProgram {
    fn clone(&self) -> Self {
        observations::record_program_copy(self.items.len());
        Self {
            source: self.source.clone(),
            items: self.items.clone(),
            structs: self.structs.clone(),
            enums: self.enums.clone(),
            target_layout: self.target_layout,
            layouts: self.layouts.clone(),
            resolved_assignment_outputs: self.resolved_assignment_outputs.clone(),
        }
    }
}

#[cfg(test)]
impl Clone for ScalarModule {
    fn clone(&self) -> Self {
        observations::record_module_copy(self.items.len());
        Self {
            source: self.source.clone(),
            items: self.items.clone(),
            namespace_bindings: self.namespace_bindings.clone(),
            members: self.members.clone(),
            initialization_nodes: self.initialization_nodes.clone(),
            structs: self.structs.clone(),
            enums: self.enums.clone(),
            target_layout: self.target_layout,
            layouts: self.layouts.clone(),
            resolved_assignment_outputs: self.resolved_assignment_outputs.clone(),
        }
    }
}

#[cfg(test)]
impl Clone for ScalarStruct {
    fn clone(&self) -> Self {
        observations::record_struct_copy(self.fields.len());
        Self {
            id: self.id.clone(),
            name: self.name.clone(),
            name_span: self.name_span,
            fields: self.fields.clone(),
            copy_policy: self.copy_policy,
            copy_modifier_span: self.copy_modifier_span,
            span: self.span,
        }
    }
}

#[cfg(test)]
impl Clone for ScalarReferenceCfg {
    fn clone(&self) -> Self {
        observations::record_cfg_copy(self.points.len() + self.initial_points.len());
        Self {
            facts: self.facts.clone(),
            points: self.points.clone(),
            initial_points: self.initial_points.clone(),
            allocations: self.allocations.clone(),
            specialized_policies: self.specialized_policies.clone(),
            primary_call_points: self.primary_call_points.clone(),
            guarded_scalar_allocation: self.guarded_scalar_allocation.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_source;

    fn program(
        structs: usize,
        body: usize,
        module: usize,
        target: ScalarTargetLayout,
    ) -> ScalarProgram {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            format!("src/module_{module}.w"),
            "revision".into(),
        );
        let mut text = String::from("%%start\n");
        for index in 0..structs {
            text.push_str(&format!(
                "struct Value{index} {{ u8 tag; *?u8 pointer; }}\n"
            ));
        }
        text.push_str("u64() payload = fn() { 1");
        for _ in 0..body {
            text.push_str(" + 1");
        }
        text.push_str(" };\n%%end");
        let parse = parse_source(source, text, &[]);
        assert!(parse.diagnostics.is_empty(), "{:?}", parse.diagnostics);
        let validation = derive_scalar_program_with_layout(&parse.result, target);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        validation.program
    }

    fn assert_partition(
        structures: &[ScalarStruct],
        layouts: &ScalarTargetLayouts,
        target: ScalarTargetLayout,
    ) {
        assert_eq!(layouts.target, target);
        assert_eq!(layouts.aggregates.len(), structures.len());
        for structure in structures {
            let result = layouts
                .aggregate(&structure.id)
                .expect("complete aggregate membership");
            assert_eq!(result.target, target);
            assert_eq!(result.fields.len(), structure.fields.len());
            assert_eq!(
                result.layout,
                Some(ScalarLayout {
                    size: target.pointer_size * 2,
                    alignment: target.pointer_alignment
                })
            );
            for (index, field) in structure.fields.iter().enumerate() {
                let observation = layouts.field(&field.id).expect("complete field membership");
                assert_eq!(observation, &result.fields[index]);
                assert_eq!(
                    observation.offset,
                    Some(if index == 0 { 0 } else { target.pointer_size })
                );
                assert_eq!(
                    observation.layout,
                    Some(ScalarLayout {
                        size: if index == 0 { 1 } else { target.pointer_size },
                        alignment: if index == 0 {
                            1
                        } else {
                            target.pointer_alignment
                        },
                    })
                );
            }
        }
    }

    fn assert_counts(counts: &observations::Counts, structs: usize, fields: usize) {
        assert_eq!(counts.contexts, 1);
        assert_eq!(counts.declarations, structs);
        assert_eq!(counts.fields, fields);
        assert_eq!(counts.completions, structs);
        assert_eq!(counts.aggregate_results, structs);
        assert_eq!(counts.field_results, fields);
        assert_eq!(counts.edges, 0);
        assert_eq!(counts.program_copies, 0);
        assert_eq!(counts.module_copies, 0);
        assert_eq!(counts.struct_copies, 0);
        assert_eq!(counts.cfg_copies, 0);
        assert_eq!(counts.copied_payload_units, 0);
    }

    #[test]
    fn independent_struct_layout_context_payload_is_body_independent() {
        for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            for structs in [4, 8, 16] {
                let mut baseline = None;
                for body in [4, 8, 16] {
                    let mut program = program(structs, body, 0, target);
                    let scope = observations::Scope::new();
                    let mut diagnostics = Vec::new();
                    resolve_program_struct_layouts(&mut program, &mut diagnostics);
                    assert!(diagnostics.is_empty());
                    assert_partition(&program.structs, &program.layouts, target);
                    let index = ScalarLayoutIndex::program(&program.layouts);
                    for structure in &program.structs {
                        assert_eq!(
                            index.aggregate(&structure.id),
                            program.layouts.aggregate(&structure.id)
                        );
                    }
                    let counts = scope.counts();
                    assert_counts(&counts, structs, structs * 2);
                    assert_eq!(counts.lookups, 1);
                    assert_eq!(counts.lookup_entries, structs);
                    if let Some(previous) = &baseline {
                        assert_eq!(&counts, previous);
                    }
                    baseline = Some(counts);
                }
            }
        }
    }

    #[test]
    fn project_layout_context_construction_is_once_per_target() {
        for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            for module_count in [4, 8, 16] {
                for structs in [4, 8, 16] {
                    for body in [4, 8, 16] {
                        let modules = (0..module_count)
                            .map(|module| {
                                ScalarModule::from_program(
                                    program(structs, body, module, target),
                                    Vec::new(),
                                )
                            })
                            .collect::<Vec<_>>();
                        let mut project = ScalarProject::new(modules, Vec::new());
                        for module in &mut project.modules {
                            for result in module.layouts.aggregates.values_mut() {
                                result.layout = None;
                                result.fields.clear();
                            }
                        }
                        let scope = observations::Scope::new();
                        let mut diagnostics = Vec::new();
                        resolve_project_struct_layouts(&mut project, &mut diagnostics);
                        assert!(diagnostics.is_empty());
                        let index = ScalarLayoutIndex::project(&project.modules);
                        let mut ids = BTreeSet::new();
                        for module in &project.modules {
                            assert_partition(&module.structs, &module.layouts, target);
                            for structure in &module.structs {
                                assert!(ids.insert(&structure.id));
                                assert_eq!(
                                    index.aggregate(&structure.id),
                                    module.layouts.aggregate(&structure.id)
                                );
                                for field in &structure.fields {
                                    assert_eq!(
                                        index.field(&field.id),
                                        module.layouts.field(&field.id)
                                    );
                                }
                            }
                        }
                        let structs = project
                            .modules
                            .iter()
                            .map(|module| module.structs.len())
                            .sum::<usize>();
                        let fields = project
                            .modules
                            .iter()
                            .flat_map(|module| &module.structs)
                            .map(|structure| structure.fields.len())
                            .sum::<usize>();
                        assert_eq!(ids.len(), structs);
                        let counts = scope.counts();
                        assert_counts(&counts, structs, fields);
                        assert_eq!(counts.lookups, 1);
                        assert_eq!(counts.lookup_entries, structs);
                    }
                }
            }
        }
    }

    #[test]
    fn module_constructor_observation_measures_actual_construction() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "main.w".into(),
            "r1".into(),
        );
        let scope = observations::Scope::new();
        let module = ScalarModule::new(source.clone(), Vec::new(), Vec::new());
        assert_eq!(module.source, source);
        assert!(module.items.is_empty());
        assert_eq!(
            scope.counts(),
            observations::Counts {
                module_constructions: 1,
                ..observations::Counts::default()
            }
        );
    }

    #[test]
    fn layout_copy_observations_measure_actual_clone_constructors() {
        let program = program(2, 4, 0, ScalarTargetLayout::WASM32);
        let scope = observations::Scope::new();
        let copy = program.clone();
        let counts = scope.counts();
        assert_eq!(counts.program_copies, 1);
        assert_eq!(counts.module_constructions, 0);
        assert_eq!(counts.struct_copies, 2);
        assert!(counts.cfg_copies > 0);
        assert!(counts.copied_payload_units > 0);
        let module = ScalarModule::from_program(copy, Vec::new());
        let _copy = module.clone();
        assert_eq!(scope.counts().module_copies, 1);
        assert_eq!(scope.counts().module_constructions, 1);
        assert_eq!(scope.counts().struct_copies, 4);
    }

    #[test]
    fn nested_array_predecessors_complete_once_in_dependency_order() {
        for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            for structs in [4, 8, 16] {
                let mut program = program(structs, 4, 0, target);
                for index in 0..structs - 1 {
                    let dependency = program.structs[index + 1].id.clone();
                    let span = program.structs[index].fields[0].name_span;
                    let element = ScalarType::Array {
                        element: Box::new(ScalarType::Struct(dependency)),
                        length: 2,
                        length_span: span,
                        span,
                    };
                    program.structs[index].fields[0].ty = ScalarType::Array {
                        element: Box::new(element),
                        length: 2,
                        length_span: span,
                        span,
                    };
                }
                let scope = observations::Scope::new();
                let mut diagnostics = Vec::new();
                resolve_program_struct_layouts(&mut program, &mut diagnostics);
                assert!(diagnostics.is_empty());
                let mut size = target.pointer_size * 2;
                for (index, structure) in program.structs.iter().enumerate().rev() {
                    let result = program.layouts.aggregate(&structure.id).unwrap();
                    let pointer_offset = if index == structs - 1 {
                        target.pointer_size
                    } else {
                        size * 4
                    };
                    size = pointer_offset + target.pointer_size;
                    assert_eq!(
                        result.layout,
                        Some(ScalarLayout {
                            size,
                            alignment: target.pointer_alignment
                        })
                    );
                    assert_eq!(result.fields.len(), structure.fields.len());
                    assert_eq!(result.fields[0].offset, Some(0));
                    assert_eq!(result.fields[1].offset, Some(pointer_offset));
                    assert!(result.fields.iter().all(|field| field.layout.is_some()));
                }
                let mut counts = scope.counts();
                assert_eq!(counts.edges, structs - 1);
                counts.edges = 0;
                assert_counts(&counts, structs, structs * 2);
            }
        }
    }

    #[test]
    fn module_from_program_moves_its_target_layout_partition() {
        for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let program = program(4, 4, 0, target);
            let fields = program
                .structs
                .iter()
                .map(|structure| {
                    program
                        .layouts
                        .aggregate(&structure.id)
                        .unwrap()
                        .fields
                        .as_ptr()
                })
                .collect::<Vec<_>>();
            let scope = observations::Scope::new();
            let module = ScalarModule::from_program(program, Vec::new());
            assert_partition(&module.structs, &module.layouts, target);
            for (structure, pointer) in module.structs.iter().zip(fields) {
                assert_eq!(
                    module
                        .layouts
                        .aggregate(&structure.id)
                        .unwrap()
                        .fields
                        .as_ptr(),
                    pointer
                );
            }
            assert_eq!(
                scope.counts(),
                observations::Counts {
                    module_constructions: 1,
                    ..observations::Counts::default()
                }
            );
        }
    }

    #[test]
    fn rejected_predecessor_retains_only_observed_prefix_fields() {
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/invalid.w".into(),
            "revision".into(),
        );
        let parsed = parse_source(source, "%%start\nstruct Outer { u8 first; Huge failed; u8 later; }\nstruct Huge { u128[18446744073709551615] values; }\n%%end".into(), &[]);
        let validation = derive_scalar_program(&parsed.result);
        assert_eq!(validation.diagnostics.len(), 1);
        assert_eq!(validation.diagnostics[0].code, "B0003");
        let outer = validation
            .program
            .layouts
            .aggregate(&validation.program.structs[0].id)
            .unwrap();
        assert_eq!(outer.layout, None);
        assert_eq!(outer.fields.len(), 3);
        assert_eq!(outer.fields[0].offset, Some(0));
        assert_eq!(
            outer.fields[0].layout,
            Some(ScalarLayout {
                size: 1,
                alignment: 1
            })
        );
        assert!(outer.fields[1..]
            .iter()
            .all(|field| field.layout.is_none() && field.offset.is_none()));
    }

    mod json_adapter {
        use super::*;

        #[test]
        fn roundtrips_populated_partitions_in_canonical_pair_order_on_both_targets() {
            for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
                let modules = (0..2)
                    .rev()
                    .map(|source| {
                        ScalarModule::from_program(program(2, 4, source, target), Vec::new())
                    })
                    .collect::<Vec<_>>();
                let sources = modules.iter().map(|module| module.source.clone()).collect();
                let project = ScalarProject::new(modules, sources);
                let first = &project.modules[0].structs[0].id;
                let second = &project.modules[1].structs[0].id;
                assert_eq!(first.index, second.index);
                assert_ne!(first.source, second.source);
                let wire = serde_json::to_value(&project).expect("serialize populated partitions");
                let restored: ScalarProject =
                    serde_json::from_value(wire.clone()).expect("decode populated partitions");
                assert_eq!(restored, project);
                for (module, encoded) in restored
                    .modules
                    .iter()
                    .zip(wire["modules"].as_array().unwrap())
                {
                    assert_partition(&module.structs, &module.layouts, target);
                    let entries = encoded["layouts"]["aggregates"]
                        .as_array()
                        .expect("typed entry sequence");
                    assert_eq!(entries.len(), module.structs.len());
                    for ((id, layout), entry) in module.layouts.aggregates.iter().zip(entries) {
                        assert_eq!(entry.as_array().unwrap().len(), 2);
                        assert_eq!(entry[0], serde_json::to_value(id).unwrap());
                        assert_eq!(entry[1], serde_json::to_value(layout).unwrap());
                    }
                }
                assert_eq!(
                    serde_json::to_string(&project).unwrap(),
                    serde_json::to_string(&restored).unwrap()
                );
                let validation = validate_scalar_project(restored);
                assert!(
                    validation.diagnostics.is_empty(),
                    "{:?}",
                    validation.diagnostics
                );
                assert_eq!(validation.project, project);
            }
        }

        #[test]
        fn accepts_complete_ids_and_canonicalizes_pair_order_across_sources() {
            for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
                let earlier = program(1, 4, 0, target);
                let later = program(1, 4, 1, target);
                let earlier_id = &earlier.structs[0].id;
                let later_id = &later.structs[0].id;
                assert_eq!(earlier_id.index, later_id.index);
                assert_ne!(earlier_id.source, later_id.source);
                assert!(earlier_id < later_id);
                let mut wire = serde_json::to_value(&later.layouts).unwrap();
                wire["aggregates"].as_array_mut().unwrap().push(
                    serde_json::to_value((
                        earlier_id,
                        earlier.layouts.aggregate(earlier_id).unwrap(),
                    ))
                    .unwrap(),
                );
                let restored: ScalarTargetLayouts = serde_json::from_value(wire).unwrap();
                assert_eq!(restored.aggregates.len(), 2);
                assert_eq!(
                    restored.aggregate(earlier_id),
                    earlier.layouts.aggregate(earlier_id)
                );
                assert_eq!(
                    restored.aggregate(later_id),
                    later.layouts.aggregate(later_id)
                );
                let encoded = serde_json::to_value(&restored).unwrap();
                assert_eq!(
                    encoded["aggregates"][0][0],
                    serde_json::to_value(earlier_id).unwrap()
                );
                assert_eq!(
                    encoded["aggregates"][1][0],
                    serde_json::to_value(later_id).unwrap()
                );
            }
        }

        #[test]
        fn rejects_duplicate_complete_struct_keys() {
            for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
                let program = program(2, 4, 0, target);
                let mut wire = serde_json::to_value(&program.layouts).unwrap();
                let entries = wire["aggregates"].as_array_mut().unwrap();
                let mut duplicate = entries[0].clone();
                duplicate[1]["layout"]["size"] = serde_json::json!(999);
                entries.push(duplicate);
                let error = serde_json::from_value::<ScalarTargetLayouts>(wire).unwrap_err();
                assert!(
                    error
                        .to_string()
                        .contains("duplicate aggregate layout struct ID"),
                    "{error}"
                );
            }
        }

        #[test]
        fn roundtrips_empty_partitions_through_the_same_sequence_format() {
            for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
                let layouts = ScalarTargetLayouts::new(target);
                let wire = serde_json::to_value(&layouts).unwrap();
                assert_eq!(wire["aggregates"], serde_json::json!([]));
                assert_eq!(wire["target"], serde_json::to_value(target).unwrap());
                let restored: ScalarTargetLayouts = serde_json::from_value(wire).unwrap();
                assert_eq!(restored, layouts);
            }
        }

        #[test]
        fn roundtrips_rejected_aggregate_and_field_observations_on_both_targets() {
            for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
                let source = SourceIdentity::new(
                    "project".into(),
                    "package".into(),
                    "src/invalid.w".into(),
                    "revision".into(),
                );
                let parsed = parse_source(source, "%%start\nstruct Outer { u8 first; Huge failed; u8 later; }\nstruct Huge { u128[18446744073709551615] values; }\n%%end".into(), &[]);
                let validation = derive_scalar_program_with_layout(&parsed.result, target);
                assert_eq!(validation.diagnostics.len(), 1);
                let layouts = &validation.program.layouts;
                let outer_id = &validation.program.structs[0].id;
                let wire = serde_json::to_string(layouts).unwrap();
                let restored: ScalarTargetLayouts = serde_json::from_str(&wire).unwrap();
                assert_eq!(&restored, layouts);
                let outer = restored.aggregate(outer_id).unwrap();
                assert_eq!(outer.layout, None);
                assert_eq!(outer.fields.len(), 3);
                assert_eq!(outer.fields[0].offset, Some(0));
                assert_eq!(
                    outer.fields[0].layout,
                    Some(ScalarLayout {
                        size: 1,
                        alignment: 1
                    })
                );
                assert!(outer.fields[1..]
                    .iter()
                    .all(|field| field.layout.is_none() && field.offset.is_none()));
            }
        }

        #[test]
        fn rejects_non_sequence_entries_and_malformed_pairs() {
            let program = program(1, 4, 0, ScalarTargetLayout::WASM32);
            let wire = serde_json::to_value(&program.layouts).unwrap();
            for invalid in [
                serde_json::json!({}),
                serde_json::json!(null),
                serde_json::json!([[]]),
                serde_json::json!([[{}]]),
            ] {
                let mut malformed = wire.clone();
                malformed["aggregates"] = invalid;
                assert!(serde_json::from_value::<ScalarTargetLayouts>(malformed).is_err());
            }
            let mut malformed = wire;
            malformed.as_object_mut().unwrap().remove("aggregates");
            assert!(serde_json::from_value::<ScalarTargetLayouts>(malformed).is_err());
        }
    }
}
