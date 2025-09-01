use std::collections::{HashMap as Map, HashSet as Set};
use std::hash::Hash;
use std::marker::PhantomData;

use anyhow::anyhow;
use walrus::Module;

use crate::MergeOptions;
use crate::error::Error;
use crate::kinds::ClashesMap;
use crate::kinds::{ConcreteExport, ExportKind, FuncType, IdentifierItem, IdentifierModule};
use crate::merge_options::{ClashingExports, ExportIdentifier, KeepExports, LinkTypeMismatch};
use crate::merge_options::{DEFAULT_RENAMER, RenameStrategy};
use crate::merger::old_to_new_mapping::{OldIdFunction, OldIdGlobal, OldIdMemory, OldIdTable};
use crate::merger::provenance_identifier::{Identifier, Old};
use crate::named_module::NamedParsedModule;
use crate::resolver::dependency_reduction::ReducedDependencies;
use crate::resolver::error::TypeMismatch;
use crate::resolver::instantiated::{
    ImportDataFunction, ImportDataGlobal, ImportDataMemory, ImportDataTable,
};
use crate::resolver::{Export, Import, Local, Resolver as GraphResolver, instantiated};

#[rustfmt::skip]
pub(crate) mod builder_instantiated {
    use crate::resolver::instantiated::{ImportDataFunction, ImportDataTable, ImportDataMemory, ImportDataGlobal};
    use crate::resolver::instantiated::{ LocalDataFunction,  LocalDataTable,  LocalDataMemory,  LocalDataGlobal};
    use crate::resolver::instantiated::{      TypeFunction,       TypeTable,       TypeMemory,       TypeGlobal};
    use crate::resolver::instantiated::{      KindFunction,       KindTable,       KindMemory,       KindGlobal};
    use crate::merger::old_to_new_mapping::{ OldIdFunction,      OldIdTable,      OldIdMemory,      OldIdGlobal};

    use super::{GraphResolver, ReducedDependencies};

    pub(crate) type ResolverFunction = GraphResolver<KindFunction, TypeFunction, OldIdFunction, ImportDataFunction, LocalDataFunction >;
    pub(crate) type ResolverTable =    GraphResolver<KindTable,    TypeTable,    OldIdTable,    ImportDataTable,    LocalDataTable    >;
    pub(crate) type ResolverMemory =   GraphResolver<KindMemory,   TypeMemory,   OldIdMemory,   ImportDataMemory,   LocalDataMemory   >;
    pub(crate) type ResolverGlobal =   GraphResolver<KindGlobal,   TypeGlobal,   OldIdGlobal,   ImportDataGlobal,   LocalDataGlobal   >;

    pub(crate) type ReducedDependenciesFunction = ReducedDependencies<KindFunction, TypeFunction, OldIdFunction, ImportDataFunction, LocalDataFunction>;
    pub(crate) type ReducedDependenciesTable =    ReducedDependencies<KindTable,    TypeTable,    OldIdTable,    ImportDataTable,    LocalDataTable   >;
    pub(crate) type ReducedDependenciesMemory =   ReducedDependencies<KindMemory,   TypeMemory,   OldIdMemory,   ImportDataMemory,   LocalDataMemory  >;
    pub(crate) type ReducedDependenciesGlobal =   ReducedDependencies<KindGlobal,   TypeGlobal,   OldIdGlobal,   ImportDataGlobal,   LocalDataGlobal  >;
}

#[derive(Debug, Clone)]
pub(crate) struct Resolver {
    function: builder_instantiated::ResolverFunction,
    table: builder_instantiated::ResolverTable,
    memory: builder_instantiated::ResolverMemory,
    global: builder_instantiated::ResolverGlobal,
}

#[derive(Debug, Clone)]
pub(crate) struct AllReducedDependencies {
    pub functions: builder_instantiated::ReducedDependenciesFunction,
    pub tables: builder_instantiated::ReducedDependenciesTable,
    pub memories: builder_instantiated::ReducedDependenciesMemory,
    pub globals: builder_instantiated::ReducedDependenciesGlobal,
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

    fn import_from<Kind, Type, Index, ImportData>(
        import: &walrus::Import,
        module: &IdentifierModule,
        imported_index: Index,
        ty: Type,
        data: ImportData,
    ) -> Import<Kind, Type, Index, ImportData> {
        Import {
            exporting_module: (*import.module).to_string().into(),
            importing_module: module.clone(),
            exporting_identifier: (*import.name).to_string().into(),
            imported_index,
            kind: PhantomData,
            ty,
            data,
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
            identifier: export.name.clone().into(),
            index: exported_index,
            kind: PhantomData,
            ty,
        }
    }

    #[allow(clippy::too_many_lines)] // TODO: fix / remove
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
        // TODO: make all related to 'covered' debug-only enabled code
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
                    let old_id: OldIdFunction = (*old_id_function).into();
                    let data = ImportDataFunction;
                    let import: crate::resolver::instantiated::ImportFunction<OldIdFunction> =
                        Self::import_from(import, &considering_module, old_id, ty, data);
                    self.function.add_import(import);
                }
                walrus::ImportKind::Table(old_id_table) => {
                    covered_table_imports.insert((old_id_table, import.id()));
                    let table = considering_tables.get(*old_id_table);
                    let ty = table.element_ty;
                    let old_id: OldIdTable = (*old_id_table).into();
                    let data = ImportDataTable;
                    let import = Self::import_from(import, &considering_module, old_id, ty, data);
                    self.table.add_import(import);
                }
                walrus::ImportKind::Memory(old_id_memory) => {
                    covered_memory_imports.insert((old_id_memory, import.id()));
                    let old_id: OldIdMemory = (*old_id_memory).into();
                    let data = ImportDataMemory;
                    let import = Self::import_from(import, &considering_module, old_id, (), data);
                    self.memory.add_import(import);
                }
                walrus::ImportKind::Global(old_id_global) => {
                    covered_global_imports.insert((old_id_global, import.id()));
                    let global = considering_globals.get(*old_id_global);
                    let ty = global.ty;
                    let old_id: OldIdGlobal = (*old_id_global).into();
                    let data = ImportDataGlobal {
                        mutable: global.mutable,
                        shared: global.shared,
                    };
                    let import = Self::import_from(import, &considering_module, old_id, ty, data);
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
                    return Err(Error::Parse(anyhow!(
                        "walrus::FunctionKind::Uninitialized during parsing of {considering_module}",
                    )));
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
    /// ref: <https://webassembly.github.io/spec/core/syntax/modules.html#exports>
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

    fn resolve_kind<Kind, Type, Index, ImportData, LocalData>(
        resolver: GraphResolver<Kind, Type, Index, ImportData, LocalData>,
        merge_options: &MergeOptions,
        keep_retriever: KeepRetriever<Kind>,
    ) -> Result<ReducedDependencies<Kind, Type, Index, ImportData, LocalData>, Error>
    where
        Index: Clone + Eq + Hash,
        Kind: Clone + Eq + Hash,
        Type: Clone + Eq + Hash,
        ImportData: Clone + Eq + Hash,
        LocalData: Clone + Eq + Hash,
    {
        let mut linked = resolver.link_nodes().map_err(|_| Error::ImportCycle)?;

        match &merge_options.link_type_mismatch {
            LinkTypeMismatch::Ignore => linked.type_check_mismatch_break(),
            LinkTypeMismatch::Signal => linked
                .type_check_mismatch_signal()
                .map_err(|TypeMismatch(mismatches)| Error::TypeMismatch(mismatches))?,
        }

        let keeper = merge_options.keep_exports.as_ref().map(keep_retriever);
        Ok(linked.reduce_dependencies(keeper))
    }
}

pub(crate) struct AllResolved {
    pub(crate) all_reduced: AllReducedDependencies,
    pub(crate) rename_map: MergeRenamer,
}

impl ClashingExports {
    fn handle(self, clashes_result: ClashesResult) -> Result<MergeRenamer, Error> {
        let ClashesResult::Some(clashes) = clashes_result else {
            return Ok(MergeRenamer::for_no_clashes_present());
        };

        match self {
            ClashingExports::Rename(strategy) => Ok(MergeRenamer::new(clashes, strategy)),
            ClashingExports::Signal => Err(Error::ExportNameClash(clashes)),
        }
    }
}

pub(crate) struct MergeRenamer {
    pub(crate) clashes_map: ClashesMap,
    pub(crate) rename_strategy: RenameStrategy,

    /// During the growing phase, set of renamed names.
    rename_encountered: Set<String>,

    /// Allow constructor to express that clashes should be present.
    #[cfg(debug_assertions)]
    clashes_should_be_present: bool,
    /// Growing set validating clashes will not occur if the flag
    /// [`MergeRenamer::no_clashes_present`] is true.
    #[cfg(debug_assertions)]
    encountered: Set<String>,
}

impl MergeRenamer {
    pub(crate) fn new(clashes_map: ClashesMap, rename_strategy: RenameStrategy) -> Self {
        Self {
            clashes_map,
            rename_strategy,
            rename_encountered: Default::default(),

            #[cfg(debug_assertions)]
            clashes_should_be_present: true,
            #[cfg(debug_assertions)]
            encountered: Default::default(),
        }
    }

    pub(crate) fn for_no_clashes_present() -> Self {
        let clashes_map = ClashesMap::new();
        let rename_strategy = DEFAULT_RENAMER; // ... unused anyway 🙈

        Self {
            clashes_map,
            rename_strategy,
            rename_encountered: Default::default(),

            #[cfg(debug_assertions)]
            clashes_should_be_present: false,
            #[cfg(debug_assertions)]
            encountered: Default::default(),
        }
    }

    /// This method will compute the export name in the output module given the
    /// configuration for merging. That is, if exports names may conflict, the
    /// configuration will determine if and how a new export name is computed.
    ///
    /// See [`ClashingExports`] for the different configuration options.
    pub(crate) fn compute_export_name<Kind: Clone, Type, Index>(
        &mut self,
        old_export: &mut Export<Kind, Type, Index>,
        rename_fetcher: RenameRetriever<Kind>,
    ) {
        #[cfg(debug_assertions)]
        {
            let clashes_not_present = !self.clashes_should_be_present;
            if clashes_not_present {
                let newly_inserted = self
                    .encountered
                    .insert(old_export.identifier.identifier().to_string());
                debug_assert!(newly_inserted);
            }
        }

        let clashes = self
            .clashes_map
            .contains_key(old_export.identifier().identifier());

        if clashes {
            let newly_inserted = self
                .rename_encountered
                .insert(String::from(old_export.identifier().identifier()));

            // If renaming the first is not enabled but the insertion was new:
            if !self.rename_strategy.first_occurrence && newly_inserted {
                // Skip the rename
                return;
            }

            // Perform the rename
            let renamer = rename_fetcher(&self.rename_strategy);
            old_export.identifier = renamer(old_export.module(), old_export.identifier().clone());
        }
    }
}

#[cfg(debug_assertions)]
impl Drop for MergeRenamer {
    /// Assert that the first phase & the effective merge agree on the outcome.
    fn drop(&mut self) {
        let rename_did_not_happen = self.rename_encountered.is_empty();
        let rename_did_happen = !rename_did_not_happen;
        if self.clashes_should_be_present {
            debug_assert!(rename_did_happen);
        } else {
            debug_assert!(rename_did_not_happen);
        }
    }
}

#[derive(Debug)]
enum ClashesResult {
    None,
    Some(ClashesMap),
}

trait CollectExports {
    fn collect_into(&self, exports: &mut Map<String, Vec<ConcreteExport>>);
}

impl From<&instantiated::ExportFunction<OldIdFunction>> for ConcreteExport {
    fn from(export: &instantiated::ExportFunction<OldIdFunction>) -> Self {
        Self {
            kind: ExportKind::Function,
            exporting_module: export.module().identifier().to_string(),
        }
    }
}

impl From<&instantiated::ExportGlobal<OldIdGlobal>> for ConcreteExport {
    fn from(export: &instantiated::ExportGlobal<OldIdGlobal>) -> Self {
        Self {
            kind: ExportKind::Global,
            exporting_module: export.module().identifier().to_string(),
        }
    }
}

impl From<&instantiated::ExportMemory<OldIdMemory>> for ConcreteExport {
    fn from(export: &instantiated::ExportMemory<OldIdMemory>) -> Self {
        Self {
            kind: ExportKind::Memory,
            exporting_module: export.module().identifier().to_string(),
        }
    }
}

impl From<&instantiated::ExportTable<OldIdTable>> for ConcreteExport {
    fn from(export: &instantiated::ExportTable<OldIdTable>) -> Self {
        Self {
            kind: ExportKind::Table,
            exporting_module: export.module().identifier().to_string(),
        }
    }
}

impl<'a, Kind: 'a, Type: 'a, Index: 'a, ImportData: 'a, LocalData: 'a> CollectExports
    for &'a ReducedDependencies<Kind, Type, Index, ImportData, LocalData>
where
    &'a Export<Kind, Type, Index>: Into<ConcreteExport>,
{
    fn collect_into(&self, exports: &mut Map<String, Vec<ConcreteExport>>) {
        for remaining_export in &self.remaining_exports {
            let entry = exports
                .entry(remaining_export.identifier().identifier().to_string())
                .or_default();
            let export: ConcreteExport = remaining_export.into();
            entry.push(export);
        }
    }
}
