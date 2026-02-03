use std::collections::HashMap;

use walrus::{DataId, ElementId, FunctionId, GlobalId, LocalId, MemoryId, TableId, TagId};

use crate::kinds::IdentifierModule;
use crate::merger::provenance_identifier::{Identifier, New, Old};

pub(crate) type OldIdTable = Identifier<Old, TableId>;
pub(crate) type NewIdTable = Identifier<New, TableId>;

pub(crate) type OldIdGlobal = Identifier<Old, GlobalId>;
pub(crate) type NewIdGlobal = Identifier<New, GlobalId>;

pub(crate) type OldIdMemory = Identifier<Old, MemoryId>;
pub(crate) type NewIdMemory = Identifier<New, MemoryId>;

pub(crate) type OldIdData = Identifier<Old, DataId>;
pub(crate) type NewIdData = Identifier<New, DataId>;

pub(crate) type OldIdElement = Identifier<Old, ElementId>;
pub(crate) type NewIdElement = Identifier<New, ElementId>;

pub(crate) type OldIdFunction = Identifier<Old, FunctionId>;
pub(crate) type NewIdFunction = Identifier<New, FunctionId>;

pub(crate) type OldIdLocal = Identifier<Old, LocalId>;
pub(crate) type NewIdLocal = Identifier<New, LocalId>;

pub(crate) type OldIdTag = Identifier<Old, TagId>;
pub(crate) type NewIdTag = Identifier<New, TagId>;

#[derive(Default, Debug, Clone)]
pub struct Mapping {
    pub tables: HashMap<(IdentifierModule, OldIdTable), NewIdTable>,
    pub globals: HashMap<(IdentifierModule, OldIdGlobal), NewIdGlobal>,
    pub memories: HashMap<(IdentifierModule, OldIdMemory), NewIdMemory>,
    pub datas: HashMap<(IdentifierModule, OldIdData), NewIdData>,
    pub elements: HashMap<(IdentifierModule, OldIdElement), NewIdElement>,
    pub funcs: HashMap<(IdentifierModule, OldIdFunction), NewIdFunction>,
    pub locals: HashMap<(IdentifierModule, OldIdLocal), NewIdLocal>,
    pub tags: HashMap<(IdentifierModule, OldIdTag), NewIdTag>,
}
