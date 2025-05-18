pub mod error;
mod merge_builder;
mod merge_configuration;
mod merge_options;
mod merger;
mod named_module;
mod resolver;

use error::Error;
use merge_builder::Resolver;
use merger::Merger;

pub use merge_configuration::MergeConfiguration;
pub use merge_options::MergeOptions;
pub use named_module::NamedBufferModule;
pub use named_module::NamedModule;

/// The methods that can be called from the public API
impl<'a> MergeConfiguration<'a, &'a [u8]> {
    pub fn new(modules: &'a [&NamedBufferModule<'a>], options: MergeOptions) -> Self {
        Self::new_empty_builder(modules, options)
    }

    pub fn merge(&mut self) -> Result<Vec<u8>, Error> {
        let parsed_modules: Vec<NamedModule<'a, walrus::Module>> =
            self.try_parse().map_err(Error::Parse)?;

        // First pass: consider each parsed module
        let mut resolver: Resolver = Default::default();
        for parsed_module in parsed_modules.iter() {
            resolver.consider(parsed_module)?;
        }

        // Next, with the given modules, resolve imports & exports
        let resolution_schema = resolver.resolve(&self.owned_names(), self.options.clone())?;
        let mut merged_builder = Merger::new(resolution_schema, self.options.clone());

        // Next follows the second pass in which content is copied over
        for parsed_module in parsed_modules {
            merged_builder.include(parsed_module)?;
        }

        // Build merged module
        Ok(merged_builder.build().emit_wasm())
    }
}
