use crate::named_module::NamedBufferModule;
use crate::named_module::NamedModule;
use crate::named_module::NamedParsedModule;
use crate::resolver::ModuleName;

/// The configuration of modules that will be merged
///
/// The order of the modules dictactes the multi-memory
/// order of the merged modules.
#[derive(Debug, Default)]
pub struct MergeConfiguration<'a, Module> {
    // Public
    //
    /// The modules that will be included in the output merged module.
    /// The order is relevant.
    pub modules: &'a [NamedModule<'a, Module>],
}

impl<'a, T> MergeConfiguration<'a, T> {
    #[must_use]
    pub(crate) fn owned_names(&self) -> Vec<ModuleName> {
        self.modules.iter().map(|m| m.name.into()).collect()
    }
}

impl<'a> MergeConfiguration<'a, &'a [u8]> {
    #[must_use]
    pub(crate) fn new_empty_builder(modules: &'a [NamedBufferModule<'a>]) -> Self {
        Self { modules }
    }

    #[must_use = "Parsing can become expensive, this result must be used"]
    pub(crate) fn try_parse(&self) -> anyhow::Result<Vec<NamedParsedModule<'a>>> {
        self.modules.iter().map(TryInto::try_into).collect()
    }
}
