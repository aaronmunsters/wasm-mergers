use walrus::{
    ConstExpr, DataKind, ElementItems, ElementKind, ExportItem, FunctionBuilder, FunctionId,
    FunctionKind, GlobalKind, IdsToIndices, ImportKind, Module, Table,
};

use crate::error::Error;
use crate::merge_options::MergeOptions;
use crate::named_module::NamedParsedModule;
use crate::resolver::identified_resolution_schema::{
    MergedExport, MergedImport, OrderedResolutionSchema,
};
use crate::resolver::resolution_schema::Before;
use crate::resolver::{
    FunctionImportSpecification, FunctionName, FunctionSpecification, ModuleName, Resolved,
};

mod old_to_new_mapping;
use old_to_new_mapping::Mapping;

mod walrus_copy;
use walrus_copy::WasmFunctionCopy;

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
                index: before_index,
            } = import_specification;
            let ty = merged.types.add(ty.params(), ty.results());
            let (new_function_index, new_import_index) =
                merged.add_import_func(exporting_module_name, function_name, ty);
            let _ = new_import_index;
            mapping.funcs.insert(
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
                    mapping
                        .locals
                        .insert((defining_module.clone(), Before(*old_id)), new_id);
                    new_id
                })
                .collect();
            let builder = FunctionBuilder::new(&mut merged.types, ty.params(), ty.results());
            let new_function_index = builder.finish(locals, &mut merged.funcs);
            mapping
                .funcs
                .insert((defining_module.clone(), index.clone()), new_function_index);
        }

        for resolved in resolution_schema.resolved_iter() {
            let Resolved {
                export_specification,
                resolved_imports,
            } = resolved;
            let local_function_index = *mapping
                .funcs
                .get(&(
                    export_specification.module.clone(),
                    export_specification.index.clone(),
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
                debug_assert_eq!(*exporting_module, export_specification.module);
                mapping.funcs.insert(
                    (importing_module.clone(), before_index.clone()),
                    local_function_index,
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
            self.mapping.globals.insert(
                (
                    ModuleName(considering_module_name.to_string()),
                    Before(global.id()),
                ),
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
            self.mapping.memories.insert(
                (
                    ModuleName(considering_module_name.to_string()),
                    Before(memory.id()),
                ),
                new_memory_id,
            );
        }

        for data in data.iter() {
            let kind = match data.kind {
                DataKind::Active { memory, offset } => DataKind::Active {
                    memory: *self
                        .mapping
                        .memories
                        .get(&(
                            ModuleName(considering_module_name.to_string()),
                            Before(memory),
                        ))
                        .unwrap(),
                    offset,
                },
                DataKind::Passive => DataKind::Passive,
            };
            let new_data_id = self.merged.data.add(kind, data.value.clone());
            self.mapping.datas.insert(
                (
                    ModuleName(considering_module_name.to_string()),
                    Before(data.id()),
                ),
                new_data_id,
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
            self.mapping.tables.insert(
                (
                    ModuleName(considering_module_name.to_string()),
                    Before(table.id()),
                ),
                new_table_id,
            );
            let new_table = self.merged.tables.get_mut(new_table_id);
            new_table.name = name.clone();
            let _ = elem_segments; // Will be copied over after all elements have been set
        }

        for element in elements.iter() {
            let items = match &element.items {
                ElementItems::Functions(ids) => ElementItems::Functions(
                    ids.iter()
                        .map(|old_function_id| {
                            *self
                                .mapping
                                .funcs
                                .get(&(considering_module_name.into(), Before(*old_function_id)))
                                .unwrap()
                        })
                        .collect(),
                ),
                ElementItems::Expressions(refttype, const_expression) => ElementItems::Expressions(
                    *refttype,
                    const_expression
                        .iter()
                        .map(|ce| match ce {
                            ConstExpr::Value(value) => ConstExpr::Value(*value),
                            ConstExpr::RefNull(ref_type) => ConstExpr::RefNull(*ref_type),
                            ConstExpr::Global(id) => ConstExpr::Global(
                                *self
                                    .mapping
                                    .globals
                                    .get(&(considering_module_name.into(), Before(*id)))
                                    .unwrap(),
                            ),
                            ConstExpr::RefFunc(id) => ConstExpr::RefFunc(
                                *self
                                    .mapping
                                    .funcs
                                    .get(&(considering_module_name.into(), Before(*id)))
                                    .unwrap(),
                            ),
                        })
                        .collect(),
                ),
            };
            let kind = match element.kind {
                ElementKind::Passive => ElementKind::Passive,
                ElementKind::Declared => ElementKind::Declared,
                ElementKind::Active { table, offset } => {
                    // This code is copied from above ... move to function!
                    let table = *self
                        .mapping
                        .tables
                        .get(&(considering_module_name.into(), Before(table)))
                        .unwrap();
                    let offset = match offset {
                        ConstExpr::Value(value) => ConstExpr::Value(value),
                        ConstExpr::RefNull(ref_type) => ConstExpr::RefNull(ref_type),
                        ConstExpr::Global(id) => ConstExpr::Global(
                            *self
                                .mapping
                                .globals
                                .get(&(considering_module_name.into(), Before(id)))
                                .unwrap(),
                        ),
                        ConstExpr::RefFunc(id) => ConstExpr::RefFunc(
                            *self
                                .mapping
                                .funcs
                                .get(&(considering_module_name.into(), Before(id)))
                                .unwrap(),
                        ),
                    };
                    ElementKind::Active { table, offset }
                }
            };
            let new_element_id = self.merged.elements.add(kind, items);
            self.mapping.elements.insert(
                (
                    ModuleName(considering_module_name.to_string()),
                    Before(element.id()),
                ),
                new_element_id,
            );
        }

        for table in tables.iter() {
            let Table { elem_segments, .. } = table;
            let new_table_id = *self
                .mapping
                .tables
                .get(&(considering_module_name.into(), Before(table.id())))
                .unwrap();
            let table = self.merged.tables.get_mut(new_table_id);
            for old_element_id in elem_segments.iter() {
                let new_element_id = *self
                    .mapping
                    .elements
                    .get(&(
                        ModuleName(considering_module_name.to_string()),
                        Before(*old_element_id),
                    ))
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
                            let FunctionImportSpecification {
                                importing_module: ModuleName(importing_module_name),
                                exporting_module: _,
                                name: FunctionName(function_name),
                                ty: _,
                                index: _,
                            } = import_spec;

                            // If it is unresolved, assert it was added in the merged output
                            let import_id = *self
                                .mapping
                                .funcs
                                .get(&(importing_module, Before(*before_id)))
                                .unwrap();
                            let new_import = self.merged.imports.get_imported_func(import_id);
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
                    let old_function_index = function.id();
                    let new_function_index = *self
                        .mapping
                        .funcs
                        .get(&(considering_module_name.into(), Before(old_function_index)))
                        .unwrap();

                    let mut visitor: WasmFunctionCopy<'_, '_> = WasmFunctionCopy::new(
                        &considering_module,
                        &mut self.merged,
                        local_function,
                        considering_module_name.to_string(),
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
                        considering_module_name.to_string(),
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
                    let exporting_module = considering_module_name.into();
                    let function_name = export.name.as_str().into();
                    match self
                        .resolution_schema
                        .determine_merged_export(&exporting_module, &function_name)
                    {
                        MergedExport::Resolved => {
                            // FIXME: allow hiding resolved functions
                            let new_function_id = *self
                                .mapping
                                .funcs
                                .get(&(considering_module_name.into(), Before(*before_id)))
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
                                .mapping
                                .funcs
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
                    let duplicate_table_export =
                        self.merged.exports.iter().find(|existing_export| {
                            existing_export.name == export.name
                                && matches!(existing_export.item, ExportItem::Table(_))
                        });
                    match duplicate_table_export {
                        Some(duplicate_table_export) => {
                            if self.options.rename_duplicate_exports {
                                let renamed = format!("{considering_module_name}:{}", export.name);
                                let new_table_id = self
                                    .mapping
                                    .tables
                                    .get(&(considering_module_name.into(), Before(*before_index)))
                                    .unwrap();
                                self.merged
                                    .exports
                                    .add(&renamed, ExportItem::Table(*new_table_id));
                            } else {
                                // TODO: nicer reporting with the duplicate_table_export
                                let _ = duplicate_table_export;
                                return Err(Error::DuplicateNameExport(export.name.clone()));
                            }
                        }
                        None => {
                            let new_table_id = self
                                .mapping
                                .tables
                                .get(&(considering_module_name.into(), Before(*before_index)))
                                .unwrap();
                            self.merged
                                .exports
                                .add(&export.name, ExportItem::Table(*new_table_id));
                        }
                    };
                }
                ExportItem::Memory(before_index) => {
                    let duplicate_memory_export =
                        self.merged.exports.iter().find(|existing_export| {
                            existing_export.name == export.name
                                && matches!(existing_export.item, ExportItem::Memory(_))
                        });
                    match duplicate_memory_export {
                        Some(duplicate_memory_export) => {
                            if self.options.rename_duplicate_exports {
                                let renamed = format!("{considering_module_name}:{}", export.name);
                                let new_memory_id = self
                                    .mapping
                                    .memories
                                    .get(&(considering_module_name.into(), Before(*before_index)))
                                    .unwrap();
                                self.merged
                                    .exports
                                    .add(&renamed, ExportItem::Memory(*new_memory_id));
                            } else {
                                // TODO: nicer reporting with the duplicate_memory_export
                                let _ = duplicate_memory_export;
                                return Err(Error::DuplicateNameExport(export.name.clone()));
                            }
                        }
                        None => {
                            let new_memory_id = self
                                .mapping
                                .memories
                                .get(&(considering_module_name.into(), Before(*before_index)))
                                .unwrap();
                            self.merged
                                .exports
                                .add(&export.name, ExportItem::Memory(*new_memory_id));
                        }
                    }
                }
                // TODO: code dupe with other export forms
                ExportItem::Global(before_index) => {
                    let duplicate_global_export =
                        self.merged.exports.iter().find(|existing_export| {
                            existing_export.name == export.name
                                && matches!(existing_export.item, ExportItem::Global(_))
                        });
                    match duplicate_global_export {
                        Some(duplicate_global_export) => {
                            if self.options.rename_duplicate_exports {
                                let renamed = format!("{considering_module_name}:{}", export.name);
                                let new_global_id = self
                                    .mapping
                                    .globals
                                    .get(&(considering_module_name.into(), Before(*before_index)))
                                    .unwrap();
                                self.merged
                                    .exports
                                    .add(&renamed, ExportItem::Global(*new_global_id));
                            } else {
                                // TODO: nicer reporting with the duplicate_global_export
                                let _ = duplicate_global_export;
                                return Err(Error::DuplicateNameExport(export.name.clone()));
                            }
                        }
                        None => {
                            let new_global_id = self
                                .mapping
                                .globals
                                .get(&(considering_module_name.into(), Before(*before_index)))
                                .unwrap();
                            self.merged
                                .exports
                                .add(&export.name, ExportItem::Global(*new_global_id));
                        }
                    }
                }
            }
        }

        if let Some(old_start_id) = start {
            let new_start_id = self
                .mapping
                .funcs
                .get(&(considering_module_name.into(), Before(*old_start_id)))
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
                .push((considering_module_name.to_string(), name.to_string()));
        }

        Ok(())
    }

    pub(crate) fn build(mut self) -> Module {
        self.merged
            .producers
            .add_processed_by("wasm-mergers", env!("CARGO_PKG_VERSION"));
        let formatted: Vec<_> = self
            .names
            .iter()
            .map(|(module, name)| format!("{module}::{name}"))
            .collect();

        self.merged.name = Some(formatted.join("-"));
        self.merged
    }
}
