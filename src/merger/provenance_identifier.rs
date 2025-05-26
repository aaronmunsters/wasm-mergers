use std::hash::Hash;

#[derive(Hash, PartialEq, Eq, Debug, Default, Clone, Copy)]
pub struct Old;

#[derive(Hash, PartialEq, Eq, Debug, Default, Clone, Copy)]
pub struct New;

pub(crate) trait ModuleOrigin {}

impl ModuleOrigin for Old {}

impl ModuleOrigin for New {}

/// An identifier struct tracing which WebAssembly module is the origin
/// of the particular identifier.
#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub(crate) struct Identifier<Origin: ModuleOrigin, Id> {
    id: Id,
    origin: std::marker::PhantomData<Origin>,
}

impl<Id, Origin: ModuleOrigin> std::ops::Deref for Identifier<Origin, Id> {
    type Target = Id;

    fn deref(&self) -> &Self::Target {
        &self.id
    }
}

impl<Id, Origin: ModuleOrigin> std::ops::DerefMut for Identifier<Origin, Id> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.id
    }
}

impl<Id, Origin: ModuleOrigin> From<Id> for Identifier<Origin, Id> {
    fn from(id: Id) -> Self {
        Self {
            id,
            origin: std::marker::PhantomData,
        }
    }
}
