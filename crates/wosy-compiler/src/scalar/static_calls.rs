use super::*;

pub(crate) struct PreparedProgram {
    program: ScalarProgram,
    aggregate: reference::aggregate::InstalledAggregateFacts,
    calls: reference::ModuleCallSites,
}

impl PreparedProgram {
    pub(crate) fn rebuild_raw(program: &ScalarProgram) -> Result<Self, String> {
        let mut prepared = Self::new(
            ScalarSourceDeclarations {
                source: program.source.clone(),
                items: program.items.clone(),
                structs: program.structs.clone(),
                enums: program.enums.clone(),
                diagnostics: Vec::new(),
            },
            program.target_layout,
        )
        .map_err(|_| "cannot emit LLVM for an invalid scalar program".to_owned())?;
        let results = prepared.analyze();
        let (errors, aggregate) = results.install_retaining_aggregate(&mut prepared.program);
        if !errors.is_empty() {
            return Err("cannot emit LLVM for an invalid scalar program".into());
        }
        prepared.aggregate = aggregate;
        Ok(prepared)
    }

    pub(crate) fn aggregate(&self, reservations: &AggregateReservations) -> Result<ClosedAggregates, String> {
        reference::aggregate::close_installed(reservations, &self.program, &self.calls, &self.aggregate)
    }

    pub(crate) fn program(&self) -> &ScalarProgram {
        &self.program
    }

    pub(crate) fn body_overload_reservations(
        &self,
        item: usize,
    ) -> Option<impl Iterator<Item = &ResolvedCallTarget>> {
        Some(
            self.calls
                .function_body(item, None)?
                .overload_reservations(),
        )
    }

    pub(crate) fn initialization_region(&self) -> &control::ModuleInitializationRegion {
        &self.calls.initialization
    }

    pub(crate) fn bind_body<'a>(
        &'a self,
        item: usize,
        arm: Option<usize>,
        body: &'a ScalarBlock,
    ) -> Result<control::CallableBodyCallNodes<'a>, String> {
        let source = &self.program.source;
        let calls = self.calls.function_body(item, arm).ok_or_else(|| {
            format!("internal error: missing callable body call topology in {source:?} at item {item}, arm {arm:?}")
        })?;
        control::CallableBodyCallNodes::bind(source, body, calls, None)
    }

    #[cfg(test)]
    pub(super) fn function_calls(&self, item: usize) -> &reference::FunctionCallSites {
        self.calls.function(item)
    }

    pub(super) fn new(
        declarations: ScalarSourceDeclarations,
        target_layout: ScalarTargetLayout,
    ) -> Result<Self, ScalarValidation> {
        let (mut validation, occurrence_typing) =
            typecheck_scalar_program_with_occurrences(declarations, target_layout);
        if !validation.diagnostics.is_empty() {
            return Err(validation);
        }
        let calls = match reference::resolve_program_call_sites_with_occurrences(
            &validation.program,
            occurrence_typing,
        ) {
            Ok(calls) => calls,
            Err(error) => {
                validation.diagnostics.push(error.diagnostic());
                return Err(validation);
            }
        };
        Ok(Self {
            program: validation.program,
            aggregate: reference::aggregate::InstalledAggregateFacts::default(),
            calls,
        })
    }

    fn analyze(&self) -> reference::ProgramReferenceResults {
        reference::analyze_program_references(&self.program, &self.calls)
    }

    pub(super) fn finish(mut self) -> ScalarValidation {
        let errors = self.analyze().install(&mut self.program);
        ScalarValidation {
            program: self.program,
            diagnostics: errors.into_iter().map(|error| error.diagnostic()).collect(),
        }
    }
}

pub struct PreparedScalarProject {
    project: ScalarProject,
    calls: reference::ProjectCallSiteResults,
    checked_access: Option<CheckedAccessDeclarations>,

    require_release_checked_access: bool,
    root_array_uses: reference::ClosedRootArrayUses,
    generated_callables: BTreeMap<super::anonymous_callable::PreparedInlineOwner, super::anonymous_callable::PreparedGeneratedCallable>,
    inline_arms: BTreeMap<super::anonymous_callable::PreparedInlineOwner, super::anonymous_callable::PreparedAnonymousCallable>,
}

pub(crate) struct CheckedAccessDeclarations {
    pub(crate) source: SourceIdentity,
    pub(crate) non_null: String,
    pub(crate) in_extent: String,
}

impl CheckedAccessDeclarations {
    fn seal(module: &ScalarModule) -> Option<Self> {
        use ScalarType::{Callable, RawPointer, U8, U64, Unit};
        if !module.structs.is_empty() || !module.enums.is_empty() {
            return None;
        }
        let [ScalarItem::Function(non_null), ScalarItem::Function(in_extent)] =
            module.items.as_slice() else { return None };
        for (function, parameters) in [(non_null, vec![RawPointer(Box::new(U8))]), (in_extent, vec![U64, U64])] {
            let Callable { parameters: actual, outputs } = &function.signature else { return None };
            if *actual != parameters || outputs.outputs.len() != 1 || outputs.outputs[0].ty != Unit
                || !function.generic_parameters.is_empty() || !function.overload_arms.is_empty()
                || function.body.unsafe_context
            {
                return None;
            }
        }
        Some(Self {
            source: module.source.clone(),
            non_null: non_null.name.clone(),
            in_extent: in_extent.name.clone(),
        })
    }
}


/// Rejected preparation with read-only `project` and `diagnostics` inspection.
pub struct ScalarProjectPreparationRejection {
    inspection: ScalarProjectValidation,
    kind: ProjectPreparationRejectionKind,
}

enum ProjectPreparationRejectionKind {
    ExtractionOrTyping,
    StaticLookup(reference::ProjectCallSiteResults),
}

impl std::ops::Deref for ScalarProjectPreparationRejection {
    type Target = ScalarProjectValidation;

    fn deref(&self) -> &Self::Target {
        &self.inspection
    }
}

impl std::fmt::Debug for ScalarProjectPreparationRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScalarProjectPreparationRejection")
            .field("project", &self.project)
            .field("diagnostics", &self.diagnostics)
            .finish_non_exhaustive()
    }
}

impl ScalarProjectPreparationRejection {
    pub(super) fn extraction_or_typing(inspection: ScalarProjectValidation) -> Self {
        Self {
            inspection,
            kind: ProjectPreparationRejectionKind::ExtractionOrTyping,
        }
    }

    /// Take ownership of the rejected project and preparation diagnostics.
    pub fn into_inspection(self) -> ScalarProjectValidation {
        self.inspection
    }

    /// Return build-facing diagnostics in module order. A static lookup rejection runs the
    /// existing origin query for modules with retained static rows and discards its results
    /// after collecting diagnostics.
    pub fn into_ordered_diagnostics(self) -> Vec<Diagnostic> {
        match self.kind {
            ProjectPreparationRejectionKind::ExtractionOrTyping => self.inspection.diagnostics,
            ProjectPreparationRejectionKind::StaticLookup(calls) => {
                reference::analyze_project_references(&self.inspection.project.modules, &calls)
                    .errors()
                    .iter()
                    .map(ReferenceAnalysisError::diagnostic)
                    .collect()
            }
        }
    }

    pub(super) fn finish(self) -> ScalarProjectValidation {
        match self.kind {
            ProjectPreparationRejectionKind::ExtractionOrTyping => self.inspection,
            ProjectPreparationRejectionKind::StaticLookup(calls) => {
                finish_project_validation(self.inspection.project, &calls)
            }
        }
    }
}

impl PreparedScalarProject {
    #[cfg(test)]
    pub(super) fn function_calls(
        &self,
        module: usize,
        item: usize,
    ) -> &reference::FunctionCallSites {
        self.calls.function(module, item)
    }

    pub(crate) fn new(project: ScalarProject) -> Result<Self, ScalarProjectPreparationRejection> {
        Self::new_with_access(project, None)
    }

    pub(super) fn new_with_access(
        project: ScalarProject,
        access: Option<(wosy_stdlib_source::CheckedAccessComponent, ScalarTargetLayout)>,
    ) -> Result<Self, ScalarProjectPreparationRejection> {
        Self::new_impl(project, access, None)
    }

    pub(super) fn new_anonymous_arm(
        module: ScalarModule,
    ) -> Result<Self, ScalarProjectPreparationRejection> {
        let [ScalarItem::Function(arm)] = module.items.as_slice() else {
            unreachable!("one isolated anonymous callable");
        };
        assert!(arm.name.is_empty() && module.namespace_bindings.is_empty()
            && module.members.is_empty() && module.structs.is_empty() && module.enums.is_empty());
        let span = arm.span;
        let source = module.source.clone();
        Self::new_impl(ScalarProject::new(vec![module], vec![source]), None, Some(span))
    }

    fn new_impl(
        project: ScalarProject,
        access: Option<(wosy_stdlib_source::CheckedAccessComponent, ScalarTargetLayout)>,
        isolated_anonymous_arm: Option<ByteSpan>,
    ) -> Result<Self, ScalarProjectPreparationRejection> {
        let (mut validation, mut occurrence_typing) =
            super::typecheck_scalar_project_with_occurrences_for(project, isolated_anonymous_arm);
        if !validation.diagnostics.is_empty() {
            return Err(ScalarProjectPreparationRejection::extraction_or_typing(validation));
        }
        let require_release_checked_access = access.is_some();
        let mut checked_access = None;
        if let Some((input, target_layout)) = access
            .filter(|_| occurrence_typing.iter().any(PreparationOccurrenceTyping::needs_checked_access))
        {
            let source = SourceIdentity::new(
                input.project().to_owned(),
                input.package().to_owned(),
                input.path().to_owned(),
                input.revision().to_owned(),
            );
            if validation.project.modules.iter().any(|module| module.source == source) {
                validation.diagnostics.push(Self::access_diagnostic());
                return Err(ScalarProjectPreparationRejection::extraction_or_typing(validation));
            }
            let (fresh, diagnostics) =
                crate::parse_source_with_snapshot(source.clone(), input.text().to_owned(), &[]);
            if !diagnostics.is_empty() {
                validation.diagnostics = diagnostics;
                return Err(ScalarProjectPreparationRejection::extraction_or_typing(validation));
            }
            let ScalarSourceDeclarations { source, items, structs, enums, diagnostics } =
                derive_scalar_declarations_from_cst(fresh.result().canonical_cst());
            if !diagnostics.is_empty() {
                validation.diagnostics = diagnostics;
                return Err(ScalarProjectPreparationRejection::extraction_or_typing(validation));
            }
            let mut module = ScalarModule::new(source.clone(), items, Vec::new());
            module.structs = structs;
            module.enums = enums;
            module.target_layout = target_layout;
            module.layouts = ScalarTargetLayouts::new(target_layout);
            // This release component is independent: type only the tiny leaf,
            // never replay user declaration/type analysis. Append its typed
            // rows before one shared project call-site resolution and build.
            let (helper, typed) = typecheck_scalar_project_with_occurrences(
                ScalarProject::new(vec![module], vec![source.clone()]),
            );
            if !helper.diagnostics.is_empty() {
                return Err(ScalarProjectPreparationRejection::extraction_or_typing(helper));
            }
            checked_access = helper.project.modules.first().and_then(CheckedAccessDeclarations::seal);
            if checked_access.is_none() {
                validation.diagnostics.push(Self::access_diagnostic());
                return Err(ScalarProjectPreparationRejection::extraction_or_typing(validation));
            }
            validation.project.modules.extend(helper.project.modules);
            validation.project.initialization_order.push(source);
            occurrence_typing.extend(typed);
        }
        let generated_count = occurrence_typing.iter().map(PreparationOccurrenceTyping::generated_callable_count).sum::<usize>();
        let inline_count = occurrence_typing.iter().map(PreparationOccurrenceTyping::anonymous_element_count).sum::<usize>();
        let project = validation.project;
        let calls = reference::resolve_project_call_sites_with_occurrences_for(
            &project.modules,
            occurrence_typing,
            isolated_anonymous_arm,
        );
        let diagnostics = calls
            .lookup_errors()
            .map(ReferenceAnalysisError::diagnostic)
            .collect::<Vec<_>>();
        if !diagnostics.is_empty() {
            return Err(ScalarProjectPreparationRejection {
                inspection: ScalarProjectValidation {
                    project,
                    diagnostics,
                },
                kind: ProjectPreparationRejectionKind::StaticLookup(calls),
            });
        }
        let mut prepared = Self { project, calls, checked_access, require_release_checked_access, inline_arms: BTreeMap::new(), generated_callables: BTreeMap::new(), root_array_uses: Default::default() };
        if inline_count != 0 {
            if isolated_anonymous_arm.is_some() {
                return Err(ScalarProjectPreparationRejection::extraction_or_typing(ScalarProjectValidation {
                    project: prepared.project,
                    diagnostics: vec![Diagnostic { code: "B0003".into(), severity: crate::DiagnosticSeverity::Error,
                        message: "nested inline callable lies outside the closed scalar pool".into(), labels: Vec::new(), notes: Vec::new() }],
                }));
            }
            let arms = match super::anonymous_callable::collect_prepared_inline_arms(&prepared) {
                Ok(arms) if arms.len() == inline_count => arms,
                result => {
                    let diagnostics = result.err().unwrap_or_else(|| vec![Diagnostic {
                        code: "B0003".into(), severity: crate::DiagnosticSeverity::Error,
                        message: "inline callable is outside its admitted direct lexical fixed array owner".into(),
                        labels: Vec::new(), notes: Vec::new(),
                    }]);
                    return Err(ScalarProjectPreparationRejection::extraction_or_typing(ScalarProjectValidation {
                        project: prepared.project, diagnostics,
                    }));
                }
            };
            prepared.inline_arms = arms;
        }
        if generated_count != 0 {
            let result = super::anonymous_callable::collect_prepared_generated_callables(&prepared);
            prepared.generated_callables = match result {
                Ok(arms) if isolated_anonymous_arm.is_none() && arms.len() == generated_count => arms,
                result => {
                    let diagnostics = result.err().unwrap_or_else(|| vec![Diagnostic {
                        code: "B0003".into(), severity: crate::DiagnosticSeverity::Error,
                        message: "generated callable is outside its admitted direct lexical fixed array owner".into(),
                        labels: Vec::new(), notes: Vec::new(),
                    }]);
                    return Err(ScalarProjectPreparationRejection::extraction_or_typing(ScalarProjectValidation {
                        project: prepared.project, diagnostics,
                    }));
                }
            };
        }
        prepared.root_array_uses = reference::close_root_array_uses(&prepared, &prepared.calls);
        Ok(prepared)
    }

    pub(crate) fn root_array_uses(&self) -> &reference::ClosedRootArrayUses {
        &self.root_array_uses
    }

    pub(crate) fn generated_callables(&self) -> impl Iterator<Item = (
        super::anonymous_callable::PreparedInlineOwner, &super::anonymous_callable::PreparedGeneratedCallable
    )> {
        self.generated_callables.iter().map(|(owner, arm)| (*owner, arm))
    }

    pub(crate) fn inline_arms(&self) -> impl Iterator<Item = (
        super::anonymous_callable::PreparedInlineOwner, &super::anonymous_callable::PreparedAnonymousCallable
    )> {
        self.inline_arms.iter().map(|(owner, arm)| (*owner, arm))
    }

    fn access_diagnostic() -> crate::Diagnostic {
        crate::Diagnostic {
            code: "B0003".to_owned(),
            severity: crate::DiagnosticSeverity::Error,
            message: "invalid release checked-access component".to_owned(),
            labels: Vec::new(),
            notes: Vec::new(),
        }
    }

    pub(crate) fn requires_release_checked_access(&self) -> bool {
        self.require_release_checked_access
    }

    pub fn checked_access_source(&self) -> Option<&SourceIdentity> {
        self.checked_access.as_ref().map(|access| &access.source)
    }

    pub(crate) fn checked_access(&self) -> Option<&CheckedAccessDeclarations> {
        self.checked_access.as_ref()
    }

    pub fn project(&self) -> &ScalarProject {
        &self.project
    }

    pub(crate) fn has_closed_shared_scalar_reader(&self, module: usize, declaration: ByteSpan) -> bool {
        self.calls.has_closed_shared_scalar_reader(module, declaration)
    }

    pub(crate) fn body_overload_reservations(
        &self,
        module: usize,
        item: usize,
    ) -> Option<impl Iterator<Item = &ResolvedCallTarget>> {
        Some(
            self.calls
                .function_body(module, item, None)?
                .overload_reservations(),
        )
    }

    pub(crate) fn initialization_region(
        &self,
        module_ordinal: usize,
    ) -> Option<&control::ModuleInitializationRegion> {
        self.calls.initialization_region(module_ordinal)
    }

    pub(crate) fn bind_callable_body_call_nodes<'a>(
        &'a self,
        module_ordinal: usize,
        item_ordinal: usize,
        arm_index: Option<usize>,
        body: &'a ScalarBlock,
    ) -> Result<control::CallableBodyCallNodes<'a>, String> {
        self.bind_body_with_proof(module_ordinal, item_ordinal, arm_index, body, None)
    }

    #[allow(dead_code)]
    pub(crate) fn initializer_facts(
        &self,
        module_ordinal: usize,
    ) -> Result<control::InitializerFacts<'_>, control::InitializerFactsError> {
        use control::InitializerFactsError as Error;

        let module = self
            .project
            .modules
            .get(module_ordinal)
            .ok_or(Error::Missing("initializer module"))?;
        let region = self
            .initialization_region(module_ordinal)
            .ok_or(Error::Missing("initializer region"))?;
        if region.source != module.source {
            return Err(Error::Malformed("initializer region source"));
        }
        control::InitializerFacts::bind(region, &module.items)
    }

    fn bind_body_with_proof<'a>(
        &'a self,
        module_ordinal: usize,
        item_ordinal: usize,
        arm_index: Option<usize>,
        body: &'a ScalarBlock,
        checked_proof: Option<reference::self_tail::checked::CheckedForwardingProof>,
    ) -> Result<control::CallableBodyCallNodes<'a>, String> {
        let module = self.project.modules.get(module_ordinal).ok_or_else(|| {
            format!("internal error: missing callable body module at ordinal {module_ordinal}")
        })?;
        let calls = self
            .calls
            .function_body(module_ordinal, item_ordinal, arm_index)
            .ok_or_else(|| {
                format!(
                    "internal error: missing callable body call topology in {:?} at item {item_ordinal}, arm {arm_index:?}",
                    module.source,
                )
            })?;
        control::CallableBodyCallNodes::bind(&module.source, body, calls, checked_proof)
    }

    #[cfg(test)]
    pub(crate) fn remove_body_allocation_length_target(
        &mut self,
        module: usize,
        item: usize,
        arm: usize,
    ) -> (wosy_syntax::ByteSpan, usize) {
        self.calls
            .remove_body_allocation_length_target(module, item, arm)
    }

    #[cfg(test)]
    fn analyze(&self) -> reference::ProjectReferenceResults {
        reference::analyze_project_references(&self.project.modules, &self.calls)
    }

    pub(super) fn finish(mut self) -> ScalarProjectValidation {
        if self.checked_access.is_none() {
            return finish_project_validation(self.project, &self.calls);
        }
        let results = reference::analyze_project_references_with_call_points(&self.project.modules, &self.calls,
            reference::ReferenceCapture { call_points: false, private_runtime_reads: true });
        let (_, pending_errors) = results.finalize_private_runtime_reads(&self.project, &self.calls, true);
        let mut errors = results.install(&mut self.project.modules);
        errors.extend(pending_errors);
        ScalarProjectValidation {
            project: self.project,
            diagnostics: errors.iter().map(ReferenceAnalysisError::diagnostic).collect(),
        }
    }
}

fn finish_project_validation(
    mut project: ScalarProject,
    calls: &reference::ProjectCallSiteResults,
) -> ScalarProjectValidation {
    let results = reference::analyze_project_references(&project.modules, calls);
    let errors = results.install(&mut project.modules);
    ScalarProjectValidation {
        project,
        diagnostics: errors
            .iter()
            .map(ReferenceAnalysisError::diagnostic)
            .collect(),
    }
}

pub struct ScalarBuildValidation<'a> {
    prepared: &'a PreparedScalarProject,
    results: reference::ProjectReferenceResults,
    private_runtime_reads: reference::aggregate::PrivateRuntimeReadPlans,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> ScalarBuildValidation<'a> {
    pub(super) fn into_reference_results(self) -> reference::ProjectReferenceResults {
        self.results
    }


    pub(crate) fn aggregate(&self, reservations: &AggregateReservations) -> Result<ClosedAggregates, String> {
        let mut result = self.results.closed_aggregates(&self.prepared.project, &self.prepared.calls, reservations)?;
        result.install_private_runtime_reads(&self.private_runtime_reads, reservations)?;
        Ok(result)
    }

    pub fn prepared(&self) -> &'a PreparedScalarProject {
        self.prepared
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub(crate) fn checked_self_tail_forwarding(
        &self,
        module: usize,
        item: usize,
    ) -> Option<reference::self_tail::checked::CheckedForwardingProof> {
        if !self.diagnostics.is_empty() {
            return None;
        }
        let module_row = self.prepared.project.modules.get(module)?;
        let ScalarItem::Function(function) = module_row.items.get(item)? else {
            return None;
        };
        reference::self_tail::checked::prove(
            &module_row.source,
            function,
            self.prepared.calls.function_body(module, item, None)?,
            self.reference_cfg(module, item, None)?,
        )
    }

    pub(crate) fn bind_original_project_body<'b>(
        &'b self,
        module: usize,
        item: usize,
        function: &'b ScalarFunction,
    ) -> Result<control::CallableBodyCallNodes<'b>, String> {
        let proof = match &self.prepared.project.modules[module].items[item] {
            ScalarItem::Function(original) if std::ptr::eq(original, function) => {
                self.checked_self_tail_forwarding(module, item)
            }
            _ => None,
        };
        self.prepared
            .bind_body_with_proof(module, item, None, &function.body, proof)
    }

    pub(crate) fn reference_cfg(
        &self,
        module_ordinal: usize,
        item_ordinal: usize,
        arm_index: Option<usize>,
    ) -> Option<&ScalarReferenceCfg> {
        self.results
            .reference_cfg(module_ordinal, item_ordinal, arm_index)
    }

    #[allow(dead_code)]
    pub(crate) fn callable_point_facts(
        &self,
        module: usize,
        item: usize,
        arm: Option<usize>,
    ) -> Option<&control::CallablePointFacts> {
        self.results.callable_point_facts(module, item, arm)
    }
}

pub fn analyze_scalar_build(prepared: &PreparedScalarProject) -> ScalarBuildValidation<'_> {
    let results = reference::analyze_project_references_with_call_points(
        &prepared.project.modules,
        &prepared.calls,
        reference::ReferenceCapture { call_points: true, private_runtime_reads: prepared.checked_access.is_some() },
    );
    let (private_runtime_reads, pending_errors) = results.finalize_private_runtime_reads(
        &prepared.project, &prepared.calls, prepared.checked_access.is_some(),
    );
    let diagnostics = results.errors().iter().chain(&pending_errors)
        .map(ReferenceAnalysisError::diagnostic).collect();
    ScalarBuildValidation {
        prepared,
        results,
        private_runtime_reads,
        diagnostics,
    }
}

#[cfg(test)]
mod tests;
