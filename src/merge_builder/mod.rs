use std::collections::HashSet;

use walrus::Module;

use crate::MergeOptions;
use crate::error::Error;
use crate::kinds::FuncType;
use crate::kinds::Locals;
use crate::merge_options::ClashingExports;
use crate::merge_options::KeepExports;
use crate::merge_options::LinkTypeMismatch;
use crate::merger::old_to_new_mapping::OldIdFunction;
use crate::merger::provenance_identifier::Identifier;
use crate::merger::provenance_identifier::Old;
use crate::named_module::NamedParsedModule;
use crate::resolver::dependency_reduction::ReducedDependencies;
use crate::resolver::{Export, Function, Import, Local, Resolver as GraphResolver};

#[derive(Debug)]
pub(crate) struct Resolver {
    graph: GraphResolver<Function, FuncType, OldIdFunction, Locals>,
}

impl Resolver {
    pub(crate) fn new() -> Self {
        Self {
            graph: GraphResolver::new(),
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
            exports: considering_exports,
            locals: considering_locals,
            // FIXME: `tables`, `globals`, `memories` could be resolved too.
            // Currently no support for.
            ..
        } = module;

        let mut covered_function_imports = HashSet::new();

        for import in considering_imports.iter() {
            if let walrus::ImportKind::Function(old_function_id) = &import.kind {
                let ty = FuncType::from_types(
                    considering_funcs.get(*old_function_id).ty(),
                    considering_types,
                );
                {
                    let old_function_id: OldIdFunction = (*old_function_id).into();
                    let import = Import {
                        exporting_module: (*import.module).to_string().into(),
                        importing_module: (*considering_module).to_string().into(),
                        exporting_identifier: (*import.name).to_string().into(),
                        imported_index: old_function_id,
                        kind: Function,
                        ty,
                    };
                    self.graph.add_import(import);
                }
                covered_function_imports.insert((old_function_id, import.id()));
            } else {
                // FIXME: Skipping resolving `tables`, `globals` & `memories`.
                println!("Skipping `tables`, `globals`, `memories`");
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

                    let local = Local {
                        module: (*considering_module).to_string().into(),
                        index: function.id().into(),
                        kind: Function,
                        ty: FuncType::from_types(local_function.ty(), considering_types),
                        data: locals.clone(),
                    };
                    self.graph.add_local(local);
                }
                walrus::FunctionKind::Import(i) => {
                    debug_assert!(covered_function_imports.contains(&(&function.id(), i.import)));
                }
                walrus::FunctionKind::Uninitialized(_) => {
                    return Err(Error::ComponentModelUnsupported(
                        (*considering_module).to_string(),
                    ));
                }
            }
        }

        for export in considering_exports.iter() {
            if let walrus::ExportItem::Function(id) = export.item {
                let old_id: Identifier<Old, _> = id.into();
                let ty = FuncType::from_types(considering_funcs.get(id).ty(), considering_types);
                let export = Export {
                    module: considering_module.to_string().into(),
                    identifier: export.name.to_string().into(),
                    index: old_id,
                    kind: Function,
                    ty,
                };
                self.graph.add_export(export);
            } else {
                // FIXME: Skipping resolving `tables`, `globals` & `memories`.
                println!("Skipping merging for `tables`, `globals`, `memories`");
            }
        }

        Ok(())
    }

    pub(crate) fn resolve(
        self,
        merge_options: &MergeOptions,
    ) -> Result<ReducedDependencies<Function, FuncType, OldIdFunction, Locals>, Error> {
        // Link all up
        let mut linked = self.graph.link_nodes().map_err(|_| Error::ImportCycle)?;

        // Assert exports are not clashing
        match &merge_options.clashing_exports {
            ClashingExports::Rename(rename_strategy) => {
                linked.clashing_rename(&rename_strategy.functions)
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

        Ok(reduced_dependencies)
    }
}
