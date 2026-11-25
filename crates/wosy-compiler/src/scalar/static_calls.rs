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
                contextual_places: contextual::ContextualPlaces::default(),
                build_proofs: Vec::new(),
                runtime_enum_matches: Vec::new(),
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

pub(super) fn prepare_project_occurrences(
    (validation, occurrence_typing): (ScalarProjectValidation, Vec<PreparationOccurrenceTyping>),
    #[cfg(any(test, feature = "internal-checker"))] checker_targets: &[reference::CheckerTargetQuery<'_>],
) -> Result<(ScalarProject, reference::ProjectCallSiteResults), ScalarProjectPreparationRejection> {
    if !validation.diagnostics.is_empty() {
        return Err(ScalarProjectPreparationRejection::extraction_or_typing(
            validation,
        ));
    }
    let project = validation.project;
    #[cfg(not(any(test, feature = "internal-checker")))]
    let calls = reference::resolve_project_call_sites_with_occurrences(&project.modules, occurrence_typing);
    #[cfg(any(test, feature = "internal-checker"))]
    let calls = reference::resolve_project_call_sites_with_checker_targets(
        &project.modules, occurrence_typing, checker_targets,
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
    Ok((project, calls))
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
        Self::from_canonical_typing(typecheck_scalar_project_with_occurrences(project))
    }

    pub(super) fn from_canonical_typing(
        result: (ScalarProjectValidation, Vec<PreparationOccurrenceTyping>),
    ) -> Result<Self, ScalarProjectPreparationRejection> {
        let (project, calls) = prepare_project_occurrences(
            result,
            #[cfg(any(test, feature = "internal-checker"))] &[],
        )?;
        Ok(Self { project, calls })
    }

    #[cfg(any(test, feature = "internal-checker"))]
    pub(super) fn from_contextual_transport(project: ScalarProject, calls: reference::ProjectCallSiteResults) -> Self {
        Self { project, calls }
    }

    #[cfg(any(test, feature = "internal-checker"))]
    pub(crate) fn contextual_formal(&self, module: usize, item: usize, arm: Option<usize>)
        -> Option<(&ReferenceBindingId, &ScalarType)>
    {
        self.calls.function_body(module, item, arm)?.contextual_formal()
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn into_contextual_parts(self) -> (ScalarProject, reference::ProjectCallSiteResults) {
        (self.project, self.calls)
    }

    pub fn project(&self) -> &ScalarProject {
        &self.project
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

    pub(super) fn finish(self) -> ScalarProjectValidation {
        finish_project_validation(self.project, &self.calls)
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
    diagnostics: Vec<Diagnostic>,
}

impl<'a> ScalarBuildValidation<'a> {
    #[cfg(any(test, feature = "internal-checker"))]
    #[allow(dead_code)]
    pub(crate) fn pointer_caller_effect(
        &self, module: usize, caller: usize, callee: usize, claimed: &BigInt,
    ) -> Option<reference::pointer_call::CallerPointeeEffect<'_>> {
        self.results.pointer_caller_effect(&self.prepared.project.modules, &self.prepared.calls,
            module, caller, callee, claimed)
    }

    #[cfg(any(test, feature = "internal-checker"))]
    #[allow(dead_code)]
    pub(crate) fn pointer_entry_wrapping_write(
        &self, module: usize, item: usize,
    ) -> Option<reference::pointer_entry::EntryPointeeWrite<'_>> {
        self.results.pointer_entry_wrapping_write(&self.prepared.project.modules, &self.prepared.calls, module, item)
    }

    pub(crate) fn aggregate(&self, reservations: &AggregateReservations) -> Result<ClosedAggregates, String> {
        self.results
            .closed_aggregates(&self.prepared.project, &self.prepared.calls, reservations)
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
        true,
    );
    let diagnostics = results
        .errors()
        .iter()
        .map(ReferenceAnalysisError::diagnostic)
        .collect();
    ScalarBuildValidation {
        prepared,
        results,
        diagnostics,
    }
}

#[cfg(test)]
mod tests;
