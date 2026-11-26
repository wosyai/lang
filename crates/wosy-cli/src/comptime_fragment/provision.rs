use super::{ClosedComptimeInvocation, ClosedComptimePaths, ClosedComptimeStaging, StagedEvaluatorIr};
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
        let scratch = ComptimeScratch::create(private_parent)?;
        let ir = {
            let mut file = scratch.create_ir()?;
            self.write_evaluator_ir(&mut file, max_ir_bytes)?
        };
        Ok(ProvisionedClosedComptime {
            ir_path: scratch.ir_path(),
            module_path: scratch.module_path(),
            manifest_path: scratch.manifest_path(),
            scratch,
            staging: self,
            ir,
        })
    }
}

impl ProvisionedClosedComptime {
    pub fn evaluator_ir(&self) -> &StagedEvaluatorIr { &self.ir }
    pub fn staging(&self) -> &ClosedComptimeStaging { &self.staging }
    pub fn expected_module_path(&self) -> &Path { &self.module_path }
    pub fn manifest_path(&self) -> &Path { &self.manifest_path }

    /// Literal bookkeeping for a future approved process boundary; neither
    /// these pathnames nor the selected command are a launch permission.
    pub fn builder_invocation(&self) -> ClosedComptimeInvocation<'_> {
        self.staging.builder(ClosedComptimePaths {
            evaluator_ir: &self.ir_path,
            output_dir: self.scratch.output_dir(),
            manifest: &self.manifest_path,
        })
    }

    pub fn runner_invocation(&self) -> ClosedComptimeInvocation<'_> {
        self.staging.runner(&self.manifest_path)
    }

    /// Bind bounded module bytes in the predeclared slot to this exact
    /// provision, not to a path, type or digest claimed by a producer.
    pub fn observe_module(&self, max_bytes: u64) -> io::Result<ClosedComptimeModuleObservation<'_>> {
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
        Ok(ClosedComptimeModuleObservation { provision: self, bytes, digest: hash.finalize() })
    }
}

/// Evidence for a future typed readback on the same selected project,
/// profile, authored site, effective reads and staged IR. Not evidence that
/// these bytes were built, loaded or executed by an authorized process.
pub struct ClosedComptimeModuleObservation<'a> {
    provision: &'a ProvisionedClosedComptime,
    bytes: u64,
    digest: blake3::Hash,
}

impl ClosedComptimeModuleObservation<'_> {
    pub fn provision(&self) -> &ProvisionedClosedComptime { self.provision }
    pub fn result_type(&self) -> &ScalarType { self.provision.staging.fragment().result_type() }
    pub fn bytes(&self) -> u64 { self.bytes }
    pub fn digest(&self) -> blake3::Hash { self.digest }
}
