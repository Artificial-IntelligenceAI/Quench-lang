//! Putting a QIR function together without writing the value numbers out by hand.
//!
//! Every method that produces a value returns it, so a caller never invents a [`Value`]
//! and never has to know that they are indices. The types are tracked as it goes, which
//! is what lets [`crate::verify`] be a check on the frontend rather than on this.
//!
//! Misusing the builder — leaving a block without a terminator, say — panics rather than
//! producing a [`Diagnostic`]. That is deliberate: a malformed IR is a bug in the
//! compiler, not a mistake in someone's program, and the two should never be reported the
//! same way.
//!
//! [`Diagnostic`]: https://docs.rs/quench-diag

use crate::{BinOp, Block, BlockId, CmpOp, FuncId, Function, Host, Inst, Stream, Target, Term, Ty, Value};

/// A block under construction: its parameters, its instructions, and its terminator once
/// it has one.
struct Building {
    params: Vec<Value>,
    insts: Vec<(Value, Inst)>,
    term: Option<Term>,
}

pub struct Builder {
    name: String,
    params: Vec<Ty>,
    ret: Ty,
    blocks: Vec<Building>,
    value_tys: Vec<Ty>,
    current: BlockId,
}

impl Builder {
    /// Start a function. The entry block is created with the function's parameters
    /// already bound, so [`Builder::param`] works immediately.
    pub fn new(name: impl Into<String>, params: &[Ty], ret: Ty) -> Self {
        let mut value_tys = Vec::with_capacity(params.len());
        let mut bound = Vec::with_capacity(params.len());
        for ty in params {
            bound.push(Value(value_tys.len() as u32));
            value_tys.push(*ty);
        }
        Self {
            name: name.into(),
            params: params.to_vec(),
            ret,
            blocks: vec![Building { params: bound, insts: Vec::new(), term: None }],
            value_tys,
            current: BlockId(0),
        }
    }

    pub fn entry(&self) -> BlockId {
        BlockId(0)
    }

    /// The nth parameter of the function.
    pub fn param(&self, n: usize) -> Value {
        self.blocks[0].params[n]
    }

    /// A new block, with parameters of the given types.
    pub fn block(&mut self, params: &[Ty]) -> BlockId {
        let bound = params
            .iter()
            .map(|ty| {
                let v = Value(self.value_tys.len() as u32);
                self.value_tys.push(*ty);
                v
            })
            .collect();
        self.blocks.push(Building { params: bound, insts: Vec::new(), term: None });
        BlockId(self.blocks.len() as u32 - 1)
    }

    /// The nth parameter of a block.
    pub fn block_param(&self, block: BlockId, n: usize) -> Value {
        self.blocks[block.0 as usize].params[n]
    }

    /// Write the following instructions into `block`.
    pub fn switch_to(&mut self, block: BlockId) {
        self.current = block;
    }

    fn push(&mut self, inst: Inst, ty: Ty) -> Value {
        let v = Value(self.value_tys.len() as u32);
        self.value_tys.push(ty);
        self.blocks[self.current.0 as usize].insts.push((v, inst));
        v
    }

    fn terminate(&mut self, term: Term) {
        let block = &mut self.blocks[self.current.0 as usize];
        assert!(block.term.is_none(), "block {} already ends in {:?}", self.current.0, block.term);
        block.term = Some(term);
    }

    pub fn const_i64(&mut self, n: i64) -> Value {
        self.push(Inst::ConstI64(n), Ty::I64)
    }

    pub fn const_bool(&mut self, b: bool) -> Value {
        self.push(Inst::ConstBool(b), Ty::Bool)
    }

    /// A piece of text, by the index [`crate::Module::intern`] gave it.
    pub fn const_text(&mut self, at: u32) -> Value {
        self.push(Inst::ConstText(at), Ty::Text)
    }

    /// Write something, somewhere.
    pub fn print(&mut self, host: Host, to: Stream, what: Value) -> Value {
        let stream = self.const_i64(to as i64);
        self.call_host(host, &[stream, what])
    }

    /// Ask the runtime for something.
    pub fn call_host(&mut self, host: Host, args: &[Value]) -> Value {
        self.push(Inst::CallHost { host, args: args.to_vec() }, host.result())
    }

    pub fn bin(&mut self, op: BinOp, lhs: Value, rhs: Value) -> Value {
        self.push(Inst::Bin { op, lhs, rhs }, Ty::I64)
    }

    pub fn add(&mut self, lhs: Value, rhs: Value) -> Value {
        self.bin(BinOp::Add, lhs, rhs)
    }

    pub fn add_trapping(&mut self, lhs: Value, rhs: Value) -> Value {
        self.bin(BinOp::AddTrapping, lhs, rhs)
    }

    pub fn sub_trapping(&mut self, lhs: Value, rhs: Value) -> Value {
        self.bin(BinOp::SubTrapping, lhs, rhs)
    }

    pub fn mul_trapping(&mut self, lhs: Value, rhs: Value) -> Value {
        self.bin(BinOp::MulTrapping, lhs, rhs)
    }

    pub fn sub(&mut self, lhs: Value, rhs: Value) -> Value {
        self.bin(BinOp::Sub, lhs, rhs)
    }

    pub fn mul(&mut self, lhs: Value, rhs: Value) -> Value {
        self.bin(BinOp::Mul, lhs, rhs)
    }

    pub fn div(&mut self, lhs: Value, rhs: Value) -> Value {
        self.bin(BinOp::DivTruncated, lhs, rhs)
    }

    pub fn rem(&mut self, lhs: Value, rhs: Value) -> Value {
        self.bin(BinOp::RemTruncated, lhs, rhs)
    }

    pub fn div_floored(&mut self, lhs: Value, rhs: Value) -> Value {
        self.bin(BinOp::DivFloored, lhs, rhs)
    }

    pub fn rem_floored(&mut self, lhs: Value, rhs: Value) -> Value {
        self.bin(BinOp::RemFloored, lhs, rhs)
    }

    pub fn cmp(&mut self, op: CmpOp, lhs: Value, rhs: Value) -> Value {
        self.push(Inst::Cmp { op, lhs, rhs }, Ty::Bool)
    }

    pub fn not(&mut self, v: Value) -> Value {
        self.push(Inst::Not(v), Ty::Bool)
    }

    /// Call another function. `ret` is the callee's return type; [`crate::verify`] checks
    /// it against the callee rather than trusting it.
    pub fn call(&mut self, func: FuncId, args: &[Value], ret: Ty) -> Value {
        self.push(Inst::Call { func, args: args.to_vec() }, ret)
    }

    pub fn ret(&mut self, v: Value) {
        self.terminate(Term::Ret(v));
    }

    pub fn jump(&mut self, to: BlockId, args: &[Value]) {
        self.terminate(Term::Jump { to, args: args.to_vec() });
    }

    pub fn br_if(
        &mut self,
        cond: Value,
        then: (BlockId, &[Value]),
        otherwise: (BlockId, &[Value]),
    ) {
        self.terminate(Term::BrIf {
            cond,
            then: Target::new(then.0, then.1.to_vec()),
            otherwise: Target::new(otherwise.0, otherwise.1.to_vec()),
        });
    }

    /// Finish. Panics if any block was left without a terminator, naming which.
    pub fn finish(self) -> Function {
        let blocks = self
            .blocks
            .into_iter()
            .enumerate()
            .map(|(i, b)| {
                let term = b.term.unwrap_or_else(|| {
                    panic!("block {i} of `{}` was left without a terminator", self.name)
                });
                Block { params: b.params, insts: b.insts, term }
            })
            .collect();
        Function {
            name: self.name,
            params: self.params,
            ret: self.ret,
            blocks,
            value_tys: self.value_tys,
        }
    }
}
