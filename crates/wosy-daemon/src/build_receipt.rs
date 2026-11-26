//! Optional storage for ordinary Build diagnostics, never a Build result.
use super::{child_edge, key, write_edge, write_node, BuildAnalysisPhase, Result, SourceStore, NODES};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use surrealkv::{Durability, LSMIterator, Mode, Transaction};
use wosy_compiler::Diagnostic;
use wosy_protocol::{Kind, Node, NodeId};

const BUILD_RECEIPTS: u8 = 15;
const ABI: &str = "wosy ordinary rooted build diagnostic receipt v1";
const MAX_DATA: usize = 64 * 1024;

#[derive(Clone, Copy)]
pub(super) struct BuildInputs {
    pub config: NodeId,
    pub executable: NodeId,
    pub dependencies: NodeId,
}

impl BuildInputs {
    pub fn ids(self) -> [NodeId; 3] {
        [self.config, self.executable, self.dependencies]
    }

    pub fn scope(self, image: NodeId) -> NodeId {
        let mut hash = blake3::Hasher::new_derive_key("wosy build diagnostic receipt attachment v1");
        hash.update(&image.0);
        for id in self.ids() { hash.update(&id.0); }
        NodeId(*hash.finalize().as_bytes())
    }
}

pub(super) struct Lookup {
    pub digest: NodeId,
    scope: Option<NodeId>,
    children: [NodeId; 4],
}

impl Lookup {
    pub fn new(image: Option<NodeId>, inputs: BuildInputs, projection: NodeId, preparation: NodeId) -> Self {
        let scope = image.map(|image| inputs.scope(image));
        let mut hash = blake3::Hasher::new_derive_key(ABI);
        if let Some(scope) = scope { hash.update(&scope.0); }
        hash.update(&preparation.0);
        let [config, executable, dependencies] = inputs.ids();
        Self {
            digest: NodeId(*hash.finalize().as_bytes()), scope,
            children: [config, executable, dependencies, projection],
        }
    }

    fn index_key(&self) -> Option<[u8; 65]> {
        let mut result = [0; 65];
        result[..33].copy_from_slice(&key(BUILD_RECEIPTS, self.scope?));
        result[33..].copy_from_slice(&self.digest.0);
        Some(result)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Payload<D> {
    phase: BuildAnalysisPhase,
    diagnostics: D,
}

struct BoundedPayload(Vec<u8>);

impl Write for BoundedPayload {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_DATA - self.0.len() {
            return Err(io::Error::other("diagnostic receipt exceeds inline storage"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

impl SourceStore {
    pub(super) fn read_build_receipt(
        &self, lookup: &Lookup, phase: BuildAnalysisPhase,
    ) -> Result<Option<Vec<Diagnostic>>> {
        let Some(index) = lookup.index_key() else { return Ok(None); };
        let tx = self.tree.begin_with_mode(Mode::ReadOnly)?;
        let Some(pointer) = tx.get(&index)? else { return Ok(None); };
        let Some(receipt) = stored_receipt(&tx, &pointer, lookup.digest, &lookup.children[..3])? else {
            return Ok(None);
        };
        Ok((receipt.payload.phase == phase && receipt.projection == lookup.children[3])
            .then_some(receipt.payload.diagnostics))
    }

    pub(super) async fn persist_build_receipt(
        &self, lookup: &Lookup, phase: BuildAnalysisPhase, diagnostics: &[Diagnostic],
    ) -> Result<bool> {
        let Some(index) = lookup.index_key() else { return Ok(false); };
        let mut data = BoundedPayload(lookup.digest.0.to_vec());
        if serde_json::to_writer(&mut data, &Payload { phase, diagnostics }).is_err() { return Ok(false); }
        let node = Node {
            kind: Kind::BuildDiagnosticReceipt, data: data.0, children: lookup.children.to_vec(),
        };
        let bytes = node.encode();
        let id = Node::id(&bytes);
        loop {
            let mut tx = self.tree.begin()?;
            tx.set_durability(Durability::Eventual);
            if tx.get(&index)?.is_some_and(|old| old.as_slice() == id.0)
                && tx.get(&key(NODES, id))?.is_some_and(|old| old.as_slice() == bytes)
            {
                return Ok(false);
            }
            write_node(&mut tx, id, &bytes)?;
            for (i, &child) in node.children.iter().enumerate() {
                write_edge(&mut tx, id, child_edge(node.kind, i).0, child)?;
            }
            tx.set(&index, &id.0)?;
            match tx.commit().await {
                Ok(()) => return Ok(true),
                Err(surrealkv::Error::TransactionWriteConflict) => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }
}

pub(super) fn attached(
    tx: &Transaction, scope: NodeId, inputs: BuildInputs, remaining: &mut usize,
    mut visit: impl FnMut(NodeId, NodeId, usize) -> Result<()>,
) -> Result<()> {
    let prefix = key(BUILD_RECEIPTS, scope);
    let mut rows = tx.range(prefix.as_slice(), [BUILD_RECEIPTS + 1].as_slice())?;
    rows.seek_first()?;
    while rows.valid() && rows.key().user_key().starts_with(&prefix) {
        *remaining = remaining.checked_sub(1).ok_or("source retention plan exceeded attached receipt budget")?;
        let index = rows.key();
        if let Ok(digest) = index.user_key()[prefix.len()..].try_into().map(NodeId) {
            if let Some(receipt) = stored_receipt(tx, &rows.value()?, digest, &inputs.ids())? {
                visit(receipt.id, receipt.projection, receipt.encoded_bytes)?;
            }
        }
        rows.next()?;
    }
    Ok(())
}

struct StoredReceipt {
    id: NodeId,
    projection: NodeId,
    encoded_bytes: usize,
    payload: Payload<Vec<Diagnostic>>,
}

fn stored_receipt(
    tx: &Transaction, pointer: &[u8], digest: NodeId, inputs: &[NodeId],
) -> Result<Option<StoredReceipt>> {
    let Ok(id) = pointer.try_into().map(NodeId) else { return Ok(None); };
    let Some(bytes) = tx.get(&key(NODES, id))? else { return Ok(None); };
    if bytes.len() > 9 + MAX_DATA + 4 * 32 || Node::id(&bytes) != id { return Ok(None); }
    let Some(node) = Node::decode(&bytes) else { return Ok(None); };
    if node.kind != Kind::BuildDiagnosticReceipt || node.children.len() != 4 || node.children[..3] != *inputs {
        return Ok(None);
    }
    let Some(body) = node.data.strip_prefix(&digest.0) else { return Ok(None); };
    let Ok(payload) = serde_json::from_slice(body) else { return Ok(None); };
    Ok(Some(StoredReceipt { id, projection: node.children[3], encoded_bytes: bytes.len(), payload }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_receipt_budget() {
        let mut out = BoundedPayload([1; 32].to_vec());
        out.write_all(&vec![2; MAX_DATA - 32]).unwrap();
        assert!(out.write_all(&[3]).is_err());
        assert_eq!(out.0.len(), MAX_DATA);
    }

    fn store_fixture() -> (std::path::PathBuf, NodeId) {
        let dir = std::path::PathBuf::from(
            std::env::var_os("WOSY_TEST_IMAGE_STORE").expect("owned fixture store")
        );
        let root = NodeId::parse_hex(&std::env::var("WOSY_TEST_IMAGE_ROOT").unwrap()).unwrap();
        (dir, root)
    }

    fn inputs(store: &SourceStore, root: NodeId) -> BuildInputs {
        let admitted = store.snapshot(root).unwrap().unwrap();
        BuildInputs {
            config: admitted.config, executable: admitted.executable, dependencies: admitted.dependencies,
        }
    }

    #[test]
    #[ignore = "requires a disposable store and an admitted release fixture root"]
    fn damaged_receipt_value_rederives() {
        use crate::BuildAnalysisOrigin;
        let (dir, root) = store_fixture();
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
            let store = SourceStore::open(&dir, 8 << 20).unwrap();
            let image = store.derivation_image().expect("verified image for this fixture");
            let inputs = inputs(&store, root);
            let cold = store.rooted_prepared_project(root, "dev").await.unwrap().unwrap();
            let expected = cold.build_analysis().await.unwrap();
            assert_eq!(expected.origin, BuildAnalysisOrigin::Derived);
            assert!(expected.receipt_written);
            let lookup = Lookup::new(Some(image), inputs, cold.declarations().entry_projection, cold.identity());
            let sibling = cold.declarations().modules()[1].projection;
            assert_ne!(lookup.children[3], sibling);
            let index = lookup.index_key().unwrap();
            let node_id = {
                let tx = store.tree.begin_with_mode(Mode::ReadOnly).unwrap();
                NodeId(tx.get(&index).unwrap().unwrap().as_slice().try_into().unwrap())
            };
            let node_key = key(NODES, node_id);
            drop(cold);
            assert_eq!(store.rooted_prepared_project(root, "dev").await.unwrap().unwrap()
                .build_analysis().await.unwrap().origin, BuildAnalysisOrigin::Stored);
            let before = store.plan_source_retention(root, &[], &[], 20_000).unwrap().nodes_visited;
            // Simulate an otherwise unavailable optional value by replacing
            // only that owned fixture's receipt with incomplete stored bytes.
            let mut damaged = store.tree.begin().unwrap();
            damaged.set(&node_key, &[0]).unwrap();
            damaged.commit().await.unwrap();
            assert_eq!(store.plan_source_retention(root, &[], &[], 20_000).unwrap().nodes_visited, before - 1);
            let recovered = store.rooted_prepared_project(root, "dev").await.unwrap().unwrap()
                .build_analysis().await.unwrap();
            assert_eq!(recovered.origin, BuildAnalysisOrigin::Derived);
            assert!(recovered.receipt_written);
            assert_eq!((recovered.key, recovered.phase, recovered.diagnostics),
                       (expected.key, expected.phase, expected.diagnostics.clone()));
            let warm = store.rooted_prepared_project(root, "dev").await.unwrap().unwrap()
                .build_analysis().await.unwrap();
            assert_eq!(warm.origin, BuildAnalysisOrigin::Stored);
            assert_eq!(warm.diagnostics, expected.diagnostics);
            assert_eq!(store.plan_source_retention(root, &[], &[], 20_000).unwrap().nodes_visited, before);

            // A dangling projection is not a source graph authority. A real
            // reachable sibling is also not this scope's exact entry.
            let original = {
                let tx = store.tree.begin_with_mode(Mode::ReadOnly).unwrap();
                Node::decode(&tx.get(&node_key).unwrap().unwrap()).unwrap()
            };
            for detached_projection in [NodeId([17; 32]), sibling] {
                let mut detached = original.clone();
                detached.children[3] = detached_projection;
                let bytes = detached.encode();
                let id = Node::id(&bytes);
                let mut tx = store.tree.begin().unwrap();
                tx.set(&key(NODES, id), &bytes).unwrap();
                tx.set(&index, &id.0).unwrap();
                tx.commit().await.unwrap();
                assert_eq!(store.plan_source_retention(root, &[], &[], 20_000).unwrap().nodes_visited, before - 1);
                assert_eq!(store.rooted_prepared_project(root, "dev").await.unwrap().unwrap()
                    .build_analysis().await.unwrap().origin, BuildAnalysisOrigin::Derived);
            }

            use wosy_compiler::{DiagnosticLabel, DiagnosticLabelKind, DiagnosticSeverity};
            use wosy_syntax::{ByteSpan, SourceIdentity, SourceSpan};
            let unrelated = Diagnostic {
                code: "fixture".into(), severity: DiagnosticSeverity::Warning, message: "unrelated".into(),
                labels: vec![DiagnosticLabel {
                    kind: DiagnosticLabelKind::Primary, message: "foreign owner".into(),
                    span: SourceSpan::new(
                        SourceIdentity::new("fixture-other".into(), "fixture-other".into(), "src/main.w".into(), "none".into()),
                        ByteSpan::new(0, 0),
                    ),
                }],
                notes: Vec::new(),
            };
            let mut foreign = original;
            foreign.data.truncate(32);
            serde_json::to_writer(&mut foreign.data, &Payload { phase: expected.phase, diagnostics: &[unrelated] }).unwrap();
            let bytes = foreign.encode();
            let id = Node::id(&bytes);
            let mut tx = store.tree.begin().unwrap();
            tx.set(&key(NODES, id), &bytes).unwrap();
            tx.set(&index, &id.0).unwrap();
            tx.commit().await.unwrap();
            let warm = store.rooted_prepared_project(root, "dev").await.unwrap().unwrap()
                .build_analysis().await.unwrap();
            assert_eq!(warm.origin, BuildAnalysisOrigin::Derived);
            assert_eq!(warm.diagnostics, expected.diagnostics);
            store.close().await.unwrap();
        });
    }
    #[test]
    #[ignore = "requires a disposable store and two admitted release fixture scopes"]
    fn census_shares_attachment_budget() {
        let (dir, root) = store_fixture();
        let other = NodeId::parse_hex(&std::env::var("WOSY_TEST_IMAGE_OTHER_ROOT").unwrap()).unwrap();
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
            let store = SourceStore::open(&dir, 8 << 20).unwrap();
            let image = store.derivation_image().expect("verified image for this fixture");
            let root_inputs = inputs(&store, root);
            let other_inputs = inputs(&store, other);
            let root_scope = root_inputs.scope(image);
            let other_scope = other_inputs.scope(image);
            assert_ne!(root_scope, other_scope);

            let derived = store.rooted_prepared_project(root, "dev").await.unwrap().unwrap();
            assert!(derived.build_analysis().await.unwrap().receipt_written);
            let lookup = Lookup::new(Some(image), root_inputs, derived.declarations().entry_projection, derived.identity());
            let index = lookup.index_key().unwrap();
            drop(derived);
            let pointer = {
                let tx = store.tree.begin_with_mode(Mode::ReadOnly).unwrap();
                tx.get(&index).unwrap().unwrap()
            };
            store.rooted_prepared_project(other, "dev").await.unwrap().unwrap();
            let limit = store.plan_source_retention(root, &[], &[], 20_000).unwrap().nodes_visited;
            let mut tx = store.tree.begin().unwrap();
            let mut wrong_index = index;
            wrong_index[..33].copy_from_slice(&key(BUILD_RECEIPTS, other_scope));
            tx.set(&wrong_index, &pointer).unwrap();
            let mut wrong_digest = index;
            wrong_digest[33..].copy_from_slice(&[17; 32]);
            tx.set(&wrong_digest, &pointer).unwrap();
            tx.commit().await.unwrap();

            // Both stale indices point to an existing typed record. Neither
            // can hold it for a different digest or a different owner scope.
            let read = store.tree.begin_with_mode(Mode::ReadOnly).unwrap();
            let mut remaining = 5;
            let mut found = 0;
            attached(&read, root_scope, root_inputs, &mut remaining, |_, _, _| {
                found += 1;
                Ok(())
            }).unwrap();
            attached(&read, other_scope, other_inputs, &mut remaining, |_, _, _| {
                found += 1;
                Ok(())
            }).unwrap();
            assert_eq!((found, remaining), (1, 2));
            drop(read);
            assert_eq!(store.plan_source_retention(root, &[], &[], limit).unwrap().nodes_visited, limit);

            // Each scope fits separately, but rejected rows also consume the
            // one aggregate budget shared by the real rooted census.
            let mut tx = store.tree.begin().unwrap();
            for scope in [root_scope, other_scope] {
                let mut index = [0; 65];
                index[..33].copy_from_slice(&key(BUILD_RECEIPTS, scope));
                for n in 0..limit / 2 {
                    let digest = blake3::derive_key("fixture retired receipt", &(n as u64).to_be_bytes());
                    index[33..].copy_from_slice(&digest);
                    tx.set(&index, &[]).unwrap();
                }
            }
            tx.commit().await.unwrap();
            let error = store.plan_source_retention(root, &[], &[], limit).unwrap_err();
            assert!(error.to_string().contains("exceeded attached receipt budget"));
            store.close().await.unwrap();
        });
    }

}
