use std::{collections::{BTreeMap, HashSet}, ffi::OsStr, path::{Component, Path, PathBuf}};

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

pub(crate) struct PlannedCheckPublication<'a> {
    pub(crate) report: &'a MainFlowProofReceipt,
    pub(crate) texts: &'a BTreeMap<SourceIdentity, String>,
    pub(crate) status: InternalCheckStatus<'a>,
    pub(crate) effects: Vec<PlannedCheckEffect<'a>>,
}

pub(crate) enum PlannedCheckEffect<'a> {
    Stdout(&'a [u8]),
    Stderr(&'a [u8]),
    DeferredFile {
        relative: PathBuf,
        request: &'a ScalarCheckerTextFile,
        source: &'a wosy_syntax::SourceIdentity,
    },
}

#[derive(Debug)]
pub(crate) enum CheckPublicationError<'a> {
    Incomplete(&'a ScalarOrderedCheckerOutcome),
    InvalidPath {
        source: &'a wosy_syntax::SourceIdentity,
        request: &'a ScalarCheckerTextFile,
    },
    DuplicatePath {
        source: &'a wosy_syntax::SourceIdentity,
        request: &'a ScalarCheckerTextFile,
    },
}

impl ConfiguredCheckReceipt {
    pub(crate) fn plan_publication(&self) -> Result<PlannedCheckPublication<'_>, CheckPublicationError<'_>> {
        let report = self.outcome.as_ref().map_err(CheckPublicationError::Incomplete)?;
        let mut effects = Vec::new();
        let mut seen = HashSet::new();
        let status = OrderedCheckEffects::new(&report.directives).visit_with_origin(|origin, effect| {
            let planned = match effect {
                InternalCheckEffect::Stdout(bytes) => PlannedCheckEffect::Stdout(bytes),
                InternalCheckEffect::Stderr(bytes) => PlannedCheckEffect::Stderr(bytes),
                InternalCheckEffect::DeferredFile(request) => {
                    let source = &origin.source;
                    let relative = normalized_output_path(&request.path)
                        .ok_or(CheckPublicationError::InvalidPath { source, request })?;
                    if !seen.insert(relative.clone()) {
                        return Err(CheckPublicationError::DuplicatePath { source, request });
                    }
                    PlannedCheckEffect::DeferredFile { relative, request, source }
                }
            };
            effects.push(planned);
            Ok(())
        })?;
        Ok(PlannedCheckPublication { report, texts: &self.texts, status, effects })
    }
}

fn normalized_output_path(requested: &str) -> Option<PathBuf> {
    if requested.as_bytes().contains(&0) {
        return None;
    }
    let mut path = PathBuf::new();
    for component in Path::new(requested).components() {
        match component {
            Component::Normal(name) => {
                let mut standalone = Path::new(name).components();
                if standalone.next() != Some(Component::Normal(name)) || standalone.next().is_some() {
                    return None;
                }
                path.push(name);
            }
            Component::CurDir => {},
            Component::ParentDir if path.pop() => {},
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => return None,
        }
    }
    let mut components = path.components();
    (matches!(components.next(), Some(Component::Normal(_)))
        && components.all(|component| matches!(component, Component::Normal(_)))).then_some(path)
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
        self.visit_with_origin(|_, effect| sink(effect)).map(|_| ())
    }

    fn visit_with_origin<E>(
        self,
        mut sink: impl FnMut(&'a ScalarOrderedCheckerFailure, InternalCheckEffect<'a>) -> Result<(), E>,
    ) -> Result<InternalCheckStatus<'a>, E> {
        let mut status = InternalCheckStatus::Zero;
        for report in self.reports {
            if matches!(status, InternalCheckStatus::Zero) && report.code().bits() != 0 {
                status = InternalCheckStatus::FirstNonzero(report);
            }
            if !report.stdout().is_empty() {
                sink(report, InternalCheckEffect::Stdout(report.stdout()))?;
            }
            if !report.stderr().is_empty() {
                sink(report, InternalCheckEffect::Stderr(report.stderr()))?;
            }
            for file in report.files() {
                sink(report, InternalCheckEffect::DeferredFile(file))?;
            }
        }
        Ok(status)
    }
}


#[cfg(test)]
mod tests {
    use super::normalized_output_path;
    use std::path::Path;

    #[test]
    fn planned_output_paths_remain_relative_after_normalization() {
        assert_eq!(normalized_output_path("sub/../preview.txt").as_deref(), Some(Path::new("preview.txt")));
        for path in ["/absolute.txt", "../escape.txt", "sub/../..", "sub/..", "", "nul\0.txt"] {
            assert!(normalized_output_path(path).is_none(), "{path:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn planned_output_paths_reject_drive_prefixes_in_interior_components() {
        for path in ["C:/absolute.txt", "sub/C:drive-relative.txt", "sub/../C:drive-relative.txt"] {
            assert!(normalized_output_path(path).is_none(), "{path:?}");
        }
    }
}
