use super::{selected_builder_invocation, selected_runner_invocation, ClosedComptimeInvocation,
    ClosedComptimePaths, ClosedComptimeStaging, StagedEvaluatorIr};
use crate::source_preparation::PreparedWasip1ComptimeInputs;
use std::{
    io::{self, Read},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
use wosy_compiler::ScalarType;
use wosy_project::packages::ComptimeScratch;

const MAX_MODULE_BYTES: u64 = 16 << 20;

/// One real closed compiler staging observation and the filesystem area it
/// provisioned. This opt-in object never launches configured commands, writes
/// a manifest, or accepts a value into the compiler.
pub struct ProvisionedClosedComptime {
    staging: ClosedComptimeStaging,
    files: EvaluatorFiles,
}

/// The explicitly selected preview1 evaluator and its original authored
/// preparation, now bound to one owned scratch directory. The command
/// descriptions are inert; no manifest, executable identity or launch.
pub struct ProvisionedWasip1Comptime {
    preparation: PreparedWasip1ComptimeInputs,
    files: EvaluatorFiles,
}

pub(super) struct EvaluatorFiles {
    scratch: ComptimeScratch,
    ir: StagedEvaluatorIr,
    ir_path: PathBuf,
    module_path: PathBuf,
    manifest_path: PathBuf,
}

impl ClosedComptimeStaging {
    pub fn provision(
        self, private_parent: &Path, max_ir_bytes: usize,
    ) -> io::Result<ProvisionedClosedComptime> {
        let files = EvaluatorFiles::create(private_parent, |file| self.write_evaluator_ir(file, max_ir_bytes))?;
        Ok(ProvisionedClosedComptime { staging: self, files })
    }
}

impl PreparedWasip1ComptimeInputs {
    pub fn provision(
        self, private_parent: &Path, max_ir_bytes: usize,
    ) -> io::Result<ProvisionedWasip1Comptime> {
        let files = EvaluatorFiles::create(private_parent, |file| self.write_evaluator_ir(file, max_ir_bytes))?;
        Ok(ProvisionedWasip1Comptime { preparation: self, files })
    }
}

impl ProvisionedClosedComptime {
    pub fn evaluator_ir(&self) -> &StagedEvaluatorIr { &self.files.ir }
    pub fn staging(&self) -> &ClosedComptimeStaging { &self.staging }
    pub fn expected_module_path(&self) -> &Path { &self.files.module_path }
    pub fn manifest_path(&self) -> &Path { &self.files.manifest_path }

    /// Literal bookkeeping for a future approved process boundary; neither
    /// these pathnames nor the selected command are a launch permission.
    pub fn builder_invocation(&self) -> ClosedComptimeInvocation<'_> {
        self.staging.builder(self.files.builder_paths())
    }

    pub fn runner_invocation(&self) -> ClosedComptimeInvocation<'_> {
        self.staging.runner(self.manifest_path())
    }

    pub fn observe_module(&self, max_bytes: u64) -> io::Result<ClosedComptimeModuleObservation<'_>> {
        Ok(EvaluatorModuleObservation { provision: self, module: self.files.observe_module(max_bytes)? })
    }
}

impl ProvisionedWasip1Comptime {
    pub fn preparation(&self) -> &PreparedWasip1ComptimeInputs { &self.preparation }
    pub fn evaluator_ir(&self) -> &StagedEvaluatorIr { &self.files.ir }
    pub fn expected_module_path(&self) -> &Path { &self.files.module_path }
    pub fn manifest_path(&self) -> &Path { &self.files.manifest_path }

    pub fn builder_invocation(&self) -> ClosedComptimeInvocation<'_> {
        selected_builder_invocation(self.preparation.project(), self.preparation.declared_tools(), self.files.builder_paths())
    }

    pub fn runner_invocation(&self) -> ClosedComptimeInvocation<'_> {
        selected_runner_invocation(self.preparation.project(), self.preparation.declared_tools(), self.manifest_path())
    }

    pub fn observe_module(&self, max_bytes: u64) -> io::Result<Wasip1ComptimeModuleObservation<'_>> {
        Ok(EvaluatorModuleObservation { provision: self, module: self.files.observe_module(max_bytes)? })
    }
}

impl ProvisionedWasip1Comptime {
    // The held directory already owns cleanup of the reserved manifest slot.
    pub(super) fn create_private_manifest(&self) -> io::Result<std::fs::File> {
        self.files.create_manifest()
    }
}

impl EvaluatorFiles {
    pub(super) fn create(private_parent: &Path, write_ir: impl FnOnce(&mut std::fs::File) -> io::Result<StagedEvaluatorIr>)
        -> io::Result<Self>
    {
        let scratch = ComptimeScratch::create(private_parent)?;
        let ir = write_ir(&mut scratch.create_ir()?)?;
        Ok(Self {
            ir_path: scratch.ir_path(),
            module_path: scratch.module_path(),
            manifest_path: scratch.manifest_path(),
            scratch, ir,
        })
    }

    pub(super) fn ir(&self) -> &StagedEvaluatorIr { &self.ir }
    pub(super) fn module_path(&self) -> &Path { &self.module_path }
    pub(super) fn manifest_path(&self) -> &Path { &self.manifest_path }

    pub(super) fn create_manifest(&self) -> io::Result<std::fs::File> {
        self.scratch.create_manifest()
    }

    pub(super) fn builder_paths(&self) -> ClosedComptimePaths<'_> {
        ClosedComptimePaths { evaluator_ir: &self.ir_path, output_dir: self.scratch.output_dir(), manifest: &self.manifest_path }
    }

    pub(super) fn verify_inputs(&self, manifest: &[u8]) -> io::Result<()> {
        let ir = self.scratch.open_ir()?;
        let mut bytes = Vec::new();
        ir.take(self.ir.bytes as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() != self.ir.bytes || blake3::hash(&bytes) != self.ir.blake3 {
            return Err(io::Error::other("staged evaluator LLVM changed"));
        }
        bytes.clear();
        self.scratch.open_manifest()?.take(manifest.len() as u64 + 1).read_to_end(&mut bytes)?;
        if bytes != manifest { return Err(io::Error::other("staged evaluator manifest changed")); }
        Ok(())
    }

    /// Bind bounded module bytes in the predeclared slot to this exact
    /// provision, not to a path, type or digest claimed by a producer.
    pub(super) fn observe_module(&self, max_bytes: u64) -> io::Result<ObservedModule> {
        let (mut module, metadata) = self.scratch.open_module()?;
        let limit = max_bytes.min(MAX_MODULE_BYTES);
        let expected = metadata.len();
        if expected == 0 || expected > limit {
            return Err(io::Error::other("evaluator module is empty or exceeds its observation budget"));
        }
        // Cap reads independently of the initial file size. Use one extra
        // byte to refuse a file that grows while being read.
        let mut bytes = 0;
        let mut buffer = [0; 8192];
        let mut hash = blake3::Hasher::new();
        let mut input = (&mut module).take(expected + 1);
        loop {
            let n = input.read(&mut buffer)?;
            if n == 0 { break; }
            bytes += n as u64;
            hash.update(&buffer[..n]);
        }
        let last = module.metadata()?;
        let version = |meta: &std::fs::Metadata| {
            (meta.len(), meta.mtime(), meta.mtime_nsec(), meta.ctime(), meta.ctime_nsec(), meta.nlink())
        };
        if bytes != expected || version(&metadata) != version(&last) {
            return Err(io::Error::other("evaluator module changed during observation"));
        }
        Ok(ObservedModule { bytes, digest: hash.finalize() })
    }
}

pub(super) struct ObservedModule {
    bytes: u64,
    pub(super) digest: blake3::Hash,
}

/// Module bytes read from one exact owned output slot. This is file
/// evidence, not a statement about the producer, what a runner loaded, or
/// an accepted typed result.
pub struct EvaluatorModuleObservation<'a, P> {
    provision: &'a P,
    module: ObservedModule,
}

pub type ClosedComptimeModuleObservation<'a> = EvaluatorModuleObservation<'a, ProvisionedClosedComptime>;
pub type Wasip1ComptimeModuleObservation<'a> = EvaluatorModuleObservation<'a, ProvisionedWasip1Comptime>;

impl<P> EvaluatorModuleObservation<'_, P> {
    pub fn provision(&self) -> &P { self.provision }
    pub fn bytes(&self) -> u64 { self.module.bytes }
    pub fn digest(&self) -> blake3::Hash { self.module.digest }
}

impl ClosedComptimeModuleObservation<'_> {
    pub fn result_type(&self) -> &ScalarType { self.provision.staging.fragment().result_type() }
}

impl Wasip1ComptimeModuleObservation<'_> {
    pub fn result_type(&self) -> &ScalarType {
        self.provision.preparation.continuation().evaluator().output().result_type()
    }
}
