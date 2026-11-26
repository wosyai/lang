//! Linux-only one-shot diagnostic transport. The already trusted local host
//! binds the source scope out of band; the socket cannot grant a new scope.
use rustix::process::geteuid;
use std::fs::{self, DirBuilder, Metadata};
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt};
use std::path::{Component, Path, PathBuf};
use tokio::net::{UnixListener, UnixStream};

const SOCKET_NAME: &str = "diagnostics.sock";

/// Fresh endpoint owned by the local activator. Same effective uid and
/// system root are the trust boundary; actors with that uid must cooperate.
pub struct NativeDiagnosticListener {
    listener: UnixListener,
    endpoint: EndpointDirectory,
}

impl NativeDiagnosticListener {
    /// A Tokio runtime with I/O enabled is required. Existing directories or
    /// sockets are never replaced or reopened. The parent must be protected.
    pub fn bind_new(directory: &Path) -> io::Result<Self> {
        let uid = geteuid().as_raw();
        check_parent(directory, uid)?;
        DirBuilder::new().mode(0o700).create(directory)?;
        let mut endpoint = EndpointDirectory::created(directory, uid)?;
        let listener = UnixListener::bind(endpoint.socket())?;
        let socket = endpoint.read_socket(uid)?;
        endpoint.socket_identity = Some(identity(&socket));
        Ok(Self { listener, endpoint })
    }

    /// The enclosing listener enforces its own accept time budget. Accept
    /// one peer only, with no pre-authentication framing or exposed root.
    pub async fn accept(&self) -> io::Result<UnixStream> {
        let (stream, _) = self.listener.accept().await?;
        require_peer(&stream, self.endpoint.owner)?;
        Ok(stream)
    }
}

/// Connect to the configured private socket, not a path received over the
/// protocol. Verify the server UID before exposing any framed traffic.
pub async fn connect(directory: &Path) -> io::Result<UnixStream> {
    let uid = geteuid().as_raw();
    check_parent(directory, uid)?;
    let endpoint = EndpointDirectory::open(directory, uid)?;
    let socket = endpoint.read_socket(uid)?;
    let stream = UnixStream::connect(endpoint.socket()).await?;
    require_peer(&stream, uid)?;
    if identity(&endpoint.read_socket(uid)?) != identity(&socket) {
        return Err(invalid("local diagnostic socket changed during connection"));
    }
    Ok(stream)
}

fn check_parent(directory: &Path, uid: u32) -> io::Result<()> {
    if !directory.is_absolute() || directory.components().any(|part| part == Component::ParentDir) {
        return Err(invalid("local diagnostics require an absolute endpoint directory without .."));
    }
    let parent = directory.parent().ok_or_else(|| invalid("missing local diagnostic endpoint parent"))?;
    // Traverse from the trust anchor; no writable or symlink ancestors.
    // A different same-UID process is already trusted by this interface.
    let parents: Vec<_> = parent.ancestors().collect();
    for path in parents.into_iter().rev() {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() || (metadata.uid() != uid && metadata.uid() != 0)
            || metadata.mode() & 0o022 != 0
        {
            return Err(invalid("local diagnostic endpoint parent is not a protected directory"));
        }
    }
    Ok(())
}

struct EndpointDirectory {
    directory: PathBuf,
    directory_identity: (u64, u64),
    socket_identity: Option<(u64, u64)>,
    owner: u32,
    owned: bool,
}

impl EndpointDirectory {
    fn created(directory: &Path, owner: u32) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(directory)?;
        // Arm identity-scoped rollback before validating the expected mode.
        // A stricter umask can leave our new directory unusable.
        let endpoint = Self::new(directory, &metadata, owner, true);
        require_private(&metadata, owner)?;
        Ok(endpoint)
    }

    fn open(directory: &Path, owner: u32) -> io::Result<Self> {
        let metadata = private_directory(directory, owner)?;
        Ok(Self::new(directory, &metadata, owner, false))
    }

    fn new(directory: &Path, metadata: &Metadata, owner: u32, owned: bool) -> Self {
        Self { directory: directory.into(), directory_identity: identity(metadata),
            socket_identity: None, owner, owned }
    }

    fn socket(&self) -> PathBuf { self.directory.join(SOCKET_NAME) }

    fn read_socket(&self, uid: u32) -> io::Result<Metadata> {
        if identity(&private_directory(&self.directory, uid)?) != self.directory_identity {
            return Err(invalid("local diagnostic endpoint directory changed"));
        }
        let metadata = fs::symlink_metadata(self.socket())?;
        if !metadata.file_type().is_socket() || metadata.uid() != uid {
            return Err(invalid("local diagnostic endpoint is not an owned Unix socket"));
        }
        Ok(metadata)
    }
}

impl Drop for EndpointDirectory {
    fn drop(&mut self) {
        if !self.owned || !fs::symlink_metadata(&self.directory).is_ok_and(|directory| {
            directory.is_dir() && directory.uid() == self.owner && identity(&directory) == self.directory_identity
        })
        {
            return;
        }
        if let Some(expected) = self.socket_identity {
            if !self.read_socket(self.owner).is_ok_and(|current| identity(&current) == expected)
                || fs::remove_file(self.socket()).is_err()
            {
                return;
            }
        }
        // Directory removal is non-recursive; never erase unexpected links
        // or debris, and never clean another process's endpoint on startup.
        let _ = fs::remove_dir(&self.directory);
    }
}

fn private_directory(path: &Path, uid: u32) -> io::Result<Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    require_private(&metadata, uid)?;
    Ok(metadata)
}

fn require_private(metadata: &Metadata, uid: u32) -> io::Result<()> {
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o777 != 0o700 {
        return Err(invalid("local diagnostic endpoint directory must be private and owned (0700)"));
    }
    Ok(())
}

fn require_peer(stream: &UnixStream, uid: u32) -> io::Result<()> {
    if stream.peer_cred()?.uid() != uid {
        return Err(invalid("wrong peer uid for local diagnostics"));
    }
    Ok(())
}

fn identity(metadata: &Metadata) -> (u64, u64) { (metadata.dev(), metadata.ino()) }

fn invalid(message: &'static str) -> io::Error { io::Error::new(io::ErrorKind::PermissionDenied, message) }
