use core::convert::From;

use walrus::{
    ConstExpr, DataKind, ElementItems, ElementKind, ExportItem, FunctionBuilder, FunctionId,
    FunctionKind, GlobalKind, IdsToIndices, ImportKind, Module, Table,
};

use crate::error::{Error, ExportKind};
use crate::merge_options::{ClashingExports, MergeOptions};
use crate::named_module::NamedParsedModule;
use crate::resolver::identified_resolution_schema::{
    MergedExport, MergedImport, OrderedResolutionSchema,
};
use crate::resolver::{
    FunctionImportSpecification, FunctionName, FunctionSpecification, GlobalName, MemoryName,
    ModuleName, Resolved, TableName,
};

pub(crate) mod old_to_new_mapping;
use old_to_new_mapping::Mapping;

mod walrus_copy;
use walrus_copy::WasmFunctionCopy;

pub(crate) mod provenance_identifier;
use provenance_identifier::{Identifier, New, Old};

pub(crate) struct Merger {
    options: MergeOptions,
    resolution_schema: OrderedResolutionSchema,
    merged: Module,
    mapping: Mapping,
    names: Vec<(String, String)>,
    starts: Vec<FunctionId>,
}

impl Merger {
    #[must_use]
    pub(crate) fn new(resolution_schema: OrderedResolutionSchema, options: MergeOptions) -> Self {
        // Create new empty Wasm module
        let mut merged = Module::default();
        let mut mapping = Mapping::default();

        for import_specification in resolution_schema.unresolved_imports_iter() {
            let FunctionImportSpecification {
                importing_module,
                exporting_module: ModuleName(exporting_module_name),
                name: FunctionName(function_name),
                ty,
                index: old_function_index,
            } = import_specification;
            let ty = merged.types.add(ty.params(), ty.results());
            let (new_function_index, new_import_index) =
                merged.add_import_func(exporting_module_name, function_name, ty);
            let _: Identifier<New, _> = new_import_index.into();
            let new_function_index: Identifier<New, _> = new_function_index.into();
            mapping.funcs.insert(
                (importing_module.clone(), *old_function_index),
                new_function_index,
            );
        }

        for local_function_specification in resolution_schema.get_local_specifications() {
            let FunctionSpecification {
                defining_module,
                ty,
                index: old_function_index,
                locals,
            } = local_function_specification;
            let locals = locals
                .iter()
                .map(|(old_id, ty)| {
                    let old_id: Identifier<Old, _> = (*old_id).into();
                    let new_id: Identifier<New, _> = merged.locals.add(*ty).into();
                    mapping
                        .locals
                        .insert((defining_module.clone(), old_id), new_id);
                    *new_id
                })
                .collect();
            let builder = FunctionBuilder::new(&mut merged.types, ty.params(), ty.results());
            let new_function_index = builder.finish(locals, &mut merged.funcs);
            let new_function_index: Identifier<New, _> = new_function_index.into();
            mapping.funcs.insert(
                (defining_module.clone(), *old_function_index),
                new_function_index,
            );
        }

        for resolved in resolution_schema.resolved_iter() {
            let Resolved {
                export_specification,
                resolved_imports,
            } = resolved;
            let old_exported_function_index: Identifier<Old, _> = export_specification.index;
            let new_resolved_function_index: Identifier<New, _> = *mapping
                .funcs
                .get(&(
                    export_specification.module.clone(),
                    old_exported_function_index,
                ))
                .unwrap();
            for resolved_import in resolved_imports {
                let FunctionImportSpecification {
                    importing_module,
                    exporting_module,
                    name: _,
                    ty: _,
                    index: old_function_index,
                } = resolved_import;
                debug_assert_eq!(*exporting_module, export_specification.module);
                mapping.funcs.insert(
                    (importing_module.clone(), *old_function_index),
                    new_resolved_function_index,
                );
            }
        }

        Self {
            resolution_schema,
            merged,
            mapping,
            names: vec![],
            starts: vec![],
            options,
        }
    }

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
        let considering_module_name = ModuleName::from(considering_module_name_str);

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
            let Table {
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
            let Table { elem_segments, .. } = table;
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
                    let before_id: Identifier<Old, _> = (*before_id).into();
                    // import_covered.insert(before_id);
                    let exporting_module = import.module.as_str().into();
                    let importing_module = considering_module_name.clone();
                    let function_name = import.name.as_str().into();
                    match self.resolution_schema.determine_merged_import(
                        &exporting_module,
                        &function_name,
                        &importing_module,
                    ) {
                        MergedImport::Resolved => {
                            // If it is resolved, another module will include it
                        }
                        MergedImport::Unresolved(import_spec) => {
                            let FunctionImportSpecification {
                                importing_module: ModuleName(importing_module_name),
                                exporting_module: _,
                                name: FunctionName(function_name),
                                ty: _,
                                index: _,
                            } = import_spec;

                            // If it is unresolved, assert it was added in the merged output
                            let import_id: Identifier<New, _> = *self
                                .mapping
                                .funcs
                                .get(&(importing_module, before_id))
                                .unwrap();
                            let new_import = self.merged.imports.get_imported_func(*import_id);
                            debug_assert!(
                                new_import.is_some_and(|import| import.name == function_name
                                    || (import.name.contains(&importing_module_name)
                                        && import.name.contains(&function_name)))
                            );
                        }
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

                    let mut visitor: WasmFunctionCopy<'_, '_> = WasmFunctionCopy::new(
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
                    return Err(Error::ComponentModelUnsupported(
                        considering_module_name_str.to_string(),
                    ));
                }
            }
        }

        for export in exports.iter() {
            match &export.item {
                // FIXME: If the function can be resolved, it could be hidden
                //        If the function cannot be resolved, & it name-clashes
                //        with another function ... then?
                ExportItem::Function(before_id) => {
                    let before_id: Identifier<Old, _> = (*before_id).into();
                    let exporting_module = considering_module_name.clone();
                    let function_name = export.name.as_str().into();
                    match self
                        .resolution_schema
                        .determine_merged_export(&exporting_module, &function_name)
                    {
                        MergedExport::Resolved => {
                            // FIXME: allow hiding resolved functions
                            let new_function_id: Identifier<New, _> = *self
                                .mapping
                                .funcs
                                .get(&(considering_module_name.clone(), before_id))
                                .unwrap();
                            let export_id = self
                                .merged
                                .exports
                                .add(&export.name, ExportItem::Function(*new_function_id));
                            let _ = export_id; // The export ID is not of interest for this module
                        }
                        MergedExport::Unresolved(export_spec) => {
                            debug_assert_eq!(export_spec.index, before_id);
                            debug_assert_eq!(export_spec.name, function_name);

                            let duplicate_function_export =
                                self.merged.exports.iter().find(|existing_export| {
                                    existing_export.name == export.name
                                        && matches!(existing_export.item, ExportItem::Function(_))
                                });
                            if let Some(duplicate_function_export) = duplicate_function_export {
                                match &self.options.clashing_exports {
                                    ClashingExports::Rename(renamer) => {
                                        let FunctionName(renamed) = (renamer.functions)(
                                            considering_module_name.clone(),
                                            function_name,
                                        );
                                        let new_function_id: Identifier<New, _> = *self
                                            .mapping
                                            .funcs
                                            .get(&(considering_module_name.clone(), before_id))
                                            .unwrap();
                                        self.merged
                                            .exports
                                            .add(&renamed, ExportItem::Function(*new_function_id));
                                    }
                                    ClashingExports::Signal => {
                                        // TODO: this could be reported early when resolving
                                        debug_assert_eq!(
                                            duplicate_function_export.name,
                                            export.name
                                        );
                                        let _ = duplicate_function_export;
                                        return Err(Error::DuplicateNameExport(
                                            export.name.clone(),
                                            ExportKind::Function,
                                        ));
                                    }
                                }
                            } else {
                                let new_function_id: Identifier<New, _> = *self
                                    .mapping
                                    .funcs
                                    .get(&(considering_module_name.clone(), before_id))
                                    .unwrap();
                                self.merged
                                    .exports
                                    .add(&export.name, ExportItem::Function(*new_function_id));
                            }
                        }
                    }
                }
                ExportItem::Table(before_index) => {
                    let duplicate_table_export =
                        self.merged.exports.iter().find(|existing_export| {
                            existing_export.name == export.name
                                && matches!(existing_export.item, ExportItem::Table(_))
                        });
                    let old_table_id: Identifier<Old, _> = (*before_index).into();
                    let table_name = export.name.as_str().into();
                    if let Some(duplicate_table_export) = duplicate_table_export {
                        match &self.options.clashing_exports {
                            ClashingExports::Rename(renamer) => {
                                let TableName(renamed) =
                                    (renamer.tables)(considering_module_name.clone(), table_name);
                                let new_table_id: Identifier<New, _> = *self
                                    .mapping
                                    .tables
                                    .get(&(considering_module_name.clone(), old_table_id))
                                    .unwrap();
                                self.merged
                                    .exports
                                    .add(&renamed, ExportItem::Table(*new_table_id));
                            }
                            ClashingExports::Signal => {
                                // TODO: this could be reported early when resolving
                                debug_assert_eq!(duplicate_table_export.name, export.name);
                                return Err(Error::DuplicateNameExport(
                                    export.name.clone(),
                                    ExportKind::Table,
                                ));
                            }
                        }
                    } else {
                        let new_table_id: Identifier<New, _> = *self
                            .mapping
                            .tables
                            .get(&(considering_module_name.clone(), old_table_id))
                            .unwrap();
                        self.merged
                            .exports
                            .add(&export.name, ExportItem::Table(*new_table_id));
                    }
                }
                ExportItem::Memory(before_index) => {
                    let duplicate_memory_export =
                        self.merged.exports.iter().find(|existing_export| {
                            existing_export.name == export.name
                                && matches!(existing_export.item, ExportItem::Memory(_))
                        });
                    let old_memory_id: Identifier<Old, _> = (*before_index).into();
                    let memory_name = export.name.as_str().into();
                    if let Some(duplicate_memory_export) = duplicate_memory_export {
                        match &self.options.clashing_exports {
                            ClashingExports::Rename(renamer) => {
                                let MemoryName(renamed) =
                                    (renamer.memory)(considering_module_name.clone(), memory_name);
                                let new_memory_id: Identifier<New, _> = *self
                                    .mapping
                                    .memories
                                    .get(&(considering_module_name.clone(), old_memory_id))
                                    .unwrap();
                                self.merged
                                    .exports
                                    .add(&renamed, ExportItem::Memory(*new_memory_id));
                            }
                            ClashingExports::Signal => {
                                // TODO: this could be reported early when resolving
                                debug_assert_eq!(duplicate_memory_export.name, export.name);
                                return Err(Error::DuplicateNameExport(
                                    export.name.clone(),
                                    ExportKind::Memory,
                                ));
                            }
                        }
                    } else {
                        let new_memory_id: Identifier<New, _> = *self
                            .mapping
                            .memories
                            .get(&(considering_module_name.clone(), old_memory_id))
                            .unwrap();
                        self.merged
                            .exports
                            .add(&export.name, ExportItem::Memory(*new_memory_id));
                    }
                }
                // TODO: code dupe with other export forms
                ExportItem::Global(before_index) => {
                    let duplicate_global_export =
                        self.merged.exports.iter().find(|existing_export| {
                            existing_export.name == export.name
                                && matches!(existing_export.item, ExportItem::Global(_))
                        });
                    let old_global_id: Identifier<Old, _> = (*before_index).into();
                    let global_name = export.name.as_str().into();
                    if let Some(duplicate_global_export) = duplicate_global_export {
                        match &self.options.clashing_exports {
                            ClashingExports::Rename(renamer) => {
                                let GlobalName(renamed) =
                                    (renamer.globals)(considering_module_name.clone(), global_name);
                                let new_global_id: Identifier<New, _> = *self
                                    .mapping
                                    .globals
                                    .get(&(considering_module_name.clone(), old_global_id))
                                    .unwrap();
                                self.merged
                                    .exports
                                    .add(&renamed, ExportItem::Global(*new_global_id));
                            }
                            ClashingExports::Signal => {
                                // TODO: this could be reported early when resolving
                                debug_assert_eq!(duplicate_global_export.name, export.name);
                                return Err(Error::DuplicateNameExport(
                                    export.name.clone(),
                                    ExportKind::Global,
                                ));
                            }
                        }
                    } else {
                        let new_global_id: Identifier<New, _> = *self
                            .mapping
                            .globals
                            .get(&(considering_module_name.clone(), old_global_id))
                            .unwrap();
                        self.merged
                            .exports
                            .add(&export.name, ExportItem::Global(*new_global_id));
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
                .push((considering_module_name_str.to_string(), name.to_string()));
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
            let mut builder = FunctionBuilder::new(&mut self.merged.types, &[], &[]);
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
    fn copy_for(&self, merger: &Merger, considering_module: ModuleName) -> Self;
}

impl CopyForMerger for ConstExpr {
    fn copy_for(&self, merger: &Merger, considering_module_name: ModuleName) -> Self {
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
