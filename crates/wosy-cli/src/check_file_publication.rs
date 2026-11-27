use std::{fs::File, io::{self, Write}, os::fd::AsRawFd, path::Path};

use rustix::{
    fd::{AsFd, OwnedFd},
    fs::{self, AtFlags, Mode, OFlags, ResolveFlags},
    io::Errno,
};

pub(super) struct PreparedFiles {
    destination: OwnedFd,
    process_fds: OwnedFd,
    pub(super) payloads: Vec<File>,
}

fn below_root() -> ResolveFlags {
    ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV
}

fn directory_flags() -> OFlags {
    OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

fn open_directory(parent: impl AsFd, name: &str, create: bool) -> io::Result<OwnedFd> {
    let open = || fs::openat2(&parent, name, directory_flags(), Mode::empty(), below_root());
    match open() {
        Err(Errno::NOENT) if create => {
            match fs::mkdirat(&parent, name, Mode::from_bits_truncate(0o700)) {
                Ok(()) | Err(Errno::EXIST) => {}
                Err(error) => return Err(error.into()),
            }
            Ok(open()?)
        }
        result => Ok(result?),
    }
}

impl PreparedFiles {
    pub(super) fn new(root: &Path, requests: &[(&Path, &[u8])]) -> io::Result<Self> {
        let project = fs::openat2(fs::CWD, root, directory_flags(), Mode::empty(), ResolveFlags::NO_SYMLINKS)?;
        let proc_root = fs::openat2(fs::CWD, "/proc", directory_flags(), Mode::empty(), ResolveFlags::NO_SYMLINKS)?;
        if fs::fstatfs(&proc_root)?.f_type != fs::PROC_SUPER_MAGIC {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "procfs unavailable for anonymous file publication"));
        }
        let process = open_directory(&proc_root, &std::process::id().to_string(), false)?;
        let process_fds = open_directory(&process, "fd", false)?;
        let state = open_directory(&project, ".wosy", true)?;
        let output = open_directory(&state, "outputs", true)?;
        let destination = open_directory(&output, "check", true)?;
        for (name, _) in requests {
            match fs::statat(&destination, *name, AtFlags::SYMLINK_NOFOLLOW) {
                Err(Errno::NOENT) => {}
                Ok(_) => return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", name.display()))),
                Err(error) => return Err(error.into()),
            }
        }
        let mut payloads = Vec::with_capacity(requests.len());
        for (_, bytes) in requests {
            let mut file = File::from(fs::openat2(
                &destination, ".", OFlags::TMPFILE | OFlags::WRONLY | OFlags::CLOEXEC,
                Mode::from_bits_truncate(0o600), below_root(),
            )?);
            file.write_all(bytes)?;
            payloads.push(file);
        }
        Ok(Self { destination, process_fds, payloads })
    }

    pub(super) fn publish(&self, payload: &File, request: &Path) -> io::Result<()> {
        fs::linkat(
            &self.process_fds, payload.as_raw_fd().to_string().as_str(),
            &self.destination, request, AtFlags::SYMLINK_FOLLOW,
        ).map_err(|error| io::Error::new(io::Error::from(error).kind(), format!("{}: {}", request.display(), io::Error::from(error))))
    }
}
