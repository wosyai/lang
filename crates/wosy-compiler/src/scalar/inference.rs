use super::control::{CheckedCallSignature, FoldedOccurrence};
use super::*;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ResolvedAssignmentOutputId {
    pub source: SourceIdentity,
    pub function_span: ByteSpan,
    pub target_span: ByteSpan,
    pub output_index: usize,
}

#[derive(Debug, Eq, PartialEq)]
struct ScalarAssignmentOwner {
    body_span: ByteSpan,
    function_span: ByteSpan,
}

pub(super) struct ScalarAssignmentOwners {
    spans: Vec<ScalarAssignmentOwner>,
}

impl ScalarAssignmentOwners {
    pub(super) fn new(items: &[ScalarItem]) -> Self {
        Self {
            spans: items
                .iter()
                .filter_map(|item| match item {
                    ScalarItem::Function(function) => Some(function),
                    _ => None,
                })
                .flat_map(|function| {
                    (function.overload_arms.is_empty() || !function.body.items.is_empty())
                        .then_some(function)
                        .into_iter()
                        .chain(&function.overload_arms)
                })
                .map(|function| ScalarAssignmentOwner {
                    body_span: function.body.span,
                    function_span: function.span,
                })
                .collect(),
        }
    }

    fn function_span(&self, assignment_span: ByteSpan) -> Option<ByteSpan> {
        self.spans
            .iter()
            .find(|owner| {
                owner.body_span.start <= assignment_span.start
                    && assignment_span.end <= owner.body_span.end
            })
            .map(|owner| owner.function_span)
    }
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

#[derive(Default)]
// Preparation-local address identity. Validation and the structural walk borrow the same
// AST nodes; intervening ownership moves preserve their Vec buffers, and overload selections
// update fields in place. Keys are compared only and never dereferenced or published.
pub(super) struct PreparationOccurrenceTyping {
    module_assignments: HashMap<*const ScalarAssignment, Vec<ScalarType>>,
    calls: HashMap<*const ScalarExpression, CheckedCallSignature>,
    tail_conditional_outputs: HashMap<*const ScalarExpression, Vec<ScalarType>>,
    expressions: HashMap<*const ScalarExpression, CheckedExpressionType>,
    members: HashMap<*const ScalarExpression, CheckedMember>,
    literal_fields: HashMap<*const ScalarExpression, ScalarStructFieldId>,
    copy_struct_literals: HashSet<*const ScalarExpression>,
    value_fields: HashMap<*const ScalarExpression, ScalarStructFieldId>,
    folded_expressions: HashMap<*const ScalarExpression, FoldedOccurrence>,
    needs_checked_access: bool,
    places: HashMap<*const ScalarPlace, ScalarType>,
    resolved_namespace_places: HashMap<*const ScalarPlace, SourceIdentity>,
}

struct CheckedExpressionType {
    ty: ScalarType,
    contextual: bool,
}

pub(super) enum CheckedMember {
    DirectField(ScalarStructFieldId),
    RawPointerField(ScalarStructFieldId),
    Enum { enumeration: ScalarEnumId, tag: u32 },
    Module(SourceIdentity),
}

impl PreparationOccurrenceTyping {
    pub(super) fn take_copy_struct_literal(&mut self, value: &ScalarExpression) -> bool {
        self.copy_struct_literals.remove(&std::ptr::from_ref(value))
    }

    pub(super) fn take_literal_field(
        &mut self,
        value: &ScalarExpression,
    ) -> Option<ScalarStructFieldId> {
        self.literal_fields.remove(&std::ptr::from_ref(value))
    }

    pub(super) fn take_value_field(&mut self, value: &ScalarExpression) -> Option<ScalarStructFieldId> {
        self.value_fields.remove(&std::ptr::from_ref(value))
    }

    pub(super) fn take_member(&mut self, expression: &ScalarExpression) -> Option<CheckedMember> {
        self.members.remove(&std::ptr::from_ref(expression))
    }

    pub(super) fn take_assignment(
        &mut self,
        assignment: &ScalarAssignment,
    ) -> std::vec::IntoIter<ScalarType> {
        self.module_assignments
            .remove(&std::ptr::from_ref(assignment))
            .unwrap_or_default()
            .into_iter()
    }

    pub(super) fn take_call(
        &mut self,
        expression: &ScalarExpression,
    ) -> Option<CheckedCallSignature> {
        self.calls.remove(&std::ptr::from_ref(expression))
    }

    pub(super) fn take_tail_conditional_outputs(
        &mut self,
        expression: &ScalarExpression,
    ) -> Option<Vec<ScalarType>> {
        self.tail_conditional_outputs
            .remove(&std::ptr::from_ref(expression))
    }

    pub(super) fn take_expression_type(
        &mut self,
        expression: &ScalarExpression,
    ) -> Option<ScalarType> {
        self.expressions
            .remove(&std::ptr::from_ref(expression))
            .map(|checked| checked.ty)
    }

    pub(super) fn take_folded_expression(
        &mut self,
        expression: &ScalarExpression,
    ) -> Option<FoldedOccurrence> {
        self.folded_expressions
            .remove(&std::ptr::from_ref(expression))
    }

    pub(super) fn needs_checked_access(&self) -> bool {
        self.needs_checked_access
    }

    pub(super) fn take_place_type(&mut self, place: &ScalarPlace) -> Option<ScalarType> {
        self.places.remove(&std::ptr::from_ref(place))
    }

    pub(super) fn take_resolved_namespace_place(
        &mut self,
        place: &ScalarPlace,
    ) -> Option<SourceIdentity> {
        self.resolved_namespace_places
            .remove(&std::ptr::from_ref(place))
    }
}

fn contains_error_type(ty: &ScalarType) -> bool {
    match ty {
        ScalarType::Error => true,
        ScalarType::RawPointer(inner)
        | ScalarType::CheckedReference { inner, .. }
        | ScalarType::Array { element: inner, .. }
        | ScalarType::RuntimeArray { element: inner, .. } => contains_error_type(inner),
        ScalarType::Callable {
            outputs,
            parameters,
        } => {
            outputs
                .outputs
                .iter()
                .any(|output| contains_error_type(&output.ty))
                || parameters.iter().any(contains_error_type)
        }
        _ => false,
    }
}

pub(super) struct ScalarInferenceBuilder<'a> {
    owners: &'a ScalarAssignmentOwners,
    assignments: HashMap<ResolvedAssignmentOutputId, ScalarType>,
    occurrences: PreparationOccurrenceTyping,
    float_literal_shapes: HashMap<*const ScalarExpression, bool>,
    in_module_initialization: bool,
    private_runtime_write_formal: Option<String>,
    nominate_local_runtime_write: bool,
    suppress_checked_occurrences: bool,
    anonymous_element_sites: HashSet<*const ScalarExpression>,
}

impl<'a> ScalarInferenceBuilder<'a> {
    pub(super) fn new(owners: &'a ScalarAssignmentOwners) -> Self {
        Self {
            owners,
            assignments: HashMap::new(),
            occurrences: PreparationOccurrenceTyping::default(),
            float_literal_shapes: HashMap::new(),
            in_module_initialization: false,
            private_runtime_write_formal: None,
            nominate_local_runtime_write: false,
            suppress_checked_occurrences: true,
            anonymous_element_sites: HashSet::new(),
        }
    }

    pub(super) fn for_validation(owners: &'a ScalarAssignmentOwners) -> Self {
        Self {
            suppress_checked_occurrences: false,
            ..Self::new(owners)
        }
    }

    #[cfg(test)]
    pub(super) fn freeze(self) -> ScalarAssignmentTyping {
        self.finish().0
    }

    pub(super) fn finish(self) -> (ScalarAssignmentTyping, PreparationOccurrenceTyping) {
        (
            ScalarAssignmentTyping {
                assignments: self.assignments,
            },
            self.occurrences,
        )
    }

    pub(super) fn nominate_inline_elements(&mut self, binding: &ScalarBinding) {
        if self.in_module_initialization || self.suppress_checked_occurrences { return; }
        let Some(elements) = super::anonymous_callable::candidate_fixed_elements(binding) else { return; };
        self.anonymous_element_sites.extend(elements.iter()
            .filter(|item| matches!(item, ScalarExpression::AnonymousCallable { .. }))
            .map(std::ptr::from_ref));
    }

    pub(super) fn is_nominated_inline_element(&self, expression: &ScalarExpression) -> bool {
        self.anonymous_element_sites.contains(&std::ptr::from_ref(expression))
    }

    pub(super) fn memoized_float_literal_shape(
        &mut self,
        expression: &ScalarExpression,
        classify: impl FnOnce(&mut Self) -> bool,
    ) -> bool {
        // These are the same preparation-local AST addresses used for checked
        // occurrence facts. Shape is independent of type context and probing.
        let key = std::ptr::from_ref(expression);
        if let Some(&shape) = self.float_literal_shapes.get(&key) {
            return shape;
        }
        let shape = classify(self);
        self.float_literal_shapes.insert(key, shape);
        shape
    }

    pub(super) fn without_checked_occurrences(
        &mut self,
        check_arguments: impl FnOnce(&mut Self) -> Vec<ScalarType>,
    ) -> Vec<ScalarType> {
        let previous = std::mem::replace(&mut self.suppress_checked_occurrences, true);
        let arguments = check_arguments(self);
        self.suppress_checked_occurrences = previous;
        arguments
    }

    pub(super) fn probe_raw_dereference(
        &mut self,
        check_pointer: impl FnOnce(&mut Self) -> ScalarType,
    ) -> bool {
        let previous = std::mem::replace(&mut self.suppress_checked_occurrences, true);
        let is_raw = matches!(check_pointer(self), ScalarType::RawPointer(_));
        self.suppress_checked_occurrences = previous;
        is_raw
    }

    pub(super) fn record_expression_type(
        &mut self,
        expression: &ScalarExpression,
        ty: &ScalarType,
        contextual: bool,
    ) {
        if self.suppress_checked_occurrences || contains_error_type(ty) {
            return;
        }
        let key = std::ptr::from_ref(expression);
        if contextual
            || !self
                .occurrences
                .expressions
                .get(&key)
                .is_some_and(|row| row.contextual)
        {
            self.occurrences.expressions.insert(
                key,
                CheckedExpressionType {
                    ty: ty.clone(),
                    contextual,
                },
            );
        }
    }

    pub(super) fn record_member(&mut self, expression: &ScalarExpression, member: CheckedMember) {
        if !self.suppress_checked_occurrences {
            self.occurrences
                .members
                .insert(std::ptr::from_ref(expression), member);
        }
    }

    pub(super) fn record_value_field(&mut self, value: &ScalarExpression, id: &ScalarStructFieldId) {
        if !self.suppress_checked_occurrences {
            self.occurrences.value_fields.insert(std::ptr::from_ref(value), id.clone());
        }
    }

    pub(super) fn record_copy_struct_literal(&mut self, value: &ScalarExpression) {
        if !self.suppress_checked_occurrences {
            self.occurrences.copy_struct_literals.insert(std::ptr::from_ref(value));
        }
    }

    pub(super) fn record_literal_field(
        &mut self,
        value: &ScalarExpression,
        id: &ScalarStructFieldId,
    ) {
        if !self.suppress_checked_occurrences {
            self.occurrences
                .literal_fields
                .insert(std::ptr::from_ref(value), id.clone());
        }
    }

    pub(super) fn record_folded_signed_integer_magnitude(&mut self, operand: &ScalarExpression) {
        if !self.suppress_checked_occurrences {
            self.occurrences.folded_expressions.insert(
                std::ptr::from_ref(operand),
                FoldedOccurrence::SignedIntegerMagnitude,
            );
        }
    }

    fn index_root_is_raw(&self, mut place: &ScalarPlace) -> bool {
        loop {
            match place {
                ScalarPlace::Field { base, .. } | ScalarPlace::Index { base, .. } => place = base,
                ScalarPlace::Dereference { pointer, .. } => return self.occurrences.expressions
                    .get(&std::ptr::from_ref(pointer.as_ref()))
                    .is_some_and(|entry| matches!(entry.ty, ScalarType::RawPointer(_))),
                ScalarPlace::Name { .. } => return false,
            }
        }
    }

    pub(super) fn record_place_type(&mut self, place: &ScalarPlace, ty: &ScalarType) {
        if !self.suppress_checked_occurrences && !contains_error_type(ty) {
            if !self.occurrences.needs_checked_access {
                self.occurrences.needs_checked_access = match place {
                    ScalarPlace::Index { base, index, .. } => match self.occurrences
                        .places.get(&std::ptr::from_ref(base.as_ref()))
                    {
                        Some(ScalarType::Array { length, .. }) =>
                            !self.index_root_is_raw(base)
                                && !matches!(index.as_ref(), ScalarExpression::Integer { value, .. }
                                    if value >= &0.into() && value < &(*length).into()),
                        Some(ScalarType::RuntimeArray { .. }) =>
                            matches!(base.as_ref(), ScalarPlace::Name { .. }),
                        _ => false,
                    },
                    ScalarPlace::Dereference { pointer, .. } => self.occurrences.expressions
                        .get(&std::ptr::from_ref(pointer.as_ref()))
                        .is_some_and(|entry| matches!(entry.ty, ScalarType::CheckedReference { .. })),
                    _ => false,
                };
            }
            self.occurrences
                .places
                .insert(std::ptr::from_ref(place), ty.clone());
        }
    }

    pub(super) fn record_resolved_namespace_place(
        &mut self,
        place: &ScalarPlace,
        source: &SourceIdentity,
    ) {
        if !self.suppress_checked_occurrences {
            self.occurrences
                .resolved_namespace_places
                .insert(std::ptr::from_ref(place), source.clone());
        }
    }

    pub(super) fn record_call_signature(
        &mut self,
        expression: &ScalarExpression,
        parameters: &[ScalarType],
        outputs: &ScalarOutputSequence,
        overload: Option<(ScalarOverloadSelection, (usize, usize))>,
    ) {
        if self.suppress_checked_occurrences
            || matches!(expression, ScalarExpression::Call { receiver: Some(receiver), .. } if receiver == "core")
        {
            return;
        }
        self.insert_checked_call(expression, parameters.to_vec(), outputs.clone(), overload);
    }

    pub(super) fn record_core_call_signature(
        &mut self,
        expression: &ScalarExpression,
        parameters: Vec<ScalarType>,
        result: &ScalarType,
        span: ByteSpan,
    ) {
        if self.suppress_checked_occurrences
            || contains_error_type(result)
            || parameters.iter().any(contains_error_type)
        {
            return;
        }
        let outputs = ScalarOutputSequence {
            outputs: if *result == ScalarType::Unit {
                Vec::new()
            } else {
                vec![ScalarOutput {
                    ty: result.clone(),
                    span,
                }]
            },
            span,
        };
        self.insert_checked_call(expression, parameters, outputs, None);
    }

    fn insert_checked_call(
        &mut self,
        expression: &ScalarExpression,
        parameters: Vec<ScalarType>,
        outputs: ScalarOutputSequence,
        overload: Option<(ScalarOverloadSelection, (usize, usize))>,
    ) {
        let (overload, overload_origin) = overload.map_or((None, None), |(selection, origin)| {
            (Some(selection), Some(origin))
        });
        self.occurrences.calls.insert(
            std::ptr::from_ref(expression),
            CheckedCallSignature {
                parameters,
                outputs,
                overload,
                overload_origin,
            },
        );
    }

    pub(super) fn checked_non_core_call_outputs(
        &self,
        expression: &ScalarExpression,
    ) -> Option<&ScalarOutputSequence> {
        if matches!(expression, ScalarExpression::Call { receiver: Some(receiver), .. } if receiver == "core")
        {
            return None;
        }
        self.occurrences
            .calls
            .get(&std::ptr::from_ref(expression))
            .map(|signature| &signature.outputs)
    }

    pub(super) fn record_tail_conditional_outputs(
        &mut self,
        expression: &ScalarExpression,
        types: &[ScalarType],
    ) {
        debug_assert!(types.len() > 1);
        if !self.suppress_checked_occurrences && !types.iter().any(contains_error_type) {
            self.occurrences
                .tail_conditional_outputs
                .insert(std::ptr::from_ref(expression), types.to_vec());
        }
    }

    pub(super) fn set_validation_root(&mut self, item: &ScalarItem) {
        self.in_module_initialization =
            matches!(item, ScalarItem::Binding(_) | ScalarItem::Executable(_));
        self.nominate_local_runtime_write = !self.suppress_checked_occurrences
            && matches!(item, ScalarItem::Function(function)
                if function.overload_arms.is_empty() && function.generic_parameters.is_empty());
        self.private_runtime_write_formal = match item {
            ScalarItem::Function(function) if !self.suppress_checked_occurrences && function.overload_arms.is_empty()
                && function.generic_parameters.is_empty() && super::typecheck::is_module_private_name(&function.name) =>
            {
                match &function.signature {
                    ScalarType::Callable { parameters, outputs }
                        if super::private_runtime_write_signature(parameters, &outputs.outputs) =>
                            function.parameters.first().cloned(),
                    _ => None,
                }
            }
            _ => None,
        };
    }

    pub(super) fn allows_private_runtime_write_candidate(&self, base: &ScalarPlace) -> bool {
        matches!((self.private_runtime_write_formal.as_deref(), base),
            (Some(formal), ScalarPlace::Dereference { pointer, .. }) if matches!(pointer.as_ref(),
                ScalarExpression::Name { name, .. } if name == formal))
    }

    // Type nomination only: scope types do not distinguish otherwise
    // identical local, formal, or imported owners. The exact reference
    // author requires a lexical holder with a Fresh local allocation,
    // and a solved unchanged extent receipt on every incoming edge.
    pub(super) fn nominates_local_runtime_write(
        &self, base: &ScalarPlace, index: &ScalarExpression, index_ty: &ScalarType,
        element: &ScalarType, scope: &BTreeMap<String, ScalarType>,
    ) -> bool {
        if !self.nominate_local_runtime_write || self.suppress_checked_occurrences
            || *index_ty != ScalarType::U64 || !super::reference::aggregate::ordinary_value(element)
        { return false; }
        let ScalarPlace::Dereference { pointer, .. } = base else { return false; };
        let ScalarExpression::Name { name, .. } = pointer.as_ref() else { return false; };
        matches!(scope.get(name), Some(ScalarType::CheckedReference {
            mutability: ScalarReferenceMutability::Mutable, inner,
        }) if matches!(inner.as_ref(), ScalarType::RuntimeArray { element: actual, .. } if actual.as_ref() == element))
            && match index {
                ScalarExpression::Integer { .. } => true,
                ScalarExpression::Name { name, .. } => scope.get(name) == Some(&ScalarType::U64),
                _ => false,
            }
    }

    pub(super) fn record_assignment_outputs(
        &mut self,
        source: &SourceIdentity,
        assignment: &ScalarAssignment,
        outputs: &ScalarOutputSequence,
    ) {
        if self.in_module_initialization {
            self.occurrences.module_assignments.insert(
                std::ptr::from_ref(assignment),
                outputs
                    .outputs
                    .iter()
                    .take(assignment.targets.len())
                    .map(|output| output.ty.clone())
                    .collect(),
            );
            return;
        }
        let Some(function_span) = self.owners.function_span(assignment.span) else {
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
    fn single_output_overload_assignments_belong_to_their_executable_arms() {
        let text = "%%start
read = overload {
    u8(u8) => fn(input) { u8 value = 0; value = input; value };
    u8(*?u8) => fn(input) { u8 value = 0; unsafe { value = core.load<u8>(input); }; value };
};
u8 seed = 7;
u8 plain = read(seed);
u8 indirect = unsafe { read(&?seed) };
%%end";
        let source = source("overload.w");
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let check = |items: &[ScalarItem], facts: &ScalarAssignmentTyping| {
            let ScalarItem::Function(overload) = &items[0] else { panic!("overload") };
            assert_eq!(facts.len(), 2);
            for (index, arm) in overload.overload_arms.iter().enumerate() {
                let item = &arm.body.items[1];
                let (assignment, unsafe_scope) = match item {
                    ScalarBlockItem::Assignment(assignment) => (assignment, false),
                    ScalarBlockItem::Expression(ScalarExpression::Block(block)) => {
                        let [ScalarBlockItem::Assignment(assignment)] = block.items.as_slice()
                        else { panic!("unsafe assignment") };
                        (assignment, block.unsafe_context)
                    }
                    _ => panic!("assignment"),
                };
                assert_eq!(unsafe_scope, index == 1);
                let key = ResolvedAssignmentOutputId {
                    source: source.clone(),
                    function_span: arm.span,
                    target_span: assignment.targets[0].target_span,
                    output_index: 0,
                };
                assert_eq!(facts.get(&key), Some(&ScalarType::U8));
                assert_eq!(facts.get(&ResolvedAssignmentOutputId {
                    function_span: overload.span, ..key
                }), None);
            }
        };
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let single = crate::derive_scalar_program_with_layout(&parsed.result, layout);
            assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
            check(&single.program.items, &single.program.resolved_assignment_outputs);
            let prepared = crate::prepare_scalar_project(
                vec![(derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
                vec![source.clone()],
                ScalarPreparationInputs { target_layout: layout },
            ).unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
            let checked = crate::analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{:?}", checked.diagnostics());
            let module = &prepared.project().modules[0];
            check(&module.items, &module.resolved_assignment_outputs);
            crate::emit_scalar_build_project_llvm(&checked).expect("selected overload calls and assignments");
        }
    }

    #[test]
    fn owners_preserve_ordered_immediate_membership_and_inclusive_first_containment() {
        let result = derive("spans.w", VALID);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let ScalarItem::Function(template) = &result.program.items[0] else {
            panic!("function")
        };
        let mut ordinary = template.clone();
        ordinary.span = ByteSpan::new(1, 2);
        ordinary.body.span = ByteSpan::new(100, 150);
        let mut first_arm = template.clone();
        first_arm.span = ByteSpan::new(3, 4);
        first_arm.body.span = ByteSpan::new(20, 40);
        let mut nested_arm = template.clone();
        nested_arm.span = ByteSpan::new(5, 6);
        nested_arm.body.span = ByteSpan::new(200, 220);
        first_arm.overload_arms.push(nested_arm);
        let mut second_arm = template.clone();
        second_arm.span = ByteSpan::new(7, 8);
        second_arm.body.span = ByteSpan::new(30, 60);
        ordinary.overload_arms = vec![first_arm, second_arm];
        let mut following = template.clone();
        following.span = ByteSpan::new(9, 10);
        following.body.span = ByteSpan::new(30, 120);
        let extern_result = derive(
            "extern.w",
            "%%start\nenv = extern wasm \"helper\" { unit() flush; };\n%%end",
        );
        assert!(extern_result.diagnostics.is_empty());
        let mut items = vec![
            ScalarItem::Function(ordinary),
            extern_result.program.items[0].clone(),
            ScalarItem::Function(following),
        ];
        let owners = ScalarAssignmentOwners::new(&items);
        assert_eq!(
            owners.spans,
            [
                ScalarAssignmentOwner {
                    body_span: ByteSpan::new(100, 150),
                    function_span: ByteSpan::new(1, 2)
                },
                ScalarAssignmentOwner {
                    body_span: ByteSpan::new(20, 40),
                    function_span: ByteSpan::new(3, 4)
                },
                ScalarAssignmentOwner {
                    body_span: ByteSpan::new(30, 60),
                    function_span: ByteSpan::new(7, 8)
                },
                ScalarAssignmentOwner {
                    body_span: ByteSpan::new(30, 120),
                    function_span: ByteSpan::new(9, 10)
                },
            ]
        );
        for (span, expected) in [
            (ByteSpan::new(100, 150), Some(ByteSpan::new(1, 2))),
            (ByteSpan::new(100, 100), Some(ByteSpan::new(1, 2))),
            (ByteSpan::new(150, 150), Some(ByteSpan::new(1, 2))),
            (ByteSpan::new(20, 40), Some(ByteSpan::new(3, 4))),
            (ByteSpan::new(20, 20), Some(ByteSpan::new(3, 4))),
            (ByteSpan::new(40, 40), Some(ByteSpan::new(3, 4))),
            (ByteSpan::new(30, 35), Some(ByteSpan::new(3, 4))),
            (ByteSpan::new(40, 41), Some(ByteSpan::new(7, 8))),
            (ByteSpan::new(60, 61), Some(ByteSpan::new(9, 10))),
            (ByteSpan::new(99, 151), None),
            (ByteSpan::new(19, 20), None),
            (ByteSpan::new(151, 151), None),
            (ByteSpan::new(200, 220), None),
        ] {
            assert_eq!(owners.function_span(span), expected, "{span:?}");
        }
        items.swap(0, 2);
        let reordered = ScalarAssignmentOwners::new(&items);
        assert_eq!(
            reordered.function_span(ByteSpan::new(30, 35)),
            Some(ByteSpan::new(9, 10))
        );
        assert_eq!(
            owners.function_span(ByteSpan::new(30, 35)),
            Some(ByteSpan::new(3, 4))
        );
        let ScalarItem::Function(ordinary) = &mut items[2] else {
            panic!("ordinary")
        };
        ordinary.overload_arms.swap(0, 1);
        items.swap(0, 2);
        let reordered_arms = ScalarAssignmentOwners::new(&items);
        assert_eq!(
            reordered_arms.function_span(ByteSpan::new(30, 35)),
            Some(ByteSpan::new(7, 8))
        );
        let ScalarItem::Function(following) = &mut items[2] else {
            panic!("following")
        };
        following.body.span = ByteSpan::new(80, 70);
        items.swap(0, 2);
        let malformed = ScalarAssignmentOwners::new(&items);
        assert_eq!(malformed.spans[0].body_span, ByteSpan::new(80, 70));
        assert_eq!(
            malformed.function_span(ByteSpan::new(90, 60)),
            Some(ByteSpan::new(9, 10))
        );
        assert_eq!(malformed.function_span(ByteSpan::new(75, 76)), None);
        assert!(ScalarAssignmentOwners::new(&[])
            .function_span(ByteSpan::new(0, 0))
            .is_none());
    }

    #[test]
    fn project_owner_lists_follow_module_ordinals_even_with_duplicate_sources() {
        let text = "%%start\ni32(i32) consume = fn(value) { next = value; next };\n%%end";
        let first = derive("same.w", text);
        let second = derive("same.w", &format!("{}{}", " ".repeat(512), text));
        assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
        assert!(second.diagnostics.is_empty(), "{:?}", second.diagnostics);
        assert_eq!(first.program.source, second.program.source);
        let expected = [
            first.program.resolved_assignment_outputs.clone(),
            second.program.resolved_assignment_outputs.clone(),
        ];
        assert_eq!(expected[0].len(), 1);
        assert_eq!(expected[1].len(), 1);
        assert_ne!(expected[0], expected[1]);
        let project = validate_scalar_project(ScalarProject::new(
            vec![
                ScalarModule::from_program(first.program, Vec::new()),
                ScalarModule::from_program(second.program, Vec::new()),
            ],
            vec![source("same.w")],
        ));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
        assert_eq!(project.project.modules.len(), 2);
        for (module, expected) in project.project.modules.iter().zip(expected) {
            assert_eq!(module.resolved_assignment_outputs, expected);
        }
    }

    #[test]
    fn owner_snapshot_rows_depend_only_on_callable_inventory_and_are_shared_by_builders() {
        for declarations in [4, 8, 16] {
            for body in [4, 8, 16] {
                for assignments in [4, 8, 16] {
                    let mut text = String::from("%%start\n");
                    for index in 0..declarations {
                        text.push_str(&format!("i32(i32) function_{index} = fn(value) {{ "));
                        for _ in 0..body {
                            text.push_str("value; ");
                        }
                        for assignment in 0..assignments {
                            text.push_str(&format!(
                                "inferred_{assignment} = value; inferred_{assignment}; "
                            ));
                        }
                        text.push_str("value };\n");
                    }
                    text.push_str("%%end");
                    let result = derive("growth.w", &text);
                    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
                    let program = result.program;
                    let scope = layout::observations::Scope::new();
                    let owners = ScalarAssignmentOwners::new(&program.items);
                    assert_eq!(scope.counts(), layout::observations::Counts::default());
                    drop(scope);
                    assert_eq!(owners.spans.len(), declarations);
                    assert_eq!(
                        std::mem::size_of_val(owners.spans.as_slice()),
                        declarations * std::mem::size_of::<ScalarAssignmentOwner>()
                    );
                    assert_eq!(program.items.len(), declarations);
                    for (item, owner) in program.items.iter().zip(&owners.spans) {
                        let ScalarItem::Function(function) = item else {
                            panic!("function")
                        };
                        assert_eq!(owner.function_span, function.span);
                        assert_eq!(owner.body_span, function.body.span);
                        assert_eq!(function.body.items.len(), body + assignments * 2 + 1);
                    }
                    let overloads =
                        ScalarOverloadEnvironment::single(&program.source, &program.items);
                    let overloads = overloads.typing(0);
                    for _ in 0..2 {
                        let mut builder = ScalarInferenceBuilder::new(&owners);
                        assert!(std::ptr::eq(builder.owners, &owners));
                        let diagnostics = typecheck::validate(&program, &overloads, &mut builder);
                        assert!(diagnostics.is_empty(), "{diagnostics:?}");
                        let frozen = builder.freeze();
                        assert_eq!(frozen.len(), declarations * assignments);
                        assert_eq!(frozen, program.resolved_assignment_outputs);
                    }
                }
            }
        }
    }

    #[test]
    fn shared_owner_lookup_records_actual_source_and_first_owner_and_ignores_unmatched_assignments()
    {
        let result = derive("record.w", VALID);
        assert!(result.diagnostics.is_empty());
        let ScalarItem::Function(function) = &result.program.items[1] else {
            panic!("function")
        };
        let ScalarBlockItem::Assignment(assignment) = &function.body.items[0] else {
            panic!("assignment")
        };
        let mut assignment = assignment.clone();
        assignment.span = ByteSpan::new(30, 35);
        let outputs = ScalarOutputSequence {
            span: assignment.span,
            outputs: vec![
                ScalarOutput {
                    ty: ScalarType::I32,
                    span: assignment.span,
                },
                ScalarOutput {
                    ty: ScalarType::Bool,
                    span: assignment.span,
                },
            ],
        };
        let owners = ScalarAssignmentOwners {
            spans: vec![
                ScalarAssignmentOwner {
                    body_span: ByteSpan::new(20, 40),
                    function_span: ByteSpan::new(1, 2),
                },
                ScalarAssignmentOwner {
                    body_span: ByteSpan::new(10, 60),
                    function_span: ByteSpan::new(3, 4),
                },
            ],
        };
        let mut first = ScalarInferenceBuilder::new(&owners);
        let mut second = ScalarInferenceBuilder::new(&owners);
        assert!(std::ptr::eq(first.owners, second.owners));
        for source in [source("one.w"), source("two.w")] {
            first.record_assignment_outputs(&source, &assignment, &outputs);
            second.record_assignment_outputs(&source, &assignment, &outputs);
        }
        assignment.span = ByteSpan::new(70, 80);
        first.record_assignment_outputs(&source("ignored.w"), &assignment, &outputs);
        let first = first.freeze();
        assert_eq!(first, second.freeze());
        assert_eq!(first.len(), 4);
        for source in [source("one.w"), source("two.w")] {
            for (output_index, ty) in [ScalarType::I32, ScalarType::Bool].iter().enumerate() {
                assert_eq!(
                    first.get(&ResolvedAssignmentOutputId {
                        source: source.clone(),
                        function_span: ByteSpan::new(1, 2),
                        target_span: assignment.targets[output_index].target_span,
                        output_index,
                    }),
                    Some(ty)
                );
            }
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
        let owners = ScalarAssignmentOwners::new(&program.program.items);
        let mut builder = ScalarInferenceBuilder::new(&owners);
        for source in [source("one.w"), source("two.w")] {
            builder.record_assignment_outputs(&source, &equal_targets, &outputs);
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
        let overloads = overloads.typing(0);
        let owners = ScalarAssignmentOwners::new(&program.items);
        let mut scratch = ScalarInferenceBuilder::new(&owners);
        assert!(typecheck::validate(&program, &overloads, &mut scratch).is_empty());
        assert_eq!(program.resolved_assignment_outputs, expected);
        assert_eq!(scratch.freeze(), expected);
        let mut module = ScalarModule::from_program(program, Vec::new());
        let overloads = ScalarOverloadEnvironment::project(std::slice::from_ref(&module));
        let overloads = overloads.typing(0);
        let (modules, items) = split_module_typing(std::slice::from_mut(&mut module));
        let current = &modules[0];
        let mut scratch = ScalarInferenceBuilder::new(&owners);
        let ScalarItem::Function(function) = &items[0][1] else {
            panic!("function")
        };
        let scope = current
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
            current,
            &modules,
            &mut diagnostics,
            false,
            &overloads,
            &mut scratch,
        );
        assert_eq!(actual, ScalarType::I32);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        drop(items);
        drop(modules);
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
        let overloads = overloads.typing(0);
        let owners = ScalarAssignmentOwners::new(&invalid.program.items);
        let mut provisional = ScalarInferenceBuilder::new(&owners);
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

    #[test]
    fn checked_overload_origin_is_required_at_the_actual_call_occurrence() {
        let text = "%%start
pick = overload { i64(i64) => fn(value) { value }; };
i64 input = 1;
i64 chosen = pick(input);
i64(i64) body = fn(n) { pick(n) };
%%end";
        let source = SourceIdentity::new(
            "project".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        );
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            for (initializer, missing) in
                [(true, true), (true, false), (false, true), (false, false)]
            {
                let parsed = crate::parse_source(source.clone(), text.into(), &[]);
                assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
                let (typed, mut occurrences) = typecheck_scalar_program_with_occurrences(
                    derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
                    layout,
                );
                assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
                let call = if initializer {
                    let ScalarItem::Binding(binding) = &typed.program.items[2] else {
                        panic!("initializer");
                    };
                    &binding.value
                } else {
                    let ScalarItem::Function(body) = &typed.program.items[3] else {
                        panic!("body");
                    };
                    body.body.final_expressions().next().unwrap().0
                };
                let address = std::ptr::from_ref(call);
                let row = occurrences.calls.get_mut(&address).unwrap();
                assert_eq!(row.overload_origin, Some((0, 0)));
                assert_eq!(row.overload.as_ref().unwrap().arm_index, 0);
                if missing {
                    occurrences.calls.remove(&address).unwrap();
                } else {
                    row.overload_origin = Some((0, 3));
                }
                let Err(error) = reference::resolve_program_call_sites_with_occurrences(
                    &typed.program,
                    occurrences,
                ) else {
                    panic!("missing or mismatched overload declaration must not resolve");
                };
                assert!(matches!(error,
                    ReferenceAnalysisError::MissingContext {
                        source_span,
                        reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
                    } if source_span.source == source && source_span.range == expression_span(call)
                ));
            }
        }
    }
}
