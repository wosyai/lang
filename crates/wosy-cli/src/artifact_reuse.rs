//! Identity of a local builder result, independent of the runner configuration.
//! A key identifies inputs; it does not authenticate a daemon or a binary.

use serde::{Deserialize, Serialize};
use wosy_project::BuilderConfig;

/// Root identities from the selected Build projection. These must be taken
/// from the same acknowledged snapshot and artifact variant.
#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRoots {
    pub package_source: String,
    pub patch_stacks: String,
    pub dependencies: String,
    pub partitioned_ir: String,
    pub comptime_results: Option<String>,
}

/// The configured builder is the same command the client actually invokes.
#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactCodegen {
    pub target: String,
    pub artifact: String,
    pub profile: String,
    pub backend: String,
    pub output: String,
    pub compiler: String,
    pub compiler_image: String,
    pub llvm: String,
    pub builder_name: String,
    pub builder: BuilderConfig,
    pub observed_builder_files: String,
    pub runtime_ir: String,
    pub comptime_parameters: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactKey(blake3::Hash);

impl ArtifactKey {
    pub fn digest(&self) -> &blake3::Hash { &self.0 }
    pub fn from_selected_inputs(
        roots: &ArtifactRoots,
        codegen: &ArtifactCodegen,
    ) -> Result<Self, serde_json::Error> {
        // All fields have a fixed serialization order and no unordered maps.
        let bytes = serde_json::to_vec(&(roots, codegen))?;
        let mut hash = blake3::Hasher::new_derive_key("Wosy local artifact key v1");
        hash.update(&bytes);
        Ok(Self(hash.finalize()))
    }
}
