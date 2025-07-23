use std::collections::HashSet as Set;

use walrus::Module;
use walrus::RefType;
use walrus::ValType;

use crate::MergeOptions;
use crate::error::Error;
use crate::kinds::FuncType;
use crate::kinds::Locals;
use crate::merge_options::ClashingExports;
use crate::merge_options::KeepExports;
use crate::merge_options::LinkTypeMismatch;
use crate::merger::old_to_new_mapping::OldIdFunction;
use crate::merger::old_to_new_mapping::OldIdGlobal;
use crate::merger::old_to_new_mapping::OldIdMemory;
use crate::merger::old_to_new_mapping::OldIdTable;
use crate::merger::provenance_identifier::Identifier;
use crate::merger::provenance_identifier::Old;
use crate::named_module::NamedParsedModule;
use crate::resolver::Global;
use crate::resolver::IdentifierModule;
use crate::resolver::Memory;
use crate::resolver::Table;
use crate::resolver::dependency_reduction::ReducedDependencies;
use crate::resolver::{Export, Function, Import, Local, Resolver as GraphResolver};

#[derive(Debug)]
pub(crate) struct Resolver {
    resolver_function: GraphResolver<Function, FuncType, OldIdFunction, Locals>,
    resolver_table: GraphResolver<Table, RefType, OldIdTable, ()>,
    resolver_memory: GraphResolver<Memory, (), OldIdMemory, ()>,
    resolver_global: GraphResolver<Global, ValType, OldIdGlobal, ()>,
}

impl Resolver {
    pub(crate) fn new() -> Self {
        Self {
            resolver_function: GraphResolver::new(),
            resolver_table: GraphResolver::new(),
            resolver_global: GraphResolver::new(),
            resolver_memory: GraphResolver::new(),
        }
    }

    fn import_from<Kind: Default, Type, Index>(
        import: &walrus::Import,
        module: &IdentifierModule,
        imported_index: Index,
        ty: Type,
    ) -> Import<Kind, Type, Index> {
        Import {
            exporting_module: (*import.module).to_string().into(),
            importing_module: module.clone(),
            exporting_identifier: (*import.name).to_string().into(),
            imported_index,
            kind: Kind::default(),
            ty,
        }
    }

    fn local_from<Kind: Default, Type, Index, LocalData>(
        module: &IdentifierModule,
        index: Index,
        ty: Type,
        data: LocalData,
    ) -> Local<Kind, Type, Index, LocalData> {
        Local {
            module: module.clone(),
            index,
            kind: Kind::default(),
            ty,
            data,
        }
    }

    fn export_from<Kind: Default, Type, Index>(
        export: &walrus::Export,
        module: &IdentifierModule,
        exported_index: Index,
        ty: Type,
    ) -> Export<Kind, Type, Index> {
        Export {
            module: module.clone(),
            identifier: export.name.to_string().into(),
            index: exported_index,
            kind: Kind::default(),
            ty,
        }
    }

    pub(crate) fn consider(&mut self, module: &NamedParsedModule<'_>) -> Result<(), Error> {
        let NamedParsedModule {
            name: considering_module,
            module,
        } = module;
        let Module {
            types: considering_types,
            imports: considering_imports,
            funcs: considering_funcs,
            globals: considering_globals,
            memories: considering_memories,
            tables: considering_tables,
            exports: considering_exports,
            locals: considering_locals,
            ..
        } = module;

        let considering_module: IdentifierModule = considering_module.to_string().into();
        let mut covered_function_imports = Set::new();

        // FIXME: move this to individual elements?
        for import in considering_imports.iter() {
            match &import.kind {
                walrus::ImportKind::Function(old_id_function) => {
                    covered_function_imports.insert((old_id_function, import.id()));
                    let func = considering_funcs.get(*old_id_function);
                    let ty = FuncType::from_types(func.ty(), considering_types);
                    let old_id_function: OldIdFunction = (*old_id_function).into();
                    let import =
                        Self::import_from(import, &considering_module, old_id_function, ty);
                    self.resolver_function.add_import(import);
                }
                walrus::ImportKind::Table(old_id_table) => {
                    let table = considering_tables.get(*old_id_table);
                    let ty = table.element_ty;
                    let old_id_table: OldIdTable = (*old_id_table).into();
                    let import = Self::import_from(import, &considering_module, old_id_table, ty);
                    self.resolver_table.add_import(import);
                }
                walrus::ImportKind::Memory(old_id_memory) => {
                    let old_id_memory: OldIdMemory = (*old_id_memory).into();
                    let import = Self::import_from(import, &considering_module, old_id_memory, ());
                    self.resolver_memory.add_import(import);
                }
                walrus::ImportKind::Global(old_id_global) => {
                    let global = considering_globals.get(*old_id_global);
                    let ty = global.ty;
                    let old_id_global: OldIdGlobal = (*old_id_global).into();
                    let import = Self::import_from(import, &considering_module, old_id_global, ty);
                    self.resolver_global.add_import(import);
                }
            }
        }

        for function in considering_funcs.iter() {
            match &function.kind {
                walrus::FunctionKind::Local(local_function) => {
                    let locals = local_function
                        .args
                        .iter()
                        .map(|local| {
                            let local = considering_locals.get(*local);
                            (local.id(), local.ty())
                        })
                        .collect::<Vec<_>>()
                        .into_boxed_slice();

                    let old_id_local: Identifier<Old, _> = function.id().into();
                    let ty = FuncType::from_types(local_function.ty(), considering_types);
                    let local = Self::local_from(&considering_module, old_id_local, ty, locals);
                    self.resolver_function.add_local(local);
                }
                walrus::FunctionKind::Import(i) => {
                    debug_assert!(covered_function_imports.contains(&(&function.id(), i.import)));
                }
                walrus::FunctionKind::Uninitialized(_) => {
                    return Err(Error::ComponentModelUnsupported(
                        considering_module.identifier().to_string(),
                    ));
                }
            }
        }

        for global in considering_globals.iter() {
            let _ = global; // TODO: should this be further processed?
        }

        for memory in considering_memories.iter() {
            let _ = memory; // TODO: should this be further processed?
        }

        for table in considering_tables.iter() {
            let _ = table; // TODO: should this be further processed?
        }

        for export in considering_exports.iter() {
            match &export.item {
                walrus::ExportItem::Function(old_id_function) => {
                    let func = considering_funcs.get(*old_id_function);
                    let old_id_function: Identifier<Old, _> = (*old_id_function).into();
                    let ty = FuncType::from_types(func.ty(), considering_types);
                    let export =
                        Self::export_from(export, &considering_module, old_id_function, ty);
                    self.resolver_function.add_export(export);
                }
                walrus::ExportItem::Table(old_id_table) => {
                    let table = considering_tables.get(*old_id_table);
                    let old_id_table: Identifier<Old, _> = (*old_id_table).into();
                    let ty = table.element_ty;
                    let export = Self::export_from(export, &considering_module, old_id_table, ty);
                    self.resolver_table.add_export(export);
                }
                walrus::ExportItem::Memory(old_id_memory) => {
                    let old_id_memory: Identifier<Old, _> = (*old_id_memory).into();
                    let export = Self::export_from(export, &considering_module, old_id_memory, ());
                    self.resolver_memory.add_export(export);
                }
                walrus::ExportItem::Global(old_id_global) => {
                    let global = considering_globals.get(*old_id_global);
                    let old_id_global: Identifier<Old, _> = (*old_id_global).into();
                    let ty = global.ty;
                    let export = Self::export_from(export, &considering_module, old_id_global, ty);
                    self.resolver_global.add_export(export);
                }
            }
        }

        Ok(())
    }

    pub(crate) fn resolve(
        self,
        merge_options: &MergeOptions,
    ) -> Result<ReducedDependencies<Function, FuncType, OldIdFunction, Locals>, Error> {
        // Link all up
        let mut linked = self
            .resolver_function
            .link_nodes()
            .map_err(|_| Error::ImportCycle)?;

        // Assert exports are not clashing
        match &merge_options.clashing_exports {
            ClashingExports::Rename(rename_strategy) => {
                linked.clashing_rename(rename_strategy.functions);
            }
            ClashingExports::Signal => linked
                .clashing_signal()
                .map_err(|_| Error::ExportNameClash)?,
        }

        // Assert types match
        match &merge_options.link_type_mismatch {
            LinkTypeMismatch::Ignore => linked.type_check_mismatch_break(),
            LinkTypeMismatch::Signal => linked
                .type_check_mismatch_signal()
                .map_err(|_| Error::TypeMismatch)?,
        }

        // Reduce all dependencies
        let reduced_dependencies = linked.reduce_dependencies(
            merge_options
                .keep_exports
                .as_ref()
                .map(KeepExports::functions),
        );

        // TODO: Resolve tables, memories, globals

        Ok(reduced_dependencies)
    }
}
