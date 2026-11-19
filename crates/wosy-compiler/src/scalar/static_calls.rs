use super::*;

pub(super) struct PreparedProgram {
    program: ScalarProgram,
    calls: reference::ModuleCallSites,
}

impl PreparedProgram {
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

impl PreparedScalarProject {
    #[cfg(test)]
    pub(super) fn function_calls(
        &self,
        module: usize,
        item: usize,
    ) -> &reference::FunctionCallSites {
        self.calls.function(module, item)
    }

    pub(super) fn new(project: ScalarProject) -> Result<Self, ScalarProjectPreparationRejection> {
        let (validation, occurrence_typing) = typecheck_scalar_project_with_occurrences(project);
        if !validation.diagnostics.is_empty() {
            return Err(ScalarProjectPreparationRejection::extraction_or_typing(
                validation,
            ));
        }
        let project = validation.project;
        let calls = reference::resolve_project_call_sites_with_occurrences(
            &project.modules,
            occurrence_typing,
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
        Ok(Self { project, calls })
    }

    pub fn project(&self) -> &ScalarProject {
        &self.project
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
        control::CallableBodyCallNodes::bind(&module.source, body, calls)
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
    pub fn prepared(&self) -> &'a PreparedScalarProject {
        self.prepared
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
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
