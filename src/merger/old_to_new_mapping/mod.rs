use std::collections::HashMap;

use walrus::{DataId, ElementId, FunctionId, GlobalId, LocalId, MemoryId, TableId};

use crate::resolver::{ModuleName, resolution_schema::Before};

#[derive(Default)]
pub struct Mapping {
    pub tables: HashMap<(ModuleName, Before<TableId>), TableId>,
    pub globals: HashMap<(ModuleName, Before<GlobalId>), GlobalId>,
    pub memories: HashMap<(ModuleName, Before<MemoryId>), MemoryId>,
    pub datas: HashMap<(ModuleName, Before<DataId>), DataId>,
    pub elements: HashMap<(ModuleName, Before<ElementId>), ElementId>,
    pub funcs: HashMap<(ModuleName, Before<FunctionId>), FunctionId>,
    pub locals: HashMap<(ModuleName, Before<LocalId>), LocalId>,
}
