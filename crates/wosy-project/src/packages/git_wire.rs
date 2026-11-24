use crate::PackageRelativePath;
use std::path::PathBuf;

/// An exact Git object ID, not a ref, abbreviation, or diagnostic string.
#[derive(Clone, Debug, Eq, PartialEq)]
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

    /// Machine output of rev-parse --verify <rev>^{commit}.
    pub(super) fn from_rev_parse(bytes: &[u8]) -> Result<Self, String> {
        let line = bytes.strip_suffix(b"\n").ok_or_else(|| "missing Git object record terminator".to_owned())?;
        Self::parse(std::str::from_utf8(line).map_err(|_| "Git object ID is not ASCII".to_owned())?)
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

pub(super) struct Blob {
    pub(super) oid: GitOid,
    pub(super) path: PackageRelativePath,
    pub(super) executable: bool,
}

/// Decode git ls-tree -r -z --full-tree records. The mode and type describe the
/// Git object, not incidental spelling in Git's human-readable diagnostics.
pub(super) fn blobs(bytes: &[u8]) -> Result<impl Iterator<Item = Result<Blob, String>> + '_, String> {
    let records = if bytes.is_empty() {
        bytes
    } else {
        bytes.strip_suffix(b"\0").ok_or_else(|| "missing Git tree record terminator".to_owned())?
    };
    Ok(records.split(|byte| *byte == 0).take(if bytes.is_empty() { 0 } else { usize::MAX }).map(blob))
}

fn blob(record: &[u8]) -> Result<Blob, String> {
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
    let executable = match (mode, object) {
        (b"100644", b"blob") => false,
        (b"100755", b"blob") => true,
        _ => return Err(format!("unsupported Git tree entry {path}: expected a regular file")),
    };
    Ok(Blob {
        oid: GitOid::parse(std::str::from_utf8(oid).map_err(|_| "Git object ID is not ASCII".to_owned())?)?,
        path: PackageRelativePath::new(PathBuf::from(path))?,
        executable,
    })
}
