use rustix::{
    fd::{AsFd, OwnedFd},
    fs::{self, AtFlags, Mode, OFlags},
    io::Errno,
    process::geteuid,
};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Write},
    os::fd::AsRawFd,
    path::{Component, Path},
    sync::atomic::{AtomicU64, Ordering},
};

pub(super) struct PreparedFiles {
    destination: OwnedFd,
    _stage: Stage,
    pub(super) payloads: Vec<File>,
}

struct Stage {
    parent: OwnedFd,
    directory: OwnedFd,
    name: String,
    files: Vec<(OsString, (u64, u64))>,
}

fn open_directory(parent: impl AsFd, name: impl rustix::path::Arg) -> io::Result<OwnedFd> {
    Ok(fs::openat(parent, name, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())?)
}

fn project_directory(root: &Path) -> io::Result<OwnedFd> {
    let mut parts = root.components();
    if parts.next() != Some(Component::RootDir) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut project = open_directory(fs::CWD, "/")?;
    for part in parts {
        let Component::Normal(name) = part else { return Err(io::ErrorKind::InvalidInput.into()); };
        project = open_directory(&project, name)?;
    }
    Ok(project)
}

fn output_directory(parent: impl AsFd, name: &str) -> io::Result<OwnedFd> {
    match fs::mkdirat(&parent, name, Mode::from_bits_truncate(0o700)) {
        Ok(()) | Err(Errno::EXIST) => {}
        Err(error) => return Err(error.into()),
    }
    let directory = open_directory(&parent, name)?;
    if fs::fstat(&parent)?.st_dev != fs::fstat(&directory)?.st_dev {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "check output crosses volumes"));
    }
    Ok(directory)
}

impl Stage {
    fn new(parent: OwnedFd) -> io::Result<Self> {
        Self::prepare(parent, make_stage_acl_private)
    }

    fn prepare(parent: OwnedFd, make_acl_private: impl FnOnce(&OwnedFd) -> io::Result<()>) -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = loop {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let name = format!(".check-stage-{}-{id}", std::process::id());
            match fs::mkdirat(&parent, name.as_str(), Mode::from_bits_truncate(0o700)) {
                Ok(()) => break name,
                Err(Errno::EXIST) => continue,
                Err(error) => return Err(error.into()),
            }
        };
        let directory = open_directory(&parent, name.as_str())?;
        let stage = Self { parent, directory, name, files: Vec::new() };
        let metadata = fs::fstat(&stage.directory)?;
        if metadata.st_uid != geteuid().as_raw()
            || metadata.st_dev != fs::fstat(&stage.parent)?.st_dev
        {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "check staging directory is not private"));
        }
        make_acl_private(&stage.directory)?;
        if fs::fstat(&stage.directory)?.st_mode & 0o077 != 0 {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "check staging directory is not private"));
        }
        Ok(stage)
    }

    fn write(&mut self, name: &OsStr, bytes: &[u8]) -> io::Result<File> {
        let mut file = File::from(fs::openat(
            &self.directory, name, OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_bits_truncate(0o600),
        )?);
        self.files.push((name.to_owned(), file_identity(fs::fstat(&file)?)));
        file.write_all(bytes)?;
        Ok(file)
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        for (name, expected) in &self.files {
            let name = name.as_os_str();
            if fs::statat(&self.directory, name, AtFlags::SYMLINK_NOFOLLOW)
                .is_ok_and(|current| file_identity(current) == *expected)
            {
                let _ = fs::unlinkat(&self.directory, name, AtFlags::empty());
            }
        }
        if let (Ok(original), Ok(current)) = (
            fs::fstat(&self.directory),
            fs::statat(&self.parent, self.name.as_str(), AtFlags::SYMLINK_NOFOLLOW),
        ) {
            if (original.st_dev, original.st_ino) == (current.st_dev, current.st_ino) {
                let _ = fs::unlinkat(&self.parent, self.name.as_str(), AtFlags::REMOVEDIR);
            }
        }
    }
}

impl PreparedFiles {
    pub(super) fn new(root: &Path, requests: &[(&Path, &[u8])]) -> io::Result<Self> {
        Self::prepare(root, requests, preflight_clone_volume)
    }

    fn prepare(
        root: &Path,
        requests: &[(&Path, &[u8])],
        preflight: impl FnOnce(&OwnedFd) -> io::Result<()>,
    ) -> io::Result<Self> {
        let project = project_directory(root)?;
        let state = output_directory(&project, ".wosy")?;
        let output = output_directory(&state, "outputs")?;
        let destination = output_directory(&output, "check")?;
        for (name, _) in requests {
            match fs::statat(&destination, *name, AtFlags::SYMLINK_NOFOLLOW) {
                Err(Errno::NOENT) => {}
                Ok(_) => return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", name.display()))),
                Err(error) => return Err(error.into()),
            }
        }
        preflight(&destination)?;
        let mut stage = Stage::new(output)?;
        // Exclusive creation on the same filesystem also catches names
        // that alias on a case-insensitive or normalization-insensitive volume.
        let payloads = requests.iter().map(|(name, bytes)| stage.write(name.as_os_str(), bytes))
            .collect::<io::Result<_>>()?;
        Ok(Self { destination, _stage: stage, payloads })
    }

    pub(super) fn publish(&self, payload: &File, request: &Path) -> io::Result<()> {
        publish_from_descriptor(payload, &self.destination, request)
            .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", request.display())))
    }
}

fn file_identity(stat: fs::Stat) -> (u64, u64) {
    (stat.st_dev as u64, stat.st_ino as u64)
}

// Unknown capability bits are not an advertisement of support.
fn require_clone_support(capabilities: u32, valid: u32, clone: u32) -> io::Result<()> {
    if capabilities & valid & clone != 0 {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::Unsupported, "check output volume does not support file cloning"))
    }
}

#[cfg(target_os = "macos")]
fn preflight_clone_volume(destination: &OwnedFd) -> io::Result<()> {
    #[repr(C)]
    struct VolumeCapabilities {
        length: u32,
        attr: libc::vol_capabilities_attr_t,
    }

    let mut request = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: 0,
        volattr: libc::ATTR_VOL_INFO | libc::ATTR_VOL_CAPABILITIES,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    let mut response = VolumeCapabilities {
        length: 0,
        attr: libc::vol_capabilities_attr_t { capabilities: [0; 4], valid: [0; 4] },
    };
    let size = std::mem::size_of::<VolumeCapabilities>();
    // SAFETY: the live destination descriptor identifies the volume, and
    // both C-layout buffers remain valid for the duration of the call.
    let result = unsafe {
        libc::fgetattrlist(
            destination.as_raw_fd(),
            (&mut request as *mut libc::attrlist).cast(),
            (&mut response as *mut VolumeCapabilities).cast(),
            size,
            0,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    if response.length as usize != size {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "check output volume did not report clone capabilities"));
    }
    let group = libc::VOL_CAPABILITIES_INTERFACES;
    require_clone_support(response.attr.capabilities[group], response.attr.valid[group], libc::VOL_CAP_INT_CLONE)
}

#[cfg(all(test, target_os = "linux"))]
fn preflight_clone_volume(_: &OwnedFd) -> io::Result<()> {
    // Linux tests emulate publication with a descriptor hard link.
    Ok(())
}

#[cfg(target_os = "macos")]
mod stage_acl {
    use super::OwnedFd;
    use std::{ffi::c_void, io, os::fd::AsRawFd, ptr};

    // Values from Darwin <sys/fcntl.h> and <sys/acl.h>.
    const FILESEC_ACL: libc::c_int = 5;
    const FILESEC_REMOVE_ACL: *mut c_void = ptr::without_provenance_mut(1);
    const ACL_FIRST_ENTRY: libc::c_int = 0;

    unsafe extern "C" {
        fn acl_get_fd(fd: libc::c_int) -> *mut c_void;
        fn acl_get_entry(acl: *mut c_void, entry_id: libc::c_int, entry: *mut *mut c_void) -> libc::c_int;
        fn acl_free(acl: *mut c_void) -> libc::c_int;
        fn filesec_init() -> *mut c_void;
        fn filesec_set_property(filesec: *mut c_void, property: libc::c_int, value: *const c_void) -> libc::c_int;
        fn fchmodx_np(fd: libc::c_int, filesec: *mut c_void) -> libc::c_int;
        fn filesec_free(filesec: *mut c_void);
    }

    pub(super) fn has_entries(descriptor: &impl AsRawFd) -> io::Result<bool> {
        // SAFETY: the descriptor is live, and the returned ACL is freed here.
        unsafe {
            let acl = acl_get_fd(descriptor.as_raw_fd());
            if acl.is_null() {
                let error = io::Error::last_os_error();
                return if error.raw_os_error() == Some(libc::ENOENT) { Ok(false) } else { Err(error) };
            }
            if acl == FILESEC_REMOVE_ACL {
                return Ok(false);
            }
            let mut entry = ptr::null_mut();
            let status = acl_get_entry(acl, ACL_FIRST_ENTRY, &mut entry);
            let result = if status == 0 {
                Ok(true)
            } else {
                let error = io::Error::last_os_error();
                // Darwin reports EINVAL for the first entry of a valid empty ACL.
                if error.raw_os_error() == Some(libc::EINVAL) { Ok(false) } else { Err(error) }
            };
            acl_free(acl);
            result
        }
    }

    pub(super) fn clear(directory: &OwnedFd) -> io::Result<()> {
        // The descriptor form of Apple's chmod -N, restricted to our empty stage.
        // SAFETY: the descriptor is live, FILESEC_REMOVE_ACL is the Darwin sentinel,
        // and the initialized filesec lives until both calls finish.
        unsafe {
            let filesec = filesec_init();
            if filesec.is_null() {
                return Err(io::Error::last_os_error());
            }
            let result = if filesec_set_property(filesec, FILESEC_ACL, FILESEC_REMOVE_ACL) != 0
                || fchmodx_np(directory.as_raw_fd(), filesec) != 0
            {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            };
            filesec_free(filesec);
            result
        }
    }
}

#[cfg(target_os = "macos")]
fn make_stage_acl_private(directory: &OwnedFd) -> io::Result<()> {
    if stage_acl::has_entries(directory)? {
        stage_acl::clear(directory)?;
        if stage_acl::has_entries(directory)? {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "check staging directory still has ACL entries"));
        }
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
fn make_stage_acl_private(_: &OwnedFd) -> io::Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn publish_from_descriptor(payload: &File, destination: &OwnedFd, request: &Path) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let leaf = CString::new(request.as_os_str().as_bytes())?;
    // APFS clones the open source file, not a staging pathname, in one
    // no-replace operation. Do not fall back to resolving the source by name.
    // SAFETY: both descriptors are live and leaf is NUL-terminated.
    let result = unsafe {
        libc::fclonefileat(payload.as_raw_fd(), destination.as_raw_fd(), leaf.as_ptr(), 0)
    };
    if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

#[cfg(all(test, target_os = "linux"))]
fn publish_from_descriptor(payload: &File, destination: &OwnedFd, request: &Path) -> io::Result<()> {
    let path = format!("/proc/self/fd/{}", payload.as_raw_fd());
    Ok(fs::linkat(fs::CWD, path.as_str(), destination, request, AtFlags::SYMLINK_FOLLOW)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::{symlink, PermissionsExt}};

    fn temp() -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir().canonicalize().unwrap();
        loop {
            let path = base.join(format!("wosy-macos-output-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            match fs::create_dir(&path) {
                Ok(()) => return path,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("{error}"),
            }
        }
    }

    #[test]
    fn clone_capability_must_be_advertised_and_valid() {
        let clone = 1 << 16;
        for (capabilities, valid) in [(0, clone), (clone, 0), (!clone, !clone)] {
            assert_eq!(require_clone_support(capabilities, valid, clone).unwrap_err().kind(), io::ErrorKind::Unsupported);
        }
        require_clone_support(clone, clone, clone).unwrap();
    }

    #[test]
    fn clone_preflight_uses_destination_and_rejects_before_staging() {
        use std::os::unix::fs::MetadataExt;

        let root = temp();
        let output = root.join(".wosy/outputs");
        let result = PreparedFiles::prepare(&root, &[(Path::new("report.json"), b"report")], |fd| {
            let pinned = rustix::fs::fstat(fd).unwrap();
            let named = fs::metadata(output.join("check")).unwrap();
            assert_eq!(file_identity(pinned), (named.dev(), named.ino()));
            require_clone_support(0, 1, 1)
        });
        assert_eq!(result.err().unwrap().kind(), io::ErrorKind::Unsupported);
        assert_eq!(fs::read_dir(output.join("check")).unwrap().count(), 0);
        assert_eq!(fs::read_dir(output).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stage_privacy_failure_leaves_no_stage_or_payload() {
        let root = temp();
        let parent = open_directory(rustix::fs::CWD, &root).unwrap();
        let result = Stage::prepare(parent, |fd| {
            let dir = rustix::fs::Dir::read_from(fd).unwrap();
            for entry in dir {
                assert!(matches!(entry.unwrap().file_name().to_bytes(), b"." | b".."));
            }
            Err(io::ErrorKind::PermissionDenied.into())
        });
        assert_eq!(result.err().unwrap().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir(root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn inherited_acl_is_removed_only_from_stage() {
        let root = temp();
        let output = root.join(".wosy/outputs");
        fs::create_dir_all(&output).unwrap();
        let status = std::process::Command::new("/bin/chmod")
            .args(["+a", "everyone allow read,execute,file_inherit,directory_inherit"])
            .arg(&output)
            .status().unwrap();
        assert!(status.success());

        let report = Path::new("report.json");
        let files = PreparedFiles::new(&root, &[(report, b"complete")]).unwrap();
        assert!(stage_acl::has_entries(&files.destination).unwrap());
        assert!(stage_acl::has_entries(&files._stage.parent).unwrap());
        assert!(!stage_acl::has_entries(&files._stage.directory).unwrap());
        assert!(!stage_acl::has_entries(&files.payloads[0]).unwrap());

        files.publish(&files.payloads[0], report).unwrap();
        let published = OwnedFd::from(File::open(output.join("check").join(report)).unwrap());
        assert!(stage_acl::has_entries(&published).unwrap());
        drop(files);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn named_staging_is_private_and_publishes_without_replacement() {
        let root = temp();
        let output = root.join(".wosy/outputs");
        let report = Path::new("report.json");
        let other = Path::new("other.json");
        let files = PreparedFiles::new(&root, &[(report, b"complete"), (other, b"second")]).unwrap();
        assert_eq!(fs::read_dir(output.join("check")).unwrap().count(), 0);
        let stage = output.join(&files._stage.name);
        assert_eq!(fs::metadata(&stage).unwrap().permissions().mode() & 0o077, 0);
        fs::write(output.join("check/other.json"), b"existing").unwrap();
        files.publish(&files.payloads[0], report).unwrap();
        assert_eq!(files.publish(&files.payloads[1], other).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        drop(files);
        assert!(!stage.exists());
        assert_eq!(fs::read(output.join("check/report.json")).unwrap(), b"complete");
        assert_eq!(fs::read(output.join("check/other.json")).unwrap(), b"existing");
        assert_eq!(PreparedFiles::new(&root, &[(report, b"third")]).err().unwrap().kind(), io::ErrorKind::AlreadyExists);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn publishing_uses_the_open_payload_not_its_staging_name() {
        let root = temp();
        let output = root.join(".wosy/outputs");
        let report = Path::new("report.json");
        let files = PreparedFiles::new(&root, &[(report, b"original")]).unwrap();
        let stage = output.join(&files._stage.name);
        fs::rename(stage.join(report), stage.join("moved")).unwrap();
        fs::write(stage.join(report), b"other entry").unwrap();
        files.publish(&files.payloads[0], report).unwrap();
        drop(files);
        assert_eq!(fs::read(output.join("check/report.json")).unwrap(), b"original");
        assert_eq!(fs::read(stage.join(report)).unwrap(), b"other entry");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn conflicting_staged_names_leave_no_payloads_or_staging_directory() {
        let root = temp();
        let report = Path::new("report.json");
        let result = PreparedFiles::new(&root, &[(report, b"first"), (report, b"second")]);
        assert_eq!(result.err().unwrap().kind(), io::ErrorKind::AlreadyExists);
        let output = root.join(".wosy/outputs");
        assert_eq!(fs::read_dir(output.join("check")).unwrap().count(), 0);
        assert_eq!(fs::read_dir(output).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn named_staging_does_not_follow_output_or_leaf_aliases() {
        let root = temp();
        let external = temp();
        let report = Path::new("report.json");
        fs::create_dir_all(root.join(".wosy/outputs")).unwrap();
        let check = root.join(".wosy/outputs/check");
        symlink(&external, &check).unwrap();
        assert!(PreparedFiles::new(&root, &[(report, b"out")]).is_err());
        fs::remove_file(&check).unwrap();
        fs::create_dir(&check).unwrap();
        symlink(external.join(report), check.join(report)).unwrap();
        assert!(PreparedFiles::new(&root, &[(report, b"out")]).is_err());
        assert_eq!(fs::read_dir(&external).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(external).unwrap();
    }
}
