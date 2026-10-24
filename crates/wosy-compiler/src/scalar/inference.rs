use super::*;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ResolvedAssignmentOutputId {
    pub source: SourceIdentity,
    pub function_span: ByteSpan,
    pub target_span: ByteSpan,
    pub output_index: usize,
}

fn assignment_function_span(items: &[ScalarItem], assignment_span: ByteSpan) -> Option<ByteSpan> {
    items.iter().find_map(|item| {
        let ScalarItem::Function(function) = item else {
            return None;
        };
        std::iter::once(function)
            .chain(&function.overload_arms)
            .find(|candidate| {
                candidate.body.span.start <= assignment_span.start
                    && assignment_span.end <= candidate.body.span.end
            })
            .map(|candidate| candidate.span)
    })
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScalarAssignmentTyping {
    assignments: HashMap<ResolvedAssignmentOutputId, ScalarType>,
}

impl ScalarAssignmentTyping {
    pub fn get(&self, key: &ResolvedAssignmentOutputId) -> Option<&ScalarType> {
        self.assignments.get(key)
    }
    pub fn len(&self) -> usize {
        self.assignments.len()
    }
    pub fn is_empty(&self) -> bool {
        self.assignments.is_empty()
    }
    pub(super) fn as_map(&self) -> &HashMap<ResolvedAssignmentOutputId, ScalarType> {
        &self.assignments
    }
}

pub(super) struct ScalarInferenceBuilder {
    assignments: HashMap<ResolvedAssignmentOutputId, ScalarType>,
}

impl ScalarInferenceBuilder {
    pub(super) fn new() -> Self {
        Self {
            assignments: HashMap::new(),
        }
    }
    pub(super) fn freeze(self) -> ScalarAssignmentTyping {
        ScalarAssignmentTyping {
            assignments: self.assignments,
        }
    }

    pub(super) fn record_assignment_outputs(
        &mut self,
        source: &SourceIdentity,
        items: &[ScalarItem],
        assignment: &ScalarAssignment,
        outputs: &ScalarOutputSequence,
    ) {
        let Some(function_span) = assignment_function_span(items, assignment.span) else {
            return;
        };
        for (output_index, (target, output)) in
            assignment.targets.iter().zip(&outputs.outputs).enumerate()
        {
            self.assignments.insert(
                ResolvedAssignmentOutputId {
                    source: source.clone(),
                    function_span,
                    target_span: target.target_span,
                    output_index,
                },
                output.ty.clone(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_source;

    const VALID: &str = "%%start\n(i32, bool)(i32, bool) pair = fn(left, right) { left, right };\ni32(i32, bool) consume = fn(left, right) { first, second = pair(left, right); second; first };\n%%end";
    const INVALID: &str = "%%start\ni32(i32) good = fn(value) { next = value; next };\ni32(bool) bad = fn(value) { value };\n%%end";

    fn source(path: &str) -> SourceIdentity {
        SourceIdentity::new("project".into(), "package".into(), path.into(), "r1".into())
    }

    fn derive(path: &str, text: &str) -> ScalarValidation {
        let parsed = parse_source(source(path), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        derive_scalar_program(&parsed.result)
    }

    fn assert_complete_rows(program: &ScalarProgram) {
        let ScalarItem::Function(function) = &program.items[1] else {
            panic!("consumer function")
        };
        let ScalarBlockItem::Assignment(assignment) = &function.body.items[0] else {
            panic!("assignment")
        };
        assert_eq!(program.resolved_assignment_outputs.len(), 2);
        for (output_index, ty) in [ScalarType::I32, ScalarType::Bool].iter().enumerate() {
            assert_eq!(
                program
                    .resolved_assignment_outputs
                    .get(&ResolvedAssignmentOutputId {
                        source: program.source.clone(),
                        function_span: function.span,
                        target_span: assignment.targets[output_index].target_span,
                        output_index,
                    }),
                Some(ty)
            );
        }
    }

    #[test]
    fn freeze_preserves_complete_source_function_target_output_keys_and_shared_reads() {
        let program = derive("one.w", VALID);
        assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
        assert_complete_rows(&program.program);
        let ScalarItem::Function(function) = &program.program.items[1] else {
            panic!("function")
        };
        let ScalarBlockItem::Assignment(assignment) = &function.body.items[0] else {
            panic!("assignment")
        };
        let mut equal_targets = assignment.clone();
        equal_targets.targets[1].target_span = equal_targets.targets[0].target_span;
        let outputs = ScalarOutputSequence {
            span: assignment.span,
            outputs: [ScalarType::I32, ScalarType::Bool]
                .into_iter()
                .map(|ty| ScalarOutput {
                    ty,
                    span: assignment.span,
                })
                .collect(),
        };
        let mut builder = ScalarInferenceBuilder::new();
        for source in [source("one.w"), source("two.w")] {
            builder.record_assignment_outputs(
                &source,
                &program.program.items,
                &equal_targets,
                &outputs,
            );
        }
        let expected = builder.assignments.clone();
        let stored_value = builder.assignments.values().next().unwrap() as *const ScalarType;
        let frozen = builder.freeze();
        assert_eq!(frozen.len(), 4);
        assert!(!frozen.is_empty());
        assert!(frozen
            .assignments
            .values()
            .any(|value| std::ptr::eq(value, stored_value)));
        for _ in 0..4 {
            for (key, ty) in &expected {
                assert_eq!(frozen.get(key), Some(ty));
            }
            assert_eq!(frozen.as_map(), &expected);
            assert_eq!(frozen, frozen.clone());
        }
        let key = expected.keys().next().unwrap();
        let mut other_function = key.clone();
        other_function.function_span = ByteSpan::new(0, 1);
        assert_eq!(frozen.get(&other_function), None);
        let mut other_target = key.clone();
        other_target.target_span = ByteSpan::new(0, 1);
        assert_eq!(frozen.get(&other_target), None);
    }

    #[test]
    fn scratch_typing_keeps_its_rows_and_preserves_frozen_program_and_module() {
        let result = derive("scratch.w", VALID);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let program = result.program;
        let expected = program.resolved_assignment_outputs.clone();
        let overloads = ScalarOverloadEnvironment::single(&program.source, &program.items);
        let mut scratch = ScalarInferenceBuilder::new();
        assert!(typecheck::validate(&program, &overloads, &mut scratch).is_empty());
        assert_eq!(program.resolved_assignment_outputs, expected);
        assert_eq!(scratch.freeze(), expected);
        let module = ScalarModule::from_program(program, Vec::new());
        let overloads = ScalarOverloadEnvironment::project(std::slice::from_ref(&module));
        let mut scratch = ScalarInferenceBuilder::new();
        let ScalarItem::Function(function) = &module.items[1] else {
            panic!("function")
        };
        let scope = module
            .members
            .iter()
            .map(|(name, ty)| (name.clone(), ty.clone()))
            .chain(
                function
                    .parameters
                    .iter()
                    .cloned()
                    .zip([ScalarType::I32, ScalarType::Bool]),
            )
            .collect();
        let mut diagnostics = Vec::new();
        let actual = typecheck::block_type_in_module(
            &function.body,
            &scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &module,
            std::slice::from_ref(&module),
            &mut diagnostics,
            false,
            &overloads,
            &mut scratch,
        );
        assert_eq!(actual, ScalarType::I32);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(module.resolved_assignment_outputs, expected);
        assert_eq!(scratch.freeze(), expected);
    }

    #[test]
    fn static_failure_discards_earlier_good_assignments_in_both_drivers() {
        let invalid = derive("bad.w", INVALID);
        assert!(!invalid.diagnostics.is_empty());
        assert!(invalid.program.resolved_assignment_outputs.is_empty());
        let overloads =
            ScalarOverloadEnvironment::single(&invalid.program.source, &invalid.program.items);
        let mut provisional = ScalarInferenceBuilder::new();
        assert!(!typecheck::validate(&invalid.program, &overloads, &mut provisional).is_empty());
        assert_eq!(provisional.assignments.len(), 1);
        let valid = derive("good.w", VALID);
        assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);
        assert_complete_rows(&valid.program);
        let project = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(valid.program, Vec::new()),
                ScalarModule::from_program(invalid.program, Vec::new()),
            ],
            vec![source("good.w"), source("bad.w")],
        ));
        assert!(!project.diagnostics.is_empty());
        assert!(project
            .project
            .modules
            .iter()
            .all(|module| module.resolved_assignment_outputs.is_empty()));
    }

    #[test]
    fn json_skips_untrusted_facts_and_successful_revalidation_rebuilds_every_row() {
        let valid = derive("restored.w", VALID);
        assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);
        assert_complete_rows(&valid.program);
        let expected = valid.program.resolved_assignment_outputs.clone();
        let json = serde_json::to_value(&valid.program).unwrap();
        assert!(json.get("resolved_assignment_outputs").is_none());
        let restored: ScalarProgram = serde_json::from_value(json).unwrap();
        assert!(restored.resolved_assignment_outputs.is_empty());
        let project = ScalarProject::new(
            vec![ScalarModule::from_program(restored, Vec::new())],
            vec![source("restored.w")],
        );
        let json = serde_json::to_value(&project).unwrap();
        let restored: ScalarProject = serde_json::from_value(json).unwrap();
        assert!(restored.modules[0].resolved_assignment_outputs.is_empty());
        let validation = validate_scalar_project(restored);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        assert_eq!(
            validation.project.modules[0].resolved_assignment_outputs,
            expected
        );
    }

    #[test]
    fn later_ownership_failure_retains_completed_static_assignment_rows() {
        let result = derive("ownership.w", "%%start\nstruct Payload { i32 value; }\nPayload(Payload) consume = fn(value) { next = value; next; next };\n%%end");
        assert!(!result.diagnostics.is_empty());
        assert_eq!(result.program.resolved_assignment_outputs.len(), 1);
        let expected = result.program.resolved_assignment_outputs.clone();
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(result.program, Vec::new())],
            vec![source("ownership.w")],
        ));
        assert!(!validation.diagnostics.is_empty());
        assert_eq!(
            validation.project.modules[0].resolved_assignment_outputs,
            expected
        );
    }
}
