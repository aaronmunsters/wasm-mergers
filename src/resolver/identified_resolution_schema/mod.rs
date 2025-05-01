use std::collections::HashMap;

use super::FunctionExportSpecification;
use super::FunctionImportSpecification;
use super::FunctionIndexYielder;
use super::FunctionName;
use super::FunctionSpecification;
use super::ModuleName;
use super::ResolutionSchema;
use super::Resolved;
use super::resolution_schema::BeforeFunctionIndex;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AfterFunctionIndex {
    pub(crate) index: usize,
}

impl From<usize> for AfterFunctionIndex {
    fn from(value: usize) -> Self {
        AfterFunctionIndex { index: value }
    }
}

pub(crate) type ResolvedIndexMap = (BeforeFunctionIndex, AfterFunctionIndex);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrderedResolutionSchema {
    /// An imported function that could not be matched with an exported function
    unresolved_imports: Vec<FunctionImportSpecification<ResolvedIndexMap>>,
    /// The resolved functions, where a single export is linked to the corresponding imports
    resolved: Vec<Resolved<ResolvedIndexMap>>,
    /// The internally defined functions
    internal: Vec<FunctionSpecification<(BeforeFunctionIndex, AfterFunctionIndex)>>,
    /// An exported function that could not be matched with an imported function
    unresolved_exports: Vec<FunctionExportSpecification<ResolvedIndexMap>>,
    /// A hashmap to determine function indices
    old_to_new_function_resolver: HashMap<(ModuleName, BeforeFunctionIndex), AfterFunctionIndex>,
}

pub(crate) enum MergedImport {
    Resolved,
    Unresolved(FunctionImportSpecification<ResolvedIndexMap>),
}

pub(crate) enum MergedExport {
    Resolved,
    Unresolved(FunctionExportSpecification<ResolvedIndexMap>),
}

impl OrderedResolutionSchema {
    pub(crate) fn determine_merged_import(
        &self,
        exporting_module: &ModuleName,
        function_name: &FunctionName,
        importing_module: &ModuleName,
    ) -> MergedImport {
        match self.unresolved_imports.iter().find(|i| {
            i.exporting_module == *exporting_module
                && i.name == *function_name
                && i.importing_module == *importing_module
        }) {
            Some(unresolved_import) => MergedImport::Unresolved(unresolved_import.clone()),
            None => MergedImport::Resolved,
        }
    }

    pub(crate) fn determine_merged_export(
        &self,
        exporting_module: &ModuleName,
        function_name: &FunctionName,
    ) -> MergedExport {
        match self
            .unresolved_exports
            .iter()
            .find(|e| e.module == *exporting_module && e.name == *function_name)
        {
            Some(unresolved_export) => MergedExport::Unresolved(unresolved_export.clone()),
            None => MergedExport::Resolved,
        }
    }

    pub(crate) fn get_indices(
        &self,
        considering_module: &str,
    ) -> HashMap<BeforeFunctionIndex, AfterFunctionIndex> {
        let Self {
            unresolved_imports,
            resolved,
            internal,
            ..
        } = self;
        // TODO: assert that the entire range is covered
        // TODO: assert that the value is not yet inserted in the hashmap
        // TODO: assert there are no holes anymore
        let mut hashmap = HashMap::new();
        for import in unresolved_imports {
            if import.importing_module.name != considering_module {
                continue;
            }
            let (before, after) = &import.index;
            hashmap.insert(before.index.into(), after.index.into());
        }
        for resolved in resolved {
            for import in &resolved.resolved_imports {
                if import.importing_module.name != considering_module {
                    continue;
                }
                let (before, after) = &import.index;
                hashmap.insert(before.index.into(), after.index.into());
            }
        }
        for internal in internal {
            let (before, after) = &internal.index;
            hashmap.insert(before.index.into(), after.index.into());
        }
        hashmap
    }
}

impl ResolutionSchema<BeforeFunctionIndex> {
    /// Takes out the unresolved imports that are imported by `module`, sorts
    /// them based on the module-local index.
    fn remove_sorted_unresolved_imports(
        &mut self,
        module: &ModuleName,
    ) -> Vec<FunctionImportSpecification<BeforeFunctionIndex>> {
        let unresolved_imports = core::mem::take(&mut self.unresolved_imports);
        let (mut unresolved_imports_from_module, unresolved_imports): (Vec<_>, Vec<_>) =
            unresolved_imports
                .into_iter()
                .partition(|i| i.importing_module == *module);
        unresolved_imports_from_module.sort_by(|a, b| a.index.cmp(&b.index));
        self.unresolved_imports = unresolved_imports.into_iter().collect();
        unresolved_imports_from_module
    }

    fn remove_sorted_resolved(
        &mut self,
        module: &ModuleName,
    ) -> Vec<Resolved<BeforeFunctionIndex>> {
        let resolveds = core::mem::take(&mut self.resolved);
        let (mut resolveds_from_module, resolveds): (Vec<_>, Vec<_>) = resolveds
            .into_iter()
            .partition(|i| i.export_specification.module == *module);
        resolveds_from_module.sort_by(|a, b| {
            a.export_specification
                .index
                .cmp(&b.export_specification.index)
        });
        self.resolved = resolveds.into_iter().collect();
        resolveds_from_module
    }

    fn remove_internal(
        &mut self,
        module: &ModuleName,
    ) -> Vec<FunctionSpecification<BeforeFunctionIndex>> {
        let internal_function_specifications =
            core::mem::take(&mut self.internal_function_specifications);
        let (mut internal_function_specifications_from_module, internal_function_specifications): (
            Vec<_>,
            Vec<_>,
        ) = internal_function_specifications
            .into_iter()
            .partition(|i| i.defining_module == *module);
        internal_function_specifications_from_module.sort_by(|a, b| a.index.cmp(&b.index));
        self.internal_function_specifications =
            internal_function_specifications.into_iter().collect();
        internal_function_specifications_from_module
    }

    fn remove_sorted_unresolved_exports(
        &mut self,
        module: &ModuleName,
    ) -> Vec<FunctionExportSpecification<BeforeFunctionIndex>> {
        let unresolved_exports = core::mem::take(&mut self.unresolved_exports);
        let (mut unresolved_exports_from_module, unresolved_exports): (Vec<_>, Vec<_>) =
            unresolved_exports
                .into_iter()
                .partition(|i| i.module == *module);
        unresolved_exports_from_module.sort_by(|a, b| a.index.cmp(&b.index));
        self.unresolved_exports = unresolved_exports.into_iter().collect();
        unresolved_exports_from_module
    }

    pub(crate) fn assign_identities(
        mut self,
        mut modules: &[ModuleName],
    ) -> OrderedResolutionSchema {
        let mut yielder: FunctionIndexYielder<AfterFunctionIndex> =
            FunctionIndexYielder::<AfterFunctionIndex>::new();
        let mut unresolved_imports = vec![];
        let mut resolved = vec![];
        let mut internal = vec![];
        let mut unresolved_exports = vec![];
        let mut old_to_new_function_resolver = HashMap::new();

        while let Some((module, rest_modules)) = modules.split_first() {
            for module_unresolved_import in self.remove_sorted_unresolved_imports(module) {
                unresolved_imports.push(module_unresolved_import);
            }
            for module_resolved in self.remove_sorted_resolved(module) {
                resolved.push(module_resolved)
            }
            for module_internal in self.remove_internal(module) {
                internal.push(module_internal)
            }
            for module_resolved in self.remove_sorted_unresolved_exports(module) {
                unresolved_exports.push(module_resolved)
            }
            modules = rest_modules;
        }

        let unresolved_imports = unresolved_imports
            .into_iter()
            .map(|i| i.for_merged(&mut yielder, &mut old_to_new_function_resolver))
            .collect();

        let resolved: Vec<Resolved<(BeforeFunctionIndex, Option<AfterFunctionIndex>)>> = resolved
            .into_iter()
            .map(|r| r.first_pass(&mut yielder, &mut old_to_new_function_resolver))
            .collect();

        let resolved = resolved
            .into_iter()
            .map(|r| r.second_pass(&old_to_new_function_resolver))
            .collect();

        let internal = internal
            .into_iter()
            .map(|i| i.for_merged(&mut yielder, &mut old_to_new_function_resolver))
            .collect();

        let unresolved_exports = unresolved_exports
            .into_iter()
            .map(|e| e.for_merged(&old_to_new_function_resolver))
            .collect();

        OrderedResolutionSchema {
            unresolved_imports,
            resolved,
            internal,
            unresolved_exports,
            old_to_new_function_resolver,
        }
    }
}

impl FunctionImportSpecification<BeforeFunctionIndex> {
    fn for_merged(
        self,
        yielder: &mut FunctionIndexYielder<AfterFunctionIndex>,
        old_to_new_function_resolver: &mut HashMap<
            (ModuleName, BeforeFunctionIndex),
            AfterFunctionIndex,
        >,
    ) -> FunctionImportSpecification<ResolvedIndexMap> {
        let FunctionImportSpecification {
            importing_module,
            exporting_module,
            name,
            ty,
            index,
        } = self;
        let new_index = yielder.give();
        assert!(
            old_to_new_function_resolver
                .insert((importing_module.clone(), index.clone()), new_index.clone())
                .is_none()
        );
        FunctionImportSpecification {
            importing_module,
            exporting_module,
            name,
            ty,
            index: (index, new_index),
        }
    }
}

impl Resolved<BeforeFunctionIndex> {
    pub(crate) fn first_pass(
        self,
        yielder: &mut FunctionIndexYielder<AfterFunctionIndex>,
        old_to_new_function_resolver: &mut HashMap<
            (ModuleName, BeforeFunctionIndex),
            AfterFunctionIndex,
        >,
    ) -> Resolved<(BeforeFunctionIndex, Option<AfterFunctionIndex>)> {
        let _ = yielder;
        let _ = old_to_new_function_resolver;
        let Resolved {
            export_specification,
            resolved_imports,
        } = self;
        let _ = export_specification;
        let _ = resolved_imports;
        // => These imports / exports are not present in the new binary.
        // => The exports will contain the new?
        // Resolved {
        //     export_specification: todo!(),
        //     resolved_imports: todo!(),
        // }
        todo!()
    }
}

impl Resolved<(BeforeFunctionIndex, Option<AfterFunctionIndex>)> {
    pub(crate) fn second_pass(
        self,
        old_to_new_function_resolver: &HashMap<
            (ModuleName, BeforeFunctionIndex),
            AfterFunctionIndex,
        >,
    ) -> Resolved<(BeforeFunctionIndex, AfterFunctionIndex)> {
        let _ = old_to_new_function_resolver;
        todo!()
    }
}

impl FunctionSpecification<BeforeFunctionIndex> {
    fn for_merged(
        self,
        yielder: &mut FunctionIndexYielder<AfterFunctionIndex>,
        old_to_new_function_resolver: &mut HashMap<
            (ModuleName, BeforeFunctionIndex),
            AfterFunctionIndex,
        >,
    ) -> FunctionSpecification<ResolvedIndexMap> {
        let FunctionSpecification {
            defining_module,
            ty,
            index: old_index,
        } = self;
        let new_index = yielder.give();
        assert!(
            old_to_new_function_resolver
                .insert(
                    (defining_module.clone(), old_index.clone()),
                    new_index.clone()
                )
                .is_none()
        );
        FunctionSpecification {
            defining_module,
            ty,
            index: (old_index, new_index),
        }
    }
}

impl FunctionExportSpecification<BeforeFunctionIndex> {
    fn for_merged(
        self,
        old_to_new_function_resolver: &HashMap<
            (ModuleName, BeforeFunctionIndex),
            AfterFunctionIndex,
        >,
    ) -> FunctionExportSpecification<ResolvedIndexMap> {
        let FunctionExportSpecification {
            module,
            name,
            ty,
            index,
        } = self;
        let reference = (module, index);
        let new_index = old_to_new_function_resolver.get(&reference).unwrap();
        let (module, index) = reference;
        FunctionExportSpecification {
            module,
            name,
            ty,
            index: (index, new_index.clone()),
        }
    }
}
