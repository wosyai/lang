use std::{collections::BTreeMap, ffi::OsStr, path::Path};

use wosy_compiler::{
    check_loaded_scalar_ordered_local_proof_contexts, MainFlowProofReceipt, ScalarCheckerScope,
    ScalarCheckerTextFile, ScalarOrderedCheckerFailure, ScalarOrderedCheckerOutcome,
    ScalarPreparationInputs, ScalarTargetLayout,
};
use wosy_project::{PackageRelativePath, ProjectConfiguration, ProjectContext, SourceIdentity};

use crate::canonical_loader::{
    load_canonical_project_inputs, load_explicit_dormant_proof_sources, CanonicalLoadFailure,
    CanonicalLoadPurpose,
};

pub(crate) struct ConfiguredCheckReceipt {
    pub(crate) outcome: Result<MainFlowProofReceipt, ScalarOrderedCheckerOutcome>,
    pub(crate) texts: BTreeMap<SourceIdentity, String>,
}

pub(crate) enum ConfiguredCheckLoadError {
    Root,
    Manifest,
    Target,
    Profile,
    Layout,
    Canonical(CanonicalLoadFailure),
    DormantPath,
    Dormant(CanonicalLoadFailure),
}

pub(crate) fn check_target(
    root: &Path,
    target: Option<&str>,
    profile: &str,
    scope: ScalarCheckerScope,
    dormant_paths: Vec<&OsStr>,
) -> Result<ConfiguredCheckReceipt, ConfiguredCheckLoadError> {
    use ConfiguredCheckLoadError as LoadError;
    let root = root.canonicalize().map_err(|_| LoadError::Root)?;
    let (configuration, manifest) =
        ProjectConfiguration::load_with_manifest(&root).map_err(|_| LoadError::Manifest)?;
    let target = configuration.resolve_target(target).map_err(|_| LoadError::Target)?;
    let project = ProjectContext::from_manifest(&root, &target, manifest.as_table())
        .map_err(|_| LoadError::Target)?;
    let (_, _, artifact, _) = configuration.artifact_profile(&target, profile)
        .map_err(|_| LoadError::Profile)?;
    let layout = artifact.target_layout().map_err(|_| LoadError::Layout)?;
    let target_layout = ScalarTargetLayout {
        pointer_size: layout.pointer_size, pointer_alignment: layout.pointer_alignment,
    };
    let mut loaded = load_canonical_project_inputs(
        &root, &project, &configuration, CanonicalLoadPurpose::LocalProofs, || {},
    ).map_err(LoadError::Canonical)?;
    let dormant = if scope == ScalarCheckerScope::CurrentPackage {
        let paths = dormant_paths.into_iter().map(|path| PackageRelativePath::new(path.into()))
            .collect::<Result<Vec<_>, _>>().map_err(|_| LoadError::DormantPath)?;
        load_explicit_dormant_proof_sources(&root, &project, &configuration, paths, &mut loaded.texts)
            .map_err(LoadError::Dormant)?
    } else {
        Vec::new()
    };
    let outcome = check_loaded_scalar_ordered_local_proof_contexts(
        loaded.declarations, loaded.proofs.into_values(), loaded.initialization_order,
        ScalarPreparationInputs { target_layout }, scope, dormant,
    );
    Ok(ConfiguredCheckReceipt { outcome, texts: loaded.texts })
}

#[derive(Debug, PartialEq)]
pub(crate) enum InternalCheckStatus<'a> {
    Zero,
    FirstNonzero(&'a ScalarOrderedCheckerFailure),
}

#[derive(Debug, PartialEq)]
pub(crate) enum InternalCheckEffect<'a> {
    Stdout(&'a [u8]),
    Stderr(&'a [u8]),
    DeferredFile(&'a ScalarCheckerTextFile),
}

pub(crate) struct OrderedCheckEffects<'a> {
    reports: &'a [ScalarOrderedCheckerFailure],
}

impl<'a> OrderedCheckEffects<'a> {
    pub(crate) fn new(reports: &'a [ScalarOrderedCheckerFailure]) -> Self {
        Self { reports }
    }

    pub(crate) fn selected_status(self) -> InternalCheckStatus<'a> {
        self.reports.iter().find(|report| report.code().bits() != 0)
            .map_or(InternalCheckStatus::Zero, InternalCheckStatus::FirstNonzero)
    }

    pub(crate) fn visit_authored_effects<E>(
        self,
        mut sink: impl FnMut(InternalCheckEffect<'a>) -> Result<(), E>,
    ) -> Result<(), E> {
        for report in self.reports {
            if !report.stdout().is_empty() {
                sink(InternalCheckEffect::Stdout(report.stdout()))?;
            }
            if !report.stderr().is_empty() {
                sink(InternalCheckEffect::Stderr(report.stderr()))?;
            }
            for file in report.files() {
                sink(InternalCheckEffect::DeferredFile(file))?;
            }
        }
        Ok(())
    }
}
