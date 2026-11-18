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
        let validation = typecheck_scalar_program(declarations, target_layout);
        if !validation.diagnostics.is_empty() {
            return Err(validation);
        }
        let program = validation.program;
        let calls = reference::resolve_program_call_sites(&program);
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

pub(super) struct PreparedProject {
    project: ScalarProject,
    calls: reference::ProjectCallSiteResults,
}

impl PreparedProject {
    pub(super) fn new(project: ScalarProject) -> Result<Self, ScalarProjectValidation> {
        let validation = typecheck_scalar_project(project);
        if !validation.diagnostics.is_empty() {
            return Err(validation);
        }
        let project = validation.project;
        let calls = reference::resolve_project_call_sites(&project.modules);
        Ok(Self { project, calls })
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

#[cfg(test)]
mod tests;
