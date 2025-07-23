use std::collections::HashMap;

use walrus::InstrSeqBuilder;
use walrus::LocalFunction;
use walrus::Module;
use walrus::TypeId;
use walrus::ir::{
    AtomicFence, AtomicNotify, AtomicRmw, AtomicWait, Binop, Block, Br, BrIf, BrTable, Call,
    CallIndirect, Cmpxchg, Const, DataDrop, Drop, ElemDrop, GlobalGet, GlobalSet, I8x16Shuffle,
    I8x16Swizzle, IfElse, Instr, InstrLocId, InstrSeqId, InstrSeqType, Load, LoadSimd, LocalGet,
    LocalSet, LocalTee, Loop, MemoryCopy, MemoryFill, MemoryGrow, MemoryInit, MemorySize, RefFunc,
    RefIsNull, RefNull, Return, ReturnCall, ReturnCallIndirect, Select, Store, TableCopy,
    TableFill, TableGet, TableGrow, TableInit, TableSet, TableSize, TernOp, Unop, Unreachable,
    V128Bitselect, Visitor,
};

use crate::kinds::{FuncType, IdentifierModule};
use crate::merger::old_to_new_mapping::Mapping;
use crate::merger::old_to_new_mapping::NewIdData;
use crate::merger::old_to_new_mapping::NewIdElement;
use crate::merger::old_to_new_mapping::NewIdFunction;
use crate::merger::old_to_new_mapping::NewIdGlobal;
use crate::merger::old_to_new_mapping::NewIdLocal;
use crate::merger::old_to_new_mapping::NewIdMemory;
use crate::merger::old_to_new_mapping::NewIdTable;
use crate::merger::old_to_new_mapping::OldIdData;
use crate::merger::old_to_new_mapping::OldIdElement;
use crate::merger::old_to_new_mapping::OldIdFunction;
use crate::merger::old_to_new_mapping::OldIdGlobal;
use crate::merger::old_to_new_mapping::OldIdLocal;
use crate::merger::old_to_new_mapping::OldIdMemory;
use crate::merger::old_to_new_mapping::OldIdTable;
use crate::merger::provenance_identifier::{Identifier, New, Old};

struct SequenceStack {
    old: Vec<InstrSeqId>,
    new: Vec<InstrSeqId>,
    id_mapping: HashMap<InstrSeqId, InstrSeqId>,
}

impl SequenceStack {
    pub fn new(old: InstrSeqId, new: InstrSeqId) -> Self {
        let mut sequence_id_mapping = HashMap::new();
        sequence_id_mapping.insert(old, new);
        Self {
            old: vec![old],
            new: vec![new],
            id_mapping: sequence_id_mapping,
        }
    }

    pub fn push(&mut self, old: &InstrSeqId, new: &InstrSeqId) {
        self.old.push(*old);
        self.new.push(*new);
        self.id_mapping.insert(*old, *new);
    }

    #[must_use]
    pub fn pop(&mut self) -> (InstrSeqId, InstrSeqId) {
        let old = self.old.pop().unwrap();
        let new = self.new.pop().unwrap();
        let expected_new = self.id_mapping.get(&old).unwrap();
        assert_eq!(new, *expected_new);
        (old, new)
    }

    pub fn bind(&mut self, old: &InstrSeqId, new: &InstrSeqId) {
        self.id_mapping.insert(*old, *new);
    }

    pub fn resolve(&self, old: &InstrSeqId) -> InstrSeqId {
        *self.id_mapping.get(old).unwrap()
    }

    pub fn last_new(&self) -> InstrSeqId {
        *self.new.last().unwrap()
    }
}

pub(super) struct WasmFunctionCopy<'old_module, 'new_module> {
    old_module: &'old_module Module,
    new_module: &'new_module mut Module,

    old_function: &'old_module LocalFunction,

    old_module_name: IdentifierModule,
    mapping: &'old_module mut Mapping,

    new_function_index: NewIdFunction,

    sequence_stack: SequenceStack,
}

/*
This will be somewhat similar to a PDA:
When entering a sequence / two sequences;
- Create a dangling instruction sequence
- Start pushing the instructions
- When ending the sequence; the dangle should end!
*/

impl<'old_module, 'new_module> WasmFunctionCopy<'old_module, 'new_module> {
    pub(super) fn new(
        old_module: &'old_module Module,
        new_module: &'new_module mut Module,

        old_function: &'old_module LocalFunction,
        old_module_name: IdentifierModule,

        mapping: &'old_module mut Mapping,

        new_function_index: NewIdFunction,
        old_function_index: OldIdFunction,
    ) -> Self {
        let old_body_id = old_function.builder().func_body_id();
        let new_body_id = new_module
            .funcs
            .get_mut(*new_function_index)
            .kind
            .unwrap_local_mut()
            .builder_mut()
            .func_body()
            .id();

        for arg in &old_module
            .funcs
            .get(*old_function_index)
            .kind
            .unwrap_local()
            .args
        {
            let local_id: Identifier<Old, _> = (*arg).into();
            debug_assert!(
                mapping
                    .locals
                    .contains_key(&(old_module_name.clone(), local_id))
            );
        }

        Self {
            old_module,
            new_module,

            old_function,

            old_module_name,
            mapping,

            new_function_index,

            sequence_stack: SequenceStack::new(old_body_id, new_body_id),
        }
    }

    fn old_to_new_fn_id(&self, old_id: OldIdFunction) -> NewIdFunction {
        self.mapping
            .funcs
            .get(&(self.old_module_name.clone(), old_id))
            .copied()
            .unwrap()
    }

    fn old_to_new_local_id(&mut self, old_id: OldIdLocal) -> NewIdLocal {
        if self
            .mapping
            .locals
            .contains_key(&(self.old_module_name.clone(), old_id))
        {
            // Found local, retrieve
            *self
                .mapping
                .locals
                .get(&(self.old_module_name.clone(), old_id))
                .unwrap()
        } else {
            // FIXME: is this allowed by the specification? If not perhaps
            //        report this to user of tool...
            // Could not find local, include in new module & add to set
            let old_local: Identifier<Old, _> = self.old_module.locals.get(*old_id).into();
            let new_local: Identifier<New, _> = self.new_module.locals.add(old_local.ty()).into();

            self.mapping
                .locals
                .insert((self.old_module_name.clone(), old_id), new_local);
            new_local
        }
    }

    fn old_to_new_table_id(&self, old_id: OldIdTable) -> NewIdTable {
        self.mapping
            .tables
            .get(&(self.old_module_name.clone(), old_id))
            .copied()
            .unwrap()
    }

    fn old_to_new_global_id(&self, old_id: OldIdGlobal) -> NewIdGlobal {
        self.mapping
            .globals
            .get(&(self.old_module_name.clone(), old_id))
            .copied()
            .unwrap()
    }

    fn old_to_new_memory_id(&self, old_id: OldIdMemory) -> NewIdMemory {
        self.mapping
            .memories
            .get(&(self.old_module_name.clone(), old_id))
            .copied()
            .unwrap()
    }

    fn old_to_new_data_id(&self, old_id: OldIdData) -> NewIdData {
        self.mapping
            .datas
            .get(&(self.old_module_name.clone(), old_id))
            .copied()
            .unwrap()
    }

    fn old_to_new_elem_id(&self, old_id: OldIdElement) -> NewIdElement {
        self.mapping
            .elements
            .get(&(self.old_module_name.clone(), old_id))
            .copied()
            .unwrap()
    }

    fn old_to_new_type_id(&mut self, old_id: TypeId) -> TypeId {
        let old_type = self.old_module.types.get(old_id);
        self.new_module
            .types
            .add(old_type.params(), old_type.results())
    }

    fn current_sequence(&mut self) -> InstrSeqBuilder<'_> {
        let current_sequence_id = self.sequence_stack.last_new();
        self.new_module
            .funcs
            .get_mut(*self.new_function_index)
            .kind
            .unwrap_local_mut()
            .builder_mut()
            .instr_seq(current_sequence_id)
    }

    fn copy_over_instr_seq_ty(&mut self, old_ty: &InstrSeqType) -> InstrSeqType {
        match old_ty {
            InstrSeqType::Simple(val_type) => InstrSeqType::Simple(*val_type),
            InstrSeqType::MultiValue(id) => InstrSeqType::MultiValue(self.old_to_new_type_id(*id)),
        }
    }

    fn push_instr(&mut self, instr: &Instr) {
        match instr {
            Instr::Block(old_block) => old_block.copy_over(self),
            Instr::Loop(old_loop) => old_loop.copy_over(self),
            Instr::Call(old_call) => old_call.copy_over(self),
            Instr::CallIndirect(old_call_indirect) => old_call_indirect.copy_over(self),
            Instr::LocalGet(old_local_get) => old_local_get.copy_over(self),
            Instr::LocalSet(old_local_set) => old_local_set.copy_over(self),
            Instr::Unop(unop) => unop.copy_over(self),
            Instr::LocalTee(old_local_tee) => old_local_tee.copy_over(self),
            Instr::GlobalGet(global_get) => global_get.copy_over(self),
            Instr::GlobalSet(global_set) => global_set.copy_over(self),
            Instr::Const(cnst) => cnst.copy_over(self),
            Instr::TernOp(tern_op) => tern_op.copy_over(self),
            Instr::Binop(binop) => binop.copy_over(self),
            Instr::Select(select) => select.copy_over(self),
            Instr::Unreachable(unreachable) => unreachable.copy_over(self),
            Instr::Br(br) => br.copy_over(self),
            Instr::BrIf(br_if) => br_if.copy_over(self),
            Instr::IfElse(if_else) => if_else.copy_over(self),
            Instr::BrTable(br_table) => br_table.copy_over(self),
            Instr::Drop(drop) => drop.copy_over(self),
            Instr::Return(return_) => return_.copy_over(self),
            Instr::MemorySize(memory_size) => memory_size.copy_over(self),
            Instr::MemoryGrow(memory_grow) => memory_grow.copy_over(self),
            Instr::MemoryInit(memory_init) => memory_init.copy_over(self),
            Instr::DataDrop(data_drop) => data_drop.copy_over(self),
            Instr::MemoryCopy(memory_copy) => memory_copy.copy_over(self),
            Instr::MemoryFill(memory_fill) => memory_fill.copy_over(self),
            Instr::Load(load) => load.copy_over(self),
            Instr::Store(store) => store.copy_over(self),
            Instr::AtomicRmw(atomic_rmw) => atomic_rmw.copy_over(self),
            Instr::Cmpxchg(cmpxchg) => cmpxchg.copy_over(self),
            Instr::AtomicNotify(atomic_notify) => atomic_notify.copy_over(self),
            Instr::AtomicWait(atomic_wait) => atomic_wait.copy_over(self),
            Instr::AtomicFence(atomic_fence) => atomic_fence.copy_over(self),
            Instr::TableGet(table_get) => table_get.copy_over(self),
            Instr::TableSet(table_set) => table_set.copy_over(self),
            Instr::TableGrow(table_grow) => table_grow.copy_over(self),
            Instr::TableSize(table_size) => table_size.copy_over(self),
            Instr::TableFill(table_fill) => table_fill.copy_over(self),
            Instr::RefNull(ref_null) => ref_null.copy_over(self),
            Instr::RefIsNull(ref_is_null) => ref_is_null.copy_over(self),
            Instr::RefFunc(ref_func) => ref_func.copy_over(self),
            Instr::V128Bitselect(v128_bitselect) => v128_bitselect.copy_over(self),
            Instr::I8x16Swizzle(i8x16_swizzle) => i8x16_swizzle.copy_over(self),
            Instr::I8x16Shuffle(i8x16_shuffle) => i8x16_shuffle.copy_over(self),
            Instr::LoadSimd(load_simd) => load_simd.copy_over(self),
            Instr::TableInit(table_init) => table_init.copy_over(self),
            Instr::ElemDrop(elem_drop) => elem_drop.copy_over(self),
            Instr::TableCopy(table_copy) => table_copy.copy_over(self),
            Instr::ReturnCall(return_call) => return_call.copy_over(self),
            Instr::ReturnCallIndirect(return_call_indirect) => return_call_indirect.copy_over(self),
        }
    }
}

impl<'instr> Visitor<'instr> for WasmFunctionCopy<'_, '_> {
    // Other visit methods are not used
    fn visit_instr(&mut self, instr: &'instr Instr, _instr_loc: &'instr InstrLocId) {
        self.push_instr(instr);
    }

    fn start_instr_seq(&mut self, instr_seq: &'instr walrus::ir::InstrSeq) {
        let new_sequence_id = self.sequence_stack.resolve(&instr_seq.id());
        self.sequence_stack.push(&instr_seq.id(), &new_sequence_id);
    }

    fn end_instr_seq(&mut self, instr_seq: &'instr walrus::ir::InstrSeq) {
        let (old, _new) = self.sequence_stack.pop();
        assert_eq!(old, instr_seq.id());
    }
}

trait CopyOver {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>);
}

impl CopyOver for &Block {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        // @SCOPE_CHANGE
        let old_ty = target.old_function.block(self.seq).ty;
        let new_ty = target.copy_over_instr_seq_ty(&old_ty);
        let mut current_sequence = target.current_sequence();
        let new_block_builder = current_sequence.dangling_instr_seq(new_ty);
        let new_block_id = new_block_builder.id();
        current_sequence.instr(Block { seq: new_block_id });
        target.sequence_stack.bind(&self.seq, &new_block_id);
    }
}

impl CopyOver for &Loop {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        // @SCOPE_CHANGE
        let old_ty = target.old_function.block(self.seq).ty;
        let new_ty = target.copy_over_instr_seq_ty(&old_ty);
        let mut current_sequence = target.current_sequence();
        let new_block_builder = current_sequence.dangling_instr_seq(new_ty);
        let new_block_id = new_block_builder.id();
        current_sequence.instr(Loop { seq: new_block_id });
        target.sequence_stack.bind(&self.seq, &new_block_id);
    }
}

impl CopyOver for &Call {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_function_id: Identifier<Old, _> = self.func.into();
        let new_function_id: Identifier<New, _> = target.old_to_new_fn_id(old_function_id);
        target.current_sequence().call(*new_function_id);
    }
}

impl CopyOver for &CallIndirect {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let owned_type = FuncType::from_types(self.ty, &target.old_module.types);
        let new_type = target
            .new_module
            .types
            .add(owned_type.params(), owned_type.results());
        let old_table_id: Identifier<Old, _> = self.table.into();
        let new_table_id: Identifier<New, _> = target.old_to_new_table_id(old_table_id);
        target
            .current_sequence()
            .call_indirect(new_type, *new_table_id);
    }
}

impl CopyOver for &LocalGet {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_local_id: Identifier<Old, _> = self.local.into();
        let new_local_id: Identifier<New, _> = target.old_to_new_local_id(old_local_id);
        target.current_sequence().local_get(*new_local_id);
    }
}

impl CopyOver for &LocalSet {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_local_id: Identifier<Old, _> = self.local.into();
        let new_local_id: Identifier<New, _> = target.old_to_new_local_id(old_local_id);
        target.current_sequence().local_set(*new_local_id);
    }
}

impl CopyOver for &Unop {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        target.current_sequence().unop(self.op);
    }
}

impl CopyOver for &LocalTee {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_local_id: Identifier<Old, _> = self.local.into();
        let new_local_id: Identifier<New, _> = target.old_to_new_local_id(old_local_id);
        target.current_sequence().local_tee(*new_local_id);
    }
}

impl CopyOver for &GlobalGet {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_global_id: Identifier<Old, _> = self.global.into();
        let new_global_id: Identifier<New, _> = target.old_to_new_global_id(old_global_id);
        target.current_sequence().global_get(*new_global_id);
    }
}

impl CopyOver for &GlobalSet {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_global_id: Identifier<Old, _> = self.global.into();
        let new_global_id: Identifier<New, _> = target.old_to_new_global_id(old_global_id);
        target.current_sequence().global_set(*new_global_id);
    }
}

impl CopyOver for &Const {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        target.current_sequence().const_(self.value);
    }
}

impl CopyOver for &TernOp {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        target.current_sequence().tern_op(self.op);
    }
}

impl CopyOver for &Binop {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        target.current_sequence().binop(self.op);
    }
}

impl CopyOver for &Select {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        target.current_sequence().select(self.ty);
    }
}

impl CopyOver for &Unreachable {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let _ = self;
        target.current_sequence().unreachable();
    }
}

impl CopyOver for &Br {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let new_label_id = target.sequence_stack.resolve(&self.block);
        target.current_sequence().br(new_label_id);
    }
}

impl CopyOver for &BrIf {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let new_label_id = target.sequence_stack.resolve(&self.block);
        target.current_sequence().br_if(new_label_id);
    }
}

impl CopyOver for &IfElse {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        // @SCOPE_CHANGE
        let IfElse {
            consequent: old_consequent_id,
            alternative: old_alternative_id,
        } = self;
        let consequent_ty = target.old_function.block(*old_consequent_id).ty;
        let consequent_ty_new = target.copy_over_instr_seq_ty(&consequent_ty);
        let alternative_ty = target.old_function.block(*old_alternative_id).ty;
        let alternative_ty_new = target.copy_over_instr_seq_ty(&alternative_ty);
        let mut current_sequence = target.current_sequence();
        let consequent_builder = current_sequence.dangling_instr_seq(consequent_ty_new);
        let consequent_builder_id = consequent_builder.id();
        let alternative_builder = current_sequence.dangling_instr_seq(alternative_ty_new);
        let alternative_builder_id = alternative_builder.id();
        target.current_sequence().instr(IfElse {
            consequent: consequent_builder_id,
            alternative: alternative_builder_id,
        });
        target
            .sequence_stack
            .bind(old_consequent_id, &consequent_builder_id);
        target
            .sequence_stack
            .bind(old_alternative_id, &alternative_builder_id);
    }
}

impl CopyOver for &BrTable {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let new_labels: Vec<_> = self
            .blocks
            .iter()
            .map(|block| target.sequence_stack.resolve(block))
            .collect();
        let default = target.sequence_stack.resolve(&self.default);
        target
            .current_sequence()
            .br_table(new_labels.into(), default);
    }
}

impl CopyOver for &Drop {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let _ = self;
        target.current_sequence().drop();
    }
}

impl CopyOver for &Return {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        target.current_sequence().return_();
    }
}

impl CopyOver for &MemorySize {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_memory_id: Identifier<Old, _> = self.memory.into();
        let new_memory_id: Identifier<New, _> = target.old_to_new_memory_id(old_memory_id);
        target.current_sequence().memory_size(*new_memory_id);
    }
}

impl CopyOver for &MemoryGrow {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_memory_id: Identifier<Old, _> = self.memory.into();
        let new_memory_id: Identifier<New, _> = target.old_to_new_memory_id(old_memory_id);
        target.current_sequence().memory_grow(*new_memory_id);
    }
}

impl CopyOver for &MemoryInit {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_memory_id: Identifier<Old, _> = self.memory.into();
        let new_memory_id: Identifier<New, _> = target.old_to_new_memory_id(old_memory_id);
        let old_data_id: Identifier<Old, _> = self.data.into();
        let new_data_id: Identifier<New, _> = target.old_to_new_data_id(old_data_id);
        target
            .current_sequence()
            .memory_init(*new_memory_id, *new_data_id);
    }
}

impl CopyOver for &DataDrop {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_data_id: Identifier<Old, _> = self.data.into();
        let new_data_id: Identifier<New, _> = target.old_to_new_data_id(old_data_id);
        target.current_sequence().data_drop(*new_data_id);
    }
}

impl CopyOver for &MemoryCopy {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_src_memory_id: Identifier<Old, _> = self.src.into();
        let new_src_memory_id: Identifier<New, _> = target.old_to_new_memory_id(old_src_memory_id);
        let old_dst_memory_id: Identifier<Old, _> = self.dst.into();
        let new_dst_memory_id: Identifier<New, _> = target.old_to_new_memory_id(old_dst_memory_id);
        target
            .current_sequence()
            .memory_copy(*new_src_memory_id, *new_dst_memory_id);
    }
}

impl CopyOver for &MemoryFill {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_memory_id: Identifier<Old, _> = self.memory.into();
        let new_memory_id: Identifier<New, _> = target.old_to_new_memory_id(old_memory_id);
        target.current_sequence().memory_fill(*new_memory_id);
    }
}

impl CopyOver for &Load {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_memory_id: Identifier<Old, _> = self.memory.into();
        let new_memory_id: Identifier<New, _> = target.old_to_new_memory_id(old_memory_id);
        target
            .current_sequence()
            .load(*new_memory_id, self.kind, self.arg);
    }
}

impl CopyOver for &Store {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_memory_id: Identifier<Old, _> = self.memory.into();
        let new_memory_id: Identifier<New, _> = target.old_to_new_memory_id(old_memory_id);
        target
            .current_sequence()
            .store(*new_memory_id, self.kind, self.arg);
    }
}

impl CopyOver for &AtomicRmw {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_memory_id: Identifier<Old, _> = self.memory.into();
        let new_memory_id: Identifier<New, _> = target.old_to_new_memory_id(old_memory_id);
        target
            .current_sequence()
            .atomic_rmw(*new_memory_id, self.op, self.width, self.arg);
    }
}

impl CopyOver for &Cmpxchg {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_memory_id: Identifier<Old, _> = self.memory.into();
        let new_memory_id: Identifier<New, _> = target.old_to_new_memory_id(old_memory_id);
        target
            .current_sequence()
            .cmpxchg(*new_memory_id, self.width, self.arg);
    }
}

impl CopyOver for &AtomicNotify {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_memory_id: Identifier<Old, _> = self.memory.into();
        let new_memory_id: Identifier<New, _> = target.old_to_new_memory_id(old_memory_id);
        target
            .current_sequence()
            .atomic_notify(*new_memory_id, self.arg);
    }
}

impl CopyOver for &AtomicWait {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_memory_id: Identifier<Old, _> = self.memory.into();
        let new_memory_id: Identifier<New, _> = target.old_to_new_memory_id(old_memory_id);
        target
            .current_sequence()
            .atomic_wait(*new_memory_id, self.arg, self.sixty_four);
    }
}

impl CopyOver for &AtomicFence {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let _ = self;
        target.current_sequence().atomic_fence();
    }
}

impl CopyOver for &TableGet {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_table_id: Identifier<Old, _> = self.table.into();
        let new_table_id: Identifier<New, _> = target.old_to_new_table_id(old_table_id);
        target.current_sequence().table_get(*new_table_id);
    }
}

impl CopyOver for &TableSet {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_table_id: Identifier<Old, _> = self.table.into();
        let new_table_id: Identifier<New, _> = target.old_to_new_table_id(old_table_id);
        target.current_sequence().table_set(*new_table_id);
    }
}

impl CopyOver for &TableGrow {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_table_id: Identifier<Old, _> = self.table.into();
        let new_table_id: Identifier<New, _> = target.old_to_new_table_id(old_table_id);
        target.current_sequence().table_grow(*new_table_id);
    }
}

impl CopyOver for &TableSize {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_table_id: Identifier<Old, _> = self.table.into();
        let new_table_id: Identifier<New, _> = target.old_to_new_table_id(old_table_id);
        target.current_sequence().table_size(*new_table_id);
    }
}

impl CopyOver for &TableFill {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_table_id: Identifier<Old, _> = self.table.into();
        let new_table_id: Identifier<New, _> = target.old_to_new_table_id(old_table_id);
        target.current_sequence().table_fill(*new_table_id);
    }
}

impl CopyOver for &RefNull {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        target.current_sequence().ref_null(self.ty);
    }
}

impl CopyOver for &RefIsNull {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let _ = self;
        target.current_sequence().ref_is_null();
    }
}

impl CopyOver for &RefFunc {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_function_id: Identifier<Old, _> = self.func.into();
        let new_function_id = target.old_to_new_fn_id(old_function_id);
        target.current_sequence().ref_func(*new_function_id);
    }
}

impl CopyOver for &V128Bitselect {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let _ = self;
        target.current_sequence().v128_bitselect();
    }
}

impl CopyOver for &I8x16Swizzle {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let _ = self;
        target.current_sequence().i8x16_swizzle();
    }
}

impl CopyOver for &I8x16Shuffle {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        target.current_sequence().i8x16_shuffle(self.indices);
    }
}

impl CopyOver for &LoadSimd {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_memory_id: Identifier<Old, _> = self.memory.into();
        let new_memory_id: Identifier<New, _> = target.old_to_new_memory_id(old_memory_id);
        target
            .current_sequence()
            .load_simd(*new_memory_id, self.kind, self.arg);
    }
}

impl CopyOver for &TableInit {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_table_id: Identifier<Old, _> = self.table.into();
        let new_table_id: Identifier<New, _> = target.old_to_new_table_id(old_table_id);
        let old_elem_id: Identifier<Old, _> = self.elem.into();
        let new_elem_id: Identifier<New, _> = target.old_to_new_elem_id(old_elem_id);
        target
            .current_sequence()
            .table_init(*new_table_id, *new_elem_id);
    }
}

impl CopyOver for &ElemDrop {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_elem_id: Identifier<Old, _> = self.elem.into();
        let new_elem_id: Identifier<New, _> = target.old_to_new_elem_id(old_elem_id);
        target.current_sequence().elem_drop(*new_elem_id);
    }
}

impl CopyOver for &TableCopy {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_src_table_id: Identifier<Old, _> = self.src.into();
        let new_src_table_id: Identifier<New, _> = target.old_to_new_table_id(old_src_table_id);
        let old_dst_table_id: Identifier<Old, _> = self.dst.into();
        let new_dst_table_id: Identifier<New, _> = target.old_to_new_table_id(old_dst_table_id);
        target
            .current_sequence()
            .table_copy(*new_src_table_id, *new_dst_table_id);
    }
}

impl CopyOver for &ReturnCall {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_function_id: Identifier<Old, _> = self.func.into();
        let new_function_id: Identifier<New, _> = target.old_to_new_fn_id(old_function_id);
        target.current_sequence().return_call(*new_function_id);
    }
}

impl CopyOver for &ReturnCallIndirect {
    fn copy_over(&self, target: &mut WasmFunctionCopy<'_, '_>) {
        let old_table_id: Identifier<Old, _> = self.table.into();
        let new_table_id: Identifier<New, _> = target.old_to_new_table_id(old_table_id);
        let owned_type = FuncType::from_types(self.ty, &target.old_module.types);
        let new_type = target
            .new_module
            .types
            .add(owned_type.params(), owned_type.results());
        let mut current_sequence = target.current_sequence();
        current_sequence.return_call_indirect(new_type, *new_table_id);
    }
}
