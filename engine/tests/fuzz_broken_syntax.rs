use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use lineage_engine::adapters::typescript::TypeScriptAdapter;
use lineage_engine::adapters::LanguageAdapter;
use lineage_engine::cache::{DirtyBufferOverlay, FileIrCache};
use lineage_engine::protocol::schema::{LspPosition, OffsetEncoding, SliceDirection, SliceParams};
use lineage_engine::resolver::NullLspProvider;
use lineage_engine::slicer::SlicerEngine;

#[test]
fn test_fuzz_truncated_and_mutated_typescript_never_panics() {
    let seed = r#"
        import { foo } from './bar';
        export function process(user: User | null | undefined, flag: boolean) {
            if (user !== null) {
                const name = user?.profile?.name ?? "anon";
                if (flag) {
                    return name!;
                }
            }
            return null;
        }
    "#;

    let cache = Arc::new(FileIrCache::new(128 * 1024 * 1024));
    let dirty = Arc::new(DirtyBufferOverlay::new());
    let resolver = Arc::new(NullLspProvider);
    let adapters: Vec<Arc<dyn LanguageAdapter>> = vec![Arc::new(TypeScriptAdapter::new())];
    let engine = SlicerEngine::new(cache, dirty, resolver, adapters);

    // Truncate at every byte boundary
    for end in (0..=seed.len()).step_by(3) {
        let slice = &seed[..end];
        let uri = format!("file:///fuzz/trunc_{end}.ts");
        engine
            .dirty_buffers
            .upsert(uri.clone(), 1, slice.to_string())
            .unwrap();
        let env = engine
            .slice_query(
                &SliceParams {
                    uri,
                    position: LspPosition {
                        line: 0,
                        character: 0,
                    },
                    offset_encoding: OffsetEncoding::Utf8,
                    direction: SliceDirection::To,
                    include_graph: false,
                    include_snippets: true,
                    exclude_tests: false,
                    query_id: Some(end as u64 + 1),
                    lsp_attached: false,
                    lsp_client_name: None,
                    workspace_root: None,
                },
                "fuzz",
                &AtomicBool::new(false),
                None::<fn(_)>,
            )
            .unwrap();
        assert!(env.projections.hover_card.is_some());
    }

    // Mutated garbage tokens
    let mutated_samples = [
        "function ((( { const x = ;;; if () { return",
        "export default class extends { [[[[ ??? === null",
        "const a = \"unterminated string; function f() { return null; }",
        "<div><span>{unclosed}</div>",
    ];
    for (i, sample) in mutated_samples.iter().enumerate() {
        let uri = format!("file:///fuzz/mut_{i}.tsx");
        engine
            .dirty_buffers
            .upsert(uri.clone(), 1, (*sample).to_string())
            .unwrap();
        let env = engine
            .slice_query(
                &SliceParams {
                    uri,
                    position: LspPosition {
                        line: 0,
                        character: 0,
                    },
                    offset_encoding: OffsetEncoding::Utf8,
                    direction: SliceDirection::To,
                    include_graph: false,
                    include_snippets: true,
                    exclude_tests: false,
                    query_id: Some(1000 + i as u64),
                    lsp_attached: false,
                    lsp_client_name: None,
                    workspace_root: None,
                },
                "fuzz-mut",
                &AtomicBool::new(false),
                None::<fn(_)>,
            )
            .unwrap();
        assert!(env.projections.hover_card.is_some());
    }
}
