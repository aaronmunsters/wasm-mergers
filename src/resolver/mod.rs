use derive_more::{Display, From, Into};
use std::{collections::HashSet, hash::Hash};
use walrus::{LocalId, TypeId, ValType};

pub(crate) mod identified_resolution_schema;
pub(crate) mod resolution_schema;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct FuncType {
    params: Box<[ValType]>,
    results: Box<[ValType]>,
}

impl FuncType {
    /// Given an arena where the types belong;
    /// return an owned copy of the types.
    pub(crate) fn from_types(id: TypeId, types: &walrus::ModuleTypes) -> Self {
        let ty = types.get(id);

        let params = ty.params().iter().copied().collect::<Box<[_]>>();
        let results = ty.results().iter().copied().collect::<Box<[_]>>();

        Self { params, results }
    }

    #[must_use]
    pub(crate) fn params(&self) -> &[ValType] {
        &self.params
    }

    #[must_use]
    pub(crate) fn results(&self) -> &[ValType] {
        &self.results
    }
}

// TODO: Check if the `Display` can be covered as Deref + DerefMut

#[derive(Debug, Clone, Hash, PartialEq, Eq, From, Into, Display)]
#[from(&str, String)]
pub struct FunctionName(pub(crate) String);

#[derive(Debug, Clone, Hash, PartialEq, Eq, From, Into, Display)]
#[from(&str, String)]
pub struct TableName(pub(crate) String);

#[derive(Debug, Clone, Hash, PartialEq, Eq, From, Into, Display)]
#[from(&str, String)]
pub struct MemoryName(pub(crate) String);

#[derive(Debug, Clone, Hash, PartialEq, Eq, From, Into, Display)]
#[from(&str, String)]
pub struct GlobalName(pub(crate) String);

#[derive(Debug, Clone, Hash, PartialEq, Eq, From, Into, Display)]
#[from(&str, String)]
pub struct ModuleName(pub(crate) String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolutionSchema<Identifier>
where
    Identifier: Hash + Eq, // The kind identification
{
    /// An imported function that could not be matched with an exported function
    unresolved_imports: HashSet<FunctionImportSpecification<Identifier>>,
    /// The resolved functions, where a single export is linked to the corresponding imports
    resolved: HashSet<Resolved<Identifier>>,
    /// The functions defined internally not exported nor imported
    local_function_specifications: HashSet<FunctionSpecification<Identifier>>,
    /// An exported function that could not be matched with an imported function
    unresolved_exports: HashSet<FunctionExportSpecification<Identifier>>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) struct FunctionExportSpecification<Identifier> {
    pub(crate) module: ModuleName,
    pub(crate) name: FunctionName,
    pub(crate) ty: FuncType,
    pub(crate) function_index: Identifier,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) struct Resolved<Identifier> {
    /// The exported function
    pub(crate) export_specification: FunctionExportSpecification<Identifier>,
    /// The imported functions with which the exported is resolved by namespace & type
    pub(crate) resolved_imports: Vec<FunctionImportSpecification<Identifier>>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) struct FunctionImportSpecification<Index> {
    pub(crate) importing_module: ModuleName,
    pub(crate) exporting_module: ModuleName,
    pub(crate) name: FunctionName,
    pub(crate) ty: FuncType,
    pub(crate) index: Index,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) struct FunctionSpecification<Index> {
    pub(crate) defining_module: ModuleName,
    pub(crate) locals: Box<[(LocalId, ValType)]>,
    pub(crate) ty: FuncType,
    pub(crate) index: Index,
}
