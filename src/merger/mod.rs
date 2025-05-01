use std::collections::HashSet;

use walrus::DataKind;
use walrus::ElementItems;
use walrus::ExportItem;
use walrus::FunctionKind;
use walrus::GlobalKind;
use walrus::ImportKind;
use walrus::Module;
use walrus::Table;

use crate::error::Error;
use crate::named_module::NamedParsedModule;
use crate::resolver::FuncType;
use crate::resolver::identified_resolution_schema::OrderedResolutionSchema;
use crate::resolver::identified_resolution_schema::{MergedExport, MergedImport};

mod old_to_new_mapping;
use old_to_new_mapping::Mapping;

mod walrus_copy;
use walrus_copy::WasmFunctionCopy;

pub(crate) struct Merger {
    resolution_schema: OrderedResolutionSchema,
    merged: Module,
}

impl Merger {
    #[must_use]
    pub(crate) fn new(resolution_schema: OrderedResolutionSchema) -> Self {
        Self {
            resolution_schema,
            merged: Module::default(),
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

        let mut import_covered = HashSet::new();
        let mut mapping = Mapping::default();
        mapping.populate_with_resolution_schema(considering_module_name, &self.resolution_schema);

        for global in globals.iter() {
            let new_global_id = match global.kind {
                GlobalKind::Import(id) => {
                    // TODO: what if this could also be resolved?
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
                    // TODO: what if this could also be resolved?
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
                            *mapping
                                .functions
                                .get(&(considering_module_name.to_string(), *old_function_id))
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
                elem_segments: _,
                name: _,
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
        }

        for import in imports.iter() {
            match &import.kind {
                ImportKind::Function(before_id) => {
                    import_covered.insert(before_id);
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
                            let (before_index, after_index) = import_spec.index;
                            let owned_ty = import_spec.ty;
                            let ty = self.merged.types.add(owned_ty.params(), owned_ty.results());
                            let (function_id, import_id) = self.merged.add_import_func(
                                &exporting_module.name,
                                &function_name.name,
                                ty,
                            );
                            let _ = import_id; // The new import ID is not of interest
                            debug_assert_eq!(before_id.index(), before_index.index);
                            debug_assert_eq!(function_id.index(), after_index.index);
                            debug_assert_eq!(import_spec.exporting_module, exporting_module);
                            debug_assert_eq!(import_spec.importing_module, importing_module);
                            debug_assert_eq!(import_spec.name, function_name);
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
                FunctionKind::Import(_) => debug_assert!(import_covered.contains(&function.id())),
                FunctionKind::Local(local_function) => {
                    let new_locals: Vec<_> = local_function
                        .args
                        .iter()
                        .map(|old_local| locals.get(*old_local))
                        .map(|old_local| self.merged.locals.add(old_local.ty()))
                        .collect();

                    let owned_type = FuncType::from_types(local_function.ty(), types);

                    let mut visitor = WasmFunctionCopy::new(
                        &considering_module,
                        &mut self.merged,
                        local_function,
                        new_locals,
                        owned_type,
                        considering_module_name.to_string(),
                        &mapping,
                    );

                    walrus::ir::dfs_in_order(
                        &mut visitor,
                        local_function,
                        local_function.entry_block(),
                    );

                    visitor.finish();
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
                    let exporting_module = considering_module_name.into();
                    let function_name = export.name.as_str().into();
                    match self
                        .resolution_schema
                        .determine_merged_export(&exporting_module, &function_name)
                    {
                        MergedExport::Resolved => {
                            // TODO: ???
                            // Do nothing, since it has been resolved.
                        }
                        MergedExport::Unresolved(export_spec) => {
                            let (before_index, after_index) = export_spec.index;
                            let after_id = self
                                .merged
                                .funcs
                                .iter()
                                .nth(after_index.index)
                                .unwrap()
                                .id();
                            let export_id = self
                                .merged
                                .exports
                                .add(&function_name.name, ExportItem::Function(after_id));
                            let _ = export_id; // The export ID is not of interest for this module
                            debug_assert_eq!(before_id.index(), before_index.index);
                            debug_assert_eq!(after_id.index(), after_index.index);
                            debug_assert_eq!(export_spec.name, function_name);
                        }
                    }
                }
                ExportItem::Table(id) => todo!("{id:?}"),
                ExportItem::Memory(id) => todo!("{id:?}"),
                ExportItem::Global(id) => todo!("{id:?}"),
            }
        }

        let _ = start; // TODO:
        let _ = producers; // TODO:
        let _ = customs; // TODO:
        let _ = debug; // TODO:
        let _ = name; // TODO:

        Ok(())
    }

    pub(crate) fn build(&mut self) -> Module {
        todo!()
    }
}
