use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

/// A directory already opened without following links. Clones retain that
/// same directory, not a path that could later name a different directory.
#[derive(Clone, Debug)]
pub struct GitDirectory {
    file: Arc<File>,
    identity: Vec<u8>,
}

impl PartialEq for GitDirectory {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}
impl Eq for GitDirectory {}

impl GitDirectory {
    #[cfg(all(test, target_os = "linux"))]
    pub(super) fn project(root: &Path) -> Result<(PathBuf, Self), String> {
        let canonical = root
            .canonicalize()
            .map_err(|error| format!("cannot resolve Git project root {}: {error}", root.display()))?;
        let file = platform::open_absolute_directory(&canonical)
            .map_err(|error| format!("cannot open Git project root {}: {error}", canonical.display()))?;
        Ok((canonical, Self::new(file)?))
    }

    pub(super) fn new(file: File) -> Result<Self, String> {
        let identity = platform::identity(&file)
            .map_err(|error| format!("cannot identify Git directory: {error}"))?;
        Ok(Self { file: Arc::new(file), identity })
    }

    pub(super) fn identity(&self) -> &[u8] {
        &self.identity
    }

    pub(crate) fn directory(&self, relative: &Path) -> Result<Option<Self>, String> {
        let Some(file) = self.open(relative)? else { return Ok(None) };
        if file.metadata().map_err(|error| describe(relative, error))?.is_dir() {
            Self::new(file).map(Some)
        } else {
            Err(format!("Git path {} must be a directory, not a symlink or file", relative.display()))
        }
    }

    /// Open an editable source without following links. The caller reads this
    /// exact opened handle; it must not reopen the path by name.
    pub fn source(&self, relative: &Path) -> Result<Option<File>, String> {
        let Some(file) = self.open(relative)? else { return Ok(None) };
        if file.metadata().map_err(|error| describe(relative, error))?.is_file() {
            Ok(Some(file))
        } else {
            Err(format!("Git path {} must be a regular file", relative.display()))
        }
    }

    pub(crate) fn read_text(&self, relative: &Path) -> Result<Option<String>, String> {
        let Some(mut file) = self.source(relative)? else { return Ok(None) };
        let mut text = String::new();
        file.read_to_string(&mut text).map_err(|error| describe(relative, error))?;
        Ok(Some(text))
    }

    pub(super) fn open(&self, relative: &Path) -> Result<Option<File>, String> {
        crate::PackageRelativePath::new(relative.to_path_buf())?;
        match platform::open_relative(&self.file, relative) {
            Ok(file) => Ok(Some(file)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(describe(relative, error)),
        }
    }

    pub(super) fn entries(&self) -> io::Result<Vec<std::ffi::OsString>> {
        platform::entries(&self.file)
    }

    #[cfg(target_os = "linux")]
    pub(super) fn entry_names(
        &self,
    ) -> io::Result<impl Iterator<Item = io::Result<std::ffi::OsString>>> {
        platform::entry_names(&self.file)
    }

    pub(super) fn mkdir(&self, name: &Path) -> io::Result<()> {
        one_component(name)?;
        platform::mkdir(&self.file, name, 0o777)
    }

    pub(super) fn mkdir_private(&self, name: &Path) -> io::Result<()> {
        one_component(name)?;
        platform::mkdir(&self.file, name, 0o700)
    }

    pub(super) fn remove_empty_directory(&self, name: &Path) -> io::Result<()> {
        one_component(name)?;
        platform::remove_directory(&self.file, name)
    }

    pub(super) fn descriptor_path(&self) -> io::Result<PathBuf> {
        platform::descriptor_path(&self.file)
    }

    pub(super) fn inherit_in(&self, command: &mut Command) -> io::Result<()> {
        platform::inherit_in(&self.file, command)
    }


    pub(super) fn rename(&self, from: &Path, into: &Self, name: &Path) -> io::Result<()> {
        one_component(from)?;
        one_component(name)?;
        platform::rename(&self.file, from, &into.file, name, false)
    }

    pub(super) fn exchange(&self, from: &Path, into: &Self, name: &Path) -> io::Result<()> {
        one_component(from)?;
        one_component(name)?;
        platform::rename(&self.file, from, &into.file, name, true)
    }
}

fn one_component(path: &Path) -> io::Result<()> {
    if matches!(path.components().next(), Some(Component::Normal(name)) if name == path.as_os_str())
    {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "expected one portable path component"))
    }
}

fn describe(relative: &Path, error: io::Error) -> String {
    #[cfg(target_os = "linux")]
    match error.raw_os_error() {
        Some(libc::ELOOP) => return format!("Git path {} must not traverse a symlink (not a symlink or file)", relative.display()),
        Some(libc::EXDEV) => return format!("Git path {} must stay inside its original filesystem", relative.display()),
        _ => {}
    }
    format!("cannot access Git path {}: {error}", relative.display())
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::ffi::{CString, OsString};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;

    const NO_LINKS: u64 = libc::RESOLVE_NO_MAGICLINKS | libc::RESOLVE_NO_SYMLINKS;
    const FILE_FLAGS: i32 = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NONBLOCK | libc::O_NOCTTY;

    fn c_path(path: &Path) -> io::Result<CString> {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
    }

    fn openat2(dir: i32, path: &Path, flags: i32, resolve: u64) -> io::Result<File> {
        let path = c_path(path)?;
        // SAFETY: the kernel ABI record contains only integer fields, and
        // zero initialization also leaves any future extension fields unset.
        let mut how: libc::open_how = unsafe { std::mem::zeroed() };
        how.flags = flags as u64;
        how.resolve = resolve;
        // SAFETY: the path and open_how record are valid for this call. A
        // nonnegative result is a new descriptor owned by the caller.
        let fd = unsafe {
            libc::syscall(libc::SYS_openat2, dir, path.as_ptr(), &how, std::mem::size_of_val(&how))
        };
        if fd < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ENOSYS) {
                Err(io::Error::new(io::ErrorKind::Unsupported, "Git dependencies require Linux openat2 (kernel 5.6 or later)"))
            } else {
                Err(error)
            }
        } else {
            // SAFETY: openat2 returned a new live descriptor.
            Ok(unsafe { File::from_raw_fd(fd as i32) })
        }
    }

    #[cfg(test)]
    pub(super) fn open_absolute_directory(path: &Path) -> io::Result<File> {
        openat2(libc::AT_FDCWD, path, FILE_FLAGS | libc::O_DIRECTORY, NO_LINKS)
    }

    pub(super) fn open_relative(dir: &File, path: &Path) -> io::Result<File> {
        openat2(dir.as_raw_fd(), path, FILE_FLAGS, NO_LINKS | libc::RESOLVE_BENEATH | libc::RESOLVE_NO_XDEV)
    }

    pub(super) fn identity(file: &File) -> io::Result<Vec<u8>> {
        let metadata = file.metadata()?;
        let mut id = Vec::new();
        id.extend_from_slice(&metadata.dev().to_le_bytes());
        id.extend_from_slice(&metadata.ino().to_le_bytes());
        if let Some(time) = metadata.created().ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        {
            id.extend_from_slice(&time.as_secs().to_le_bytes());
            id.extend_from_slice(&time.subsec_nanos().to_le_bytes());
        }
        Ok(id)
    }

    pub(super) fn descriptor_path(dir: &File) -> io::Result<PathBuf> {
        // The caller retains the descriptor throughout this path's lifetime.
        Ok(PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd())))
    }

    pub(super) fn inherit_in(dir: &File, command: &mut Command) -> io::Result<()> {
        use std::os::unix::process::CommandExt;
        let fd = dir.as_raw_fd();
        // SAFETY: the parent retains the descriptor until the child exits;
        // only the child's copy loses close-on-exec. fcntl is async-signal-safe
        // and the hook does not allocate or use parent-process locks.
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        Ok(())
    }

    pub(super) fn entry_names(
        dir: &File,
    ) -> io::Result<impl Iterator<Item = io::Result<OsString>>> {
        // /proc supplies names from the held descriptor, not from the original
        // checkout path. Actual file opens use openat2 relative to that handle.
        Ok(std::fs::read_dir(descriptor_path(dir)?)?
            .map(|entry| entry.map(|entry| entry.file_name())))
    }

    pub(super) fn entries(dir: &File) -> io::Result<Vec<OsString>> {
        entry_names(dir)?.collect()
    }

    pub(super) fn mkdir(dir: &File, name: &Path, mode: u32) -> io::Result<()> {
        let name = c_path(name)?;
        // SAFETY: dir and the NUL-terminated component are valid.
        let result = unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), mode as libc::mode_t) };
        if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
    }

    pub(super) fn remove_directory(dir: &File, name: &Path) -> io::Result<()> {
        let name = c_path(name)?;
        // SAFETY: dir and the NUL-terminated component are valid. unlinkat
        // does not follow a link and only removes an empty directory.
        let result = unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) };
        if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
    }

    pub(super) fn rename(from: &File, path: &Path, into: &File, name: &Path, exchange: bool) -> io::Result<()> {
        let path = c_path(path)?;
        let name = c_path(name)?;
        // SAFETY: descriptors and both NUL-terminated components are valid.
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2, from.as_raw_fd(), path.as_ptr(),
                into.as_raw_fd(), name.as_ptr(),
                if exchange { libc::RENAME_EXCHANGE } else { libc::RENAME_NOREPLACE },
            )
        };
        if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use super::*;
    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "Git dependency sync and loading are currently supported only on Linux"))
    }
    pub(super) fn open_relative(_: &File, _: &Path) -> io::Result<File> { unsupported() }
    pub(super) fn identity(_: &File) -> io::Result<Vec<u8>> { unsupported() }
    pub(super) fn entries(_: &File) -> io::Result<Vec<std::ffi::OsString>> { unsupported() }
    pub(super) fn mkdir(_: &File, _: &Path, _: u32) -> io::Result<()> { unsupported() }
    pub(super) fn remove_directory(_: &File, _: &Path) -> io::Result<()> { unsupported() }
    pub(super) fn descriptor_path(_: &File) -> io::Result<PathBuf> { unsupported() }
    pub(super) fn inherit_in(_: &File, _: &mut Command) -> io::Result<()> { unsupported() }
    pub(super) fn rename(_: &File, _: &Path, _: &File, _: &Path, _: bool) -> io::Result<()> { unsupported() }
}


#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn an_open_directory_does_not_switch_to_a_replacement_path() {
        let root = std::env::temp_dir().join(format!(
            "wosy-anchored-checkout-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(root.join("first")).unwrap();
        std::fs::write(root.join("first/source.w"), "first source").unwrap();
        let project = GitDirectory::project(&root).unwrap().1;
        let first = project.directory(Path::new("first")).unwrap().unwrap();
        std::fs::rename(root.join("first"), root.join("moved")).unwrap();
        std::fs::create_dir(root.join("first")).unwrap();
        std::fs::write(root.join("first/source.w"), "replacement source").unwrap();
        let text = first.read_text(Path::new("source.w")).unwrap().unwrap();
        let replacement = project.directory(Path::new("first")).unwrap().unwrap();
        assert_eq!(text, "first source");
        assert_ne!(first, replacement);
        std::fs::remove_dir_all(root).unwrap();
    }

}
