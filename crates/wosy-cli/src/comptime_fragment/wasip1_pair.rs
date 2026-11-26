//! Private local manifest for the selected WASIp1 tool pair.
//! Staging never authenticates tools, invokes them, or accepts a result.
use super::{ClosedComptimeInvocation, ProvisionedWasip1Comptime};
use serde::Serialize;
use std::{collections::BTreeMap, io, path::Path};

#[derive(Serialize)]
struct ResolvedPrivateManifest<'a> {
    // Same path term as the ordinary local artifact manifest, not an
    // artifact identity and not an incremental wire payload.
    executable: &'a Path,
    parameters: BTreeMap<(), ()>,
    permissions: [(); 0],
    mounts: [(); 0],
}

pub struct StagedWasip1ToolPair {
    provision: ProvisionedWasip1Comptime,
}

impl ProvisionedWasip1Comptime {
    /// Materialize the one shared 3/1 manifest inside the existing owned
    /// scratch and retain the original prepared input. The P1 continuation
    /// has already admitted no build capabilities and no parameter calls.
    /// This first private handoff has no representation for authored
    /// parameter declarations. Other project paths remain unchanged.
    pub fn stage_tool_pair(self) -> io::Result<StagedWasip1ToolPair> {
        if self.preparation().has_declared_parameters() {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "the private WASIp1 pair needs empty resolved parameters"));
        }
        let manifest = ResolvedPrivateManifest {
            executable: self.expected_module_path(), parameters: BTreeMap::new(), permissions: [], mounts: [],
        };
        serde_json::to_writer(self.create_private_manifest()?, &manifest).map_err(io::Error::other)?;
        Ok(StagedWasip1ToolPair { provision: self })
    }
}

impl StagedWasip1ToolPair {
    pub fn provision(&self) -> &ProvisionedWasip1Comptime { &self.provision }
    pub fn builder(&self) -> ClosedComptimeInvocation<'_> { self.provision.builder_invocation() }
    pub fn runner(&self) -> ClosedComptimeInvocation<'_> { self.provision.runner_invocation() }
}
