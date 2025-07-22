use derive_more::{Display, From, Into};
use std::hash::Hash;
use walrus::{Module, TypeId, ValType};

pub(crate) mod graph_resolution;

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

    pub(crate) fn add_to_module(&self, module: &mut Module) -> TypeId {
        module.types.add(&self.params, &self.results)
    }
}

// TODO: Check if the `Display` can be covered as Deref + DerefMut

#[derive(Debug, Clone, Hash, PartialEq, Eq, From, Into, Display)]
#[from(&str, String)]
pub struct TableName(pub(crate) String);

#[derive(Debug, Clone, Hash, PartialEq, Eq, From, Into, Display)]
#[from(&str, String)]
pub struct MemoryName(pub(crate) String);

#[derive(Debug, Clone, Hash, PartialEq, Eq, From, Into, Display)]
#[from(&str, String)]
pub struct GlobalName(pub(crate) String);
