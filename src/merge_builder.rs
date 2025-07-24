use std::collections::{HashMap as Map, HashSet as Set};
use std::hash::Hash;
use std::marker::PhantomData;

use walrus::Module;
use walrus::RefType;
use walrus::ValType;

use crate::MergeOptions;
use crate::error::Error;
use crate::kinds::FuncType;
use crate::kinds::Function;
use crate::kinds::Global;
use crate::kinds::IdentifierItem;
use crate::kinds::IdentifierModule;
use crate::kinds::Locals;
use crate::kinds::Memory;
use crate::kinds::Table;
use crate::merge_options::ClashingExports;
use crate::merge_options::ExportIdentifier;
use crate::merge_options::KeepExports;
use crate::merge_options::LinkTypeMismatch;
use crate::merge_options::RenameStrategy;
use crate::merger::old_to_new_mapping::OldIdFunction;
use crate::merger::old_to_new_mapping::OldIdGlobal;
use crate::merger::old_to_new_mapping::OldIdMemory;
use crate::merger::old_to_new_mapping::OldIdTable;
use crate::merger::provenance_identifier::Identifier;
use crate::merger::provenance_identifier::Old;
use crate::named_module::NamedParsedModule;
use crate::resolver::dependency_reduction::{ReducedDependencies, ReductionMap};
use crate::resolver::{Export, Import, Local, Resolver as GraphResolver};

#[derive(Debug, Clone)]
pub(crate) struct Resolver {
    function: GraphResolver<Function, FuncType, OldIdFunction, Locals>,
    table: GraphResolver<Table, RefType, OldIdTable, ()>,
    memory: GraphResolver<Memory, (), OldIdMemory, ()>,
    global: GraphResolver<Global, ValType, OldIdGlobal, ()>,
}

#[derive(Debug, Clone)]
pub(crate) struct AllMergeStrategies {
    pub functions: MergeStrategy<Function, FuncType, OldIdFunction, Locals>,
    pub tables: MergeStrategy<Table, RefType, OldIdTable, ()>,
    pub memories: MergeStrategy<Memory, (), OldIdMemory, ()>,
    pub globals: MergeStrategy<Global, ValType, OldIdGlobal, ()>,
}

#[derive(Debug, Clone)]
pub(crate) struct MergeStrategy<Kind, Type, Index, LocalData> {
    /// Maps each node to its reduction source (either a remaining import or a local)
    pub(crate) reduction_map: ReductionMap<Kind, Type, Index, LocalData>,

    /// The remaining imports that should be present after resolution
    pub(crate) remaining_imports: Set<Import<Kind, Type, Index>>,

    /// The remaining exports that should be present after resolution
    pub(crate) remaining_exports: Set<Export<Kind, Type, Index>>,

    /// The rename map
    pub(crate) renames: Map<ExportIdentifier<IdentifierItem<Kind>>, IdentifierItem<Kind>>,
}

type KeepRetriever<Kind> = fn(&KeepExports) -> &Set<ExportIdentifier<IdentifierItem<Kind>>>;
type RenameRetriever<Kind> =
    fn(&RenameStrategy) -> &fn(&IdentifierModule, IdentifierItem<Kind>) -> IdentifierItem<Kind>;

impl Resolver {
    pub(crate) fn new() -> Self {
        Self {
            function: GraphResolver::new(),
            table: GraphResolver::new(),
            global: GraphResolver::new(),
            memory: GraphResolver::new(),
        }
    }

    fn import_from<Kind, Type, Index>(
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
            kind: PhantomData,
            ty,
        }
    }

    fn local_from<Kind, Type, Index, LocalData>(
        module: &IdentifierModule,
        index: Index,
        ty: Type,
        data: LocalData,
    ) -> Local<Kind, Type, Index, LocalData> {
        Local {
            module: module.clone(),
            index,
            kind: PhantomData,
            ty,
            data,
        }
    }

    fn export_from<Kind, Type, Index>(
        export: &walrus::Export,
        module: &IdentifierModule,
        exported_index: Index,
        ty: Type,
    ) -> Export<Kind, Type, Index> {
        Export {
            module: module.clone(),
            identifier: export.name.to_string().into(),
            index: exported_index,
            kind: PhantomData,
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

        let considering_module: IdentifierModule = (*considering_module).to_string().into();
        let mut covered_function_imports = Set::new();
        let mut covered_table_imports = Set::new();
        let mut covered_memory_imports = Set::new();
        let mut covered_global_imports = Set::new();

        // Process all imports
        for import in considering_imports.iter() {
            match &import.kind {
                walrus::ImportKind::Function(old_id_function) => {
                    covered_function_imports.insert((old_id_function, import.id()));
                    let func = considering_funcs.get(*old_id_function);
                    let ty = FuncType::from_types(func.ty(), considering_types);
                    let old_id_function: OldIdFunction = (*old_id_function).into();
                    let import =
                        Self::import_from(import, &considering_module, old_id_function, ty);
                    self.function.add_import(import);
                }
                walrus::ImportKind::Table(old_id_table) => {
                    covered_table_imports.insert((old_id_table, import.id()));
                    let table = considering_tables.get(*old_id_table);
                    let ty = table.element_ty;
                    let old_id_table: OldIdTable = (*old_id_table).into();
                    let import = Self::import_from(import, &considering_module, old_id_table, ty);
                    self.table.add_import(import);
                }
                walrus::ImportKind::Memory(old_id_memory) => {
                    covered_memory_imports.insert((old_id_memory, import.id()));
                    let old_id_memory: OldIdMemory = (*old_id_memory).into();
                    let import = Self::import_from(import, &considering_module, old_id_memory, ());
                    self.memory.add_import(import);
                }
                walrus::ImportKind::Global(old_id_global) => {
                    covered_global_imports.insert((old_id_global, import.id()));
                    let global = considering_globals.get(*old_id_global);
                    let ty = global.ty;
                    let old_id_global: OldIdGlobal = (*old_id_global).into();
                    let import = Self::import_from(import, &considering_module, old_id_global, ty);
                    self.global.add_import(import);
                }
            }
        }

        // Process functions
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

                    let local = Local {
                        module: considering_module.clone(),
                        index: function.id().into(),
                        kind: PhantomData,
                        ty: FuncType::from_types(local_function.ty(), considering_types),
                        data: locals.clone(),
                    };
                    self.function.add_local(local);
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

        // Process globals
        for global in considering_globals.iter() {
            match &global.kind {
                walrus::GlobalKind::Local(local_global) => {
                    let _ = local_global; // Particular expression is not of interest
                    let local =
                        Self::local_from(&considering_module, global.id().into(), global.ty, ());
                    self.global.add_local(local);
                }
                walrus::GlobalKind::Import(i) => {
                    debug_assert!(covered_global_imports.contains(&(&global.id(), *i)));
                }
            }
        }

        // Process memories
        for memory in considering_memories.iter() {
            if let Some(i) = &memory.import {
                debug_assert!(covered_memory_imports.contains(&(&memory.id(), *i)));
            } else {
                let local = Self::local_from(&considering_module, memory.id().into(), (), ());
                self.memory.add_local(local);
            }
        }

        // Process tables
        for table in considering_tables.iter() {
            if let Some(i) = &table.import {
                debug_assert!(covered_table_imports.contains(&(&table.id(), *i)));
            } else {
                let local =
                    Self::local_from(&considering_module, table.id().into(), table.element_ty, ());
                self.table.add_local(local);
            }
        }

        // Process exports
        for export in considering_exports.iter() {
            match &export.item {
                walrus::ExportItem::Function(old_id_function) => {
                    let func = considering_funcs.get(*old_id_function);
                    let old_id_function: Identifier<Old, _> = (*old_id_function).into();
                    let ty = FuncType::from_types(func.ty(), considering_types);
                    let export =
                        Self::export_from(export, &considering_module, old_id_function, ty);
                    self.function.add_export(export);
                }
                walrus::ExportItem::Table(old_id_table) => {
                    let table = considering_tables.get(*old_id_table);
                    let old_id_table: Identifier<Old, _> = (*old_id_table).into();
                    let ty = table.element_ty;
                    let export = Self::export_from(export, &considering_module, old_id_table, ty);
                    self.table.add_export(export);
                }
                walrus::ExportItem::Memory(old_id_memory) => {
                    let old_id_memory: Identifier<Old, _> = (*old_id_memory).into();
                    let export = Self::export_from(export, &considering_module, old_id_memory, ());
                    self.memory.add_export(export);
                }
                walrus::ExportItem::Global(old_id_global) => {
                    let global = considering_globals.get(*old_id_global);
                    let old_id_global: Identifier<Old, _> = (*old_id_global).into();
                    let ty = global.ty;
                    let export = Self::export_from(export, &considering_module, old_id_global, ty);
                    self.global.add_export(export);
                }
            }
        }

        Ok(())
    }

    pub(crate) fn resolve(self, merge_options: &MergeOptions) -> Result<AllMergeStrategies, Error> {
        let Self {
            function: resolver_function,
            table: resolver_table,
            memory: resolver_memory,
            global: resolver_global,
        } = self;

        let resolved_functions = Self::resolve_kind(
            resolver_function,
            merge_options,
            KeepExports::functions,
            RenameStrategy::functions,
        )?;

        let resolved_tables = Self::resolve_kind(
            resolver_table,
            merge_options,
            KeepExports::tables,
            RenameStrategy::tables,
        )?;

        let resolved_memories = Self::resolve_kind(
            resolver_memory,
            merge_options,
            KeepExports::memories,
            RenameStrategy::memories,
        )?;

        let resolved_globals = Self::resolve_kind(
            resolver_global,
            merge_options,
            KeepExports::globals,
            RenameStrategy::globals,
        )?;

        Ok(AllMergeStrategies {
            functions: resolved_functions,
            tables: resolved_tables,
            memories: resolved_memories,
            globals: resolved_globals,
        })
    }

    fn resolve_kind<Kind, Type, Index, LocalData>(
        resolver: GraphResolver<Kind, Type, Index, LocalData>,
        merge_options: &MergeOptions,
        keep_retriever: KeepRetriever<Kind>,
        rename_retriever: RenameRetriever<Kind>,
    ) -> Result<MergeStrategy<Kind, Type, Index, LocalData>, Error>
    where
        Index: Clone + Eq + Hash,
        Kind: Clone + Eq + Hash,
        Type: Clone + Eq + Hash,
        LocalData: Clone + Eq + Hash,
    {
        let mut linked = resolver.link_nodes().map_err(|_| Error::ImportCycle)?;

        match &merge_options.link_type_mismatch {
            LinkTypeMismatch::Ignore => linked.type_check_mismatch_break(),
            LinkTypeMismatch::Signal => linked
                .type_check_mismatch_signal()
                .map_err(|_| Error::TypeMismatch)?,
        }

        let keeper = merge_options.keep_exports.as_ref().map(keep_retriever);
        let ReducedDependencies {
            reduction_map,
            remaining_imports,
            remaining_exports,
            clashing_exports,
        } = linked.reduce_dependencies(keeper);

        let mut renames = Map::new();
        match &merge_options.clashing_exports {
            ClashingExports::Rename(rename_strategy) => {
                let renamer = rename_retriever(rename_strategy);
                for clash in clashing_exports {
                    let renamed = renamer(&clash.module, clash.name.clone());
                    renames.insert(clash, renamed);
                }
            }
            ClashingExports::Signal => {
                if !clashing_exports.is_empty() {
                    return Err(Error::ExportNameClash);
                }
            }
        }

        Ok(MergeStrategy {
            reduction_map,
            remaining_imports,
            remaining_exports,
            renames,
        })
    }
}
