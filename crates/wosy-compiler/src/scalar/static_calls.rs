use super::*;

pub(super) struct PreparedProgram {
    program: ScalarProgram,
    calls: Result<reference::ModuleCallSites, ReferenceAnalysisError>,
}

impl PreparedProgram {
    pub(super) fn new(
        declarations: ScalarSourceDeclarations,
        target_layout: ScalarTargetLayout,
    ) -> Result<Self, ScalarValidation> {
        let (validation, occurrence_typing) =
            typecheck_scalar_program_with_occurrences(declarations, target_layout);
        if !validation.diagnostics.is_empty() {
            return Err(validation);
        }
        let program = validation.program;
        let calls =
            reference::resolve_program_call_sites_with_occurrences(&program, occurrence_typing);
        Ok(Self { program, calls })
    }

    fn analyze(&self) -> Result<reference::ProgramReferenceResults, ReferenceAnalysisError> {
        match &self.calls {
            Ok(calls) => Ok(reference::analyze_program_references(&self.program, calls)),
            Err(error) => Err(error.clone()),
        }
    }

    pub(super) fn finish(mut self) -> ScalarValidation {
        let errors = match self.analyze() {
            Ok(results) => results.install(&mut self.program),
            Err(error) => vec![error],
        };
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

impl PreparedScalarProject {
    pub(super) fn new(project: ScalarProject) -> Result<Self, ScalarProjectValidation> {
        let (validation, occurrence_typing) = typecheck_scalar_project_with_occurrences(project);
        if !validation.diagnostics.is_empty() {
            return Err(validation);
        }
        let project = validation.project;
        let calls = reference::resolve_project_call_sites_with_occurrences(
            &project.modules,
            occurrence_typing,
        );
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

    fn analyze(&self) -> reference::ProjectReferenceResults {
        reference::analyze_project_references(&self.project.modules, &self.calls)
    }

    pub(super) fn finish(mut self) -> ScalarProjectValidation {
        let results = self.analyze();
        let errors = results.install(&mut self.project.modules);
        ScalarProjectValidation {
            project: self.project,
            diagnostics: errors.into_iter().map(|error| error.diagnostic()).collect(),
        }
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
    let results = prepared.analyze();
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
