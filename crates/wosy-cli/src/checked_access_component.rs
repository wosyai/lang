//! Selection and freshness for the release-owned ordinary Wosy access source.
//! This root is deliberately not exposed as a user namespace or package alias.

use wosy_compiler::{derive_scalar_declarations_from_cst, parse_source, ScalarSourceDeclarations};
use wosy_project::{
    ContentRevision, PackageRelativePath, SourceIdentity, SourceNodeInput, SourceObservation,
};
use wosy_stdlib_source::{
    CHECKED_ACCESS_CONTRACT, CHECKED_ACCESS_SOURCE, PACKAGE_VERSION,
};

const RELEASE_ORIGIN: &str = "wosy-release://stdlib";
const INTERNAL_PACKAGE: &str = "wosy_toolchain_stdlib";
const MODULE_PATH: &str = "src/checked_access.w";

pub(super) struct CheckedAccessComponent {
    observation: SourceObservation,
}

impl CheckedAccessComponent {
    // The next tranche will invoke this only for projects whose prepared
    // language operations request this capability. Do not infer that request
    // from a user-supplied module, namespace, or function spelling.
    pub(super) fn linked() -> Self {
        let mut hash = blake3::Hasher::new();
        for field in [CHECKED_ACCESS_CONTRACT, PACKAGE_VERSION, CHECKED_ACCESS_SOURCE] {
            hash.update(&(field.len() as u64).to_le_bytes());
            hash.update(field.as_bytes());
        }
        Self {
            observation: SourceObservation {
                source: SourceIdentity::new(
                    RELEASE_ORIGIN.to_owned(),
                    INTERNAL_PACKAGE.to_owned(),
                    PackageRelativePath::new(MODULE_PATH.into()).expect("fixed relative module"),
                ),
                content_revision: ContentRevision(hash.finalize().to_hex().to_string()),
            },
        }
    }

    pub(super) fn into_preparation(
        self,
    ) -> Result<(SourceNodeInput, ScalarSourceDeclarations, String), String> {
        let SourceObservation { source, content_revision } = self.observation;
        let compiler_identity = super::compiler_source_identity(&source, &content_revision);
        let parsed = parse_source(compiler_identity, CHECKED_ACCESS_SOURCE.to_owned(), &[]);
        if !parsed.diagnostics.is_empty() {
            return Err("toolchain checked-access source has syntax diagnostics".to_owned());
        }
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        if !declarations.diagnostics().is_empty() || declarations.namespaces().next().is_some() {
            return Err("toolchain checked-access source is not an independent module".to_owned());
        }
        Ok((
            SourceNodeInput { source, content_revision, namespace_edges: Vec::new() },
            declarations,
            parsed.result.text,
        ))
    }
}

pub(super) fn refresh_observation(source: &SourceIdentity) -> Result<Option<SourceObservation>, String> {
    if source.project != RELEASE_ORIGIN {
        return Ok(None);
    }
    let current = CheckedAccessComponent::linked().observation;
    if &current.source != source {
        return Err("unknown toolchain source observation".to_owned());
    }
    Ok(Some(current))
}
