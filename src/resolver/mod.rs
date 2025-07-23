use std::collections::HashMap as Map;
use std::fmt::Debug;
use std::hash::Hash;
use std::marker::PhantomData;

use petgraph::acyclic::{Acyclic, AcyclicEdgeError};
use petgraph::data::Build;
use petgraph::graph::{Graph, NodeIndex};
use petgraph::visit::{EdgeRef, IntoNodeReferences};

use crate::kinds::{IdentifierItem, IdentifierModule};

pub(crate) mod dependency_reduction;

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) struct Import<Kind, Type, Index> {
    pub(crate) exporting_module: IdentifierModule,
    pub(crate) importing_module: IdentifierModule,
    pub(crate) exporting_identifier: IdentifierItem<Kind>,
    pub(crate) imported_index: Index,
    pub(crate) kind: PhantomData<Kind>,
    pub(crate) ty: Type,
}

impl<Kind, Type, Index> Import<Kind, Type, Index> {
    pub(crate) fn exporting_module(&self) -> &IdentifierModule {
        &self.exporting_module
    }
    pub(crate) fn importing_module(&self) -> &IdentifierModule {
        &self.importing_module
    }

    pub(crate) fn exporting_identifier(&self) -> &IdentifierItem<Kind> {
        &self.exporting_identifier
    }

    pub(crate) fn imported_index(&self) -> &Index {
        &self.imported_index
    }

    pub(crate) fn ty(&self) -> &Type {
        &self.ty
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) struct Local<Kind, Type, Index, Data> {
    pub(crate) module: IdentifierModule,
    pub(crate) index: Index,
    pub(crate) kind: PhantomData<Kind>,
    pub(crate) ty: Type,
    pub(crate) data: Data,
}

impl<Kind, Type, Index, Data> Local<Kind, Type, Index, Data> {
    pub(crate) fn module(&self) -> &IdentifierModule {
        &self.module
    }

    pub(crate) fn index(&self) -> &Index {
        &self.index
    }

    pub(crate) fn ty(&self) -> &Type {
        &self.ty
    }

    pub(crate) fn data(&self) -> &Data {
        &self.data
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) struct Export<Kind, Type, Index> {
    pub(crate) module: IdentifierModule,
    pub(crate) identifier: IdentifierItem<Kind>,
    pub(crate) index: Index,
    pub(crate) kind: PhantomData<Kind>,
    pub(crate) ty: Type,
}

impl<Kind, Type, Index> Export<Kind, Type, Index> {
    pub(crate) fn module(&self) -> &IdentifierModule {
        &self.module
    }

    pub(crate) fn identifier(&self) -> &IdentifierItem<Kind> {
        &self.identifier
    }

    pub(crate) fn index(&self) -> &Index {
        &self.index
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) enum Node<Kind, Type, Index, LocalData> {
    Import(Import<Kind, Type, Index>),
    Local(Local<Kind, Type, Index, LocalData>),
    Export(Export<Kind, Type, Index>),
}

impl<Kind, Type, Index, LocalData> Node<Kind, Type, Index, LocalData> {
    pub fn as_local(&self) -> Option<&Local<Kind, Type, Index, LocalData>> {
        match self {
            Node::Local(local) => Some(local),
            Node::Import(_) | Node::Export(_) => None,
        }
    }

    pub fn as_export(&self) -> Option<&Export<Kind, Type, Index>> {
        match self {
            Node::Export(export) => Some(export),
            Node::Local(_) | Node::Import(_) => None,
        }
    }
}

impl<Kind, Type, Index, LocalData> Node<Kind, Type, Index, LocalData> {
    fn ty_(&self) -> &Type {
        match self {
            Node::Import(import) => &import.ty,
            Node::Local(local) => &local.ty,
            Node::Export(export) => &export.ty,
        }
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
enum Edge {
    /// Import ---{edge}---> Export
    Imports,
    /// Export ---{edge}---> Local
    Exports,
}

// This is only a node representing an export
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
struct GraphIndexImportOrLocal(NodeIndex);

// This is only a node representing an item / import
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
struct GraphIndexExport(NodeIndex);

#[derive(Debug, Clone)]
struct ModuleReferences<Kind, Index> {
    /// Map of the exported identifier -> export's node index
    exports: Map<IdentifierItem<Kind>, GraphIndexExport>,
    /// Map of the index -> node index
    indices: Map<Index, GraphIndexImportOrLocal>,
}

impl<Kind, Index> ModuleReferences<Kind, Index>
where
    Index: Hash + Eq,
    Kind: Hash + Eq,
{
    fn new() -> Self {
        let exports = Map::default();
        let indices = Map::default();
        Self { exports, indices }
    }

    fn add_export(&mut self, node_index: NodeIndex, export_identifier: IdentifierItem<Kind>) {
        let unique_export = self
            .exports
            .insert(export_identifier, GraphIndexExport(node_index));

        // For a single identifier the export must be unique
        debug_assert!(unique_export.is_none());
    }

    fn add_import_or_local(&mut self, index: Index, node_index: NodeIndex) {
        let unique_import_or_local = self
            .indices
            .insert(index, GraphIndexImportOrLocal(node_index));

        // The newly added item index must be unique
        debug_assert!(unique_import_or_local.is_none());
    }
}

type AcyclicDependencyGraph<Kind, Type, Index, LocalData> =
    Acyclic<Graph<Node<Kind, Type, Index, LocalData>, Edge, petgraph::Directed>>;

#[derive(Debug, Clone)]
pub(crate) struct Resolver<Kind, Type, Index, LocalData> {
    graph: AcyclicDependencyGraph<Kind, Type, Index, LocalData>,
    ref_map: Map<IdentifierModule, ModuleReferences<Kind, Index>>,
}

pub(crate) mod error {
    /// Import cycle
    ///
    /// Eg.
    /// ```wat
    /// (module "A" (import "Bf" (result i32))
    ///             (export "Af" (result i32)))
    /// (module "B" (import "Af" (result i32))
    ///             (export "Bf" (result i32)))
    /// ;; ==> Merging results in ... infinite loop ?
    /// ```
    /// Would result in a `Set { A:f:i32 -> { B:f:i64, C:f:f64 } }`.
    #[derive(Debug, Clone, Hash, PartialEq, Eq)]
    pub(crate) struct Cycles; // TODO: cycles should report information on what the breaking cycle is

    /// Types Mismatch
    ///
    /// Eg.
    /// ```wat
    /// (module "A" (export "f" (result i32)))
    /// (module "B" (import "A" "f" (result i64)))
    /// (module "C" (import "A" "f" (result f64)))
    /// ```
    /// Would result in a `Set { A:f:i32 -> { B:f:i64, C:f:f64 } }`.
    #[derive(Debug, Clone, Hash, PartialEq, Eq)]
    pub(crate) struct TypeMismatch; // TODO: type mismatch should report conflicting types

    /// Name Clashes
    ///
    /// Eg.
    /// ```wat
    /// (module "A" (export "f")) ;; (a)
    /// (module "B" (export "f")) ;; (b)
    /// ;; ==>
    /// (module "M" (export "f")) ;; (a) or (b) ?
    /// ```
    ///
    /// If no other module imports "f", then M
    /// Would result in a `Map { "f" -> { A:f, B:f } }`.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct ExportNameClash; // TODO: clashing names should be reported + module
}

struct Link {
    from: NodeIndex,
    to: NodeIndex,
    edge: Edge,
}

impl<Kind, Type, Index, LocalData> Resolver<Kind, Type, Index, LocalData>
where
    Index: Clone + Eq + Hash,
    Kind: Clone + Eq + Hash,
{
    pub(crate) fn new() -> Self {
        let graph = Acyclic::new();
        let ref_map = Map::default();
        Self { graph, ref_map }
    }

    fn get_module_ref_mut(
        &mut self,
        module: &IdentifierModule,
    ) -> &mut ModuleReferences<Kind, Index> {
        self.ref_map
            .entry(module.clone())
            .or_insert_with(ModuleReferences::new)
    }

    pub(crate) fn add_import(&mut self, import: Import<Kind, Type, Index>) {
        let index = import.imported_index.clone();
        let module = import.importing_module.clone();
        let node_index = self.graph.add_node(Node::Import(import));
        self.get_module_ref_mut(&module)
            .add_import_or_local(index, node_index);
    }

    pub(crate) fn add_local(&mut self, local: Local<Kind, Type, Index, LocalData>) {
        let index = local.index.clone();
        let module = local.module.clone();
        let node_index = self.graph.add_node(Node::Local(local));
        self.get_module_ref_mut(&module)
            .add_import_or_local(index, node_index);
    }

    pub(crate) fn add_export(&mut self, export: Export<Kind, Type, Index>) {
        // The `export.index` is used during linking,
        // not yet here (as linking is decoupled)
        let module = export.module.clone();
        let export_identifier = export.identifier.clone();
        let node_index = self.graph.add_node(Node::Export(export));
        self.get_module_ref_mut(&module)
            .add_export(node_index, export_identifier);
    }

    fn identify_links(&self) -> Vec<Link> {
        let mut links = vec![];
        // loop over all exports, link each to its import / local
        for (node_index, node) in self.graph.node_references() {
            match node {
                // An import link is made to wherever the corresponding export is
                Node::Import(import) => {
                    let import_node_index = node_index;
                    if let Some(module) = self.ref_map.get(&import.exporting_module)
                        && let Some(GraphIndexExport(export_node_index)) =
                            module.exports.get(&import.exporting_identifier)
                    {
                        links.push(Link {
                            from: import_node_index,
                            to: *export_node_index,
                            edge: Edge::Imports,
                        });
                    }
                }
                // A local is not linked to anything else, it is self-defined
                Node::Local(local) => {
                    let _ = local;
                }
                // An export link is made to wherever the corresponding definition is
                Node::Export(export) => {
                    let export_node_index = node_index;
                    #[cfg(debug_assertions)] // assert module exists
                    self.ref_map.contains_key(&export.module);
                    if let Some(module) = self.ref_map.get(&export.module) {
                        #[cfg(debug_assertions)] // assert item exists
                        module.indices.contains_key(&export.index);
                        if let Some(GraphIndexImportOrLocal(local_node_index)) =
                            module.indices.get(&export.index)
                        {
                            links.push(Link {
                                from: export_node_index,
                                to: *local_node_index,
                                edge: Edge::Exports,
                            });
                        }
                    }
                }
            }
        }
        links
    }

    pub fn link_nodes(mut self) -> Result<Linked<Kind, Type, Index, LocalData>, error::Cycles> {
        for Link { from, to, edge } in self.identify_links() {
            #[cfg(debug_assertions)] // assert no edge is doubled (over all iterations)
            debug_assert!(self.graph.find_edge(from, to).is_none());
            self.graph
                .try_add_edge(from, to, edge.clone())
                .map_err(|cycle_err| {
                    debug_assert!(matches!(cycle_err, AcyclicEdgeError::Cycle(_)));
                    error::Cycles
                })?;
        }

        Ok(Linked {
            graph: self.graph.into_inner(),
        })
    }
}

// TODO: can I turn this into an acyclic graph?
#[derive(Debug, Clone)]
pub(crate) struct Linked<Kind, Type, Index, LocalData> {
    graph: Graph<Node<Kind, Type, Index, LocalData>, Edge, petgraph::Directed>,
}

struct Mismatch {
    from: NodeIndex,
    to: NodeIndex,
}

impl<Kind, Type: Eq, Index, LocalData> Linked<Kind, Type, Index, LocalData> {
    fn type_mismatches(&self) -> Vec<Mismatch> {
        let mut mismatches = vec![];
        for edge_ref in self.graph.edge_references() {
            let index_from = edge_ref.source();
            let index_to = edge_ref.target();

            let from = self.graph.node_weight(index_from).unwrap();
            let to = self.graph.node_weight(index_to).unwrap();

            let edge = edge_ref.weight();

            let equal_type = from.ty_() == to.ty_();

            match edge {
                Edge::Imports => {
                    let index_import = index_from;
                    let index_export = index_to;

                    if !equal_type {
                        mismatches.push(Mismatch {
                            from: index_import,
                            to: index_export,
                        });
                    }
                }
                Edge::Exports => {
                    let export = from;
                    let local = to;

                    let (_, _) = (export, local);

                    // When a local is exported, only in debugging mode the type
                    // match for an export & the target is asserted
                    #[cfg(debug_assertions)]
                    debug_assert!(equal_type);
                }
            }
        }
        mismatches
    }

    pub(crate) fn type_check_mismatch_break(&mut self) {
        for Mismatch { from, to } in self.type_mismatches() {
            let edge = self.graph.find_edge(from, to);
            #[cfg(debug_assertions)]
            debug_assert!(edge.is_some());
            if let Some(edge) = edge {
                self.graph.remove_edge(edge);
            }
        }
    }

    pub(crate) fn type_check_mismatch_signal(&self) -> Result<(), error::TypeMismatch> {
        self.type_mismatches()
            .is_empty()
            .then_some(())
            .ok_or(error::TypeMismatch)
    }
}

impl<Kind, Type, Index, LocalData> Linked<Kind, Type, Index, LocalData>
where
    Kind: Hash + Eq + Clone,
{
    fn clashes(&self) -> Map<IdentifierItem<Kind>, Vec<NodeIndex>> {
        let mut exports: Map<IdentifierItem<Kind>, Vec<NodeIndex>> = Map::new();
        for (index, node) in self.graph.node_references() {
            if let Some(export_identifier) = node.as_export().map(Export::identifier) {
                exports
                    .entry(export_identifier.clone())
                    .or_default()
                    .push(index);
            }
        }
        exports.retain(|_, export| export.len() > 1);
        exports
    }

    pub(crate) fn clashing_rename(
        &mut self,
        rename_strategy: fn(&IdentifierModule, IdentifierItem<Kind>) -> IdentifierItem<Kind>,
    ) {
        let clashes = self.clashes();

        for (name, node_indices) in clashes {
            for node_index in node_indices {
                let node = self.graph.node_weight_mut(node_index).unwrap();
                #[cfg(debug_assertions)]
                debug_assert!(matches!(node, Node::Export(_)));

                if let Node::Export(export) = node {
                    let module = &export.module;
                    export.identifier = rename_strategy(module, name.clone());
                }
            }
        }

        #[cfg(debug_assertions)]
        debug_assert!(self.clashes().is_empty());
    }

    pub(crate) fn clashing_signal(&self) -> Result<(), error::ExportNameClash> {
        self.clashes()
            .is_empty()
            .then_some(())
            .ok_or(error::ExportNameClash)
    }
}
