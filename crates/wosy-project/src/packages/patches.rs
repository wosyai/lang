use super::git_sync::{git_command, materialize_tree, FrozenTree, PristinePackage, Stage, TreeIdentity};
use super::git_wire::{GitOid, GitTreeOid};
use super::{GitDirectory, GitProject};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use uuid::{NoContext, Timestamp, Uuid, Variant, Version};

pub(super) const PATCHES: &str = "patches";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct PatchId(Uuid);

impl PatchId {
    fn parse(stem: &str) -> Result<Self, String> {
        let id = Uuid::parse_str(stem).map_err(|_| "expected a UUIDv7".to_owned())?;
        if id.get_version() != Some(Version::SortRand)
            || id.get_variant() != Variant::RFC4122
            || id.hyphenated().to_string() != stem
        {
            return Err("expected a canonical lowercase UUIDv7".to_owned());
        }
        Ok(Self(id))
    }

    fn after(tail: Option<Self>) -> Result<Self, String> {
        let now = Self(Uuid::now_v7());
        let Some(tail) = tail.filter(|tail| *tail >= now) else {
            return Ok(now);
        };
        let (seconds, nanos) = tail.0.get_timestamp().expect("UUIDv7 has a timestamp").to_unix();
        let millis = seconds
            .checked_mul(1000)
            .and_then(|ms| ms.checked_add(u64::from(nanos / 1_000_000)))
            .and_then(|ms| ms.checked_add(1))
            .ok_or_else(|| "patch ordering timestamp exhausted".to_owned())?;
        let timestamp = Timestamp::from_unix(NoContext, millis / 1000, (millis % 1000) as u32 * 1_000_000);
        let id = Self(Uuid::new_v7(timestamp));
        if id <= tail {
            return Err("patch ordering timestamp exhausted".to_owned());
        }
        Ok(id)
    }

    pub(super) fn filename(&self) -> String {
        format!("{}.patch", self.0.hyphenated())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct PatchStackId([u8; 32]);

impl PatchStackId {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

pub(super) struct PatchStack {
    entries: BTreeMap<PatchId, Vec<u8>>,
    digest: blake3::Hasher,
}

impl PatchStack {
    pub(super) fn load(project: &GitDirectory, name: &str) -> Result<Self, String> {
        let path = Path::new(PATCHES).join(name);
        let mut entries = BTreeMap::new();
        if let Some(directory) = project.directory(&path)? {
            for entry in directory.entries().map_err(|error| format!("cannot read {}: {error}", path.display()))? {
                let entry = Path::new(&entry);
                if entry.extension() != Some(std::ffi::OsStr::new("patch")) {
                    continue;
                }
                let stem = entry.file_stem().and_then(|stem| stem.to_str())
                    .ok_or_else(|| format!("invalid patch name {}", entry.display()))?;
                let id = PatchId::parse(stem)
                    .map_err(|error| format!("invalid patch {}: {error}", entry.display()))?;
                let mut file = directory.source(entry)?
                    .ok_or_else(|| format!("patch {} disappeared", entry.display()))?;
                let mut payload = Vec::new();
                std::io::Read::read_to_end(&mut file, &mut payload)
                    .map_err(|error| format!("cannot read patch {}: {error}", entry.display()))?;
                entries.insert(id, payload);
            }
        }
        let mut stack = Self::empty();
        for (id, bytes) in &entries {
            stack.hash_patch(*id, bytes);
        }
        stack.entries = entries;
        Ok(stack)
    }

    pub(super) fn empty() -> Self {
        let mut digest = blake3::Hasher::new();
        digest.update(b"wosy-patch-stack-v1\0");
        Self { entries: BTreeMap::new(), digest }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn identity(&self) -> PatchStackId {
        PatchStackId(*self.digest.finalize().as_bytes())
    }

    pub(super) fn append(&mut self, bytes: Vec<u8>) -> Result<PatchId, String> {
        let id = PatchId::after(self.entries.last_key_value().map(|(id, _)| *id))?;
        self.hash_patch(id, &bytes);
        self.entries.insert(id, bytes);
        Ok(id)
    }

    fn hash_patch(&mut self, id: PatchId, bytes: &[u8]) {
        self.digest.update(id.0.as_bytes());
        self.digest.update(blake3::hash(bytes).as_bytes());
    }

    pub(super) fn bytes(&self, id: PatchId) -> &[u8] {
        &self.entries[&id]
    }
}

/// Private scratch repository and index. The consuming worktree/index are
/// never a patch input or a Git output.
pub(super) struct GitOverlay {
    project: GitDirectory,
    stage: GitDirectory,
    repository: PathBuf,
}

impl GitOverlay {
    pub(super) fn for_pristine(project: &GitProject, stage: &Stage, pristine: &PristinePackage) -> Self {
        Self { project: project.directory.clone(), stage: stage.handle.clone(), repository: pristine.repository.clone() }
    }

    fn command(&self, repository: Option<&Path>) -> Result<Command, String> {
        let mut command = git_command(&self.project, &self.stage, repository)?;
        // Git user config is not needed for local object transforms.
        // Null global/system config keeps diff/index results independent of it.
        command
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_CONFIG_COUNT")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env("GIT_ATTR_NOSYSTEM", "1");
        Ok(command)
    }

    fn repo_command(&self) -> Result<Command, String> {
        self.command(Some(&self.repository))
    }

    pub(super) fn capture(&self, frozen: &FrozenTree) -> Result<GitTreeOid, String> {
        self.check(
            self.repo_command()?.args(["read-tree", "--empty"]).output(),
            "reset patch index",
        )?;
        let mut index = Vec::new();
        for (path, (_, executable)) in &frozen.files {
            let file = frozen.directory.source(Path::new(path))?
                .ok_or_else(|| format!("staged snapshot file {path} disappeared"))?;
            let object = self.check(
                self.repo_command()?.args(["hash-object", "-w", "--no-filters", "--stdin"])
                    .stdin(Stdio::from(file)).output(),
                "capture patch input",
            )?;
            let oid = GitOid::from_rev_parse(&object.stdout)?;
            index.extend_from_slice(if *executable { b"100755 " } else { b"100644 " });
            index.extend_from_slice(oid.as_str().as_bytes());
            index.push(b'\t');
            index.extend_from_slice(path.as_bytes());
            index.push(0);
        }
        if !index.is_empty() {
            let mut command = self.repo_command()?;
            command.args(["update-index", "-z", "--index-info"]);
            self.check(with_input(command, &index), "write patch index")?;
        }
        self.write_tree()
    }

    pub(super) fn apply(&self, name: &str, base: &GitTreeOid, stack: &PatchStack) -> Result<GitTreeOid, String> {
        if stack.is_empty() {
            return Ok(base.clone());
        }
        self.check(
            self.repo_command()?.args(["read-tree", base.as_str()]).output(),
            "prepare patch index",
        )?;
        for (id, payload) in &stack.entries {
            let mut command = self.repo_command()?;
            command.args(["apply", "--cached", "--binary", "--whitespace=nowarn", "-"]);
            let output = with_input(command, payload)
                .map_err(|error| format!("cannot apply patch {} for {name}: {error}", id.filename()))?;
            if !output.status.success() {
                return Err(format!(
                    "patch {} for {name} does not apply: {}",
                    id.filename(), String::from_utf8_lossy(&output.stderr)
                ));
            }
        }
        self.write_tree()
    }

    pub(super) fn difference(&self, base: &GitTreeOid, edited: &GitTreeOid) -> Result<Option<Vec<u8>>, String> {
        let output = self.repo_command()?
            .args(["diff-tree", "-r", "-p", "--exit-code", "--binary", "--full-index", "--find-renames",
                "--no-ext-diff", "--no-textconv", "--no-color", "--src-prefix=a/", "--dst-prefix=b/"])
            .arg(base.as_str()).arg(edited.as_str()).output().map_err(|error| error.to_string())?;
        match output.status.code() {
            Some(0) => Ok(None),
            Some(1) => Ok(Some(output.stdout)),
            _ => succeed(output, "make project patch").map(|_| None),
        }
    }

    pub(super) fn materialize(&self, tree: &GitTreeOid, into: &Path) -> Result<TreeIdentity, String> {
        materialize_tree(&self.project, &self.stage, &self.repository, into, tree)
    }

    fn write_tree(&self) -> Result<GitTreeOid, String> {
        let output = self.check(self.repo_command()?.arg("write-tree").output(), "write patch tree")?;
        GitTreeOid::from_write_tree(&output.stdout)
    }

    fn check(&self, result: std::io::Result<Output>, operation: &str) -> Result<Output, String> {
        succeed(result.map_err(|error| format!("{operation}: {error}"))?, operation)
    }
}

fn succeed(output: Output, operation: &str) -> Result<Output, String> {
    if output.status.success() {
        Ok(output)
    } else {
        Err(format!("{operation}: {}", String::from_utf8_lossy(&output.stderr)))
    }
}

fn with_input(mut command: Command, bytes: &[u8]) -> std::io::Result<Output> {
    use std::process::Stdio;
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let mut input = child.stdin.take().expect("piped stdin");
    // Git can reject a patch early. Drain its diagnostics concurrently rather
    // than blocking on a full pipe and losing the actual applicability result.
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || input.write_all(bytes));
        let output = child.wait_with_output()?;
        let delivery = writer.join().map_err(|_| std::io::Error::other("Git input writer failed"))?;
        if output.status.success() {
            delivery?;
        }
        Ok(output)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_order_is_typed_and_generated_id_follows_future_tail() {
        let earlier = PatchId::parse("0fff0000-0000-7000-8000-000000000000").unwrap();
        let latest = PatchId::parse("0fff0000-0000-7001-8000-000000000000").unwrap();
        assert!(earlier < latest);
        let next = PatchId::after(Some(latest)).unwrap();
        assert!(next > latest);
        assert!(next.filename() > latest.filename());
        assert!(PatchId::parse("0fff0000-0000-4001-8000-000000000000").is_err());
        assert!(PatchId::parse("0FFF0000-0000-7001-8000-000000000000").is_err());
    }
}
