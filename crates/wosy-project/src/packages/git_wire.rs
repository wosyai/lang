use crate::PackageRelativePath;
use std::path::PathBuf;

/// An exact Git object ID, not a ref, abbreviation, or diagnostic string.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct GitOid(String);

impl GitOid {
    pub(super) fn parse(text: &str) -> Result<Self, String> {
        if matches!(text.len(), 40 | 64)
            && text.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            Ok(Self(text.to_owned()))
        } else {
            Err("expected an exact Git object ID".to_owned())
        }
    }

    /// The exact object-ID line from Git's plumbing commands.
    pub(super) fn from_rev_parse(bytes: &[u8]) -> Result<Self, String> {
        let line = bytes.strip_suffix(b"\n").ok_or_else(|| "missing Git object record terminator".to_owned())?;
        Self::parse(std::str::from_utf8(line).map_err(|_| "Git object ID is not ASCII".to_owned())?)
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A tree object, not merely a tree-ish value (which could be a commit).
#[derive(Clone)]
pub(super) struct GitTreeOid(GitOid);

impl GitTreeOid {
    pub(super) fn from_cat_file_type(oid: GitOid, record: &[u8]) -> Result<Self, String> {
        // cat-file -t reports the stored object's type, unlike ls-tree's mode-derived type.
        if record != b"tree\n" {
            return Err(format!("Git object {} is not a tree", oid.as_str()));
        }
        Ok(Self(oid))
    }

    pub(super) fn from_write_tree(record: &[u8]) -> Result<Self, String> {
        GitOid::from_rev_parse(record).map(Self)
    }

    pub(super) fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

pub(super) struct Blob {
    pub(super) oid: GitOid,
    pub(super) path: PackageRelativePath,
    pub(super) executable: bool,
}

enum NodeKind {
    File { executable: bool },
    Directory,
    Symlink,
    Gitlink,
}

struct Node {
    oid: GitOid,
    path: PackageRelativePath,
    kind: NodeKind,
}

/// Decode git ls-tree -r -z --full-tree records. The mode and type describe the
/// Git object, not incidental spelling in Git's human-readable diagnostics.
pub(super) fn blobs(bytes: &[u8]) -> Result<impl Iterator<Item = Result<Blob, String>> + '_, String> {
    Ok(nodes(bytes)?.map(|record| {
        let Node { oid, path, kind } = record?;
        let NodeKind::File { executable } = kind else {
            return Err(format!("unsupported Git tree entry {}: expected a regular file", path.as_path().display()));
        };
        Ok(Blob { oid, path, executable })
    }))
}

pub(super) fn selected_tree(bytes: &[u8], path: &std::path::Path) -> Result<GitOid, String> {
    let mut records = nodes(bytes)?;
    let failure = || format!("{} is not a package directory in the Git tree", path.display());
    let node = records.next().ok_or_else(failure)??;
    if !matches!(node.kind, NodeKind::Directory) || node.path.as_path() != path || records.next().is_some() {
        return Err(failure());
    }
    Ok(node.oid)
}

fn nodes(bytes: &[u8]) -> Result<impl Iterator<Item = Result<Node, String>> + '_, String> {
    let records = if bytes.is_empty() {
        bytes
    } else {
        bytes.strip_suffix(b"\0").ok_or_else(|| "missing Git tree record terminator".to_owned())?
    };
    Ok(records.split(|byte| *byte == 0).take(if bytes.is_empty() { 0 } else { usize::MAX }).map(node))
}

fn node(record: &[u8]) -> Result<Node, String> {
    let tab = record.iter().position(|byte| *byte == b'\t')
        .ok_or_else(|| "invalid Git tree record".to_owned())?;
    let (header, path) = record.split_at(tab);
    let path = std::str::from_utf8(&path[1..]).map_err(|_| "Git package path is not UTF-8".to_owned())?;
    let mut fields = header.split(|byte| *byte == b' ');
    let (Some(mode), Some(object), Some(oid), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return Err("invalid Git tree record header".to_owned());
    };
    let kind = match (mode, object) {
        (b"100644", b"blob") => NodeKind::File { executable: false },
        (b"100755", b"blob") => NodeKind::File { executable: true },
        (b"040000", b"tree") => NodeKind::Directory,
        (b"120000", b"blob") => NodeKind::Symlink,
        (b"160000", b"commit") => NodeKind::Gitlink,
        _ => return Err(format!("unsupported Git tree entry {path}")),
    };
    Ok(Node {
        oid: GitOid::parse(std::str::from_utf8(oid).map_err(|_| "Git object ID is not ASCII".to_owned())?)?,
        path: PackageRelativePath::new(PathBuf::from(path))?,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selected_package_node_must_be_a_native_tree() {
        let path = std::path::Path::new("packages/leaf_lib");
        let oid = "0123456789abcdef0123456789abcdef01234567";
        for (mode, kind) in [("100644", "blob"), ("120000", "blob"), ("160000", "commit")] {
            let record = format!("{mode} {kind} {oid}\tpackages/leaf_lib\0");
            assert!(selected_tree(record.as_bytes(), path).is_err());
        }
        let record = format!("040000 tree {oid}\tpackages/leaf_lib\0");
        assert_eq!(selected_tree(record.as_bytes(), path).unwrap().as_str(), oid);
    }

    #[test]
    fn cat_file_proves_an_actual_tree_with_one_exact_type_record() {
        let oid = GitOid::parse("0123456789abcdef0123456789abcdef01234567").unwrap();
        for not_a_tree in [b"blob\n".as_slice(), b"commit\n", b"tag\n", b"tree", b"tree\n\n"] {
            assert!(GitTreeOid::from_cat_file_type(oid.clone(), not_a_tree).is_err());
        }
        assert_eq!(
            GitTreeOid::from_cat_file_type(oid.clone(), b"tree\n").unwrap().as_str(),
            oid.as_str(),
        );
    }
}
