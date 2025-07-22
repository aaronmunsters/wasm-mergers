use crate::merge_options::MergeOptions;
use crate::named_module::NamedBufferModule;
use crate::named_module::NamedModule;
use crate::named_module::NamedParsedModule;

/// The configuration of modules that will be merged
///
/// The order of the modules dictactes the multi-memory
/// order of the merged modules.
#[derive(Debug)]
pub struct MergeConfiguration<'a, Module> {
    // Public
    //
    /// The modules that will be included in the output merged module.
    /// The order is relevant.
    pub modules: &'a [&'a NamedModule<'a, Module>],
    pub options: MergeOptions,
}

impl<'a> MergeConfiguration<'a, &'a [u8]> {
    #[must_use]
    pub(crate) fn new_empty_builder(
        modules: &'a [&'a NamedBufferModule<'a>],
        options: MergeOptions,
    ) -> Self {
        Self { modules, options }
    }

    #[must_use = "Parsing can become expensive, this result must be used"]
    pub(crate) fn try_parse(&self) -> anyhow::Result<Vec<NamedParsedModule<'a>>> {
        self.modules
            .iter()
            .copied()
            .map(TryInto::try_into)
            .collect()
    }
}
