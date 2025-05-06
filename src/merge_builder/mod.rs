use std::collections::HashSet;

use walrus::Module;

use crate::error::Error;
use crate::named_module::NamedParsedModule;
use crate::resolver::FuncType;
use crate::resolver::FunctionExportSpecification;
use crate::resolver::FunctionImportSpecification;
use crate::resolver::FunctionSpecification;
use crate::resolver::ModuleName;
use crate::resolver::identified_resolution_schema::OrderedResolutionSchema;
use crate::resolver::resolution_schema::Before;
use crate::resolver::resolution_schema::ResolutionSchemaBuilder;

#[derive(Debug, Default)]
pub(crate) struct Resolver {
    resolver: ResolutionSchemaBuilder,
}

impl Resolver {
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
                let function_import_specification = FunctionImportSpecification {
                    importing_module: (*considering_module).into(),
                    exporting_module: (*import.module).into(),
                    name: (*import.name).into(),
                    ty,
                    index: Before::from(*old_function_id),
                };
                self.resolver.add_import(function_import_specification);
                covered_function_imports.insert((old_function_id, import.id()));
            } else {
                // FIXME: Skipping resolving `tables`, `globals` & `memories`.
                println!("Skipping `tables`, `globals`, `memories`")
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
                    self.resolver.add_local_function(FunctionSpecification {
                        locals,
                        defining_module: (*considering_module).into(),
                        ty: FuncType::from_types(local_function.ty(), considering_types),
                        index: function.id().into(),
                    });
                }
                walrus::FunctionKind::Import(i) => {
                    debug_assert!(covered_function_imports.contains(&(&function.id(), i.import)))
                }
                walrus::FunctionKind::Uninitialized(_) => {
                    return Err(Error::ComponentModelUnsupported(
                        considering_module.to_string(),
                    ));
                }
            }
        }

        for export in considering_exports.iter() {
            if let walrus::ExportItem::Function(id) = export.item {
                let export = FunctionExportSpecification {
                    module: (*considering_module).into(),
                    name: export.name.as_str().into(),
                    ty: FuncType::from_types(considering_funcs.get(id).ty(), considering_types),
                    index: Before::from(id),
                };
                self.resolver.add_export(export);
            } else {
                // FIXME: Skipping resolving `tables`, `globals` & `memories`.
                println!("Skipping merging for `tables`, `globals`, `memories`")
            }
        }

        Ok(())
    }

    pub(crate) fn resolve(self, modules: &[ModuleName]) -> Result<OrderedResolutionSchema, Error> {
        let resolved = self.resolver.validate().map_err(Error::Validation)?;
        Ok(resolved.assign_identities(modules))
    }
}
