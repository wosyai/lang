use super::{GitDirectory, GitProject, LockedPackage, PackageLock, LOCKFILE};
use super::graph::GitRequirement;
use std::cell::OnceCell;
use std::collections::BTreeSet;
use std::fs::File;
#[cfg(not(target_os = "linux"))]
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) const MANIFEST: &str = "wosy.toml";

/// One project metadata generation for cooperating Wosy commands. The
/// caller retains the guard through planning or publication.
pub(crate) struct ProjectSnapshot {
    root: PathBuf,
    directory_lock: Option<File>,
    lock: Result<Option<Vec<u8>>, String>,
    parsed: OnceCell<Result<Option<PackageLock>, String>>,
}

impl ProjectSnapshot {
    pub(crate) fn reader(root: &Path) -> Result<Self, String> {
        Self::open(root, false)
    }

    /// The local comptime observer already owns its exact canonical
    /// workspace path. Do not recanonicalize or reopen its directory by name.
    #[cfg(target_os = "linux")]
    pub(super) fn anchored_compiler_reader(root: &Path) -> Result<(Self, GitDirectory), String> {
        if !root.is_absolute() { return Err("selected project path must be absolute".into()); }
        let file = GitDirectory::open_absolute(root).map_err(|error| error.to_string())?;
        file.lock_shared().map_err(|error| error.to_string())?;
        let anchored = GitDirectory::new(file.try_clone().map_err(|error| error.to_string())?)?;
        Ok((Self {
            root: root.to_path_buf(), directory_lock: Some(file), lock: Ok(None), parsed: OnceCell::new(),
        }, anchored))
    }

    #[cfg(target_os = "linux")]
    pub(super) fn set_observed_lock(&mut self, lock: Option<Vec<u8>>) {
        self.lock = Ok(lock);
    }

    pub(super) fn writer(root: &Path) -> Result<Self, String> {
        Self::open(root, true)
    }

    pub(super) fn sync(root: &Path) -> Result<Self, String> {
        // Outside Unix only local, read-only behavior is inherited.
        Self::open(root, cfg!(unix))
    }

    fn open(root: &Path, writer: bool) -> Result<Self, String> {
        let root = root.canonicalize()
            .map_err(|error| format!("cannot resolve project root {}: {error}", root.display()))?;
        let directory_lock = platform::lock_file(&root, writer)
            .map_err(|error| format!("cannot lock project metadata: {error}"))?;
        let mut snapshot = Self {
            root,
            directory_lock,
            lock: Ok(None),
            parsed: OnceCell::new(),
        };
        // Capture the bytes under the guard, but do not treat unrelated legacy
        // files as Wosy pin data until the package graph makes them relevant.
        snapshot.lock = snapshot.read_bytes(LOCKFILE);
        Ok(snapshot)
    }

    pub(crate) fn manifest(&self) -> Result<String, String> {
        let bytes = self.read_bytes(MANIFEST)?.ok_or_else(|| format!("project has no {MANIFEST}"))?;
        String::from_utf8(bytes).map_err(|_| format!("{MANIFEST} is not UTF-8"))
    }

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    fn read_bytes(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        let mut file = match platform::existing(&self.root, self.directory_lock.as_ref(), name) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("cannot open {name}: {error}")),
        };
        if !file.metadata().map_err(|error| error.to_string())?.is_file() {
            return Err(format!("{name} must be a regular file"));
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|error| format!("cannot read {name}: {error}"))?;
        Ok(Some(bytes))
    }

    pub(super) fn lock_bytes(&self) -> Result<Option<&[u8]>, String> {
        self.lock.as_ref().map(|lock| lock.as_deref()).map_err(Clone::clone)
    }

    pub(super) fn parsed_lock(&self) -> Result<Option<&PackageLock>, String> {
        self.parsed.get_or_init(|| {
            self.lock_bytes()?.map(|bytes| {
                let text = std::str::from_utf8(bytes).map_err(|_| format!("invalid {LOCKFILE}: not UTF-8"))?;
                PackageLock::parse(text.to_owned()).map_err(|error| format!("invalid {LOCKFILE}: {error}"))
            }).transpose()
        }).as_ref().map(|lock| lock.as_ref()).map_err(Clone::clone)
    }

    pub(super) fn pin_for(&self, name: &str, requirement: &GitRequirement) -> Result<&LockedPackage, String> {
        let lock = self.parsed_lock()?
            .ok_or_else(|| format!("Git dependencies require {LOCKFILE}; dep sync does not select revisions"))?;
        lock.pin_for_requirement(name, requirement)
    }

    pub(super) fn check_reached(&self, reached: &BTreeSet<String>) -> Result<(), String> {
        match self.parsed_lock() {
            Ok(Some(lock)) => lock.check_reached(reached),
            Ok(None) => Ok(()),
            Err(_) if reached.is_empty() => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub(super) fn git_project(&self) -> Result<GitProject, String> {
        #[cfg(target_os = "linux")]
        {
            let file = self.directory_lock.as_ref().expect("Linux directory lock").try_clone()
                .map_err(|error| format!("cannot clone project directory handle: {error}"))?;
            Ok(GitProject { root: self.root.clone(), directory: GitDirectory::new(file)? })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err("Git dependency sync and loading are currently supported only on Linux".to_owned())
        }
    }

    pub(super) fn require_unchanged(&self, manifest: &str) -> Result<(), String> {
        let lock_changed = match &self.lock {
            Ok(before) => self.read_bytes(LOCKFILE)? != *before,
            Err(_) => false,
        };
        if self.manifest()? != manifest || lock_changed {
            return Err("project manifest or lock changed during package operation; nothing was published".to_owned());
        }
        Ok(())
    }

    pub(super) fn replace_local_manifest(&self, content: &str) -> Result<(), String> {
        let permissions = platform::open_manifest_to_replace(&self.root, self.directory_lock.as_ref())
            .and_then(|file| file.metadata())
            .map_err(|error| format!("cannot replace {MANIFEST}: {error}"))?
            .permissions();
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let serial = NEXT.fetch_add(1, Ordering::Relaxed);
            let name = format!(".wosy-manifest-{}-{serial}", std::process::id());
            let mut file = match platform::create(&self.root, self.directory_lock.as_ref(), &name) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("cannot stage {MANIFEST}: {error}")),
            };
            let result = (|| {
                file.write_all(content.as_bytes())?;
                file.set_permissions(permissions.clone())?;
                drop(file);
                platform::replace(&self.root, self.directory_lock.as_ref(), &name, MANIFEST)
            })();
            if let Err(error) = result {
                let _ = platform::remove(&self.root, self.directory_lock.as_ref(), &name);
                return Err(format!("cannot publish {MANIFEST}: {error}"));
            }
            return Ok(());
        }
    }
}

impl Drop for ProjectSnapshot {
    fn drop(&mut self) {
        if let Some(file) = &self.directory_lock {
            let _ = file.unlock();
        }
    }
}

mod platform {
    use super::*;

    #[cfg(unix)]
    pub(super) fn lock_file(root: &Path, writer: bool) -> io::Result<Option<File>> {
        let file = File::open(root)?;
        if writer { file.lock()?; } else { file.lock_shared()?; }
        Ok(Some(file))
    }

    #[cfg(not(unix))]
    pub(super) fn lock_file(_: &Path, writer: bool) -> io::Result<Option<File>> {
        if writer {
            Err(io::Error::new(io::ErrorKind::Unsupported, "project metadata writes require a directory lock"))
        } else {
            Ok(None)
        }
    }

    #[cfg(target_os = "linux")]
    mod operations {
        use super::*;
        use std::ffi::CString;
        use std::os::fd::{AsRawFd, FromRawFd};

        fn name(component: &str) -> io::Result<CString> {
            CString::new(component).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file name contains NUL"))
        }

        fn open(directory: &File, component: &str, flags: i32) -> io::Result<File> {
            let component = name(component)?;
            // SAFETY: the fd and component are live. A successful openat owns
            // a new descriptor. Names are fixed or generated by this module.
            let fd = unsafe {
                libc::openat(directory.as_raw_fd(), component.as_ptr(),
                    flags | libc::O_CLOEXEC | libc::O_NONBLOCK, 0o666 as libc::mode_t)
            };
            if fd < 0 { Err(io::Error::last_os_error()) } else {
                Ok(unsafe { File::from_raw_fd(fd) })
            }
        }

        pub(in crate::packages::snapshot) fn existing(_: &Path, dir: Option<&File>, component: &str) -> io::Result<File> {
            let flags = if component == LOCKFILE { libc::O_NOFOLLOW } else { 0 };
            open(dir.expect("Linux directory lock"), component, libc::O_RDONLY | flags)
        }
        pub(in crate::packages::snapshot) fn open_manifest_to_replace(_: &Path, dir: Option<&File>) -> io::Result<File> {
            open(dir.expect("Linux directory lock"), MANIFEST, libc::O_RDONLY | libc::O_NOFOLLOW)
        }

        pub(in crate::packages::snapshot) fn create(_: &Path, dir: Option<&File>, component: &str) -> io::Result<File> {
            open(dir.expect("Linux directory lock"), component, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW)
        }
        pub(in crate::packages::snapshot) fn replace(_: &Path, dir: Option<&File>, from: &str, to: &str) -> io::Result<()> {
            let from = name(from)?;
            let to = name(to)?;
            // SAFETY: the fd and names are valid for the call.
            let dir = dir.expect("Linux directory lock").as_raw_fd();
            let result = unsafe { libc::renameat(dir, from.as_ptr(), dir, to.as_ptr()) };
            if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
        }
        pub(in crate::packages::snapshot) fn remove(_: &Path, dir: Option<&File>, component: &str) -> io::Result<()> {
            let component = name(component)?;
            // SAFETY: the fd and name are valid for the call.
            let dir = dir.expect("Linux directory lock").as_raw_fd();
            let result = unsafe { libc::unlinkat(dir, component.as_ptr(), 0) };
            if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
        }
    }

    #[cfg(not(target_os = "linux"))]
    mod operations {
        use super::*;
        pub(in crate::packages::snapshot) fn existing(root: &Path, _: Option<&File>, name: &str) -> io::Result<File> {
            File::open(root.join(name))
        }
        pub(in crate::packages::snapshot) fn open_manifest_to_replace(root: &Path, _: Option<&File>) -> io::Result<File> {
            let manifest = root.join(MANIFEST);
            if fs::symlink_metadata(&manifest)?.file_type().is_symlink() {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "manifest is a symlink"));
            }
            File::open(manifest)
        }
        pub(in crate::packages::snapshot) fn create(root: &Path, _: Option<&File>, name: &str) -> io::Result<File> {
            OpenOptions::new().write(true).create_new(true).open(root.join(name))
        }
        pub(in crate::packages::snapshot) fn replace(root: &Path, _: Option<&File>, from: &str, to: &str) -> io::Result<()> {
            fs::rename(root.join(from), root.join(to))
        }
        pub(in crate::packages::snapshot) fn remove(root: &Path, _: Option<&File>, name: &str) -> io::Result<()> {
            fs::remove_file(root.join(name))
        }
    }

    pub(super) use operations::{create, existing, open_manifest_to_replace, remove, replace};
}
