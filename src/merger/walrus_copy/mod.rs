use std::collections::HashMap;

use walrus::DataId;
use walrus::ElementId;
use walrus::FunctionId;
use walrus::GlobalId;
use walrus::InstrSeqBuilder;
use walrus::LocalFunction;
use walrus::LocalId;
use walrus::MemoryId;
use walrus::Module;
use walrus::TableId;
use walrus::ir::Block;
use walrus::ir::IfElse;
use walrus::ir::InstrLocId;
use walrus::ir::InstrSeqId;
use walrus::ir::Loop;
use walrus::ir::{Instr, Visitor};

use crate::resolver::FuncType;
use crate::resolver::resolution_schema::Before;

use super::old_to_new_mapping::Mapping;

struct SequenceStack {
    old_sequence_stack: Vec<InstrSeqId>,
    new_sequence_stack: Vec<InstrSeqId>,
    sequence_id_mapping: HashMap<InstrSeqId, InstrSeqId>,
}

impl SequenceStack {
    pub fn new(old: InstrSeqId, new: InstrSeqId) -> Self {
        let mut sequence_id_mapping = HashMap::new();
        sequence_id_mapping.insert(old, new);
        Self {
            old_sequence_stack: vec![old],
            new_sequence_stack: vec![new],
            sequence_id_mapping,
        }
    }

    pub fn push(&mut self, old: &InstrSeqId, new: &InstrSeqId) {
        self.old_sequence_stack.push(*old);
        self.new_sequence_stack.push(*new);
        self.sequence_id_mapping.insert(*old, *new);
    }

    #[must_use]
    pub fn pop(&mut self) -> (InstrSeqId, InstrSeqId) {
        let old = self.old_sequence_stack.pop().unwrap();
        let new = self.new_sequence_stack.pop().unwrap();
        let expected_new = self.sequence_id_mapping.get(&old).unwrap();
        assert_eq!(new, *expected_new);
        (old, new)
    }

    pub fn bind(&mut self, old: &InstrSeqId, new: &InstrSeqId) {
        self.sequence_id_mapping.insert(*old, *new);
    }

    pub fn resolve(&self, old: &InstrSeqId) -> InstrSeqId {
        *self.sequence_id_mapping.get(old).unwrap()
    }

    pub fn last_new(&self) -> InstrSeqId {
        *self.new_sequence_stack.last().unwrap()
    }
}

pub(super) struct WasmFunctionCopy<'old_module, 'new_module> {
    old_module: &'old_module Module,
    new_module: &'new_module mut Module,

    old_function: &'old_module LocalFunction,

    old_module_name: String,
    mapping: &'old_module Mapping,

    new_function_index: FunctionId,
    old_function_index: FunctionId,

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

        old_module_name: String,
        mapping: &'old_module Mapping,

        new_function_index: FunctionId,
        old_function_index: FunctionId,
    ) -> Self {
        let old_body_id = old_function.builder().func_body_id();
        let new_body_id = new_module
            .funcs
            .get_mut(new_function_index)
            .kind
            .unwrap_local_mut()
            .builder_mut()
            .func_body()
            .id();

        Self {
            old_module,
            new_module,

            old_function,

            old_module_name,
            mapping,

            new_function_index,
            old_function_index,

            sequence_stack: SequenceStack::new(old_body_id, new_body_id),
        }
    }

    fn old_to_new_fn_id(&self, old_id: FunctionId) -> FunctionId {
        self.mapping
            .function_mapping
            .get(&(self.old_module_name.as_str().into(), Before(old_id)))
            .copied()
            .unwrap()
    }

    fn old_to_new_table_id(&self, old_id: TableId) -> TableId {
        self.mapping
            .tables
            .get(&(self.old_module_name.to_string(), old_id))
            .copied()
            .unwrap()
    }

    fn old_to_new_local_id(&self, old_id: LocalId) -> LocalId {
        *self
            .mapping
            .locals_mapping
            .get(&(
                self.old_module_name.as_str().into(),
                Before(self.old_function_index),
                old_id,
            ))
            .unwrap()
    }

    fn old_to_new_global_id(&self, old_id: GlobalId) -> GlobalId {
        self.mapping
            .globals
            .get(&(self.old_module_name.to_string(), old_id))
            .copied()
            .unwrap()
    }

    fn old_to_new_memory_id(&self, old_id: MemoryId) -> MemoryId {
        self.mapping
            .memories
            .get(&(self.old_module_name.to_string(), old_id))
            .copied()
            .unwrap()
    }

    fn old_to_new_data_id(&self, old_id: DataId) -> DataId {
        self.mapping
            .datas
            .get(&(self.old_module_name.to_string(), old_id))
            .copied()
            .unwrap()
    }

    fn old_to_new_elem_id(&self, old_id: ElementId) -> ElementId {
        self.mapping
            .elements
            .get(&(self.old_module_name.to_string(), old_id))
            .copied()
            .unwrap()
    }

    fn current_sequence(&mut self) -> InstrSeqBuilder<'_> {
        let current_sequence_id = self.sequence_stack.last_new();
        self.new_module
            .funcs
            .get_mut(self.new_function_index)
            .kind
            .unwrap_local_mut()
            .builder_mut()
            .instr_seq(current_sequence_id)
    }

    fn push_instr(&mut self, instr: &Instr) {
        match instr {
            Instr::Block(old_block) => {
                // @SCOPE_CHANGE
                let ty = self.old_function.block(old_block.seq).ty;
                let mut current_sequence = self.current_sequence();
                let new_block_builder = current_sequence.dangling_instr_seq(ty);
                let new_block_id = new_block_builder.id();
                current_sequence.instr(Block { seq: new_block_id });
                self.sequence_stack.bind(&old_block.seq, &new_block_id);
            }
            Instr::Loop(old_loop) => {
                // @SCOPE_CHANGE
                let ty = self.old_function.block(old_loop.seq).ty;
                let mut current_sequence = self.current_sequence();
                let new_block_builder = current_sequence.dangling_instr_seq(ty);
                let new_block_id = new_block_builder.id();
                current_sequence.instr(Loop { seq: new_block_id });
                self.sequence_stack.bind(&old_loop.seq, &new_block_id);
            }
            Instr::Call(old_call) => {
                let new_function_id = self.old_to_new_fn_id(old_call.func);
                self.current_sequence().call(new_function_id);
            }
            Instr::CallIndirect(old_call_indirect) => {
                let owned_type = FuncType::from_types(old_call_indirect.ty, &self.old_module.types);
                let new_type = self
                    .new_module
                    .types
                    .find(owned_type.params(), owned_type.results())
                    .unwrap();
                let new_table_id = self.old_to_new_table_id(old_call_indirect.table);
                self.current_sequence()
                    .call_indirect(new_type, new_table_id);
            }
            Instr::LocalGet(old_local_get) => {
                let new_local = self.old_to_new_local_id(old_local_get.local);
                self.current_sequence().local_get(new_local);
            }
            Instr::LocalSet(old_local_set) => {
                let new_local = self.old_to_new_local_id(old_local_set.local);
                self.current_sequence().local_set(new_local);
            }
            Instr::Unop(unop) => {
                self.current_sequence().unop(unop.op);
            }
            Instr::LocalTee(old_local_tee) => {
                let new_local = self.old_to_new_local_id(old_local_tee.local);
                self.current_sequence().local_tee(new_local);
            }
            Instr::GlobalGet(global_get) => {
                let new_global_id = self.old_to_new_global_id(global_get.global);
                self.current_sequence().global_get(new_global_id);
            }
            Instr::GlobalSet(global_set) => {
                let new_global_id = self.old_to_new_global_id(global_set.global);
                self.current_sequence().global_set(new_global_id);
            }
            Instr::Const(cnst) => {
                self.current_sequence().const_(cnst.value);
            }
            Instr::TernOp(tern_op) => {
                self.current_sequence().tern_op(tern_op.op);
            }
            Instr::Binop(binop) => {
                self.current_sequence().binop(binop.op);
            }
            Instr::Select(select) => {
                self.current_sequence().select(select.ty);
            }
            Instr::Unreachable(unreachable) => {
                let _ = unreachable;
                self.current_sequence().unreachable();
            }
            Instr::Br(br) => {
                let new_label_id = self.sequence_stack.resolve(&br.block);
                self.current_sequence().br(new_label_id);
            }
            Instr::BrIf(br_if) => {
                let new_label_id = self.sequence_stack.resolve(&br_if.block);
                self.current_sequence().br_if(new_label_id);
            }
            Instr::IfElse(if_else) => {
                // @SCOPE_CHANGE
                let IfElse {
                    consequent: old_consequent_id,
                    alternative: old_alternative_id,
                } = if_else;
                let consequent_ty = self.old_function.block(*old_consequent_id).ty;
                let alternative_ty = self.old_function.block(*old_alternative_id).ty;
                let mut current_sequence = self.current_sequence();
                let consequent_builder = current_sequence.dangling_instr_seq(consequent_ty);
                let consequent_builder_id = consequent_builder.id();
                let alternative_builder = current_sequence.dangling_instr_seq(alternative_ty);
                let alternative_builder_id = alternative_builder.id();
                self.current_sequence().instr(IfElse {
                    consequent: consequent_builder_id,
                    alternative: alternative_builder_id,
                });
                self.sequence_stack
                    .bind(old_consequent_id, &consequent_builder_id);
                self.sequence_stack
                    .bind(old_alternative_id, &alternative_builder_id);
            }
            Instr::BrTable(br_table) => {
                let new_labels: Vec<_> = br_table
                    .blocks
                    .iter()
                    .map(|block| self.sequence_stack.resolve(block))
                    .collect();
                let default = self.sequence_stack.resolve(&br_table.default);
                self.current_sequence().br_table(new_labels.into(), default);
            }
            Instr::Drop(drop) => {
                let _ = drop;
                self.current_sequence().drop();
            }
            Instr::Return(_) => {
                self.current_sequence().return_();
            }
            Instr::MemorySize(memory_size) => {
                let new_memory_id = self.old_to_new_memory_id(memory_size.memory);
                self.current_sequence().memory_size(new_memory_id);
            }
            Instr::MemoryGrow(memory_grow) => {
                let new_memory_id = self.old_to_new_memory_id(memory_grow.memory);
                self.current_sequence().memory_grow(new_memory_id);
            }
            Instr::MemoryInit(memory_init) => {
                let new_data_id = self.old_to_new_data_id(memory_init.data);
                let new_memory_id = self.old_to_new_memory_id(memory_init.memory);
                self.current_sequence()
                    .memory_init(new_memory_id, new_data_id);
            }
            Instr::DataDrop(data_drop) => {
                let new_data_id = self.old_to_new_data_id(data_drop.data);
                self.current_sequence().data_drop(new_data_id);
            }
            Instr::MemoryCopy(memory_copy) => {
                let new_src_memory_id = self.old_to_new_memory_id(memory_copy.src);
                let new_dst_memory_id = self.old_to_new_memory_id(memory_copy.dst);
                self.current_sequence()
                    .memory_copy(new_src_memory_id, new_dst_memory_id);
            }
            Instr::MemoryFill(memory_fill) => {
                let new_memory_id = self.old_to_new_memory_id(memory_fill.memory);
                self.current_sequence().memory_fill(new_memory_id);
            }
            Instr::Load(load) => {
                let new_memory_id = self.old_to_new_memory_id(load.memory);
                self.current_sequence()
                    .load(new_memory_id, load.kind, load.arg);
            }
            Instr::Store(store) => {
                let new_memory_id = self.old_to_new_memory_id(store.memory);
                self.current_sequence()
                    .store(new_memory_id, store.kind, store.arg);
            }
            Instr::AtomicRmw(atomic_rmw) => {
                let new_memory_id = self.old_to_new_memory_id(atomic_rmw.memory);
                self.current_sequence().atomic_rmw(
                    new_memory_id,
                    atomic_rmw.op,
                    atomic_rmw.width,
                    atomic_rmw.arg,
                );
            }
            Instr::Cmpxchg(cmpxchg) => {
                let new_memory_id = self.old_to_new_memory_id(cmpxchg.memory);
                self.current_sequence()
                    .cmpxchg(new_memory_id, cmpxchg.width, cmpxchg.arg);
            }
            Instr::AtomicNotify(atomic_notify) => {
                let new_memory_id = self.old_to_new_memory_id(atomic_notify.memory);
                self.current_sequence()
                    .atomic_notify(new_memory_id, atomic_notify.arg);
            }
            Instr::AtomicWait(atomic_wait) => {
                let new_memory_id = self.old_to_new_memory_id(atomic_wait.memory);
                self.current_sequence().atomic_wait(
                    new_memory_id,
                    atomic_wait.arg,
                    atomic_wait.sixty_four,
                );
            }
            Instr::AtomicFence(atomic_fence) => {
                let _ = atomic_fence;
                self.current_sequence().atomic_fence();
            }
            Instr::TableGet(table_get) => {
                let new_table_id = self.old_to_new_table_id(table_get.table);
                self.current_sequence().table_get(new_table_id);
            }
            Instr::TableSet(table_set) => {
                let new_table_id = self.old_to_new_table_id(table_set.table);
                self.current_sequence().table_set(new_table_id);
            }

            Instr::TableGrow(table_grow) => {
                let new_table_id = self.old_to_new_table_id(table_grow.table);
                self.current_sequence().table_grow(new_table_id);
            }
            Instr::TableSize(table_size) => {
                let new_table_id = self.old_to_new_table_id(table_size.table);
                self.current_sequence().table_size(new_table_id);
            }
            Instr::TableFill(table_fill) => {
                let new_table_id = self.old_to_new_table_id(table_fill.table);
                self.current_sequence().table_fill(new_table_id);
            }
            Instr::RefNull(ref_null) => {
                self.current_sequence().ref_null(ref_null.ty);
            }
            Instr::RefIsNull(ref_is_null) => {
                let _ = ref_is_null;
                self.current_sequence().ref_is_null();
            }
            Instr::RefFunc(ref_func) => {
                let new_function_id = self.old_to_new_fn_id(ref_func.func);
                self.current_sequence().ref_func(new_function_id);
            }

            Instr::V128Bitselect(v128_bitselect) => {
                let _ = v128_bitselect;
                self.current_sequence().v128_bitselect();
            }
            Instr::I8x16Swizzle(i8x16_swizzle) => {
                let _ = i8x16_swizzle;
                self.current_sequence().i8x16_swizzle();
            }
            Instr::I8x16Shuffle(i8x16_shuffle) => {
                self.current_sequence().i8x16_shuffle(i8x16_shuffle.indices);
            }
            Instr::LoadSimd(load_simd) => {
                let new_memory_id = self.old_to_new_memory_id(load_simd.memory);
                self.current_sequence()
                    .load_simd(new_memory_id, load_simd.kind, load_simd.arg);
            }
            Instr::TableInit(table_init) => {
                let new_table_id = self.old_to_new_table_id(table_init.table);
                let new_elem_id = self.old_to_new_elem_id(table_init.elem);
                self.current_sequence()
                    .table_init(new_table_id, new_elem_id);
            }

            Instr::ElemDrop(elem_drop) => {
                let new_elem_id = self.old_to_new_elem_id(elem_drop.elem);
                self.current_sequence().elem_drop(new_elem_id);
            }
            Instr::TableCopy(table_copy) => {
                let new_src_table_id = self.old_to_new_table_id(table_copy.src);
                let new_dst_table_id = self.old_to_new_table_id(table_copy.dst);
                self.current_sequence()
                    .table_copy(new_src_table_id, new_dst_table_id);
            }
            Instr::ReturnCall(return_call) => {
                let new_function_id = self.old_to_new_fn_id(return_call.func);
                self.current_sequence().return_call(new_function_id);
            }
            Instr::ReturnCallIndirect(return_call_indirect) => {
                let new_table_id = self.old_to_new_table_id(return_call_indirect.table);
                let owned_type =
                    FuncType::from_types(return_call_indirect.ty, &self.old_module.types);
                let new_type = self
                    .new_module
                    .types
                    .find(owned_type.params(), owned_type.results())
                    .unwrap();
                let mut current_sequence = self.current_sequence();
                current_sequence.return_call_indirect(new_type, new_table_id);
            }
        }
    }
}

impl<'instr, 'builder, 'old_function> Visitor<'instr>
    for WasmFunctionCopy<'builder, 'old_function>
{
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
