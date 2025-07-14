use std::slice::Iter;

use crate::merger::old_to_new_mapping::OldIdFunction;

use super::FunctionExportSpecification;
use super::FunctionImportSpecification;
use super::FunctionName;
use super::FunctionSpecification;
use super::ModuleName;
use super::ResolutionSchema;
use super::Resolved;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrderedResolutionSchema {
    /// An imported function that could not be matched with an exported function
    unresolved_imports: Vec<FunctionImportSpecification<OldIdFunction>>,
    /// The resolved functions, where a single export is linked to the corresponding imports
    resolved: Vec<Resolved<OldIdFunction>>,
    /// The internally defined functions
    local: Vec<FunctionSpecification<OldIdFunction>>,
    /// An exported function that could not be matched with an imported function
    unresolved_exports: Vec<FunctionExportSpecification<OldIdFunction>>,
}

pub(crate) enum MergedImport {
    Resolved,
    Unresolved(FunctionImportSpecification<OldIdFunction>),
}

#[derive(Debug)]
pub(crate) enum MergedExport {
    Resolved,
    Unresolved(FunctionExportSpecification<OldIdFunction>),
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

    pub(crate) fn unresolved_imports_iter(
        &self,
    ) -> Iter<'_, FunctionImportSpecification<OldIdFunction>> {
        self.unresolved_imports.iter()
    }

    pub(crate) fn get_local_specifications(
        &self,
    ) -> Iter<'_, FunctionSpecification<OldIdFunction>> {
        self.local.iter()
    }

    pub(crate) fn resolved_iter(&self) -> Iter<'_, Resolved<OldIdFunction>> {
        self.resolved.iter()
    }
}

impl ResolutionSchema<OldIdFunction> {
    /// Takes out the unresolved imports that are imported by `module`, sorts
    /// them based on the module-local index.
    fn remove_sorted_unresolved_imports(
        &mut self,
        module: &ModuleName,
    ) -> Vec<FunctionImportSpecification<OldIdFunction>> {
        let unresolved_imports = core::mem::take(&mut self.unresolved_imports);
        let (mut unresolved_imports_from_module, unresolved_imports): (Vec<_>, Vec<_>) =
            unresolved_imports
                .into_iter()
                .partition(|i| i.importing_module == *module);
        unresolved_imports_from_module.sort_by(|a, b| a.index.cmp(&b.index));
        self.unresolved_imports = unresolved_imports.into_iter().collect();
        unresolved_imports_from_module
    }

    /// Remove resolved functions from the given module.
    /// It is the exported functions from the given module
    ///  that are returned here.
    fn remove_sorted_resolved(&mut self, module: &ModuleName) -> Vec<Resolved<OldIdFunction>> {
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
    ) -> Vec<FunctionSpecification<OldIdFunction>> {
        let internal_function_specifications =
            core::mem::take(&mut self.local_function_specifications);
        let (mut internal_function_specifications_from_module, internal_function_specifications): (
            Vec<_>,
            Vec<_>,
        ) = internal_function_specifications
            .into_iter()
            .partition(|i| i.defining_module == *module);
        internal_function_specifications_from_module.sort_by(|a, b| a.index.cmp(&b.index));
        self.local_function_specifications = internal_function_specifications.into_iter().collect();
        internal_function_specifications_from_module
    }

    fn remove_sorted_unresolved_exports(
        &mut self,
        module: &ModuleName,
    ) -> Vec<FunctionExportSpecification<OldIdFunction>> {
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
        let mut unresolved_imports = vec![];
        let mut resolved = vec![];
        let mut internal = vec![];
        let mut unresolved_exports = vec![];

        while let Some((module, rest_modules)) = modules.split_first() {
            for module_unresolved_import in self.remove_sorted_unresolved_imports(module) {
                unresolved_imports.push(module_unresolved_import);
            }

            for module_internal in self.remove_internal(module) {
                internal.push(module_internal);
            }

            for module_resolved in self.remove_sorted_resolved(module) {
                resolved.push(module_resolved);
            }

            for module_resolved in self.remove_sorted_unresolved_exports(module) {
                unresolved_exports.push(module_resolved);
            }
            modules = rest_modules;
        }

        OrderedResolutionSchema {
            unresolved_imports,
            resolved,
            local: internal,
            unresolved_exports,
        }
    }
}
