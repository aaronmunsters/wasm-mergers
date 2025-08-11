use core::convert::From;

use std::marker::PhantomData;

use anyhow::anyhow;
use walrus::IdsToIndices;
use walrus::Module;
use walrus::ValType;
use walrus::{ConstExpr, ElementItems, ExportItem, FunctionBuilder, FunctionId};
use walrus::{DataKind, ElementKind, FunctionKind, GlobalKind, ImportKind};

pub(crate) mod old_to_new_mapping;
pub(crate) mod provenance_identifier;
mod walrus_copy;

use crate::error::Error;
use crate::kinds::{FuncType, IdentifierModule};
use crate::merge_builder::AllResolved;
use crate::merge_builder::RenameMap;
use crate::merge_builder::builder_instantiated::ReducedDependenciesFunction;
use crate::merge_builder::builder_instantiated::ReducedDependenciesGlobal;
use crate::merge_options::{IdentifierFunction, RenameStrategy};
use crate::merger::old_to_new_mapping::NewIdGlobal;
use crate::merger::old_to_new_mapping::OldIdGlobal;
use crate::named_module::NamedParsedModule;
use crate::resolver::Local;
use crate::resolver::instantiated::ImportDataFunction;
use crate::resolver::instantiated::ImportGlobal;
use crate::resolver::{Export, Import, Node};

use old_to_new_mapping::{Mapping, NewIdFunction, OldIdFunction};
use provenance_identifier::{Identifier, New, Old};

pub(crate) struct Merger {
    merged: Module,
    mapping: Mapping,
    names: Vec<(String, String)>,
    starts: Vec<FunctionId>,
    all_resolved: AllResolved,
}

trait AsOldToNewMapIndex<KindIdentifier> {
    fn to_mapping_ref(&self) -> (IdentifierModule, Identifier<Old, KindIdentifier>);
}

impl<Kind, Type, KindIdentifier, ImportData, LocalData> AsOldToNewMapIndex<KindIdentifier>
    for Node<Kind, Type, Identifier<Old, KindIdentifier>, ImportData, LocalData>
where
    Identifier<Old, KindIdentifier>: Copy,
{
    fn to_mapping_ref(&self) -> (IdentifierModule, Identifier<Old, KindIdentifier>) {
        match self {
            Node::Import(import) => (import.importing_module().clone(), *import.imported_index()),
            Node::Local(local) => (local.module().clone(), *local.index()),
            Node::Export(export) => (export.module().clone(), *export.index()),
        }
    }
}

impl<Kind, Type, KindIdentifier, ImportData> AsOldToNewMapIndex<KindIdentifier>
    for Import<Kind, Type, Identifier<Old, KindIdentifier>, ImportData>
where
    Identifier<Old, KindIdentifier>: Copy,
{
    fn to_mapping_ref(&self) -> (IdentifierModule, Identifier<Old, KindIdentifier>) {
        (self.importing_module().clone(), *self.imported_index())
    }
}

impl<Kind, Type, KindIdentifier, ImportData> AsOldToNewMapIndex<KindIdentifier>
    for Local<Kind, Type, Identifier<Old, KindIdentifier>, ImportData>
where
    Identifier<Old, KindIdentifier>: Copy,
{
    fn to_mapping_ref(&self) -> (IdentifierModule, Identifier<Old, KindIdentifier>) {
        (self.module().clone(), *self.index())
    }
}

impl<Kind, Type, KindIdentifier> AsOldToNewMapIndex<KindIdentifier>
    for Export<Kind, Type, Identifier<Old, KindIdentifier>>
where
    Identifier<Old, KindIdentifier>: Copy,
{
    fn to_mapping_ref(&self) -> (IdentifierModule, Identifier<Old, KindIdentifier>) {
        (self.module().clone(), *self.index())
    }
}

use super::resolver::instantiated::{ImportFunction, LocalFunction};

impl Merger {
    fn add_new_import_function(
        module: &mut Module,
        old_import: &ImportFunction<OldIdFunction>,
    ) -> NewIdFunction {
        let module_identifier = old_import.exporting_module().identifier();
        let name = old_import.exporting_identifier().identifier();
        let ty = old_import.ty().add_to_module(module);
        // The particular ID is not relevant post merge
        let (new_id, _new_id_import) = module.add_import_func(module_identifier, name, ty);
        new_id.into() // Consider it as a new function
    }

    fn add_new_import_global(
        module: &mut Module,
        old_import: &ImportGlobal<OldIdGlobal>,
    ) -> NewIdGlobal {
        // Standard data:
        let module_identifier = old_import.exporting_module().identifier();
        let name = old_import.exporting_identifier().identifier();
        // Specific data:
        let ty = *old_import.ty();
        let mutable = old_import.mutable();
        let shared = old_import.shared();
        // The particular ID is not relevant post merge
        let (new_id, _new_id_import) =
            module.add_import_global(module_identifier, name, ty, mutable, shared);
        new_id.into() // Consider it as a new id
    }

    fn add_new_local_function(
        module: &mut Module,
        mapping: &mut Mapping,
        old_local: &LocalFunction<OldIdFunction>,
    ) -> NewIdFunction {
        let old_module: IdentifierModule = old_local.module().identifier().to_string().into();
        let ty = old_local.ty();
        let locals = old_local
            .data()
            .iter()
            .map(|(old_id, ty)| {
                let old_id: Identifier<Old, _> = (*old_id).into();
                let new_id: Identifier<New, _> = module.locals.add(*ty).into();
                mapping.locals.insert((old_module.clone(), old_id), new_id);
                *new_id
            })
            .collect();
        let builder = FunctionBuilder::new(&mut module.types, ty.params(), ty.results());
        let new_index = builder.finish(locals, &mut module.funcs);
        new_index.into()
    }

    fn add_new_export_function(
        module: &mut Module,
        new_export_identifier: &IdentifierFunction,
        new_index: NewIdFunction,
    ) {
        let export_index = module.exports.add(
            new_export_identifier.identifier(),
            ExportItem::Function(*new_index),
        );
        let _ = export_index; // The particular ID is not relevant post merge
    }

    #[must_use]
    pub(crate) fn new(resolved: AllResolved) -> Self {
        // Create new empty Wasm module
        let mut merged = Module::default();
        let mut mapping = Mapping::default();

        let _ = resolved.all_reduced.memories; // TODO: cover in this pass
        let _ = resolved.all_reduced.tables; // TODO: cover in this pass

        resolved
            .all_reduced
            .functions
            .join(&mut merged, &mut mapping, &resolved.rename_map);

        resolved
            .all_reduced
            .globals
            .join(&mut merged, &mut mapping, &resolved.rename_map);

        Self {
            merged,
            mapping,
            names: vec![],
            starts: vec![],
            all_resolved: resolved,
        }
    }

    #[allow(clippy::too_many_lines)] // TODO: fix / remove
    pub(crate) fn include(&mut self, module: NamedParsedModule<'_>) -> Result<(), Error> {
        let NamedParsedModule {
            name: considering_module_name_str,
            module: considering_module,
        } = module;
        let Module {
            ref imports,
            ref tables,
            ref types,
            ref funcs,
            ref globals,
            ref locals,
            ref exports,
            ref memories,
            ref data,
            ref elements,
            ref start,
            ref producers,
            ref customs,
            ref debug,
            ref name,
            ..
        } = considering_module;

        // let mut import_covered = HashSet::new();
        let considering_module_name: IdentifierModule = considering_module_name_str.into();

        for ty in types.iter() {
            self.merged.types.add(ty.params(), ty.results());
        }

        for global in globals.iter() {
            let new_global_id = match global.kind {
                GlobalKind::Import(id) => {
                    let import = imports.get(id);
                    let (new_global_id, new_import_id) = self.merged.add_import_global(
                        &import.module,
                        &import.name,
                        global.ty,
                        global.mutable,
                        global.shared,
                    );
                    let _ = new_import_id;
                    new_global_id
                }
                GlobalKind::Local(const_expr) => self.merged.globals.add_local(
                    global.ty,
                    global.mutable,
                    global.shared,
                    const_expr.copy_for(self, considering_module_name.clone()),
                ),
            };
            let old_global_id: Identifier<Old, _> = global.id().into();
            let new_global_id: Identifier<New, _> = new_global_id.into();
            self.mapping.globals.insert(
                (considering_module_name.clone(), old_global_id),
                new_global_id,
            );
        }

        for memory in memories.iter() {
            let new_memory_id = match memory.import {
                Some(id) => {
                    let import = imports.get(id);
                    let (new_memory_id, new_import_id) = self.merged.add_import_memory(
                        &import.module,
                        &import.name,
                        memory.shared,
                        memory.memory64,
                        memory.initial,
                        memory.maximum,
                        memory.page_size_log2,
                    );
                    let _ = new_import_id;
                    new_memory_id
                }
                None => self.merged.memories.add_local(
                    memory.shared,
                    memory.memory64,
                    memory.initial,
                    memory.maximum,
                    memory.page_size_log2,
                ),
            };
            let old_memory_id: Identifier<Old, _> = memory.id().into();
            let new_memory_id: Identifier<New, _> = new_memory_id.into();
            self.mapping.memories.insert(
                (considering_module_name.clone(), old_memory_id),
                new_memory_id,
            );
        }

        for data in data.iter() {
            let old_data_id: Identifier<Old, _> = data.id().into();
            let kind = match data.kind {
                DataKind::Active { memory, offset } => {
                    let old_memory_id: Identifier<Old, _> = memory.into();
                    let new_memory_id: Identifier<New, _> = *self
                        .mapping
                        .memories
                        .get(&(considering_module_name.clone(), old_memory_id))
                        .unwrap();
                    DataKind::Active {
                        memory: *new_memory_id,
                        offset,
                    }
                }
                DataKind::Passive => DataKind::Passive,
            };
            let new_data_id: Identifier<New, _> =
                self.merged.data.add(kind, data.value.clone()).into();
            self.mapping
                .datas
                .insert((considering_module_name.clone(), old_data_id), new_data_id);
        }

        for table in tables.iter() {
            let walrus::Table {
                table64,
                initial,
                maximum,
                element_ty,
                import,
                elem_segments,
                name,
                ..
            } = table;
            let new_table_id = match import {
                Some(import_id) => {
                    let import = imports.get(*import_id);
                    let (new_table_id, new_import_id) = self.merged.add_import_table(
                        considering_module_name_str,
                        &import.name,
                        *table64,
                        *initial,
                        *maximum,
                        *element_ty,
                    );
                    let _ = new_import_id;
                    new_table_id
                }
                None => self
                    .merged
                    .tables
                    .add_local(*table64, *initial, *maximum, *element_ty),
            };
            let new_table_id: Identifier<New, _> = new_table_id.into();
            let old_table_id: Identifier<Old, _> = table.id().into();
            self.mapping.tables.insert(
                (considering_module_name.clone(), old_table_id),
                new_table_id,
            );
            let new_table = self.merged.tables.get_mut(*new_table_id);
            new_table.name.clone_from(name);
            let _ = elem_segments; // Will be copied over after all elements have been set
        }

        for element in elements.iter() {
            let old_element_id: Identifier<Old, _> = element.id().into();
            let items = match &element.items {
                ElementItems::Functions(ids) => ElementItems::Functions(
                    ids.iter()
                        .map(|old_function_id| {
                            let old_function_id: Identifier<Old, _> = (*old_function_id).into();
                            let new_function_id: Identifier<New, _> = *self
                                .mapping
                                .funcs
                                .get(&(considering_module_name.clone(), old_function_id))
                                .unwrap();
                            *new_function_id
                        })
                        .collect(),
                ),
                ElementItems::Expressions(refttype, const_expression) => ElementItems::Expressions(
                    *refttype,
                    const_expression
                        .iter()
                        .map(|ce| ce.copy_for(self, considering_module_name.clone()))
                        .collect(),
                ),
            };
            let kind = match element.kind {
                ElementKind::Passive => ElementKind::Passive,
                ElementKind::Declared => ElementKind::Declared,
                ElementKind::Active { table, offset } => {
                    // This code is copied from above ... move to function!
                    let old_table_id: Identifier<Old, _> = table.into();
                    let new_table_id: Identifier<New, _> = *self
                        .mapping
                        .tables
                        .get(&(considering_module_name.clone(), (old_table_id)))
                        .unwrap();
                    let offset = offset.copy_for(self, considering_module_name.clone());
                    ElementKind::Active {
                        table: *new_table_id,
                        offset,
                    }
                }
            };
            let new_element_id: Identifier<New, _> = self.merged.elements.add(kind, items).into();
            self.mapping.elements.insert(
                (considering_module_name.clone(), old_element_id),
                new_element_id,
            );
        }

        for table in tables.iter() {
            let walrus::Table { elem_segments, .. } = table;
            let before_table_id: Identifier<Old, _> = table.id().into();
            let new_table_id: Identifier<New, _> = *self
                .mapping
                .tables
                .get(&(considering_module_name.clone(), before_table_id))
                .unwrap();
            let table = self.merged.tables.get_mut(*new_table_id);
            for old_element_id in elem_segments {
                let old_element_id: Identifier<Old, _> = (*old_element_id).into();
                let new_element_id = *self
                    .mapping
                    .elements
                    .get(&(considering_module_name.clone(), old_element_id))
                    .unwrap();
                table.elem_segments.insert(*new_element_id);
            }
        }

        for import in imports.iter() {
            match &import.kind {
                ImportKind::Function(before_id) => {
                    let ty = funcs.get(*before_id).ty();
                    let ty = FuncType::from_types(ty, types);

                    let import = Import {
                        exporting_module: import.module.clone().into(),
                        importing_module: module.name.into(),
                        exporting_identifier: import.name.clone().into(),
                        imported_index: Identifier::<Old, _>::from(*before_id),
                        kind: PhantomData,
                        ty,
                        data: ImportDataFunction,
                    };

                    if self
                        .all_resolved
                        .all_reduced
                        .functions
                        .remaining_imports
                        .contains(&import)
                    {
                        // Assert it is present
                        #[cfg(debug_assertions)]
                        debug_assert!(
                            self.merged
                                .imports
                                .get_func(
                                    import.exporting_module.identifier(),
                                    import.exporting_identifier.identifier()
                                )
                                .is_ok(),
                            "Function import should exist: {import:?}",
                        );
                    } else {
                        #[cfg(debug_assertions)]
                        debug_assert!(
                            self.mapping
                                .funcs
                                .contains_key(&(import.importing_module, (*before_id).into(),))
                        );
                    }
                }
                // What if the imported value is duplicate BUT different in shape (eg. element_ty) among multiple imports?
                ImportKind::Table(id) => {
                    let table = tables.get(*id);
                    self.merged.add_import_table(
                        &import.module,
                        &import.name,
                        table.table64,
                        table.initial,
                        table.maximum,
                        table.element_ty,
                    );
                }
                ImportKind::Memory(id) => {
                    let memory = memories.get(*id);
                    self.merged.add_import_memory(
                        &import.module,
                        &import.name,
                        memory.shared,
                        memory.memory64,
                        memory.initial,
                        memory.maximum,
                        memory.page_size_log2,
                    );
                }
                ImportKind::Global(id) => {
                    let global = globals.get(*id);
                    self.merged.add_import_global(
                        &import.module,
                        &import.name,
                        global.ty,
                        global.mutable,
                        global.shared,
                    );
                }
            }
        }

        for function in funcs.iter() {
            match &function.kind {
                FunctionKind::Import(_) => {
                    // debug_assert!(import_covered.contains(&function.id()))
                }
                FunctionKind::Local(local_function) => {
                    let old_function_index: Identifier<Old, _> = function.id().into();
                    let new_function_index: Identifier<New, _> = *self
                        .mapping
                        .funcs
                        .get(&(considering_module_name.clone(), old_function_index))
                        .unwrap();

                    let mut visitor = walrus_copy::WasmFunctionCopy::new(
                        &considering_module,
                        &mut self.merged,
                        local_function,
                        considering_module_name.clone(),
                        &mut self.mapping,
                        new_function_index,
                        old_function_index,
                    );

                    walrus::ir::dfs_in_order(
                        &mut visitor,
                        local_function,
                        local_function.entry_block(),
                    );
                }
                FunctionKind::Uninitialized(_) => {
                    return Err(Error::Parse(anyhow!(
                        "walrus::FunctionKind::Uninitialized during parsing of {considering_module_name_str}",
                    )));
                }
            }
        }

        for export in exports.iter() {
            match &export.item {
                // FIXME: assert based on renamed injection, not old identifier
                ExportItem::Function(before_id) => {
                    let ty = funcs.get(*before_id).ty();
                    let ty = FuncType::from_types(ty, types);
                    let lookup_export = Export {
                        module: considering_module_name.identifier().to_string().into(),
                        identifier: export.name.clone().into(),
                        index: (*before_id).into(),
                        kind: PhantomData,
                        ty,
                    };

                    if self
                        .all_resolved
                        .all_reduced
                        .functions
                        .remaining_exports
                        .contains(&lookup_export)
                    {
                        // TODO: assert that the exports includes the *renamed*
                        //       export.
                        // #[cfg(debug_assertions)]
                        // debug_assert!(self.merged.exports.iter().any(|potential_export| {
                        //     matches!(potential_export.item, ExportItem::Function(_))
                        //         && potential_export.name == export.name
                        // }));
                    } else {
                        #[cfg(debug_assertions)]
                        debug_assert!(self.mapping.funcs.contains_key(&(
                            considering_module_name.to_string().into(),
                            (*before_id).into()
                        )));
                    }
                }
                ExportItem::Table(before_index) => {
                    let old_id: Identifier<Old, _> = (*before_index).into();
                    let new_id: Identifier<New, _> = *self
                        .mapping
                        .tables
                        .get(&(considering_module_name.clone(), old_id))
                        .unwrap();
                    let new = self.merged.tables.get(*new_id);

                    let mut old_export = Export {
                        module: considering_module_name.clone(),
                        identifier: export.name.clone().into(),
                        index: old_id,
                        kind: PhantomData,
                        ty: new.element_ty,
                    };
                    let remaining = self
                        .all_resolved
                        .all_reduced
                        .tables
                        .remaining_exports
                        .contains(&old_export);
                    if remaining {
                        self.all_resolved
                            .rename_map
                            .rename_if_required(&mut old_export, RenameStrategy::tables);
                        self.merged.exports.add(
                            old_export.identifier().identifier(),
                            ExportItem::Table(*new_id),
                        );
                    } else {
                        // TODO: ... move insertion higher up and keep here only
                        //           debug assertions
                        // #[cfg(debug_assertions)]
                        // debug_assert!(self.mapping.funcs.contains_key(&(
                        //     considering_module_name.to_string().into(),
                        //     (*before_id).into()
                        // )));
                    }
                }
                ExportItem::Memory(before_index) => {
                    let old_id: Identifier<Old, _> = (*before_index).into();
                    let new_id: Identifier<New, _> = *self
                        .mapping
                        .memories
                        .get(&(considering_module_name.clone(), old_id))
                        .unwrap();
                    let new = self.merged.memories.get(*new_id);
                    let _ = new; // its type is not used downstream

                    let mut old_export = Export {
                        module: considering_module_name.clone(),
                        identifier: export.name.clone().into(),
                        index: old_id,
                        kind: PhantomData,
                        ty: (),
                    };
                    let remaining = self
                        .all_resolved
                        .all_reduced
                        .memories
                        .remaining_exports
                        .contains(&old_export);
                    if remaining {
                        self.all_resolved
                            .rename_map
                            .rename_if_required(&mut old_export, RenameStrategy::memories);
                        self.merged.exports.add(
                            old_export.identifier().identifier(),
                            ExportItem::Memory(*new_id),
                        );
                    } else {
                        // TODO: ... move insertion higher up and keep here only
                        //           debug assertions
                        // #[cfg(debug_assertions)]
                        // debug_assert!(self.mapping.funcs.contains_key(&(
                        //     considering_module_name.to_string().into(),
                        //     (*before_id).into()
                        // )));
                    }
                }
                // TODO: code dupe with other export forms
                ExportItem::Global(before_index) => {
                    let old_id: Identifier<Old, _> = (*before_index).into();
                    let new_id: Identifier<New, _> = *self
                        .mapping
                        .globals
                        .get(&(considering_module_name.clone(), old_id))
                        .unwrap();
                    let new = self.merged.globals.get(*new_id);

                    let mut old_export = Export {
                        module: considering_module_name.clone(),
                        identifier: export.name.clone().into(),
                        index: old_id,
                        kind: PhantomData,
                        ty: new.ty,
                    };
                    let remaining = self
                        .all_resolved
                        .all_reduced
                        .globals
                        .remaining_exports
                        .contains(&old_export);
                    if remaining {
                        self.all_resolved
                            .rename_map
                            .rename_if_required(&mut old_export, RenameStrategy::globals);
                        self.merged.exports.add(
                            old_export.identifier().identifier(),
                            ExportItem::Global(*new_id),
                        );
                    } else {
                        // TODO: ... move insertion higher up and keep here only
                        //           debug assertions
                        // #[cfg(debug_assertions)]
                        // debug_assert!(self.mapping.funcs.contains_key(&(
                        //     considering_module_name.to_string().into(),
                        //     (*before_id).into()
                        // )));
                    }
                }
            }
        }

        if let Some(old_start_id) = start {
            let old_start_id: Identifier<Old, _> = (*old_start_id).into();
            let new_start_id: Identifier<New, _> = *self
                .mapping
                .funcs
                .get(&(considering_module_name, old_start_id))
                .unwrap();
            self.starts.push(*new_start_id);
        }

        let _ = producers; // Handled when build is called
        let _ = locals; // Handled before, when going through first pass

        for (custom_id, custom_section) in customs.iter() {
            let _ = custom_id;
            let name = custom_section.name().into();
            let ids_to_idcs: IdsToIndices = walrus::IdsToIndices::default();
            let data = custom_section.data(&ids_to_idcs).to_vec();
            let raw_custom_section = walrus::RawCustomSection { name, data };
            self.merged.customs.add(raw_custom_section);
        }

        let _ = debug; // FIXME: merge DWARF info

        if let Some(name) = name {
            self.names
                .push((considering_module_name_str.to_string(), name.clone()));
        }

        Ok(())
    }

    pub(crate) fn build(mut self) -> Module {
        self.merged
            .producers
            .add_processed_by("webassembly-mergers", env!("CARGO_PKG_VERSION"));
        let formatted: Vec<_> = self
            .names
            .iter()
            .map(|(module, name)| format!("{module}::{name}"))
            .collect();

        if !self.starts.is_empty() {
            const EMPTY_PARAMS: &[ValType] = &[];
            const EMPTY_RESULTS: &[ValType] = &[];

            let mut builder =
                FunctionBuilder::new(&mut self.merged.types, EMPTY_PARAMS, EMPTY_RESULTS);

            for start in self.starts {
                builder.func_body().call(start);
            }
            let merged_start = builder.finish(vec![], &mut self.merged.funcs);
            self.merged.start = Some(merged_start);
        }

        self.merged.name = Some(formatted.join("-"));
        self.merged
    }
}

trait CopyForMerger {
    fn copy_for(&self, merger: &Merger, considering_module: IdentifierModule) -> Self;
}

impl CopyForMerger for ConstExpr {
    fn copy_for(&self, merger: &Merger, considering_module_name: IdentifierModule) -> Self {
        match self {
            ConstExpr::Value(value) => ConstExpr::Value(*value),
            ConstExpr::RefNull(ref_type) => ConstExpr::RefNull(*ref_type),
            ConstExpr::Global(id) => {
                let old_id: Identifier<Old, _> = (*id).into();
                let new_id: Identifier<New, _> = *merger
                    .mapping
                    .globals
                    .get(&(considering_module_name, old_id))
                    .unwrap();
                ConstExpr::Global(*new_id)
            }
            ConstExpr::RefFunc(id) => {
                let old_id: Identifier<Old, _> = (*id).into();
                let new_id: Identifier<New, _> = *merger
                    .mapping
                    .funcs
                    .get(&(considering_module_name, old_id))
                    .unwrap();
                ConstExpr::RefFunc(*new_id)
            }
        }
    }
}

/* [1]: This case is impossible since in an earlier pass clashing names had been covered. */

trait MergedJoinable {
    fn join(&self, module: &mut Module, mapping: &mut Mapping, rename_map: &RenameMap);
}

impl MergedJoinable for ReducedDependenciesFunction {
    fn join(&self, module: &mut Module, mapping: &mut Mapping, rename_map: &RenameMap) {
        // 1. Include all remaining imports:
        for old_import in &self.remaining_imports {
            let new_import = Merger::add_new_import_function(module, old_import);
            mapping
                .funcs
                .insert(old_import.to_mapping_ref(), new_import);
        }

        // 2. Include all locals:
        self.reduction_map
            .keys()
            .filter_map(|node| node.as_local())
            .for_each(|old_local| {
                let new_local = Merger::add_new_local_function(module, mapping, old_local);
                mapping.funcs.insert(old_local.to_mapping_ref(), new_local);
            });

        for (node, reduced) in &self.reduction_map {
            // Find location of reduced node:
            let reduced = mapping.funcs.get(&reduced.to_mapping_ref()).copied();

            // The reduced should be present in the new mapping
            #[cfg(debug_assertions)]
            debug_assert!(reduced.is_some());

            // Inject pointer from old to new
            if let Some(reduced) = reduced {
                mapping.funcs.insert(node.to_mapping_ref(), reduced);
            }
        }

        for old_export in &self.remaining_exports {
            let reduced = mapping.funcs.get(&old_export.to_mapping_ref());

            let mut old_export = old_export.clone();
            rename_map.rename_if_required(&mut old_export, RenameStrategy::functions);

            // TODO: I did this multiple times, unwrapping should be turned into an error throwing?
            // The reduced should be present in the new mapping
            #[cfg(debug_assertions)]
            debug_assert!(
                reduced.is_some(),
                "Reduced node should exist in mapping for {old_export:?}",
            );

            // Inject pointer from old to new
            if let Some(reduced) = reduced {
                Merger::add_new_export_function(module, old_export.identifier(), *reduced);
            }
        }
    }
}

impl MergedJoinable for ReducedDependenciesGlobal {
    fn join(&self, module: &mut Module, mapping: &mut Mapping, rename_map: &RenameMap) {
        // 1. Include all remaining imports:
        for old_import in &self.remaining_imports {
            let new_import = Merger::add_new_import_global(module, old_import);
            mapping
                .globals
                .insert(old_import.to_mapping_ref(), new_import);
            let _ = rename_map; // TODO: ... put to use
        }

        // // 2. Include all locals:
        // self.reduction_map
        //     .keys()
        //     .filter_map(|node| node.as_local())
        //     .for_each(|old_local| {
        //         let new_local = Merger::add_new_local_global(module, mapping, old_local);
        //         mapping
        //             .globals
        //             .insert(old_local.to_mapping_ref(), new_local);
        //     });

        // for (node, reduced) in &self.reduction_map {
        //     // Find location of reduced node:
        //     let reduced = mapping.globals.get(&reduced.to_mapping_ref()).copied();

        //     // The reduced should be present in the new mapping
        //     #[cfg(debug_assertions)]
        //     debug_assert!(reduced.is_some());

        //     // Inject pointer from old to new
        //     if let Some(reduced) = reduced {
        //         mapping.globals.insert(node.to_mapping_ref(), reduced);
        //     }
        // }

        // for old_export in &self.remaining_exports {
        //     let reduced = mapping.globals.get(&old_export.to_mapping_ref());

        //     let mut old_export = old_export.clone();
        //     rename_map.rename_if_required(&mut old_export, RenameStrategy::globals);

        //     // TODO: I did this multiple times, unwrapping should be turned into an error throwing?
        //     // The reduced should be present in the new mapping
        //     #[cfg(debug_assertions)]
        //     debug_assert!(reduced.is_some());

        //     // Inject pointer from old to new
        //     if let Some(reduced) = reduced {
        //         Merger::add_new_export(module, old_export.identifier(), *reduced);
        //     }
        // }
    }
}

// TODO: implement this for Tables, Memories & Globals
