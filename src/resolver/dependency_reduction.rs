use std::collections::{HashMap as Map, HashSet as Set};
use std::fmt::Debug;
use std::hash::Hash;

use petgraph::{Direction, prelude::*, visit::IntoNodeReferences};

use crate::kinds::IdentifierItem;
use crate::merge_options::ExportIdentifier;

use super::{Export, Import, Linked, Node};

pub(crate) type ReductionMap<Kind, Type, Index, LocalData> =
    Map<Node<Kind, Type, Index, LocalData>, Node<Kind, Type, Index, LocalData>>;

#[derive(Debug, Clone)]
pub(crate) struct ReducedDependencies<Kind, Type, Index, LocalData> {
    /// Maps each node to its reduction source (either a remaining import or a local)
    pub(crate) reduction_map: ReductionMap<Kind, Type, Index, LocalData>,

    /// The remaining imports that should be present after resolution
    pub(crate) remaining_imports: Set<Import<Kind, Type, Index>>,

    /// The remaining exports that should be present after resolution
    pub(crate) remaining_exports: Set<Export<Kind, Type, Index>>,

    /// The clashing exports that should be renamed
    pub(crate) clashing_exports: Set<ExportIdentifier<IdentifierItem<Kind>>>,
}

impl<Kind, Type, Index, LocalData> Linked<Kind, Type, Index, LocalData>
where
    Index: Clone + Eq + Hash,
    Kind: Clone + Eq + Hash,
    Type: Clone + Eq + Hash,
    LocalData: Clone + Eq + Hash,
{
    /// Find remaining imports and exports after dependency resolution
    pub(crate) fn reduce_dependencies(
        &self,
        keep_exports: Option<&Set<ExportIdentifier<IdentifierItem<Kind>>>>,
    ) -> ReducedDependencies<Kind, Type, Index, LocalData> {
        let mut remaining_imports = Set::new();
        let mut remaining_exports = Set::new();
        let mut reduction_map = Map::new();

        // Step 1: Identify sources, remaining_imports and remaining_exports
        let mut sources = Set::new(); // locals / imports

        for (node_idx, node_weight) in self.graph.node_references() {
            match node_weight {
                Node::Import(import) => {
                    let has_successors = self
                        .graph
                        .neighbors_directed(node_idx, Direction::Outgoing)
                        .next()
                        .is_some();

                    if !has_successors {
                        remaining_imports.insert(import.clone());
                        sources.insert(node_idx);
                    }
                }
                Node::Export(export) => {
                    let has_predecessors = self
                        .graph
                        .neighbors_directed(node_idx, Direction::Incoming)
                        .next()
                        .is_some();

                    if !has_predecessors {
                        remaining_exports.insert(export.clone());
                    }

                    if let Some(keep_exports) = keep_exports {
                        let identifier: ExportIdentifier<IdentifierItem<Kind>> = ExportIdentifier {
                            module: export.module().clone(), // TODO: prevent clone, use it as a ref?
                            name: export.identifier().clone(), // TODO: prevent clone, use it as a ref?
                        };
                        if keep_exports.contains(&identifier) {
                            remaining_exports.insert(export.clone());
                        }
                    }
                }
                // Locals are self-defined
                Node::Local(_) => {
                    sources.insert(node_idx);
                }
            }
        }

        // Step 2: For each node, find what it reduces to via forward traversal
        for (node_idx, node_weight) in self.graph.node_references() {
            let source = self.find_reduction_source(node_idx, &sources);
            reduction_map.insert(node_weight.clone(), source);
        }

        // Step 3: Check for clashing exports
        let clashing_exports = self.clashes();

        ReducedDependencies {
            reduction_map,
            remaining_imports,
            remaining_exports,
            clashing_exports,
        }
    }

    /// Find what a given node reduces to by following the dependency chain
    fn find_reduction_source(
        &self,
        start_idx: NodeIndex,
        sources: &Set<NodeIndex>,
    ) -> Node<Kind, Type, Index, LocalData> {
        let mut current = start_idx;

        // Follow successors until we reach a source
        loop {
            // If this is a source, we found our answer
            if sources.contains(&current) {
                return self.graph.node_weight(current).unwrap().clone();
            }

            // Find the next successor to follow
            let mut successors = self.graph.neighbors_directed(current, Direction::Outgoing);

            #[cfg(debug_assertions)]
            debug_assert_eq!(successors.clone().count(), 1);

            if let Some(successor) = successors.next() {
                current = successor;
            }
        }
    }
}

#[cfg(test)]
mod dependency_tests {
    use super::*;
    use crate::resolver::{Export, Import, Local, Resolver};
    use std::marker::PhantomData;

    type TestKind = ();
    type TestType = ();
    type TestLocalData = ();
    type TestIndexType = i32;
    type TestResolver = Resolver<TestKind, TestType, TestIndexType, TestLocalData>;
    const TEST_TYPE: TestType = ();
    const TEST_LOCAL_DATA: TestLocalData = ();

    fn create_import(
        exporting_module: &str,
        importing_module: &str,
        export_name: &str,
        index: i32,
    ) -> Import<TestKind, TestType, TestIndexType> {
        Import {
            exporting_module: exporting_module.to_string().into(),
            importing_module: importing_module.to_string().into(),
            exporting_identifier: export_name.to_string().into(),
            imported_index: index,
            kind: PhantomData,
            ty: TEST_TYPE,
        }
    }

    fn create_local(
        module: &str,
        index: i32,
    ) -> Local<TestKind, TestType, TestIndexType, TestLocalData> {
        Local {
            module: module.to_string().into(),
            index,
            kind: PhantomData,
            ty: TEST_TYPE,
            data: TEST_LOCAL_DATA,
        }
    }

    fn create_export(
        module: &str,
        export_name: &str,
        index: i32,
    ) -> Export<TestKind, TestType, TestIndexType> {
        Export {
            module: module.to_string().into(),
            identifier: export_name.to_string().into(),
            index,
            kind: PhantomData,
            ty: TEST_TYPE,
        }
    }

    #[test]
    fn test_no_dependencies() {
        // Graph structure:
        //
        // [Local]   A : @ 1                            (local)
        //    ↘
        // [Export]  A : "standalone" @ 1               (export)
        //
        // Expectations:
        // - No imports.
        // - Export is resolved.
        // - Nothing remains.

        // Test case: Module with only locals and exports (no imports)
        let mut resolver = TestResolver::new();

        resolver.add_local(create_local("A", 1));
        resolver.add_export(create_export("A", "standalone", 1));

        let linked = resolver.link_nodes().unwrap();
        linked.type_check_mismatch_signal().unwrap();
        linked.type_check_mismatch_signal().unwrap();

        let ReducedDependencies {
            remaining_imports,
            remaining_exports,
            reduction_map,
            clashing_exports,
        } = linked.reduce_dependencies(None);

        // Nothing should remain since export is backed by local
        assert!(clashing_exports.is_empty(), "No exports should clash");
        assert!(remaining_imports.is_empty(), "No imports should be present");
        assert!(remaining_exports.len() == 1, "Export should remain");

        let _ = reduction_map;
    }

    #[test]
    fn test_isolated_imports() {
        // Graph structure:
        //
        // [Import]  NonExistent → A : "func1" @ 0      (unresolved)
        // [Import]  NonExistent → A : "func2" @ 1      (unresolved)
        // [Import]  AnotherMissing → B : "func3" @ 0   (unresolved)
        //
        // Expectations:
        // - All imports remain unresolved.
        // - No exports involved.

        // Test case: Imports that have no corresponding exports anywhere
        let mut resolver = TestResolver::new();

        // Multiple imports from non-existent modules/exports
        resolver.add_import(create_import("NonExistent", "A", "func1", 0));
        resolver.add_import(create_import("NonExistent", "A", "func2", 1));
        resolver.add_import(create_import("AnotherMissing", "B", "func3", 0));

        let linked = resolver.link_nodes().unwrap();
        linked.type_check_mismatch_signal().unwrap();

        let ReducedDependencies {
            remaining_imports,
            remaining_exports,
            reduction_map,
            clashing_exports,
        } = linked.reduce_dependencies(None);

        // All imports should remain since none have exports
        assert!(clashing_exports.is_empty(), "No exports should clash");
        assert_eq!(remaining_imports.len(), 3, "All imports should remain");
        assert!(remaining_exports.is_empty(), "No exports should be present");

        let _ = reduction_map;
    }

    #[test]
    fn test_unresolved_import() {
        // Graph structure:
        //
        // [Import]  B → A : "missing" @ 0              (unresolved import)
        // [Local]   A : @ 1                            (local function)
        //    ↘
        // [Export]  A : "existing" @ 1                 (resolved export)
        //
        // Expectations:
        // - One unresolved import remains.
        // - Export is resolved via local.

        // Test case: Module A imports something that doesn't exist
        let mut resolver = TestResolver::new();

        // A imports "missing" from B, but B doesn't export it
        resolver.add_import(create_import("B", "A", "missing", 0));
        // A has a local at index 1
        resolver.add_local(create_local("A", 1));
        // A exports something from its local
        resolver.add_export(create_export("A", "existing", 1));

        let linked = resolver.link_nodes().unwrap();
        linked.type_check_mismatch_signal().unwrap();

        let ReducedDependencies {
            remaining_imports,
            remaining_exports,
            reduction_map,
            clashing_exports,
        } = linked.reduce_dependencies(None);

        // The import & export should remain
        assert!(clashing_exports.is_empty(), "No exports should clash");
        assert_eq!(remaining_imports.len(), 1, "Should have one import");
        assert_eq!(remaining_exports.len(), 1, "Export should remain local");

        let unresolved_import = remaining_imports.iter().next().unwrap();
        assert_eq!(
            unresolved_import.exporting_identifier.identifier(),
            "missing"
        );

        let _ = reduction_map;
    }

    #[test]
    fn test_unresolved_export() {
        // Graph structure:
        //
        // [Import]  B → A : "func" @ 0                 (unresolved import)
        //    ↘
        // [Export]  A : "func" @ 0                     (re-exporting import)
        //
        // Expectations:
        // - No backing export for B::func.
        // - Both import and export remain unresolved.

        let mut resolver = TestResolver::new();
        // A imports "func" from B at index 0
        resolver.add_import(create_import("B", "A", "func", 0));
        // A exports "func" from index 0 (the import)
        resolver.add_export(create_export("A", "func", 0));
        // B doesn't actually export "func"

        let linked = resolver.link_nodes().unwrap();
        linked.type_check_mismatch_signal().unwrap();

        let ReducedDependencies {
            remaining_imports,
            remaining_exports,
            reduction_map,
            clashing_exports,
        } = linked.reduce_dependencies(None);

        // Both should remain since there's no local to resolve them
        assert!(clashing_exports.is_empty(), "No exports should clash");
        assert_eq!(remaining_imports.len(), 1, "Import should remain");
        assert_eq!(remaining_exports.len(), 1, "Export should remain");

        let _ = reduction_map;
    }

    #[test]
    fn test_multiple_locals_same_module() {
        // Graph structure:
        //
        // [Local]   A : @ 1                            (func1)
        //    ↘
        // [Export]  A : "func1" @ 1                    (resolved)
        //
        // [Local]   A : @ 2                            (func2)
        //    ↘
        // [Export]  A : "func2" @ 2                    (resolved)
        //
        // [Import]  B → A : "missing" @ 0              (unresolved)
        //    ↘
        // [Export]  A : "broken" @ 0                   (re-export of missing)
        //
        // Expectations:
        // - Two exports are resolved via locals.
        // - One import and one export remain unresolved.

        // Test case: Module with multiple locals, some exports resolve, others don't
        let mut resolver = TestResolver::new();

        // A has two locals
        resolver.add_local(create_local("A", 1));
        resolver.add_local(create_local("A", 2));

        // A exports from both locals
        resolver.add_export(create_export("A", "func1", 1));
        resolver.add_export(create_export("A", "func2", 2));

        // A also has an import that doesn't resolve
        resolver.add_import(create_import("B", "A", "missing", 0));
        resolver.add_export(create_export("A", "broken", 0));

        let linked = resolver.link_nodes().unwrap();
        linked.type_check_mismatch_signal().unwrap();

        let ReducedDependencies {
            remaining_imports,
            remaining_exports,
            reduction_map,
            clashing_exports,
        } = linked.reduce_dependencies(None);

        assert!(clashing_exports.is_empty(), "No exports should clash");
        assert_eq!(remaining_imports.len(), 1, "Unresolved import remains");
        assert_eq!(remaining_exports.len(), 3, "All exporst should remain");

        let _ = reduction_map;
    }

    #[test]
    fn test_partial_resolution() {
        // Graph structure:
        //
        // [Import]  B → A : "missing" @ 0              (unresolved import)
        // [Local]   A : @ 1                            (local function)
        //    ↘
        // [Export]  A : "resolved" @ 1                 (resolved export)
        // [Export]  A : "unresolved" @ 0               (re-export of missing)
        //
        // Expectations:
        // - One unresolved import remains.
        // - One export remains (depends on unresolved import).
        // - One export is resolved via local.

        // Test case: Some imports/exports resolve, others don't
        let mut resolver = TestResolver::new();

        // A has local at index 1
        resolver.add_local(create_local("A", 1));
        // A exports "resolved" from local
        resolver.add_export(create_export("A", "resolved", 1));
        // A exports "unresolved" from import at index 0
        resolver.add_export(create_export("A", "unresolved", 0));
        // A imports "missing" from B
        resolver.add_import(create_import("B", "A", "missing", 0));

        let linked = resolver.link_nodes().unwrap();
        linked.type_check_mismatch_signal().unwrap();

        let ReducedDependencies {
            remaining_imports,
            remaining_exports,
            reduction_map,
            clashing_exports,
        } = linked.reduce_dependencies(None);

        assert!(clashing_exports.is_empty(), "No exports should clash");
        assert_eq!(remaining_imports.len(), 1, "One import should remain");
        assert_eq!(remaining_exports.len(), 2, "One export should remain");

        let _ = reduction_map;
    }

    #[test]
    fn test_with_locals() {
        // Graph structure:
        //
        // [Import]  EXTERNAL → A : "value" @ 0         (external_import)
        //    ↘
        // [Local]   A : @ 1                            (local_a)
        //    ↘
        // [Export]  A : "processed" @ 1                (export_to_b)
        //    ↘
        // [Import]  A → B : "processed" @ 2            (import_from_a)
        //    ↘
        // [Export]  B : "final" @ 2                    (final_export)
        //
        // Expectations:
        // - One external import remains (external_import).
        // - One final export remains (final_export).
        // - Internal nodes reduce to external_import transitively via local_a.

        // Test case: mix of imports, exports, and locals
        let mut resolver = TestResolver::new();

        let external_import = create_import("EXTERNAL", "A", "value", 0);
        resolver.add_import(external_import.clone());

        let local_a = create_local("A", 1);
        resolver.add_local(local_a.clone());

        let export_to_b = create_export("A", "processed", 1);
        resolver.add_export(export_to_b.clone());

        let import_from_a = create_import("A", "B", "processed", 2);
        resolver.add_import(import_from_a.clone());

        let final_export = create_export("B", "final", 2);
        resolver.add_export(final_export.clone());

        let linked = resolver.link_nodes().unwrap();
        linked.type_check_mismatch_signal().unwrap();
        let reduced_dependencies = linked.reduce_dependencies(None);

        assert!(
            reduced_dependencies.clashing_exports.is_empty(),
            "No exports should clash"
        );

        // Should have one external import and one external export
        assert_eq!(reduced_dependencies.remaining_imports.len(), 1);
        assert_eq!(reduced_dependencies.remaining_exports.len(), 1);

        let reduction_map = reduced_dependencies.reduction_map;

        // The external import should reduce to itself
        assert_eq!(
            reduction_map.get(&Node::Import(external_import.clone())),
            Some(&Node::Import(external_import.clone()))
        );

        // The local should reduce to itself
        assert_eq!(
            reduction_map.get(&Node::Local(local_a.clone())),
            Some(&Node::Local(local_a.clone()))
        );

        // The exports should reduce to their source definition
        assert_eq!(
            reduction_map.get(&Node::Export(export_to_b.clone())),
            Some(&Node::Local(local_a.clone()))
        );

        assert_eq!(
            reduction_map.get(&Node::Export(final_export.clone())),
            Some(&Node::Local(local_a.clone()))
        );

        // The resolved import should reduce to the local
        assert_eq!(
            reduction_map.get(&Node::Import(import_from_a.clone())),
            Some(&Node::Local(local_a.clone()))
        );
    }

    #[test]
    fn test_chain_resolution() {
        // Graph structure:
        //
        // [Local]   C : @ 1                            (local)
        //    ↘
        // [Export]  C : "base" @ 1                     (exported)
        //    ↘
        // [Import]  C → B : "base" @ 0                 (import)
        //    ↘
        // [Export]  B : "intermediate" @ 0             (exported)
        //    ↘
        // [Import]  B → A : "intermediate" @ 0         (import)
        //    ↘
        // [Export]  A : "final" @ 0                    (exported)
        //
        // Expectations:
        // - All nodes resolve through to C's local.
        // - No remaining imports or exports.

        // Test case: Long chain A -> B -> C -> local
        let mut resolver = TestResolver::new();

        // C has local at index 1
        resolver.add_local(create_local("C", 1));
        // C exports "base" from local
        resolver.add_export(create_export("C", "base", 1));

        // B imports "base" from C at index 0
        resolver.add_import(create_import("C", "B", "base", 0));
        // B exports "intermediate" from imported index
        resolver.add_export(create_export("B", "intermediate", 0));

        // A imports "intermediate" from B at index 0
        resolver.add_import(create_import("B", "A", "intermediate", 0));
        // A exports "final" from imported index
        resolver.add_export(create_export("A", "final", 0));

        let linked = resolver.link_nodes().unwrap();
        linked.type_check_mismatch_signal().unwrap();

        let ReducedDependencies {
            remaining_imports,
            remaining_exports,
            reduction_map,
            clashing_exports,
        } = linked.reduce_dependencies(None);

        // Everything should resolve since it traces back to C's local
        assert!(clashing_exports.is_empty(), "No exports should clash");
        assert!(remaining_imports.is_empty(), "All imports should resolve");
        assert_eq!(remaining_exports.len(), 1, "The final export remains");

        let _ = reduction_map;
    }

    #[test]
    fn test_chain_of_imports() {
        // Graph structure:
        //
        //     FOREIGN::A (import)
        //         ↓
        //     A::a (export)
        //         ↓
        //     B::b (export of imported A::a)
        //         ↓
        //     C::c (export of imported B::b)
        //
        // Node Types:
        // - Imports:   "from_module" → "to_module" : identifier @ index
        // - Exports:   "module" : identifier @ index (points to source index in same module)
        //
        // Chain:
        // [Import]  FOREIGN → A : "foreign" @ 0      (import_foreign)
        //    ↘
        // [Export]  A : "a" @ 0                       (export_a)
        //    ↘
        // [Import]  A → B : "a" @ 1                  (import_a)
        //    ↘
        // [Export]  B : "b" @ 1                       (export_b)
        //    ↘
        // [Import]  B → C : "b" @ 2                  (import_b)
        //    ↘
        // [Export]  C : "c" @ 2                       (export_foreign)
        //
        // Expectations:
        // - Only the initial import (import_foreign) remains unresolved (entry point from outside).
        // - Only the final export (export_foreign) remains with no consumers (exit point).
        // - All internal nodes transitively reduce to the foreign import.

        // Test case: a long chain of imports
        let mut resolver = TestResolver::new();

        let a_index = 0;
        let import_foreign = create_import("FOREIGN", "A", "foreign", a_index);
        resolver.add_import(import_foreign.clone());
        let export_a = create_export("A", "a", a_index);
        resolver.add_export(export_a.clone());

        let b_index = 1;
        let import_a = create_import("A", "B", "a", b_index);
        resolver.add_import(import_a.clone());
        let export_b = create_export("B", "b", b_index);
        resolver.add_export(export_b.clone());

        let c_index = 2;
        let import_b = create_import("B", "C", "b", c_index);
        resolver.add_import(import_b.clone());
        let export_foreign = create_export("C", "c", c_index);
        resolver.add_export(export_foreign.clone());

        let linked = resolver.link_nodes().unwrap();
        linked.type_check_mismatch_signal().unwrap();
        let reduced_dependencies = linked.reduce_dependencies(None);

        assert!(
            reduced_dependencies.clashing_exports.is_empty(),
            "No exports should clash"
        );

        // Verify the external boundary
        assert_eq!(reduced_dependencies.remaining_imports.len(), 1);
        assert_eq!(reduced_dependencies.remaining_exports.len(), 1);
        assert!(
            reduced_dependencies
                .remaining_imports
                .contains(&import_foreign)
        );
        assert!(
            reduced_dependencies
                .remaining_exports
                .contains(&export_foreign)
        );

        // Verify the reduction mapping
        let reduction_map = reduced_dependencies.reduction_map;

        // All internal imports should reduce to the foreign import
        assert_eq!(
            reduction_map.get(&Node::Import(import_foreign.clone())),
            Some(&Node::Import(import_foreign.clone()))
        );
        assert_eq!(
            reduction_map.get(&Node::Import(import_a.clone())),
            Some(&Node::Import(import_foreign.clone()))
        );
        assert_eq!(
            reduction_map.get(&Node::Import(import_b.clone())),
            Some(&Node::Import(import_foreign.clone()))
        );

        // All exports should also trace back to the foreign import
        assert_eq!(
            reduction_map.get(&Node::Export(export_a.clone())),
            Some(&Node::Import(import_foreign.clone()))
        );
        assert_eq!(
            reduction_map.get(&Node::Export(export_b.clone())),
            Some(&Node::Import(import_foreign.clone()))
        );
        assert_eq!(
            reduction_map.get(&Node::Export(export_foreign.clone())),
            Some(&Node::Import(import_foreign.clone()))
        );
    }

    #[test]
    fn test_circular_dependency_resolution() {
        // Graph structure:
        //
        // [Import]  B → A : "ind_fib" @ 0              (import into A)
        //    ↘
        // [Local]   A : @ 1                            (local function)
        //    ↘
        // [Export]  A : "fib" @ 1                      (exports fib)
        //    ↘
        // [Import]  A → B : "fib" @ 0                  (import into B)
        //    ↘
        // [Export]  B : "ind_fib" @ 0                  (exported from B)
        //
        // Expectations:
        // - Circular dependency, but resolvable due to A's local.
        // - All imports and exports are resolved.

        // Test case from your original example:
        // Module A imports "ind_fib" from B, exports "fib"
        // Module B imports "fib" from A, exports "ind_fib"
        // Both should resolve since A has a local for "fib"

        let mut resolver = TestResolver::new();

        // A imports ind_fib from B at index 0
        resolver.add_import(create_import("B", "A", "ind_fib", 0));
        // A has local function at index 1
        resolver.add_local(create_local("A", 1));
        // A exports fib from index 1
        resolver.add_export(create_export("A", "fib", 1));
        // B imports fib from A at index 0
        resolver.add_import(create_import("A", "B", "fib", 0));
        // B exports ind_fib from index 0
        resolver.add_export(create_export("B", "ind_fib", 0));

        let linked = resolver.link_nodes().unwrap();
        linked.type_check_mismatch_signal().unwrap();

        let ReducedDependencies {
            remaining_imports,
            remaining_exports,
            reduction_map,
            clashing_exports,
        } = linked.reduce_dependencies(None);

        assert!(clashing_exports.is_empty(), "No exports should clash");

        // Both imports should be resolved since they eventually trace back to A's local
        assert!(
            remaining_imports.is_empty(),
            "Expected all imports to be resolved"
        );
        assert!(
            remaining_exports.is_empty(),
            "Expected all exports to be resolved"
        );

        let _ = reduction_map;
    }
}
