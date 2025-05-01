use walrus::DataId;
use walrus::ElementId;
use walrus::FunctionBuilder;
use walrus::FunctionId;
use walrus::GlobalId;
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

use super::old_to_new_mapping::Mapping;

pub(super) struct WasmFunctionCopy<'old_module, 'new_module> {
    old_module: &'old_module Module,
    new_module: &'new_module mut Module,

    old_function: &'old_module LocalFunction,

    locals: Vec<LocalId>,

    new_function_builder: FunctionBuilder,
    old_sequence_stack: Vec<InstrSeqId>,
    new_sequence_stack: Vec<InstrSeqId>,

    old_module_name: String,
    mapping: &'old_module Mapping,
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

        locals: Vec<LocalId>,
        owned_type: FuncType,

        old_module_name: String,
        mapping: &'old_module Mapping,
    ) -> Self {
        let mut new_function_builder = FunctionBuilder::new(
            &mut new_module.types,
            owned_type.params(),
            owned_type.results(),
        );

        let old_body_id = old_function.builder().func_body_id();
        let new_body_id = new_function_builder.func_body().id();

        Self {
            old_module,
            new_module,

            old_function,

            locals,
            new_function_builder,

            old_sequence_stack: vec![old_body_id],
            new_sequence_stack: vec![new_body_id],

            old_module_name,
            mapping,
        }
    }

    pub fn finish(self) {
        todo!()
    }

    fn old_to_new_fn_id(&self, old_id: FunctionId) -> FunctionId {
        self.mapping
            .functions
            .get(&(self.old_module_name.to_string(), old_id))
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

    fn push_instr(&mut self, instr: &Instr) {
        println!("Pushing {instr:?}");
        match instr {
            Instr::Block(old_block) => {
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                let ty = self.old_function.block(old_block.seq).ty;
                let new_block_builder = current_sequence.dangling_instr_seq(ty);
                let new_block_id = new_block_builder.id();
                self.new_sequence_stack.push(new_block_id);
                current_sequence.instr(Block { seq: new_block_id });
            }
            Instr::Loop(old_loop) => {
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                let ty = self.old_function.block(old_loop.seq).ty;
                let new_block_builder = current_sequence.dangling_instr_seq(ty);
                let new_block_id = new_block_builder.id();
                self.new_sequence_stack.push(new_block_id);
                current_sequence.instr(Loop { seq: new_block_id });
            }
            Instr::Call(old_call) => {
                let new_function_id = self.old_to_new_fn_id(old_call.func);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.call(new_function_id);
            }
            Instr::CallIndirect(old_call_indirect) => {
                let owned_type = FuncType::from_types(old_call_indirect.ty, &self.old_module.types);
                let new_type = self
                    .new_module
                    .types
                    .add(owned_type.params(), owned_type.results());
                let new_table_id = self.old_to_new_table_id(old_call_indirect.table);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.call_indirect(new_type, new_table_id);
            }
            Instr::LocalGet(old_local_get) => {
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.local_get(*self.locals.get(old_local_get.local.index()).unwrap());
            }
            Instr::LocalSet(old_local_set) => {
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.local_set(*self.locals.get(old_local_set.local.index()).unwrap());
            }
            Instr::Unop(unop) => {
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.unop(unop.op);
            }
            Instr::LocalTee(local_tee) => {
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.local_tee(*self.locals.get(local_tee.local.index()).unwrap());
            }
            Instr::GlobalGet(global_get) => {
                let new_global_id = self.old_to_new_global_id(global_get.global);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.global_get(new_global_id);
            }
            Instr::GlobalSet(global_set) => {
                let new_global_id = self.old_to_new_global_id(global_set.global);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.global_set(new_global_id);
            }
            Instr::Const(cnst) => {
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.const_(cnst.value);
            }
            Instr::TernOp(tern_op) => {
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.tern_op(tern_op.op);
            }
            Instr::Binop(binop) => {
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.binop(binop.op);
            }
            Instr::Select(select) => {
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.select(select.ty);
            }
            Instr::Unreachable(unreachable) => {
                let _ = unreachable;
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.unreachable();
            }
            Instr::Br(br) => {
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                let old_index_of = self
                    .old_sequence_stack
                    .iter()
                    .position(|x| *x == br.block)
                    .unwrap();
                let new_label_id = self.new_sequence_stack.get(old_index_of).unwrap();
                current_sequence.br(*new_label_id);
            }
            Instr::BrIf(br_if) => {
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                let old_index_of = self
                    .old_sequence_stack
                    .iter()
                    .position(|x| *x == br_if.block)
                    .unwrap();
                let new_label_id = self.new_sequence_stack.get(old_index_of).unwrap();
                current_sequence.br_if(*new_label_id);
            }
            Instr::IfElse(if_else) => {
                let IfElse {
                    consequent: old_consequent_id,
                    alternative: old_alternative_id,
                } = if_else;
                let consequent_ty = self.old_function.block(*old_consequent_id).ty;
                let alternative_ty = self.old_function.block(*old_alternative_id).ty;
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                let consequent_builder = current_sequence.dangling_instr_seq(consequent_ty);
                let consequent_builder_id = consequent_builder.id();
                self.new_sequence_stack.push(consequent_builder_id);
                let alternative_builder = current_sequence.dangling_instr_seq(alternative_ty);
                let alternative_builder_id = alternative_builder.id();
                self.new_sequence_stack.push(alternative_builder_id);
                // TODO: what if you enter and enxit a block here? Not sure how the order is defined?
                current_sequence.instr(IfElse {
                    consequent: consequent_builder_id,
                    alternative: alternative_builder_id,
                });
            }
            Instr::BrTable(br_table) => {
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                let new_labels: Vec<_> = br_table
                    .blocks
                    .iter()
                    .map(|block| *self.new_sequence_stack.get(block.index()).unwrap())
                    .collect();
                current_sequence.br_table(new_labels.into(), br_table.default);
            }
            Instr::Drop(drop) => {
                let _ = drop;
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.drop();
            }
            Instr::Return(_) => {
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.return_();
            }
            Instr::MemorySize(memory_size) => {
                let new_memory_id = self.old_to_new_memory_id(memory_size.memory);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.memory_size(new_memory_id);
            }
            Instr::MemoryGrow(memory_grow) => {
                let new_memory_id = self.old_to_new_memory_id(memory_grow.memory);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.memory_grow(new_memory_id);
            }
            Instr::MemoryInit(memory_init) => {
                let new_data_id = self.old_to_new_data_id(memory_init.data);
                let new_memory_id = self.old_to_new_memory_id(memory_init.memory);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.memory_init(new_memory_id, new_data_id);
            }
            Instr::DataDrop(data_drop) => {
                let new_data_id = self.old_to_new_data_id(data_drop.data);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.data_drop(new_data_id);
            }
            Instr::MemoryCopy(memory_copy) => {
                let new_src_memory_id = self.old_to_new_memory_id(memory_copy.src);
                let new_dst_memory_id = self.old_to_new_memory_id(memory_copy.dst);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.memory_copy(new_src_memory_id, new_dst_memory_id);
            }
            Instr::MemoryFill(memory_fill) => {
                let new_memory_id = self.old_to_new_memory_id(memory_fill.memory);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.memory_fill(new_memory_id);
            }
            Instr::Load(load) => {
                let new_memory_id = self.old_to_new_memory_id(load.memory);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.load(new_memory_id, load.kind, load.arg);
            }
            Instr::Store(store) => {
                let new_memory_id = self.old_to_new_memory_id(store.memory);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.store(new_memory_id, store.kind, store.arg);
            }
            Instr::AtomicRmw(atomic_rmw) => {
                let new_memory_id = self.old_to_new_memory_id(atomic_rmw.memory);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.atomic_rmw(
                    new_memory_id,
                    atomic_rmw.op,
                    atomic_rmw.width,
                    atomic_rmw.arg,
                );
            }
            Instr::Cmpxchg(cmpxchg) => {
                let new_memory_id = self.old_to_new_memory_id(cmpxchg.memory);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.cmpxchg(new_memory_id, cmpxchg.width, cmpxchg.arg);
            }
            Instr::AtomicNotify(atomic_notify) => {
                let new_memory_id = self.old_to_new_memory_id(atomic_notify.memory);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.atomic_notify(new_memory_id, atomic_notify.arg);
            }
            Instr::AtomicWait(atomic_wait) => {
                let new_memory_id = self.old_to_new_memory_id(atomic_wait.memory);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.atomic_wait(
                    new_memory_id,
                    atomic_wait.arg,
                    atomic_wait.sixty_four,
                );
            }
            Instr::AtomicFence(atomic_fence) => {
                let _ = atomic_fence;
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.atomic_fence();
            }
            Instr::TableGet(table_get) => {
                let new_table_id = self.old_to_new_table_id(table_get.table);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.table_get(new_table_id);
            }
            Instr::TableSet(table_set) => {
                let new_table_id = self.old_to_new_table_id(table_set.table);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.table_set(new_table_id);
            }

            Instr::TableGrow(table_grow) => {
                let new_table_id = self.old_to_new_table_id(table_grow.table);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.table_grow(new_table_id);
            }
            Instr::TableSize(table_size) => {
                let new_table_id = self.old_to_new_table_id(table_size.table);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.table_size(new_table_id);
            }
            Instr::TableFill(table_fill) => {
                let new_table_id = self.old_to_new_table_id(table_fill.table);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.table_fill(new_table_id);
            }
            Instr::RefNull(ref_null) => {
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.ref_null(ref_null.ty);
            }
            Instr::RefIsNull(ref_is_null) => {
                let _ = ref_is_null;
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.ref_is_null();
            }
            Instr::RefFunc(ref_func) => {
                let new_function_id = self.old_to_new_fn_id(ref_func.func);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.ref_func(new_function_id);
            }

            Instr::V128Bitselect(v128_bitselect) => {
                let _ = v128_bitselect;
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.v128_bitselect();
            }
            Instr::I8x16Swizzle(i8x16_swizzle) => {
                let _ = i8x16_swizzle;
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.i8x16_swizzle();
            }
            Instr::I8x16Shuffle(i8x16_shuffle) => {
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.i8x16_shuffle(i8x16_shuffle.indices);
            }
            Instr::LoadSimd(load_simd) => {
                let new_memory_id = self.old_to_new_memory_id(load_simd.memory);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.load_simd(new_memory_id, load_simd.kind, load_simd.arg);
            }
            Instr::TableInit(table_init) => {
                let new_table_id = self.old_to_new_table_id(table_init.table);
                let new_elem_id = self.old_to_new_elem_id(table_init.elem);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.table_init(new_table_id, new_elem_id);
            }

            Instr::ElemDrop(elem_drop) => {
                let new_elem_id = self.old_to_new_elem_id(elem_drop.elem);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.elem_drop(new_elem_id);
            }
            Instr::TableCopy(table_copy) => {
                let new_src_table_id = self.old_to_new_table_id(table_copy.src);
                let new_dst_table_id = self.old_to_new_table_id(table_copy.dst);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.table_copy(new_src_table_id, new_dst_table_id);
            }
            Instr::ReturnCall(return_call) => {
                let new_function_id = self.old_to_new_fn_id(return_call.func);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                current_sequence.return_call(new_function_id);
            }
            Instr::ReturnCallIndirect(return_call_indirect) => {
                let new_table_id = self.old_to_new_table_id(return_call_indirect.table);
                let current_sequence_id = *self.new_sequence_stack.last().unwrap();
                let mut current_sequence = self.new_function_builder.instr_seq(current_sequence_id);
                let owned_type =
                    FuncType::from_types(return_call_indirect.ty, &self.old_module.types);
                let new_type = self
                    .new_module
                    .types
                    .add(owned_type.params(), owned_type.results());
                current_sequence.return_call_indirect(new_type, new_table_id);
            }
        }
    }
}

impl<'instr, 'builder, 'old_function> Visitor<'instr>
    for WasmFunctionCopy<'builder, 'old_function>
{
    // TODO: implement the copy functionality
    // TODO: there are other 'visit' methods
    fn visit_instr(&mut self, instr: &'instr Instr, instr_loc: &'instr InstrLocId) {
        self.push_instr(instr);
        let _ = instr_loc; // The 'old' location is not relevant after a copy
    }
}
