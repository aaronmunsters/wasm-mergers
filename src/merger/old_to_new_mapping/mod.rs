use std::collections::HashMap;

use walrus::{DataId, ElementId, FunctionId, GlobalId, MemoryId, TableId};

use crate::resolver::identified_resolution_schema::OrderedResolutionSchema;

#[derive(Default)]
pub struct Mapping {
    pub functions: HashMap<(String, FunctionId), FunctionId>,
    pub tables: HashMap<(String, TableId), TableId>,
    pub globals: HashMap<(String, GlobalId), GlobalId>,
    pub memories: HashMap<(String, MemoryId), MemoryId>,
    pub datas: HashMap<(String, DataId), DataId>,
    pub elements: HashMap<(String, ElementId), ElementId>,
}

impl Mapping {
    pub fn populate_with_resolution_schema(
        &mut self,
        considering_module: &str,
        resolution_schema: &OrderedResolutionSchema,
    ) {
        let indices = resolution_schema.get_indices(considering_module);
        for (before_function_index, after_function_index) in indices.iter() {
            let _ = before_function_index;
            let _ = after_function_index;
            todo!()
            // FIXME: cannot insert FunctionId here as they are
            // arena-allocated

            // self.functions.insert(
            //     (
            //         considering_module.to_string(),
            //         before_function_index.index.into(),
            //     ),
            //     after_function_index.index.into(),
            // );
        }
    }
}
