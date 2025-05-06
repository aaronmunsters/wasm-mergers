mod error;
mod merge_builder;
mod merge_configuration;
mod merger;
mod named_module;
mod resolver;

use merge_builder::Resolver;
pub use merge_configuration::MergeConfiguration;
use merger::Merger;
pub use named_module::NamedBufferModule;
pub use named_module::NamedModule;

/// The methods that can be called from the public API
impl<'a> MergeConfiguration<'a, &'a [u8]> {
    pub fn new(modules: &'a [NamedBufferModule<'a>]) -> Self {
        Self::new_empty_builder(modules)
    }

    pub fn merge(&mut self) -> anyhow::Result<Vec<u8>> {
        let parsed_modules = self.try_parse()?;

        // First pass: consider each parsed module
        let mut resolver: Resolver = Default::default();
        for parsed_module in parsed_modules.iter() {
            resolver.consider(parsed_module)?;
        }

        // Next, with the given modules, resolve imports & exports
        let resolution_schema = resolver.resolve(&self.owned_names())?;
        let mut merged_builder = Merger::new(resolution_schema);

        // Next follows the second pass in which content is copied over
        for parsed_module in parsed_modules {
            merged_builder.include(parsed_module)?;
        }

        // Build merged module
        Ok(merged_builder.build().emit_wasm())
    }
}
