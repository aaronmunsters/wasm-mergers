use std::collections::HashMap;

use walrus::{DataId, ElementId, FunctionId, GlobalId, LocalId, MemoryId, TableId};

use crate::resolver::{ModuleName, resolution_schema::BeforeFunctionIndex};

#[derive(Default)]
pub struct Mapping {
    // pub functions: HashMap<(String, FunctionId), FunctionId>,
    pub tables: HashMap<(String, TableId), TableId>,
    pub globals: HashMap<(String, GlobalId), GlobalId>,
    pub memories: HashMap<(String, MemoryId), MemoryId>,
    pub datas: HashMap<(String, DataId), DataId>,
    pub elements: HashMap<(String, ElementId), ElementId>,

    pub function_mapping: HashMap<(ModuleName, BeforeFunctionIndex), FunctionId>,
    pub locals_mapping: HashMap<(ModuleName, BeforeFunctionIndex, LocalId), LocalId>,
}
