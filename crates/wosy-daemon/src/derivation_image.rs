//! Fence persisted compiler derivations by the exact local process image.
//! Source identities and ordinary compiler acceptance do not depend on it.
use crate::SourceStore;
use std::sync::OnceLock;
use wosy_protocol::NodeId;

impl SourceStore {
    pub(super) fn derivation_image(&self) -> Option<NodeId> {
        #[cfg(test)]
        if self.test_without_derivation_image { return None; }

        static IMAGE: OnceLock<Option<NodeId>> = OnceLock::new();
        *IMAGE.get_or_init(current_image)
    }
}

#[cfg(target_os = "linux")]
fn current_image() -> Option<NodeId> {
    // Open the running executable's inode, not its replaceable pathname or
    // a version label. One bounded-buffer stream on the process's first
    // derived lookup or retention census.
    let file = std::fs::File::open("/proc/self/exe").ok()?;
    wosy_protocol::compiler_image_digest(file).ok()
}

#[cfg(not(target_os = "linux"))]
fn current_image() -> Option<NodeId> {
    // No portable claim that a path denotes the process's loaded image.
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use surrealkv::{LSMIterator, Mode};
    use wosy_project::PackageRelativePath;
    use wosy_protocol::FileKey;

    fn records(store: &SourceStore) -> usize {
        let tx = store.tree.begin_with_mode(Mode::ReadOnly).unwrap();
        let mut rows = tx.range([0].as_slice(), [255].as_slice()).unwrap();
        rows.seek_first().unwrap();
        let mut count = 0;
        while rows.valid() {
            count += 1;
            rows.next().unwrap();
        }
        count
    }

    #[test]
    #[ignore = "requires the owned release fixture store and accepted root"]
    fn without_image_recomputes_without_persisting() {
        let store = PathBuf::from(std::env::var_os("WOSY_TEST_IMAGE_STORE").expect("owned source fixture"));
        let root = NodeId::parse_hex(&std::env::var("WOSY_TEST_IMAGE_ROOT").unwrap()).unwrap();
        let profile = "dev";
        let source = FileKey {
            package: "app".to_owned(),
            path: PackageRelativePath::new("src/main.w".into()).unwrap(),
        };
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
            let mut store = SourceStore::open(&store, 8 << 20).unwrap();
            store.test_without_derivation_image = true;
            let initial_records = records(&store);
            let initial_inventory = store.inspect_source_retention(root, &[], &[], 10_000).unwrap();
            let expected_body = NodeId::parse_hex(&std::env::var("WOSY_TEST_IMAGE_ENTRY_BODY").unwrap()).unwrap();
            for _ in 0..2 {
                let base = store.base_declarations(root, &source).await.unwrap().unwrap();
                assert_eq!(base.id, expected_body);
                assert_eq!(base.disposition, crate::QueryDisposition::Derived);
                assert!(base.authored_diagnostics().is_empty());
                let project = store.rooted_prepared_project(root, profile).await.unwrap().unwrap();
                assert!(project.prepared().is_some());
                let analysis = project.build_analysis().await.unwrap();
                assert_eq!(analysis.origin, crate::BuildAnalysisOrigin::Derived);
                assert!(!analysis.receipt_written && analysis.diagnostics.is_empty());
                assert_eq!(project.declarations().report.modules_reused, 0);
                assert_eq!(project.declarations().report.modules_derived, 3);
                assert_eq!(project.declarations().modules()[0].body, expected_body);
            }
            assert_eq!(records(&store), initial_records);
            assert_eq!(store.inspect_source_retention(root, &[], &[], 10_000).unwrap(), initial_inventory);
            store.close().await.unwrap();
        });
    }
}
