use derive_more::From;
use std::collections::{HashMap, HashSet};
use walrus::FunctionId;

use super::{
    FunctionExportSpecification, FunctionImportSpecification, FunctionName, FunctionSpecification,
    ResolutionSchema, Resolved,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, From)]
pub(crate) struct Before<T>(pub(crate) T);

#[derive(Debug, Clone, PartialEq, Eq)]
struct NameResolved {
    /// The exported function
    export_specification: FunctionExportSpecification<Before<FunctionId>>,
    /// The imported functions with which the exported is resolved by namespace
    resolved_imports: HashSet<FunctionImportSpecification<Before<FunctionId>>>,
}

enum FunctionResolveResult {
    NoImportPresent(FunctionExportSpecification<Before<FunctionId>>),
    AllImportsResolve(Resolved<Before<FunctionId>>),
    AllImportsMismatch(TypeMismatch),
    PartialResolvePartialMismatch {
        resolve: Resolved<Before<FunctionId>>,
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

        for import in resolved_imports.into_iter() {
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
    export_specification: FunctionExportSpecification<Before<FunctionId>>,
    /// The imported functions with which the exported is resolved but mismatches in type
    resolved_imports: Vec<FunctionImportSpecification<Before<FunctionId>>>,
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
    name_clashes: HashMap<FunctionName, HashSet<FunctionExportSpecification<Before<FunctionId>>>>,

    /// Part of the resolution that was a success.
    resolved_schema: ResolutionSchema<Before<FunctionId>>,
}

#[derive(Debug, PartialEq, Eq, Default, Clone)]
pub struct ResolutionSchemaBuilder {
    expected_imports: HashSet<FunctionImportSpecification<Before<FunctionId>>>,
    local_function_specifications: HashSet<FunctionSpecification<Before<FunctionId>>>,
    provided_exports: HashSet<FunctionExportSpecification<Before<FunctionId>>>,
}

impl ResolutionSchemaBuilder {
    pub(crate) fn add_import(
        &mut self,
        specification: FunctionImportSpecification<Before<FunctionId>>,
    ) {
        let newly_inserted = self.expected_imports.insert(specification);
        assert!(newly_inserted);
    }

    pub(crate) fn add_local_function(
        &mut self,
        specification: FunctionSpecification<Before<FunctionId>>,
    ) {
        let newly_inserted = self.local_function_specifications.insert(specification);
        assert!(newly_inserted);
    }

    pub(crate) fn add_export(
        &mut self,
        specification: FunctionExportSpecification<Before<FunctionId>>,
    ) {
        let newly_inserted = self.provided_exports.insert(specification);
        assert!(newly_inserted);
    }

    pub(crate) fn validate(
        self,
    ) -> Result<ResolutionSchema<Before<FunctionId>>, Box<ValidationFailure>> {
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
        for import in expected_imports.into_iter() {
            if let Some(export) = potentially_resolved_exports.iter_mut().find(|export| {
                export.export_specification.module.name == import.exporting_module.name
                    && export.export_specification.name == import.name
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
                    unresolved_exports.push(export_specification)
                }
                FunctionResolveResult::AllImportsResolve(resolve) => {
                    assert!(resolved.insert(resolve))
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

        // Split the unresolved exports into name clashes and single unresolved ones

        // The names of exported functions
        let mut export_names = HashMap::new();

        // A mapping of functions names to clashing exports
        let mut name_clashes = HashMap::new();

        enum Encountered {
            First(FunctionExportSpecification<Before<FunctionId>>),
            Before,
        }

        for export in unresolved_exports {
            let name = export.name.clone();
            match export_names.remove(&export.name) {
                Some(Encountered::First(earlier_export)) => {
                    assert!(
                        export_names
                            .insert(name.clone(), Encountered::Before)
                            .is_none()
                    );
                    assert!(
                        name_clashes
                            .insert(name, HashSet::from_iter(vec![earlier_export, export]))
                            .is_none()
                    );
                }
                Some(Encountered::Before) => {
                    assert!(
                        export_names
                            .insert(name.clone(), Encountered::Before)
                            .is_none()
                    );
                    name_clashes.entry(name).and_modify(
                        |c: &mut HashSet<FunctionExportSpecification<Before<FunctionId>>>| {
                            assert!(c.insert(export));
                        },
                    );
                }
                None => {
                    assert!(
                        export_names
                            .insert(name, Encountered::First(export))
                            .is_none()
                    );
                }
            }
        }

        let unresolved_exports: HashSet<FunctionExportSpecification<Before<FunctionId>>> =
            export_names
                .into_values()
                .filter_map(|e| match e {
                    Encountered::First(f) => Some(f),
                    Encountered::Before => None,
                })
                .collect();

        let resolved_schema = ResolutionSchema {
            local_function_specifications,
            resolved,
            unresolved_exports,
            unresolved_imports,
        };

        if types_mismatch.is_empty() && name_clashes.is_empty() {
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
