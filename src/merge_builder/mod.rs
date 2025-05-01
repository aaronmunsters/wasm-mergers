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
            imports,
            tables: _, // TODO:
            types,
            funcs,
            globals: _,
            locals: _, // TODO:
            exports,
            memories: _,  // TODO:
            data: _,      // TODO:
            elements: _,  // TODO:
            start: _,     // TODO:
            producers: _, // TODO:
            customs: _,   // TODO:
            debug: _,     // TODO:
            name: _,      // TODO:
            ..
        } = module;

        let mut covered_function_imports = HashSet::new();

        for import in imports.iter() {
            match &import.kind {
                walrus::ImportKind::Function(id) => {
                    let function_import_specification = FunctionImportSpecification {
                        importing_module: (*considering_module).into(),
                        exporting_module: (*import.module).into(),
                        name: (*import.name).into(),
                        ty: FuncType::from_types(funcs.get(*id).ty(), types),
                        index: import.id().index().into(),
                    };
                    self.resolver
                        .add_import(function_import_specification)
                        .map_err(Error::Resolve)?;
                    covered_function_imports.insert((id, import.id()));
                }
                walrus::ImportKind::Table(_id) => todo!(),
                walrus::ImportKind::Memory(_id) => todo!(),
                walrus::ImportKind::Global(_id) => todo!(),
            }
        }

        // Consider functions
        for function in funcs.iter() {
            match &function.kind {
                walrus::FunctionKind::Import(i) => {
                    debug_assert!(covered_function_imports.contains(&(&function.id(), i.import)))
                }
                walrus::FunctionKind::Local(local_function) => {
                    let local = FunctionSpecification {
                        defining_module: (*considering_module).into(),
                        ty: FuncType::from_types(local_function.ty(), types),
                        index: function.id().index().into(),
                    };
                    self.resolver.add_function(local);
                }
                walrus::FunctionKind::Uninitialized(_) => {
                    return Err(Error::ComponentModelUnsupported(
                        considering_module.to_string(),
                    ));
                }
            }
        }

        for export in exports.iter() {
            match export.item {
                walrus::ExportItem::Function(id) => {
                    let export = FunctionExportSpecification {
                        module: (*considering_module).into(),
                        name: export.name.as_str().into(),
                        ty: FuncType::from_types(funcs.get(id).ty(), types),
                        index: export.id().index().into(),
                    };
                    self.resolver.add_export(export).map_err(Error::Resolve)?;
                }
                walrus::ExportItem::Table(_id) => todo!(),
                walrus::ExportItem::Memory(_id) => todo!(),
                walrus::ExportItem::Global(_id) => todo!(),
            }
        }

        Ok(())
    }

    pub(crate) fn resolve(self, modules: &[ModuleName]) -> Result<OrderedResolutionSchema, Error> {
        let resolved = self.resolver.validate().map_err(|_| Error::Validation)?; // TODO: this could be more informative
        Ok(resolved.assign_identities(modules))
    }
}
