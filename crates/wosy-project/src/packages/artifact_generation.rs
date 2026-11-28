//! Generation storage for an explicitly selected controlled artifact.
//!
//! The caller must own the parent namespace, handle source/configuration and
//! driver custody, and exclude non-participating writers. Ordinary artifacts
//! do not use this store. Creation grants no permission to launch a process.
use super::GitDirectory;
use crate::ArtifactManifest;
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const MANIFEST: &str = "artifact-manifest.json";
const OUTPUT: &str = "main";
const STAGE_FILES: [&str; 5] = [
    "partition.ll",
    "runtime.ll",
    "artifact-manifest.pending.json",
    OUTPUT,
    MANIFEST,
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
    pending_pointer: Option<PathBuf>,
}

/// A materialized manifest and pointer candidate, not yet visible as current.
#[doc(hidden)]
pub struct PreparedArtifactGeneration(StagedArtifactGeneration);

impl StagedArtifactGeneration {
    /// Open a preselected, owner-private directory exclusive to this service.
    /// Hold its cooperative writer reservation for this generation.
    pub fn create(parent_path: &Path) -> io::Result<Self> {
        if !parent_path.is_absolute() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "artifact generation parent must be absolute"));
        }
        let writer = GitDirectory::open_absolute(parent_path)?;
        require_private(&writer.metadata()?)?;
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
            parent_path: parent_path.to_owned(), name, published: false, pending_pointer: None,
        };
        require_private(&fs::metadata(&stage.descriptor)?)?;
        Ok(stage)
    }

    pub fn output_dir(&self) -> PathBuf {
        self.parent_path.join(&self.name)
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
    use std::{io::Read, os::unix::fs::DirBuilderExt};

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
    fn atomic_pointer_keeps_old_generation_and_discards_unpublished_candidate() {
        let scratch = Scratch::new();
        let pointer = scratch.0.join(MANIFEST);
        let one = StagedArtifactGeneration::create(&scratch.0).unwrap();
        let first = manifest(&one, "first");
        fs::write(&first.executable, b"first fixture output").unwrap();
        let prepared = one.prepare(&first).unwrap();
        assert!(!pointer.exists());
        let old_path = prepared.publish().unwrap();
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
        let new_path = prepared.publish().unwrap();
        assert_ne!(old_path, new_path);
        assert_eq!(load_artifact_manifest(&pointer).unwrap(), next);
        assert_eq!(load_artifact_manifest(&new_path).unwrap(), next);
        assert_eq!(load_artifact_manifest(&old_path).unwrap(), first);
        let mut old_output = Vec::new();
        old_reader.read_to_end(&mut old_output).unwrap();
        assert_eq!(old_output, b"first fixture output");
        assert_eq!(fs::read(&first.executable).unwrap(), old_output);
    }
}
