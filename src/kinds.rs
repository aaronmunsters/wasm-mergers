use std::{hash::Hash, marker::PhantomData};

use derive_more::{Display, From, Into};
use walrus::{LocalId, Module, TypeId, ValType};

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub(crate) struct FuncType {
    params: Box<[ValType]>,
    results: Box<[ValType]>,
}

pub(crate) type Locals = Box<[(LocalId, ValType)]>;

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

// Supported kinds
#[derive(Debug, Clone, Hash, PartialEq, Eq, Default)]
pub struct Function;
#[derive(Debug, Clone, Hash, PartialEq, Eq, Default)]
pub struct Table;
#[derive(Debug, Clone, Hash, PartialEq, Eq, Default)]
pub struct Memory;
#[derive(Debug, Clone, Hash, PartialEq, Eq, Default)]
pub struct Global;

// Identifiers
#[derive(Debug, Clone, Hash, PartialEq, Eq, From, Into)]
pub struct IdentifierItem<Kind> {
    identifier: String,
    kind: PhantomData<Kind>,
}

impl<Kind> From<String> for IdentifierItem<Kind> {
    fn from(value: String) -> Self {
        Self {
            identifier: value,
            kind: PhantomData,
        }
    }
}

impl<Kind> From<IdentifierItem<Kind>> for String {
    fn from(val: IdentifierItem<Kind>) -> Self {
        val.identifier
    }
}

impl<Kind> IdentifierItem<Kind> {
    pub(crate) fn identifier(&self) -> &str {
        &self.identifier
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, From, Into, Display)]
#[from(String, &str)]
pub struct IdentifierModule(String);

impl IdentifierModule {
    pub(crate) fn identifier(&self) -> &str {
        let Self(identifier) = self;
        identifier
    }
}
