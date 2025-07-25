use std::collections::{HashMap as Map, HashSet as Set};
use std::hash::Hash;
use std::marker::PhantomData;

use walrus::{FunctionId, Module, TableId};
use walrus::{GlobalId, RefType};
use walrus::{MemoryId, ValType};

use crate::MergeOptions;
use crate::error::Error;
use crate::kinds::{FuncType, IdentifierItem, IdentifierModule, Locals};
use crate::kinds::{Function, Global, Memory, Table};
use crate::merge_options::{ClashingExports, ExportIdentifier, KeepExports, LinkTypeMismatch};
use crate::merge_options::{DEFAULT_RENAMER, RenameStrategy};
use crate::merger::old_to_new_mapping::{OldIdFunction, OldIdGlobal, OldIdMemory, OldIdTable};
use crate::merger::provenance_identifier::{Identifier, Old};
use crate::named_module::NamedParsedModule;
use crate::resolver::dependency_reduction::ReducedDependencies;
use crate::resolver::{Export, Import, Local, Resolver as GraphResolver, instantiated};

#[derive(Debug, Clone)]
pub(crate) struct Resolver {
    function: GraphResolver<Function, FuncType, OldIdFunction, Locals>,
    table: GraphResolver<Table, RefType, OldIdTable, ()>,
    memory: GraphResolver<Memory, (), OldIdMemory, ()>,
    global: GraphResolver<Global, ValType, OldIdGlobal, ()>,
}

pub(crate) type ReducedDependenciesFunction =
    ReducedDependencies<Function, FuncType, OldIdFunction, Locals>;

pub(crate) type ReducedDependenciesTable /*. */ =
    ReducedDependencies<Table, RefType, OldIdTable, ()>;

#[derive(Debug, Clone)]
pub(crate) struct AllReducedDependencies {
    pub functions: ReducedDependenciesFunction,
    pub tables: ReducedDependenciesTable,
    pub memories: ReducedDependencies<Memory, (), OldIdMemory, ()>,
    pub globals: ReducedDependencies<Global, ValType, OldIdGlobal, ()>,
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

    pub(crate) fn resolve(self, merge_options: &MergeOptions) -> Result<AllResolved, Error> {
        let all_reduced = AllReducedDependencies {
            functions: Self::resolve_kind(self.function, merge_options, KeepExports::functions)?,
            tables: Self::resolve_kind(self.table, merge_options, KeepExports::tables)?,
            memories: Self::resolve_kind(self.memory, merge_options, KeepExports::memories)?,
            globals: Self::resolve_kind(self.global, merge_options, KeepExports::globals)?,
        };

        let clashes_result = Self::identify_clashes(&all_reduced);
        let rename_map = merge_options
            .clashing_exports
            .clone()
            .handle(clashes_result)?;

        Ok(AllResolved {
            all_reduced,
            rename_map,
        })
    }

    /// Identifies all name clashes, as all export names should be unique.
    /// ref: https://webassembly.github.io/spec/core/syntax/modules.html#exports
    fn identify_clashes(reduced_dependencies: &AllReducedDependencies) -> ClashesResult {
        let mut module_exports: Map<String, Vec<ConcreteExport>> = Map::new();

        let dependencies: &[Box<dyn CollectExports>] = &[
            Box::new(&reduced_dependencies.functions),
            Box::new(&reduced_dependencies.globals),
            Box::new(&reduced_dependencies.memories),
            Box::new(&reduced_dependencies.tables),
        ];

        for dependency in dependencies {
            dependency.collect_into(&mut module_exports);
        }

        // Remove all non-clashes
        module_exports.retain(|_, exports| {
            debug_assert!(!exports.is_empty());
            exports.len() > 1
        });

        if module_exports.is_empty() {
            ClashesResult::None
        } else {
            ClashesResult::Some(module_exports)
        }
    }

    fn resolve_kind<Kind, Type, Index, LocalData>(
        resolver: GraphResolver<Kind, Type, Index, LocalData>,
        merge_options: &MergeOptions,
        keep_retriever: KeepRetriever<Kind>,
    ) -> Result<ReducedDependencies<Kind, Type, Index, LocalData>, Error>
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
        Ok(linked.reduce_dependencies(keeper))
    }
}

pub(crate) struct AllResolved {
    pub(crate) all_reduced: AllReducedDependencies,
    pub(crate) rename_map: RenameMap,
}

impl ClashingExports {
    fn handle(self, clashes_result: ClashesResult) -> Result<RenameMap, Error> {
        match (clashes_result, self) {
            (ClashesResult::None, ClashingExports::Signal) => Ok(RenameMap::empty()),
            (ClashesResult::None, ClashingExports::Rename(_)) => Ok(RenameMap::empty()),
            (ClashesResult::Some(_), ClashingExports::Signal) => Err(Error::ExportNameClash),
            (ClashesResult::Some(clashes_map), ClashingExports::Rename(rename_strategy)) => {
                Ok(RenameMap::new(clashes_map, rename_strategy))
            }
        }
    }
}

pub(crate) struct RenameMap {
    pub(crate) clashes_map: ClashesMap,
    pub(crate) rename_strategy: RenameStrategy,
}

impl RenameMap {
    pub(crate) fn new(clashes_map: ClashesMap, rename_strategy: RenameStrategy) -> Self {
        Self {
            clashes_map,
            rename_strategy,
        }
    }

    pub(crate) fn empty() -> Self {
        let clashes_map = ClashesMap::new();
        let rename_strategy = DEFAULT_RENAMER; // ... unused anyway 🙈
        Self {
            clashes_map,
            rename_strategy,
        }
    }

    /// If the `old_export` will be exported, then optionally provide a new name
    pub(crate) fn rename_if_required<Kind, Type, Index>(
        &self,
        old_export: Box<Export<Kind, Type, Index>>,
        rename_fetcher: RenameRetriever<Kind>,
    ) -> Box<Export<Kind, Type, Index>>
    where
        Kind: Clone,
        Type: Clone,
        Index: Clone,
    {
        let clashes = self
            .clashes_map
            .contains_key(old_export.identifier().identifier());
        if clashes {
            let mut renamed_export = (*old_export).clone();
            let renamer = rename_fetcher(&self.rename_strategy);
            renamed_export.identifier =
                renamer(renamed_export.module(), renamed_export.identifier.clone());
            Box::new(renamed_export)
        } else {
            old_export
        }
    }
}

type ClashesMap = Map<String, Vec<ConcreteExport>>;

#[derive(Debug)]
enum ClashesResult {
    None,
    Some(ClashesMap),
}

#[derive(Debug)]
pub(crate) enum ConcreteExport {
    Function,
    Global,
    Memory,
    Table,
}

trait CollectExports {
    fn collect_into(&self, exports: &mut Map<String, Vec<ConcreteExport>>);
}

impl From<&instantiated::ExportFunction<Identifier<Old, FunctionId>>> for ConcreteExport {
    fn from(_: &instantiated::ExportFunction<Identifier<Old, FunctionId>>) -> Self {
        Self::Function
    }
}
impl From<&instantiated::ExportGlobal<Identifier<Old, GlobalId>>> for ConcreteExport {
    fn from(_: &instantiated::ExportGlobal<Identifier<Old, GlobalId>>) -> Self {
        Self::Global
    }
}

impl From<&instantiated::ExportMemory<Identifier<Old, MemoryId>>> for ConcreteExport {
    fn from(_: &instantiated::ExportMemory<Identifier<Old, MemoryId>>) -> Self {
        Self::Memory
    }
}

impl From<&instantiated::ExportTable<Identifier<Old, TableId>>> for ConcreteExport {
    fn from(_: &instantiated::ExportTable<Identifier<Old, TableId>>) -> Self {
        Self::Table
    }
}

impl<'a, Kind: 'a, Type: 'a, Index: 'a, LocalData: 'a> CollectExports
    for &'a ReducedDependencies<Kind, Type, Index, LocalData>
where
    &'a Export<Kind, Type, Index>: Into<ConcreteExport>,
{
    fn collect_into(&self, exports: &mut Map<String, Vec<ConcreteExport>>) {
        for remaining_export in self.remaining_exports.iter() {
            let entry = exports
                .entry(remaining_export.identifier().identifier().to_string())
                .or_default();
            let export: ConcreteExport = remaining_export.into();
            entry.push(export);
        }
    }
}
