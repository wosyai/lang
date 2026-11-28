//! Generation storage for an explicitly selected controlled artifact.
//!
//! The caller must own the parent namespace, handle source/configuration and
//! driver custody, and exclude non-participating writers. Ordinary artifacts
//! do not use this store. Creation grants no permission to launch a process.
use super::GitDirectory;
use crate::ArtifactManifest;
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const MANIFEST: &str = "artifact-manifest.json";
const OUTPUT: &str = "main";
const CACHE_RECEIPT: &str = "artifact-cache-v1";
const COMPILER_PARTITION: &str = "compiler-partition-v1";
const STAGE_FILES: [&str; 7] = [
    "partition.ll",
    "runtime.ll",
    "artifact-manifest.pending.json",
    OUTPUT,
    MANIFEST,
    CACHE_RECEIPT,
    COMPILER_PARTITION,
];

#[doc(hidden)]
pub struct StagedArtifactGeneration {
    parent: GitDirectory,
    _writer: File,
    dir: GitDirectory,
    descriptor: PathBuf,
    parent_descriptor: PathBuf,
    parent_path: PathBuf,
    name: PathBuf,
    published: bool,
    recorded_cache_receipt: bool,
    recorded_compiler_partition: bool,
    pending_pointer: Option<PathBuf>,
}

/// A materialized manifest and pointer candidate, not yet visible as current.
#[doc(hidden)]
pub struct PreparedArtifactGeneration(StagedArtifactGeneration);

/// A reader's selected generation. Committed generations are retained by
/// this storage protocol; this handle is not a GC lease or a tool authority.
#[doc(hidden)]
pub struct SelectedArtifactGeneration {
    _directory: GitDirectory,
    manifest_path: PathBuf,
    manifest: ArtifactManifest,
}

impl SelectedArtifactGeneration {
    /// Resolve the controlled store's current pointer once. Ignore any next
    /// pointer update, then pass this generation's path to a controlled runner.
    /// The caller still validates source, configuration and runner identity.
    pub fn open(parent_path: &Path) -> io::Result<Option<Self>> {
        let file = open_parent(parent_path)?;
        let parent = GitDirectory::new(file).map_err(io::Error::other)?;
        let Some(pointer) = parent.source(Path::new(MANIFEST)).map_err(io::Error::other)? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        read_manifest_file(pointer, &mut bytes, ManifestLink::Current)?;
        let manifest: ArtifactManifest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        let relative = manifest.executable.strip_prefix(parent_path).map_err(io::Error::other)?;
        let mut components = relative.components();
        let name = match (components.next(), components.next(), components.next()) {
            (Some(std::path::Component::Normal(name)), Some(std::path::Component::Normal(binary)), None)
                if binary == OUTPUT => name,
            _ => return Err(io::Error::other("current artifact pointer does not identify a generation")),
        };
        let id = name.to_str().and_then(|name| name.strip_prefix("build-"))
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .filter(|id| id.get_version_num() == 7)
            .ok_or_else(|| io::Error::other("current artifact pointer has an invalid generation name"))?;
        if name != std::ffi::OsStr::new(&format!("build-{id}")) {
            return Err(io::Error::other("current artifact pointer has a noncanonical generation name"));
        }
        let directory = parent.directory(Path::new(name)).map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("current artifact generation is missing"))?;
        let Some(generation) = directory.source(Path::new(MANIFEST)).map_err(io::Error::other)? else {
            return Err(io::Error::other("current artifact generation manifest is missing"));
        };
        let mut generation_bytes = Vec::new();
        read_manifest_file(generation, &mut generation_bytes, ManifestLink::Retained)?;
        if bytes != generation_bytes {
            return Err(io::Error::other("current artifact pointer differs from its generation manifest"));
        }
        Ok(Some(Self {
            _directory: directory,
            manifest_path: parent_path.join(name).join(MANIFEST),
            manifest,
        }))
    }

    /// Reuse compiler-owned LLVM only if its exact selection key and bytes
    /// match. This method never admits the generated executable to reuse.
    pub fn retained_compiler_partition(&self, key: &blake3::Hash) -> io::Result<Option<Vec<u8>>> {
        let Some(mut key_file) = self._directory.source(Path::new(COMPILER_PARTITION)).map_err(io::Error::other)? else {
            return Ok(None);
        };
        let meta = key_file.metadata()?;
        if meta.uid() != owner() || meta.nlink() != 1 || meta.len() != 32 { return Ok(None); }
        let mut observed_key = [0; 32];
        key_file.read_exact(&mut observed_key)?;
        if &observed_key != key.as_bytes() { return Ok(None); }
        let Some(mut partition) = self._directory.source(Path::new("partition.ll")).map_err(io::Error::other)? else {
            return Ok(None);
        };
        let meta = partition.metadata()?;
        if meta.uid() != owner() || meta.nlink() != 1 { return Ok(None); }
        let mut bytes = Vec::new();
        partition.read_to_end(&mut bytes)?;
        if blake3::hash(&bytes).to_hex().as_str() != self.manifest.identity.llvm_input_identity {
            return Ok(None);
        }
        Ok(Some(bytes))
    }

    /// A stored receipt is evidence only when both the input key and the
    /// exact local executable bytes still match. No receipt means no reuse.
    pub fn matches_cache_receipt(&self, key: &blake3::Hash) -> io::Result<bool> {
        let Some(mut receipt) = self._directory.source(Path::new(CACHE_RECEIPT)).map_err(io::Error::other)? else {
            return Ok(false);
        };
        let metadata = receipt.metadata()?;
        if metadata.uid() != owner() || metadata.nlink() != 1 || metadata.len() != 64 {
            return Ok(false);
        }
        let mut bytes = [0; 64];
        receipt.read_exact(&mut bytes)?;
        if &bytes[..32] != key.as_bytes() { return Ok(false); }
        let Some(binary) = self._directory.source(Path::new(OUTPUT)).map_err(io::Error::other)? else {
            return Ok(false);
        };
        let metadata = binary.metadata()?;
        if metadata.uid() != owner() || metadata.nlink() != 1 { return Ok(false); }
        let digest = blake3::Hasher::new().update_reader(binary)?.finalize();
        Ok(&bytes[32..] == digest.as_bytes())
    }

    pub fn manifest_path(&self) -> &Path { &self.manifest_path }

    pub fn manifest(&self) -> &ArtifactManifest { &self.manifest }
}

impl StagedArtifactGeneration {
    /// Open a preselected, owner-private directory exclusive to this service.
    /// Hold its cooperative writer reservation for this generation.
    pub fn create(parent_path: &Path) -> io::Result<Self> {
        let writer = open_parent(parent_path)?;
        writer.lock()?;
        let parent = GitDirectory::new(writer.try_clone()?).map_err(io::Error::other)?;
        let parent_descriptor = parent.descriptor_path()?;
        let name = PathBuf::from(format!("build-{}", uuid::Uuid::now_v7()));
        parent.mkdir_private(&name)?;
        let opened = (|| {
            let dir = parent.directory(&name).map_err(io::Error::other)?
                .ok_or_else(|| io::Error::other("new artifact generation is missing"))?;
            let descriptor = dir.descriptor_path()?;
            Ok::<_, io::Error>((dir, descriptor))
        })();
        let (dir, descriptor) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                let _ = parent.remove_empty_directory(&name);
                return Err(error);
            }
        };
        let stage = Self {
            parent, _writer: writer, dir, descriptor, parent_descriptor,
            parent_path: parent_path.to_owned(), name, published: false, recorded_cache_receipt: false, recorded_compiler_partition: false, pending_pointer: None,
        };
        require_private(&fs::metadata(&stage.descriptor)?)?;
        Ok(stage)
    }

    pub fn output_dir(&self) -> PathBuf {
        self.parent_path.join(&self.name)
    }

    /// The controlled store must be a separate private directory within
    /// the selected project, not an alias of the project owner lock.
    pub fn is_workspace_subdirectory(&self, project: &Path) -> io::Result<bool> {
        let project_dir = GitDirectory::new(GitDirectory::open_absolute(project)?)
            .map_err(io::Error::other)?;
        let resolved_parent = self.parent_path.canonicalize()?;
        let current_parent = GitDirectory::new(GitDirectory::open_absolute(&self.parent_path)?)
            .map_err(io::Error::other)?;
        Ok(resolved_parent != project
            && resolved_parent.starts_with(project)
            && self.parent != project_dir
            && self.parent == current_parent)
    }

    /// Materialize the existing Builder ABI's two LLVM inputs in this held
    /// directory. The ordinary invocation still owns the pending manifest.
    /// This neither selects nor runs a driver.
    pub fn materialize_llvm_inputs(&self, partition: &[u8], runtime: &[u8]) -> io::Result<()> {
        write_new(&self.descriptor.join("partition.ll"), partition)?;
        write_new(&self.descriptor.join("runtime.ll"), runtime)
    }

    /// Record a compiler-owned result key, only after the external builder
    /// has returned. The caller verifies the source cut and the input bytes.
    pub fn record_compiler_partition(
        &mut self, key: &blake3::Hash, expected: &str,
    ) -> io::Result<bool> {
        let Some(file) = self.dir.source(Path::new("partition.ll")).map_err(io::Error::other)? else {
            return Ok(false);
        };
        let meta = file.metadata()?;
        if meta.uid() != owner() || meta.nlink() != 1
            || blake3::Hasher::new().update_reader(file)?.finalize().to_hex().as_str() != expected
        {
            return Ok(false);
        }
        write_new(&self.descriptor.join(COMPILER_PARTITION), key.as_bytes())?;
        self.recorded_compiler_partition = true;
        Ok(true)
    }

    /// Bind the already returned local builder output to the selected input
    /// key. Only the builder's caller can establish that input provenance.
    pub fn record_cache_receipt(&mut self, key: &blake3::Hash) -> io::Result<()> {
        let binary = self.dir.source(Path::new(OUTPUT)).map_err(io::Error::other)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "artifact generation has no executable"))?;
        let metadata = binary.metadata()?;
        if metadata.uid() != owner() || metadata.nlink() != 1 {
            return Err(io::Error::other("expected an artifact owned only by this generation"));
        }
        let digest = blake3::Hasher::new().update_reader(binary)?.finalize();
        let mut bytes = [0; 64];
        bytes[..32].copy_from_slice(key.as_bytes());
        bytes[32..].copy_from_slice(digest.as_bytes());
        write_new(&self.descriptor.join(CACHE_RECEIPT), &bytes)?;
        self.recorded_cache_receipt = true;
        Ok(())
    }

    /// Prepare pointer bytes after the controlled driver relinquishes output.
    /// Validate file ownership, not executable contents or driver authority.
    pub fn prepare(mut self, manifest: &ArtifactManifest) -> io::Result<PreparedArtifactGeneration> {
        if manifest.executable != self.output_dir().join(OUTPUT) {
            return Err(io::Error::other("artifact executable path does not match its reservation"));
        }
        let binary = self.dir.source(Path::new(OUTPUT)).map_err(io::Error::other)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "artifact generation has no executable"))?;
        let meta = binary.metadata()?;
        if meta.uid() != owner() || meta.nlink() != 1 {
            return Err(io::Error::other("expected an artifact owned only by this generation"));
        }

        if !self.recorded_cache_receipt {
            match fs::remove_file(self.descriptor.join(CACHE_RECEIPT)) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        if !self.recorded_compiler_partition {
            match fs::remove_file(self.descriptor.join(COMPILER_PARTITION)) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        let bytes = serde_json::to_vec_pretty(manifest).map_err(io::Error::other)?;
        write_new(&self.descriptor.join(MANIFEST), &bytes)?;
        let pointer = self.parent_descriptor.join(self.name.with_extension("manifest"));
        let mut pending = OpenOptions::new().write(true).create_new(true).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW).open(&pointer)?;
        self.pending_pointer = Some(pointer);
        pending.write_all(&bytes)?;
        Ok(PreparedArtifactGeneration(self))
    }
}

impl PreparedArtifactGeneration {
    /// The caller holds its final selected source/configuration reservation
    /// around this short cut. Return the specific generation manifest, not
    /// the mutable current path. Atomic replacement, not crash durability.
    pub fn publish(self) -> io::Result<PathBuf> {
        let mut stage = self.0;
        let current_parent = GitDirectory::new(GitDirectory::open_absolute(&stage.parent_path)?)
            .map_err(io::Error::other)?;
        if current_parent != stage.parent
            || stage.parent.directory(&stage.name).map_err(io::Error::other)?.as_ref() != Some(&stage.dir)
        {
            return Err(io::Error::other("artifact generation path does not match its reservation"));
        }
        let generation_path = stage.output_dir().join(MANIFEST);
        let pointer = stage.pending_pointer.as_ref().expect("prepared generation pointer");
        fs::rename(pointer, stage.parent_descriptor.join(MANIFEST))?;
        // No operations that can fail after visibility.
        stage.published = true;
        Ok(generation_path)
    }
}

impl Drop for StagedArtifactGeneration {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        if let Some(pointer) = &self.pending_pointer {
            let _ = fs::remove_file(pointer);
        }
        // Only the controlled contract's reserved files, not arbitrary
        // scratch contents or a replacement directory.
        for name in STAGE_FILES {
            let _ = fs::remove_file(self.descriptor.join(name));
        }
        if self.parent.directory(&self.name).ok().flatten().as_ref() == Some(&self.dir) {
            let _ = self.parent.remove_empty_directory(&self.name);
        }
    }
}

fn open_parent(path: &Path) -> io::Result<File> {
    if !path.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "artifact generation parent must be absolute"));
    }
    let file = GitDirectory::open_absolute(path)?;
    require_private(&file.metadata()?)?;
    Ok(file)
}

enum ManifestLink { Current, Retained }

fn read_manifest_file(mut file: File, bytes: &mut Vec<u8>, link: ManifestLink) -> io::Result<()> {
    let meta = file.metadata()?;
    let valid_links = match link {
        // A concurrent committed selection can unlink the old current
        // pointer while its open descriptor still supplies the old bytes.
        ManifestLink::Current => meta.nlink() <= 1,
        ManifestLink::Retained => meta.nlink() == 1,
    };
    if meta.uid() != owner() || !valid_links {
        return Err(io::Error::other("expected a manifest owned only by this artifact store"));
    }
    file.read_to_end(bytes)?;
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    OpenOptions::new().write(true).create_new(true).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW).open(path)?.write_all(bytes)
}

fn owner() -> u32 {
    // SAFETY: the effective UID syscall takes no argument.
    unsafe { libc::geteuid() }
}

fn require_private(meta: &Metadata) -> io::Result<()> {
    if meta.is_dir() && meta.uid() == owner() && meta.mode() & 0o777 == 0o700 {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "artifact generation requires an owner-private 0700 directory"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{load_artifact_manifest, ArtifactIdentity};
    use std::os::unix::fs::DirBuilderExt;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("wosy-generation-{}", uuid::Uuid::now_v7()));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn manifest(stage: &StagedArtifactGeneration, revision: &str) -> ArtifactManifest {
        ArtifactManifest {
            identity: ArtifactIdentity {
                source_identity: revision.to_owned(),
                configuration_identity: "fixture".into(),
                artifact_identity: "fixture".into(),
                backend: "llvm".into(),
                output: "wasm-wasip1".into(),
                builder_identity: "fixture".into(),
                runner_identity: "fixture".into(),
                llvm_input_identity: revision.to_owned(),
                compiler_identity: "fixture".into(),
                llvm_identity: "fixture".into(),
                builder_toolchain_identity: "fixture".into(),
            },
            executable: stage.output_dir().join(OUTPUT),
            source_observations: Vec::new(),
        }
    }

    #[test]
    fn materializes_exact_llvm_inputs_without_pending_manifest_or_publication() {
        let scratch = Scratch::new();
        let stage = StagedArtifactGeneration::create(&scratch.0).unwrap();
        let output = stage.output_dir();
        assert!(!stage.is_workspace_subdirectory(&scratch.0).unwrap());
        stage.materialize_llvm_inputs(b"partition bytes", b"runtime bytes").unwrap();
        assert_eq!(fs::read(output.join("partition.ll")).unwrap(), b"partition bytes");
        assert_eq!(fs::read(output.join("runtime.ll")).unwrap(), b"runtime bytes");
        assert_eq!(stage.materialize_llvm_inputs(b"other", b"other").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(output.join("partition.ll")).unwrap(), b"partition bytes");
        assert!(!output.join("artifact-manifest.pending.json").exists());
        assert!(!scratch.0.join(MANIFEST).exists());
        drop(stage);
        assert!(!output.exists());
    }

    #[test]
    fn atomic_pointer_keeps_old_generation_and_discards_unpublished_candidate() {
        let scratch = Scratch::new();
        let pointer = scratch.0.join(MANIFEST);
        let one = StagedArtifactGeneration::create(&scratch.0).unwrap();
        let first = manifest(&one, "first");
        fs::write(&first.executable, b"first fixture output").unwrap();
        let prepared = one.prepare(&first).unwrap();
        assert!(!pointer.exists());
        assert!(SelectedArtifactGeneration::open(&scratch.0).unwrap().is_none());
        let old_path = prepared.publish().unwrap();
        let first_selection = SelectedArtifactGeneration::open(&scratch.0).unwrap().unwrap();
        assert_eq!(first_selection.manifest_path(), old_path);
        assert_eq!(first_selection.manifest(), &first);
        let old_bytes = fs::read(&pointer).unwrap();
        assert_eq!(load_artifact_manifest(&old_path).unwrap(), first);
        assert_eq!(fs::read(&old_path).unwrap(), old_bytes);
        let mut old_reader = File::open(&first.executable).unwrap();

        let discarded = StagedArtifactGeneration::create(&scratch.0).unwrap();
        let failed = manifest(&discarded, "failed");
        assert_eq!(discarded.prepare(&failed).err().unwrap().kind(), io::ErrorKind::NotFound);
        assert!(!failed.executable.parent().unwrap().exists());
        assert_eq!(fs::read(&pointer).unwrap(), old_bytes);

        let abandoned = StagedArtifactGeneration::create(&scratch.0).unwrap();
        let stale = manifest(&abandoned, "stale");
        fs::write(&stale.executable, b"unselected").unwrap();
        drop(abandoned.prepare(&stale).unwrap());
        assert!(!stale.executable.parent().unwrap().exists());
        assert_eq!(fs::read(&pointer).unwrap(), old_bytes);

        let two = StagedArtifactGeneration::create(&scratch.0).unwrap();
        let next = manifest(&two, "next");
        fs::write(&next.executable, b"second fixture output").unwrap();
        let prepared = two.prepare(&next).unwrap();
        assert_eq!(fs::read(&pointer).unwrap(), old_bytes);
        let opened_pointer = File::open(&pointer).unwrap();
        assert_eq!(opened_pointer.metadata().unwrap().nlink(), 1);
        let new_path = prepared.publish().unwrap();
        assert_eq!(opened_pointer.metadata().unwrap().nlink(), 0);
        let mut held_bytes = Vec::new();
        read_manifest_file(opened_pointer, &mut held_bytes, ManifestLink::Current).unwrap();
        assert_eq!(held_bytes, old_bytes);
        assert_ne!(old_path, new_path);
        assert_eq!(load_artifact_manifest(&pointer).unwrap(), next);
        let next_selection = SelectedArtifactGeneration::open(&scratch.0).unwrap().unwrap();
        assert_eq!(next_selection.manifest_path(), new_path);
        assert_eq!(next_selection.manifest(), &next);
        assert_eq!(first_selection.manifest_path(), old_path);
        assert_eq!(first_selection.manifest(), &first);
        assert_eq!(load_artifact_manifest(&old_path).unwrap(), first);
        let mut old_output = Vec::new();
        old_reader.read_to_end(&mut old_output).unwrap();
        assert_eq!(old_output, b"first fixture output");
        assert_eq!(fs::read(&first.executable).unwrap(), old_output);
    }
}
