use std::collections::{HashMap, HashSet};

use crate::merge_options::ClashingExports;
use crate::merger::old_to_new_mapping::OldIdFunction;

use crate::{MergeOptions, resolver::ModuleName};

use super::{
    FunctionExportSpecification, FunctionImportSpecification, FunctionName, FunctionSpecification,
    ResolutionSchema, Resolved,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct NameResolved {
    /// The exported function
    export_specification: FunctionExportSpecification<OldIdFunction>,
    /// The imported functions with which the exported is resolved by namespace
    resolved_imports: HashSet<FunctionImportSpecification<OldIdFunction>>,
}

enum FunctionResolveResult {
    NoImportPresent(FunctionExportSpecification<OldIdFunction>),
    AllImportsResolve(Resolved<OldIdFunction>),
    AllImportsMismatch(TypeMismatch),
    PartialResolvePartialMismatch {
        resolve: Resolved<OldIdFunction>,
        mismatch: TypeMismatch,
    },
}

impl NameResolved {
    /// Attempting to resolve can result in different cases
    /// None, None => No import was present
    /// Some, None => All imports resolve successfully
    /// None, Some => All imports fail to match types
    /// Some, Some => Some success resolves, some type-fail
    fn attempt_resolve(self) -> FunctionResolveResult {
        let Self {
            export_specification,
            resolved_imports,
        } = self;

        let mut imports_type_matching = vec![];
        let mut imports_type_mismatch = vec![];

        for import in resolved_imports {
            if import.ty.eq(&export_specification.ty) {
                // Types match
                imports_type_matching.push(import);
            } else {
                // Types do not match
                imports_type_mismatch.push(import);
            }
        }

        let resolved = if imports_type_matching.is_empty() {
            None
        } else {
            Some(Resolved {
                export_specification: export_specification.clone(),
                resolved_imports: imports_type_matching,
            })
        };
        let mismatch = if imports_type_mismatch.is_empty() {
            None
        } else {
            Some(TypeMismatch {
                export_specification: export_specification.clone(),
                resolved_imports: imports_type_mismatch,
            })
        };

        match (resolved, mismatch) {
            (None, None) => FunctionResolveResult::NoImportPresent(export_specification),
            (None, Some(mismatch)) => FunctionResolveResult::AllImportsMismatch(mismatch),
            (Some(resolve), None) => FunctionResolveResult::AllImportsResolve(resolve),
            (Some(resolve), Some(mismatch)) => {
                FunctionResolveResult::PartialResolvePartialMismatch { resolve, mismatch }
            }
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct TypeMismatch {
    /// The exported function
    export_specification: FunctionExportSpecification<OldIdFunction>,
    /// The imported functions with which the exported is resolved but mismatches in type
    resolved_imports: Vec<FunctionImportSpecification<OldIdFunction>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationFailure {
    /// Types Mismatch
    ///
    /// Eg.
    /// ```wat
    /// (module "A" (export "f" (result i32)))
    /// (module "B" (import "A" "f" (result i64)))
    /// (module "C" (import "A" "f" (result f64)))
    /// ```
    /// Would result in a `HashSet { A:f:i32 -> { B:f:i64, C:f:f64 } }`.
    types_mismatch: HashSet<TypeMismatch>,

    /// Name Clashes
    ///
    /// Eg.
    /// ```wat
    /// (module "A" (export "f")) ;; (a)
    /// (module "B" (export "f")) ;; (b)
    /// ;; ==>
    /// (module "M" (export "f")) ;; (a) or (b) ?
    /// ```
    ///
    /// If no other module imports "f", then M
    /// Would result in a `HashMap { "f" -> { A:f, B:f } }`.
    name_clashes: HashMap<FunctionName, HashSet<FunctionExportSpecification<OldIdFunction>>>,

    /// Part of the resolution that was a success.
    resolved_schema: ResolutionSchema<OldIdFunction>,
}

#[derive(Debug, PartialEq, Eq, Default, Clone)]
pub struct ResolutionSchemaBuilder {
    expected_imports: HashSet<FunctionImportSpecification<OldIdFunction>>,
    local_function_specifications: HashSet<FunctionSpecification<OldIdFunction>>,
    provided_exports: HashSet<FunctionExportSpecification<OldIdFunction>>,
}

impl ResolutionSchemaBuilder {
    pub(crate) fn add_import(&mut self, specification: FunctionImportSpecification<OldIdFunction>) {
        let newly_inserted = self.expected_imports.insert(specification);
        assert!(newly_inserted);
    }

    pub(crate) fn add_local_function(
        &mut self,
        specification: FunctionSpecification<OldIdFunction>,
    ) {
        let newly_inserted = self.local_function_specifications.insert(specification);
        assert!(newly_inserted);
    }

    pub(crate) fn add_export(&mut self, specification: FunctionExportSpecification<OldIdFunction>) {
        let newly_inserted = self.provided_exports.insert(specification);
        assert!(newly_inserted);
    }

    pub(crate) fn validate(
        self,
        merge_options: &MergeOptions,
    ) -> Result<ResolutionSchema<OldIdFunction>, Box<ValidationFailure>> {
        let Self {
            expected_imports,
            local_function_specifications,
            provided_exports,
        } = self;

        // Get all exported functions, 'prepare' to attempt linking this to other imports
        let mut potentially_resolved_exports: Vec<_> = provided_exports
            .into_iter()
            .map(|export_specification| NameResolved {
                export_specification,
                resolved_imports: HashSet::new(),
            })
            .collect();

        let mut unresolved_imports = HashSet::new();

        // For each imported function, attempt to resolve it with an export based on naming
        for import in expected_imports {
            if let Some(export) = potentially_resolved_exports.iter_mut().find(|export| {
                let ModuleName(export_module_name) = &export.export_specification.module;
                let ModuleName(import_module_name) = &import.exporting_module;
                let FunctionName(export_function_name) = &export.export_specification.name;
                let FunctionName(import_function_name) = &import.name;

                export_module_name == import_module_name
                    && export_function_name == import_function_name
            }) {
                assert!(export.resolved_imports.insert(import));
            } else {
                assert!(unresolved_imports.insert(import));
            }
        }

        // At this point, provided_exports and expected_imports are consumed.
        // We are left with potentially_resolved_exports and unresolved_imports

        // We will now attempt to resolve & populate the resolved / unresolved
        // datastructures

        let mut unresolved_exports = vec![];

        let mut types_mismatch = HashSet::new();
        let mut resolved = HashSet::new();

        // Iterate over
        for export in potentially_resolved_exports {
            match export.attempt_resolve() {
                FunctionResolveResult::NoImportPresent(export_specification) => {
                    unresolved_exports.push(export_specification);
                }
                FunctionResolveResult::AllImportsResolve(resolve) => {
                    assert!(resolved.insert(resolve));
                }
                FunctionResolveResult::AllImportsMismatch(mismatch) => {
                    assert!(types_mismatch.insert(mismatch));
                }
                FunctionResolveResult::PartialResolvePartialMismatch { resolve, mismatch } => {
                    assert!(resolved.insert(resolve));
                    assert!(types_mismatch.insert(mismatch));
                }
            }
        }

        // Determine name clashes among exports
        let name_clashes = unresolved_exports
            .iter()
            .cloned()
            .map(|export| (export.name.clone(), export))
            .fold(
                HashMap::<_, HashSet<_>>::new(),
                |mut acc, (export_name, export)| {
                    acc.entry(export_name)
                        .and_modify(|specifications| {
                            debug_assert!(!specifications.contains(&export));
                            specifications.insert(export.clone());
                        })
                        .or_insert_with(|| HashSet::from_iter(vec![export]));
                    acc
                },
            )
            .into_iter()
            .filter(|(_, exports)| exports.len() > 1)
            .collect::<HashMap<_, _>>();

        // FIXME: Can the `unresolved_exports` not be a hashset to begin with?
        let unresolved_exports = unresolved_exports.iter().cloned().collect();

        let resolved_schema = ResolutionSchema {
            unresolved_imports,
            resolved,
            local_function_specifications,
            unresolved_exports,
        };

        if !types_mismatch.is_empty() {
            return Err(ValidationFailure {
                types_mismatch,
                name_clashes,
                resolved_schema,
            }
            .into());
        }

        let allow_rename = matches!(merge_options.clashing_exports, ClashingExports::Rename(_));
        if name_clashes.is_empty() || allow_rename {
            Ok(resolved_schema)
        } else {
            Err(ValidationFailure {
                types_mismatch,
                name_clashes,
                resolved_schema,
            }
            .into())
        }
    }
}
