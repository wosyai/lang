use super::GitDirectory;
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const IR: &str = "evaluator.ll";
const MODULE: &str = "evaluator.module";
const MANIFEST: &str = "comptime-manifest";

/// A fresh output directory within an explicitly supplied, owner-private
/// scratch parent. Held handles, not public path strings, govern its I/O.
/// This neither executes a tool nor keeps the name valid across other
/// same-owner writers; the caller owns that coordination.
pub struct ComptimeScratch {
    parent: GitDirectory,
    name: PathBuf,
    dir: GitDirectory,
    path: PathBuf,
    descriptor: PathBuf,
}

impl ComptimeScratch {
    pub fn create(parent_path: &Path) -> io::Result<Self> {
        let parent_path = parent_path.canonicalize()?;
        let file = GitDirectory::open_absolute(&parent_path)?;
        require_private(&file.metadata()?)?;
        let parent = GitDirectory::new(file).map_err(io::Error::other)?;
        let name = PathBuf::from(format!("wosy-evaluator-{}", uuid::Uuid::now_v7()));
        parent.mkdir_private(&name)?;
        let opened = (|| {
            let dir = parent.directory(&name).map_err(io::Error::other)?
                .ok_or_else(|| io::Error::other("new evaluator scratch directory is missing"))?;
            let descriptor = dir.descriptor_path()?;
            Ok((dir, descriptor))
        })();
        let (dir, descriptor) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                let _ = parent.remove_empty_directory(&name);
                return Err(error);
            }
        };
        // Arm rollback before checking the effective mode (umask may have
        // restricted it). No files are created until this verification.
        let scratch = Self { path: parent_path.join(&name), parent, name, dir, descriptor };
        require_private(&fs::metadata(&scratch.descriptor)?)?;
        Ok(scratch)
    }

    pub fn output_dir(&self) -> &Path { &self.path }
    pub fn ir_path(&self) -> PathBuf { self.path.join(IR) }
    pub fn module_path(&self) -> PathBuf { self.path.join(MODULE) }
    pub fn manifest_path(&self) -> PathBuf { self.path.join(MANIFEST) }

    pub fn create_ir(&self) -> io::Result<File> {
        OpenOptions::new().write(true).create_new(true).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW).open(self.descriptor.join(IR))
    }

    pub fn open_module(&self) -> io::Result<(File, fs::Metadata)> {
        let module = self.dir.open(Path::new(MODULE)).map_err(io::Error::other)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "expected evaluator module is absent"))?;
        let metadata = module.metadata()?;
        if !metadata.is_file() || metadata.uid() != owner() || metadata.nlink() != 1 {
            return Err(io::Error::other("expected an evaluator module owned only by this scratch directory"));
        }
        Ok((module, metadata))
    }
}

impl Drop for ComptimeScratch {
    fn drop(&mut self) {
        // The reserved names live in the held fresh directory. Do not
        // traverse or remove anything else (nor a replacement directory).
        for name in [IR, MODULE, MANIFEST] {
            let _ = fs::remove_file(self.descriptor.join(name));
        }
        if self.parent.directory(&self.name).ok().flatten().as_ref() == Some(&self.dir) {
            let _ = self.parent.remove_empty_directory(&self.name);
        }
    }
}

fn owner() -> u32 {
    // SAFETY: effective UID needs no arguments or borrowed resources.
    unsafe { libc::geteuid() }
}

fn require_private(metadata: &fs::Metadata) -> io::Result<()> {
    if metadata.is_dir() && metadata.uid() == owner() && metadata.mode() & 0o777 == 0o700 {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "evaluator scratch requires an owner-private 0700 directory"))
    }
}
