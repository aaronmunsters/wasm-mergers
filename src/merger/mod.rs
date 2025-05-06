use std::collections::HashMap;

use walrus::{DataKind, ElementItems, ExportItem, GlobalKind, ImportKind, LocalId, Module, Table};
use walrus::{FunctionBuilder, FunctionId, FunctionKind};

use crate::error::Error;
use crate::named_module::NamedParsedModule;
use crate::resolver::FunctionImportSpecification;
use crate::resolver::FunctionSpecification;
use crate::resolver::ModuleName;
use crate::resolver::Resolved;
use crate::resolver::identified_resolution_schema::OrderedResolutionSchema;
use crate::resolver::identified_resolution_schema::{MergedExport, MergedImport};
use crate::resolver::resolution_schema::BeforeFunctionIndex;

mod old_to_new_mapping;
use old_to_new_mapping::Mapping;

mod walrus_copy;
use walrus_copy::WasmFunctionCopy;

pub(crate) struct Merger {
    resolution_schema: OrderedResolutionSchema,
    merged: Module,
    function_mapping: HashMap<(ModuleName, BeforeFunctionIndex), FunctionId>,
    locals_mapping: HashMap<(ModuleName, BeforeFunctionIndex, LocalId), LocalId>,
}

impl Merger {
    #[must_use]
    pub(crate) fn new(resolution_schema: OrderedResolutionSchema) -> Self {
        // Create new empty Wasm module
        let mut merged = Module::default();

        let mut bef_aft_mapping: HashMap<(ModuleName, BeforeFunctionIndex), FunctionId> =
            HashMap::new();
        let mut bef_aft_locals_mapping: HashMap<
            (ModuleName, BeforeFunctionIndex, LocalId),
            LocalId,
        > = HashMap::new();

        for import_specification in resolution_schema.unresolved_imports_iter() {
            let FunctionImportSpecification {
                importing_module,
                exporting_module,
                name,
                ty,
                index: before_index,
            } = import_specification;
            let ty = merged.types.add(ty.params(), ty.results());
            let (new_function_index, new_import_index) =
                merged.add_import_func(&exporting_module.name, &name.name, ty);
            let _ = new_import_index;
            bef_aft_mapping.insert(
                (importing_module.clone(), before_index.clone()),
                new_function_index,
            );
        }

        for local_function_specification in resolution_schema.get_local_specifications() {
            let FunctionSpecification {
                defining_module,
                ty,
                index,
                locals,
            } = local_function_specification;

            let locals = locals
                .iter()
                .map(|(old_id, ty)| {
                    let new_id = merged.locals.add(*ty);
                    bef_aft_locals_mapping
                        .insert((defining_module.clone(), index.clone(), *old_id), new_id);
                    new_id
                })
                .collect();
            let builder = FunctionBuilder::new(&mut merged.types, ty.params(), ty.results());
            let new_function_index = builder.finish(locals, &mut merged.funcs);
            bef_aft_mapping.insert((defining_module.clone(), index.clone()), new_function_index);
        }

        for resolved in resolution_schema.resolved_iter() {
            let Resolved {
                export_specification,
                resolved_imports,
            } = resolved;
            let local_function_index = *bef_aft_mapping
                .get(&(
                    export_specification.module.name.as_str().into(),
                    export_specification.index.index.into(),
                ))
                .unwrap();
            for resolved_import in resolved_imports {
                let FunctionImportSpecification {
                    importing_module,
                    exporting_module,
                    name: _,
                    ty: _,
                    index: before_index,
                } = resolved_import;
                debug_assert_eq!(exporting_module, &export_specification.module);
                bef_aft_mapping.insert(
                    (importing_module.clone(), before_index.clone()),
                    local_function_index,
                );
            }
        }

        Self {
            resolution_schema,
            merged,
            function_mapping: bef_aft_mapping,
            locals_mapping: bef_aft_locals_mapping,
        }
    }

    pub(crate) fn include(&mut self, module: NamedParsedModule<'_>) -> Result<(), Error> {
        let NamedParsedModule {
            name: considering_module_name,
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
        let mut mapping = Mapping::with(&self.function_mapping, &self.locals_mapping);

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
                    const_expr,
                ),
            };
            mapping.globals.insert(
                (considering_module_name.to_string(), global.id()),
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
            mapping.memories.insert(
                (considering_module_name.to_string(), memory.id()),
                new_memory_id,
            );
        }

        for data in data.iter() {
            let kind = match data.kind {
                DataKind::Active { memory, offset } => DataKind::Active {
                    memory: *mapping
                        .memories
                        .get(&(considering_module_name.to_string(), memory))
                        .unwrap(),
                    offset,
                },
                DataKind::Passive => DataKind::Passive,
            };
            let new_data_id = self.merged.data.add(kind, data.value.clone());
            mapping.datas.insert(
                (considering_module_name.to_string(), data.id()),
                new_data_id,
            );
        }

        for element in elements.iter() {
            let items = match &element.items {
                ElementItems::Functions(ids) => ElementItems::Functions(
                    ids.iter()
                        .map(|old_function_id| {
                            *self
                                .function_mapping
                                .get(&(
                                    considering_module_name.into(),
                                    old_function_id.index().into(),
                                ))
                                .unwrap()
                        })
                        .collect(),
                ),
                ElementItems::Expressions(_, _) => element.items.clone(),
            };
            let new_element_id = self.merged.elements.add(element.kind, items);
            mapping.elements.insert(
                (considering_module_name.to_string(), element.id()),
                new_element_id,
            );
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
                        considering_module_name,
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
            mapping.tables.insert(
                (considering_module_name.to_string(), table.id()),
                new_table_id,
            );

            let table = self.merged.tables.get_mut(new_table_id);
            table.name = name.clone();
            for old_element_id in elem_segments.iter() {
                let new_element_id = *mapping
                    .elements
                    .get(&(considering_module_name.to_string(), *old_element_id))
                    .unwrap();
                table.elem_segments.insert(new_element_id);
            }
        }

        for import in imports.iter() {
            match &import.kind {
                ImportKind::Function(before_id) => {
                    // import_covered.insert(before_id);
                    let exporting_module = import.module.as_str().into();
                    let importing_module = considering_module_name.into();
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
                            // If it is unresolved, assert it was added in the merged output
                            let _ = import_spec; // FIXME: unused?
                            debug_assert!(
                                self.function_mapping
                                    .contains_key(&(importing_module, before_id.index().into()))
                            );
                        }
                    }
                }
                ImportKind::Table(id) => todo!("{id:?}"),
                ImportKind::Memory(id) => todo!("{id:?}"),
                ImportKind::Global(id) => todo!("{id:?}"),
            }
        }

        for function in funcs.iter() {
            match &function.kind {
                FunctionKind::Import(_) => {
                    // debug_assert!(import_covered.contains(&function.id()))
                }
                FunctionKind::Local(local_function) => {
                    let old_function_index = function.id();
                    let new_function_index = *self
                        .function_mapping
                        .get(&(considering_module_name.into(), function.id().index().into()))
                        .unwrap();

                    let mut visitor: WasmFunctionCopy<'_, '_> = WasmFunctionCopy::new(
                        &considering_module,
                        &mut self.merged,
                        local_function,
                        considering_module_name.to_string(),
                        &mapping,
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
                        considering_module_name.to_string(),
                    ));
                }
            }
        }

        for export in exports.iter() {
            match &export.item {
                ExportItem::Function(before_id) => {
                    let _ = before_id;
                    let exporting_module = considering_module_name.into();
                    let function_name = export.name.as_str().into();
                    match self
                        .resolution_schema
                        .determine_merged_export(&exporting_module, &function_name)
                    {
                        MergedExport::Resolved => {
                            // FIXME: allow hiding resolved functions
                            let new_function_id = *self
                                .function_mapping
                                .get(&(considering_module_name.into(), before_id.index().into()))
                                .unwrap();
                            let export_id = self
                                .merged
                                .exports
                                .add(&export.name, ExportItem::Function(new_function_id));
                            let _ = export_id; // The export ID is not of interest for this module
                        }
                        MergedExport::Unresolved(export_spec) => {
                            let before_index = export_spec.index;
                            let after_index = *self
                                .function_mapping
                                .get(&(considering_module_name.into(), before_index))
                                .unwrap();
                            let export_id = self
                                .merged
                                .exports
                                .add(&export.name, ExportItem::Function(after_index));
                            let _ = export_id; // The export ID is not of interest for this module
                            debug_assert_eq!(export_spec.name, function_name);
                        }
                    }
                }
                ExportItem::Table(before_index) => {
                    let new_table_id = mapping
                        .tables
                        .get(&(considering_module_name.into(), *before_index))
                        .unwrap();
                    self.merged
                        .exports
                        .add(&export.name, ExportItem::Table(*new_table_id));
                }
                ExportItem::Memory(before_index) => {
                    let new_memory_id = mapping
                        .memories
                        .get(&(considering_module_name.into(), *before_index))
                        .unwrap();
                    self.merged
                        .exports
                        .add(&export.name, ExportItem::Memory(*new_memory_id));
                }
                ExportItem::Global(before_index) => {
                    let new_global_id = mapping
                        .globals
                        .get(&(considering_module_name.into(), *before_index))
                        .unwrap();
                    self.merged
                        .exports
                        .add(&export.name, ExportItem::Global(*new_global_id));
                }
            }
        }

        let _ = start; // TODO:
        let _ = producers; // TODO:
        let _ = customs; // TODO:
        let _ = debug; // TODO:
        let _ = name; // TODO:
        let _ = locals; // TODO:

        Ok(())
    }

    pub(crate) fn build(self) -> Module {
        self.merged
    }
}
