use walrus::Module;

/// A named WebAssembly module.
/// The name will be used to resolve function name lookup.
#[derive(Debug, Clone, Default, Hash, PartialEq, Eq)]
pub struct NamedModule<'a, M> {
    pub name: &'a str,
    pub module: M,
}

impl<'a, T> NamedModule<'a, T> {
    pub fn new(name: &'a str, module: T) -> Self {
        Self { name, module }
    }
}

/// A named module that points to a byte-buffer
pub type NamedBufferModule<'a> = NamedModule<'a, &'a [u8]>;

/// A named module that points to the internal parsed module representation
pub(crate) type NamedParsedModule<'a> = NamedModule<'a, Module>;

/// Attempt to convert from buffer to internal parsed module representation
impl<'a> TryFrom<&NamedBufferModule<'a>> for NamedParsedModule<'a> {
    type Error = anyhow::Error;

    fn try_from(module: &NamedBufferModule<'a>) -> Result<Self, Self::Error> {
        let NamedModule { name, module } = module;
        let module = Module::from_buffer(module)?;
        Result::Ok(NamedModule { name, module })
    }
}
