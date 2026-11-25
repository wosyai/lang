//! Selection and freshness for the release-owned ordinary Wosy access source.
//! This root is deliberately not exposed as a user namespace or package alias.

use wosy_compiler::{derive_scalar_declarations_from_cst, parse_source_with_snapshot, ScalarSourceDeclarations};
use wosy_project::{
    ContentRevision, PackageRelativePath, SourceIdentity, SourceNodeInput, SourceObservation,
};
use wosy_stdlib_source::CheckedAccessComponent as ReleaseAccess;

pub(super) struct CheckedAccessComponent {
    observation: SourceObservation,
    release: ReleaseAccess,
}

impl CheckedAccessComponent {
    pub(super) fn linked() -> Self {
        let release = ReleaseAccess::linked();
        Self {
            observation: SourceObservation {
                source: SourceIdentity::new(
                    release.project().to_owned(),
                    release.package().to_owned(),
                    PackageRelativePath::new(release.path().into()).expect("fixed relative module"),
                ),
                content_revision: ContentRevision(release.revision().to_owned()),
            },
            release,
        }
    }

    pub(super) fn into_source(self) -> (SourceNodeInput, ReleaseAccess) {
        let SourceObservation { source, content_revision } = self.observation;
        (
            SourceNodeInput { source, content_revision, namespace_edges: Vec::new() },
            self.release,
        )
    }

    pub(super) fn into_preparation(
        self,
    ) -> Result<(SourceNodeInput, ScalarSourceDeclarations, String), String> {
        let SourceObservation { source, content_revision } = self.observation;
        let compiler_identity = super::compiler_source_identity(&source, &content_revision);
        let (fresh, diagnostics) =
            parse_source_with_snapshot(compiler_identity, self.release.text().to_owned(), &[]);
        if !diagnostics.is_empty() {
            return Err("toolchain checked-access source has syntax diagnostics".to_owned());
        }
        let declarations = derive_scalar_declarations_from_cst(fresh.result().canonical_cst());
        if !declarations.diagnostics().is_empty() || declarations.namespaces().next().is_some() {
            return Err("toolchain checked-access source is not an independent module".to_owned());
        }
        Ok((
            SourceNodeInput { source, content_revision, namespace_edges: Vec::new() },
            declarations,
            fresh.into_result().text,
        ))
    }
}

pub(super) fn refresh_observation(source: &SourceIdentity) -> Result<Option<SourceObservation>, String> {
    if source.project != ReleaseAccess::linked().project() {
        return Ok(None);
    }
    let current = CheckedAccessComponent::linked().observation;
    if &current.source != source {
        return Err("unknown toolchain source observation".to_owned());
    }
    Ok(Some(current))
}
