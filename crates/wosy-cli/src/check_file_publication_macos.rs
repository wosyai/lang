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
    path::{Component, Path},
    sync::atomic::{AtomicU64, Ordering},
};

pub(super) struct PreparedFiles {
    destination: OwnedFd,
    stage: Stage,
    pub(super) payloads: Vec<OsString>,
}

struct Stage {
    parent: OwnedFd,
    directory: OwnedFd,
    name: String,
    files: Vec<OsString>,
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
        let metadata = fs::fstat(&directory)?;
        if metadata.st_uid != geteuid().as_raw() || metadata.st_mode & 0o077 != 0
            || metadata.st_dev != fs::fstat(&parent)?.st_dev
        {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "check staging directory is not private"));
        }
        Ok(Self { parent, directory, name, files: Vec::new() })
    }

    fn write(&mut self, name: &OsStr, bytes: &[u8]) -> io::Result<OsString> {
        let mut file = File::from(fs::openat(
            &self.directory, name, OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_bits_truncate(0o600),
        )?);
        self.files.push(name.to_owned());
        file.write_all(bytes)?;
        Ok(name.to_owned())
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        for name in &self.files {
            let _ = fs::unlinkat(&self.directory, name.as_os_str(), AtFlags::empty());
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
        let mut stage = Stage::new(output)?;
        // Exclusive creation on the same filesystem also catches names
        // that alias on a case-insensitive or normalization-insensitive volume.
        let payloads = requests.iter().map(|(name, bytes)| stage.write(name.as_os_str(), bytes))
            .collect::<io::Result<_>>()?;
        Ok(Self { destination, stage, payloads })
    }

    pub(super) fn publish(&self, payload: &OsStr, request: &Path) -> io::Result<()> {
        fs::linkat(&self.stage.directory, payload, &self.destination, request, AtFlags::empty())
            .map_err(|error| io::Error::new(io::Error::from(error).kind(), format!("{}: {}", request.display(), io::Error::from(error))))
    }
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
    fn named_staging_is_private_and_publishes_without_replacement() {
        let root = temp();
        let output = root.join(".wosy/outputs");
        let report = Path::new("report.json");
        let other = Path::new("other.json");
        let files = PreparedFiles::new(&root, &[(report, b"complete"), (other, b"second")]).unwrap();
        assert_eq!(fs::read_dir(output.join("check")).unwrap().count(), 0);
        let stage = output.join(&files.stage.name);
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
