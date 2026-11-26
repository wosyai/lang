//! Project the selected entry through the ordinary tooling schema.
use super::{
    checked_file_key, load_usage, read_selection, AcknowledgedSourceGeneration, Result,
    SqliteSourceState, StoredNodes,
};
use rowan::GreenNodeBuilder;
use rusqlite::TransactionBehavior;
use std::io::{self, Write};
use wosy_protocol::{walk_cst, CstEvent, CstExpansionBudget, FileEntry, Kind};
use wosy_syntax::{typed_comment_from_token, CstNode, SourceIdentity, SyntaxKind, TypedComment};

impl AcknowledgedSourceGeneration {
    /// Return the exact `wosy parse` current.json bytes for this selected,
    /// successfully parsed entry, or None if the capture is no longer
    /// current here. Never read workspace source or update tooling files.
    /// Bound authored graph expansion and the JSON separately; this is
    /// neither a total allocator bound nor a lock on future selection.
    pub fn entry_cst_current_json(
        &self, local: &mut SqliteSourceState, mut budget: CstExpansionBudget, json_bytes: usize,
    ) -> Result<Option<Vec<u8>>> {
        let selected = self.selection();
        if local.project.to_str() != Some(selected.project.as_str()) { return Ok(None); }
        let tx = local.db.transaction_with_behavior(TransactionBehavior::Deferred)?;
        if !self.matches(read_selection(&tx, &selected.target)?) { return Ok(None); }
        if let Some(usage) = load_usage(&tx, selected.root)? { usage.admit(local.limits)?; }
        let mut read = StoredNodes::new(&tx)?;
        let (config, files) = SqliteSourceState::header_using(&local.project, &selected.target, selected.root,
            |id| read.charged_record(id, false, |bytes| budget.read(bytes)))?;
        if files.len() > local.limits.nodes { return Err("CST view file index exceeds the node budget".into()); }
        let policy = config.parser.id();
        let mut entry = None;
        for id in files {
            let record = read.charged_record(id, false, |bytes| budget.read(bytes))?;
            let file = FileEntry::from_node(id, record)?;
            if checked_file_key(&config, policy, &file)? == config.entry && entry.replace(file).is_some() {
                return Err("duplicate selected CST entry".into());
            }
        }
        let entry = entry.ok_or("persisted entry file is missing")?;
        let path = entry.metadata.source.path.as_path().to_str().ok_or("source path is not UTF-8")?;
        // Only the legacy tool view uses display separators; the rooted
        // lookup above compares the actual typed package-relative path.
        let display_source = SourceIdentity::new(
            selected.project.clone(), config.entry.package, path.replace('\\', "/"), entry.metadata.revision.hex(),
        );
        let (tree, comments) = read_authored_entry(&mut read, &mut budget, &entry, &config.parser.categories, json_bytes)?;
        drop(read);
        tx.commit()?;

        let projected = wosy_compiler::publication_from_cst(&display_source, &tree, &comments, &[]);
        let mut encoded = BoundedJson { bytes: Vec::new(), limit: json_bytes };
        serde_json::to_writer_pretty(&mut encoded, &projected)?;
        encoded.write_all(b"\n")?;
        Ok(self.is_current(local)?.then_some(encoded.bytes))
    }
}

fn read_authored_entry(
    read: &mut StoredNodes<'_>, budget: &mut CstExpansionBudget, entry: &FileEntry,
    configured: &[String], mut text_budget: usize,
) -> Result<(CstNode, Vec<TypedComment>)> {
    let declared_bytes = entry.metadata.bytes as usize;
    budget.file(declared_bytes)?;
    let annotation = read.charged_record(entry.annotation, false, |bytes| budget.read(bytes))?;
    if annotation.kind != Kind::Annotations || !annotation.children.is_empty() {
        return Err("invalid selected CST annotation record".into());
    }
    let comments: Vec<TypedComment> = serde_json::from_slice(&annotation.data)?;
    let mut expected = comments.iter();
    let mut green = GreenNodeBuilder::new();
    let mut revision = blake3::Hasher::new();
    let (mut offset, mut level) = (0usize, 0usize);
    let mut state = (read, budget);
    walk_cst(entry.authored, &mut state, |(read, budget), id| {
        read.charged_record(id, true, |bytes| budget.read(bytes))
    }, |(_, budget), event| {
        match event {
            CstEvent::Start { kind, depth, children } => {
                if (depth == 1 && kind != SyntaxKind::Root) || kind == SyntaxKind::Error {
                    return Err("selected CST entry is not an admitted authored tree".into());
                }
                budget.children(depth, children)?;
                level = depth;
                green.start_node(rowan::SyntaxKind(kind as u16));
            }
            CstEvent::Token { kind, text } => {
                if text.len() > declared_bytes - offset {
                    return Err("selected CST entry exceeds its authored extent".into());
                }
                // Legacy tooling JSON repeats each token's bytes in its
                // token record and each ancestor's text. Bound that owned
                // projection before constructing it, not just on egress.
                text_budget = text.len().checked_mul(level + 1).and_then(|cost| text_budget.checked_sub(cost))
                    .ok_or("CST view exceeds the JSON byte budget")?;
                if kind == SyntaxKind::TypedComment {
                    let comment = typed_comment_from_token(text, offset as u32, configured)
                        .ok_or("invalid selected typed comment token")?;
                    if expected.next() != Some(&comment) {
                        return Err("selected CST annotations do not match their authored tokens".into());
                    }
                }
                revision.update(text.as_bytes());
                offset += text.len();
                green.token(rowan::SyntaxKind(kind as u16), text);
            }
            CstEvent::Finish => {
                level -= 1;
                green.finish_node();
            }
        }
        Ok(())
    })?;
    if offset != declared_bytes || revision.finalize().as_bytes() != &entry.metadata.revision.0 || expected.next().is_some() {
        return Err("selected CST entry does not match its authored revision".into());
    }
    Ok((CstNode::new_root(green.finish()), comments))
}

struct BoundedJson {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for BoundedJson {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        if input.len() > self.limit - self.bytes.len() {
            return Err(io::Error::other("CST view exceeds the JSON byte budget"));
        }
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
