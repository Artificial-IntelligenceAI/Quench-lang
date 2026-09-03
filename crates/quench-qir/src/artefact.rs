//! QIR, written down and read back — the thing a program is once it stops being source.
//!
//! This is the artefact `notes/compile-once-run-anywhere.md` is about. Not a second
//! format invented for the purpose: the same QIR every engine already runs, put in a
//! file so it can be carried to a machine this compiler never saw.
//!
//! # Little-endian everywhere
//!
//! Every number in a file is little-endian, whatever machine wrote it and whatever
//! machine reads it. The encoding is one thing rather than a property of who was
//! holding the pen. Nothing in QIR is target-specific — no pointer width, no ABI, no
//! host-sized integer — so a file is what it is wherever it lands.
//!
//! # A file is input
//!
//! The moment an artefact travels it stops being something this compiler produced.
//! Everything here is total: no reader panics, no reader indexes without looking, and
//! an enum code nobody has heard of is an error rather than a guess. What comes out is
//! an ordinary diagnostic, because a reader should not have to learn a second kind of
//! message because the trouble is in a file rather than a line.

use crate::{
    verify, Audience, BinOp, Block, BlockId, CmpOp, FuncId, Function, Host, Inst, Module, Target,
    Term, Ty, Value,
};
use quench_diag::Diagnostic;

/// What every Quench artefact starts with, so that a file which is not one says so
/// rather than being read as a very strange program.
const MAGIC: [u8; 4] = *b"QNL\x00";

/// The shape of the file. A reader refuses a version it does not know, because reading
/// a later one halfway is how a file turns into a plausible program nobody wrote.
const VERSION: u32 = 1;

/// Which section of a file a chunk is.
const TEXT: u32 = 1;
const TABLES: u32 = 2;
const FUNCTIONS: u32 = 3;
const ENTRY: u32 = 4;

/// A sum of a chunk's bytes, for accidents.
///
/// FNV-1a, which is five lines and needs no dependency. It is here for a copy that
/// stopped early, a transfer that went wrong, a disk going bad — and it says so. It is
/// **not** a defence: anybody editing a chunk on purpose recomputes this in a line.
fn sum(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

// --- writing ---------------------------------------------------------------------

/// Everything a module is, as bytes.
pub fn write(module: &Module) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());

    let mut text = Vec::new();
    put_many(&mut text, &module.text, |to, s| put_text(to, s));
    chunk(&mut out, TEXT, &text);

    let mut tables = Vec::new();
    put_many(&mut tables, &module.tables, |to, values| {
        put_many(to, values, |to, n| to.extend_from_slice(&n.to_le_bytes()))
    });
    chunk(&mut out, TABLES, &tables);

    let mut functions = Vec::new();
    put_many(&mut functions, &module.functions, put_function);
    chunk(&mut out, FUNCTIONS, &functions);

    let mut entry = Vec::new();
    match module.entry {
        Some(id) => {
            entry.push(1);
            entry.extend_from_slice(&id.0.to_le_bytes());
        }
        None => entry.push(0),
    }
    chunk(&mut out, ENTRY, &entry);
    out
}

fn chunk(out: &mut Vec<u8>, kind: u32, body: &[u8]) {
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&(body.len() as u64).to_le_bytes());
    out.extend_from_slice(&sum(body).to_le_bytes());
    out.extend_from_slice(body);
}

fn put_many<T>(out: &mut Vec<u8>, items: &[T], mut each: impl FnMut(&mut Vec<u8>, &T)) {
    out.extend_from_slice(&(items.len() as u64).to_le_bytes());
    for item in items {
        each(out, item);
    }
}

fn put_text(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u64).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn put_u32(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_le_bytes());
}

fn put_value(out: &mut Vec<u8>, v: &Value) {
    put_u32(out, v.0);
}

fn put_ty(out: &mut Vec<u8>, ty: &Ty) {
    out.push(match ty {
        Ty::I64 => 0,
        Ty::Bool => 1,
        Ty::Handle => 2,
        Ty::F64 => 3,
        Ty::F32 => 6,
        Ty::F16 => 7,
        Ty::Exact => 4,
        Ty::Text => 5,
    });
}

fn put_function(out: &mut Vec<u8>, func: &Function) {
    put_text(out, &func.name);
    put_many(out, &func.params, put_ty);
    put_ty(out, &func.ret);
    put_many(out, &func.value_tys, put_ty);
    put_many(out, &func.blocks, put_block);
}

fn put_block(out: &mut Vec<u8>, block: &Block) {
    put_many(out, &block.params, put_value);
    put_many(out, &block.insts, |to, (result, inst)| {
        put_value(to, result);
        put_inst(to, inst);
    });
    put_term(out, &block.term);
}

fn put_inst(out: &mut Vec<u8>, inst: &Inst) {
    match inst {
        Inst::ConstI64(n) => {
            out.push(0);
            out.extend_from_slice(&n.to_le_bytes());
        }
        Inst::ConstBool(yes) => {
            out.push(1);
            out.push(u8::from(*yes));
        }
        Inst::ConstHandle(at) => {
            out.push(2);
            put_u32(out, *at);
        }
        Inst::ConstFloat(bits) => {
            out.push(3);
            out.extend_from_slice(&bits.to_le_bytes());
        }
        Inst::ConstText(at) => {
            out.push(4);
            put_u32(out, *at);
        }
        Inst::Bin { op, lhs, rhs } => {
            out.push(5);
            out.push(bin_code(*op));
            put_value(out, lhs);
            put_value(out, rhs);
        }
        Inst::Cmp { op, lhs, rhs } => {
            out.push(6);
            out.push(cmp_code(*op));
            put_value(out, lhs);
            put_value(out, rhs);
        }
        Inst::FCmp { op, lhs, rhs } => {
            out.push(7);
            out.push(cmp_code(*op));
            put_value(out, lhs);
            put_value(out, rhs);
        }
        Inst::Not(v) => {
            out.push(8);
            put_value(out, v);
        }
        Inst::Call { func, args } => {
            out.push(9);
            put_u32(out, func.0);
            put_many(out, args, put_value);
        }
        Inst::CallHost { host, args } => {
            out.push(10);
            out.push(host_code(*host));
            put_many(out, args, put_value);
        }
    }
}

fn put_term(out: &mut Vec<u8>, term: &Term) {
    match term {
        Term::Ret(v) => {
            out.push(0);
            put_value(out, v);
        }
        Term::Jump { to, args } => {
            out.push(1);
            put_u32(out, to.0);
            put_many(out, args, put_value);
        }
        Term::BrIf { cond, then, otherwise } => {
            out.push(2);
            put_value(out, cond);
            for target in [then, otherwise] {
                put_u32(out, target.block.0);
                put_many(out, &target.args, put_value);
            }
        }
    }
}

/// The codes for the enums. Written out one by one rather than derived from the
/// declaration order, because a file outlives the source it was written by: reordering
/// a variant must not silently change what an old file means.
fn bin_code(op: BinOp) -> u8 {
    match op {
        BinOp::Add => 0,
        BinOp::Sub => 1,
        BinOp::Mul => 2,
        BinOp::AddTrapping => 3,
        BinOp::SubTrapping => 4,
        BinOp::MulTrapping => 5,
        BinOp::DivTruncated => 6,
        BinOp::RemTruncated => 7,
        BinOp::DivFloored => 8,
        BinOp::RemFloored => 9,
        BinOp::FAdd => 10,
        BinOp::FSub => 11,
        BinOp::FMul => 12,
        BinOp::FDiv => 13,
        BinOp::FAddChecked => 14,
        BinOp::FSubChecked => 15,
        BinOp::FMulChecked => 16,
        BinOp::FDivChecked => 17,
        BinOp::And => 18,
        BinOp::Or => 19,
    }
}

fn cmp_code(op: CmpOp) -> u8 {
    match op {
        CmpOp::Eq => 0,
        CmpOp::Ne => 1,
        CmpOp::Lt => 2,
        CmpOp::Le => 3,
        CmpOp::Gt => 4,
        CmpOp::Ge => 5,
    }
}

fn host_code(host: Host) -> u8 {
    match host {
        Host::PrintText => 0,
        Host::PrintI64 => 1,
        Host::PrintBool => 2,
        Host::ArrayNew => 3,
        Host::ArraySet => 4,
        Host::ArrayGet => 5,
        Host::ArrayLen => 6,
        Host::ArrayPush => 7,
        Host::ArrayCopy => 8,
        Host::ArrayEqual => 9,
        Host::PrintArray => 10,
        Host::ExactRead => 11,
        Host::ExactAdd => 12,
        Host::ExactSub => 13,
        Host::ExactMul => 14,
        Host::ExactDiv => 15,
        Host::ExactCompare => 16,
        Host::TextJoin => 17,
        Host::TextCompare => 18,
        Host::PrintExact => 19,
        Host::PrintFloat => 20,
        Host::ToB16 => 24,
        Host::ExactPow => 21,
        Host::PowI64 => 22,
        Host::PowI64Trapping => 23,
    }
}

// --- reading ---------------------------------------------------------------------

/// What went wrong with a file, in the terms somebody holding one can act on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Wrong {
    /// It does not begin the way a Quench artefact begins.
    NotAnArtefact,
    /// A version this reader does not know. Reading one halfway is how a file turns
    /// into a plausible program nobody wrote.
    Version(u32),
    /// It stopped before it should have.
    Short,
    /// A chunk's bytes do not add up to what the chunk said they would.
    Damaged(u32),
    /// A section this reader does not know, or one said twice.
    Section(u32),
    /// A code no version of Quench ever wrote, or one a later version did.
    Unknown { what: &'static str, code: u8 },
    /// A section that has to be there and was not.
    Missing(&'static str),
}

impl Wrong {
    fn says(&self) -> String {
        match self {
            Wrong::NotAnArtefact => "this does not begin the way a Quench artefact does.".into(),
            Wrong::Version(n) => format!("this artefact says it is version {n}, and this Quench reads version {VERSION}."),
            Wrong::Short => "this artefact stops before it should.".into(),
            Wrong::Damaged(kind) => format!("section {kind} does not add up to what it said it would."),
            Wrong::Section(kind) => format!("section {kind} is not one this reads, or is here twice."),
            Wrong::Unknown { what, code } => format!("`{code}` is not {what} this knows."),
            Wrong::Missing(what) => format!("this artefact has no {what}."),
        }
    }

    fn fixes(&self) -> &'static str {
        match self {
            Wrong::Version(_) => "build it again with this Quench, or read it with the one that wrote it",
            _ => "build it again from source, or check the file was copied whole",
        }
    }
}

/// A module read back, or an ordinary diagnostic saying why not.
///
/// Everything a file says is checked before it is believed, and `verify` runs on what
/// comes out — as [`Audience::AFileWeWereGiven`], because a module that arrived and a
/// module this compiler built are wrong in different ways and deserve different words.
pub fn read(bytes: &[u8], origin: &str) -> Result<Module, Diagnostic> {
    match read_it(bytes) {
        Ok(module) => match verify(&module) {
            Ok(()) => Ok(module),
            Err(wrong) => Err(crate::diagnose(&wrong, Audience::AFileWeWereGiven, origin)),
        },
        Err(wrong) => Err(Diagnostic::new("E0801", wrong.says())
            .rule("an artefact is read as something that arrived rather than as something this compiler made, so nothing in it is believed before it is checked")
            .tip("a copy that stopped early and a module built by another version of Quench both look like this.")
            .fix(wrong.fixes())),
    }
}

fn read_it(bytes: &[u8]) -> Result<Module, Wrong> {
    let mut at = Reader { bytes, at: 0 };
    if at.take(4)? != MAGIC {
        return Err(Wrong::NotAnArtefact);
    }
    let version = at.u32()?;
    if version != VERSION {
        return Err(Wrong::Version(version));
    }

    let (mut text, mut tables, mut functions, mut entry) = (None, None, None, None);
    while !at.done() {
        let kind = at.u32()?;
        let length = at.u64()? as usize;
        let said = at.u64()?;
        let body = at.take(length)?;
        if sum(body) != said {
            return Err(Wrong::Damaged(kind));
        }
        let mut body = Reader { bytes: body, at: 0 };
        match kind {
            TEXT if text.is_none() => text = Some(body.many(|r| r.text())?),
            TABLES if tables.is_none() => {
                tables = Some(body.many(|r| r.many(|r| r.i64()))?)
            }
            FUNCTIONS if functions.is_none() => functions = Some(body.many(|r| r.function())?),
            ENTRY if entry.is_none() => {
                entry = Some(match body.byte()? {
                    0 => None,
                    1 => Some(FuncId(body.u32()?)),
                    code => return Err(Wrong::Unknown { what: "an entry", code }),
                })
            }
            _ => return Err(Wrong::Section(kind)),
        }
    }

    Ok(Module {
        functions: functions.ok_or(Wrong::Missing("functions"))?,
        entry: entry.ok_or(Wrong::Missing("entry"))?,
        text: text.ok_or(Wrong::Missing("text"))?,
        tables: tables.ok_or(Wrong::Missing("tables"))?,
    })
}

/// A cursor over bytes nobody here wrote.
///
/// Every method is total: it either hands back something whole or says the file ran
/// out. Nothing indexes without looking first, which is the difference between reading
/// a damaged file and being read *by* one.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn done(&self) -> bool {
        self.at >= self.bytes.len()
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Wrong> {
        let end = self.at.checked_add(n).ok_or(Wrong::Short)?;
        let slice = self.bytes.get(self.at..end).ok_or(Wrong::Short)?;
        self.at = end;
        Ok(slice)
    }

    fn byte(&mut self) -> Result<u8, Wrong> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, Wrong> {
        let bytes: [u8; 4] = self.take(4)?.try_into().map_err(|_| Wrong::Short)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, Wrong> {
        let bytes: [u8; 8] = self.take(8)?.try_into().map_err(|_| Wrong::Short)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn i64(&mut self) -> Result<i64, Wrong> {
        Ok(self.u64()? as i64)
    }

    fn text(&mut self) -> Result<String, Wrong> {
        let length = self.u64()? as usize;
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| Wrong::Short)
    }

    /// A counted run of things.
    ///
    /// The count is checked against what is left before anything is reserved for it, so
    /// a file claiming four billion of something is refused rather than asking for the
    /// memory to hold them.
    fn many<T>(&mut self, mut each: impl FnMut(&mut Self) -> Result<T, Wrong>) -> Result<Vec<T>, Wrong> {
        let count = self.u64()? as usize;
        if count > self.bytes.len().saturating_sub(self.at) {
            return Err(Wrong::Short);
        }
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            out.push(each(self)?);
        }
        Ok(out)
    }

    fn value(&mut self) -> Result<Value, Wrong> {
        Ok(Value(self.u32()?))
    }

    fn ty(&mut self) -> Result<Ty, Wrong> {
        Ok(match self.byte()? {
            0 => Ty::I64,
            1 => Ty::Bool,
            2 => Ty::Handle,
            3 => Ty::F64,
            6 => Ty::F32,
            7 => Ty::F16,
            4 => Ty::Exact,
            5 => Ty::Text,
            code => return Err(Wrong::Unknown { what: "a type", code }),
        })
    }

    fn function(&mut self) -> Result<Function, Wrong> {
        Ok(Function {
            name: self.text()?,
            params: self.many(|r| r.ty())?,
            ret: self.ty()?,
            value_tys: self.many(|r| r.ty())?,
            blocks: self.many(|r| r.block())?,
        })
    }

    fn block(&mut self) -> Result<Block, Wrong> {
        Ok(Block {
            params: self.many(|r| r.value())?,
            insts: self.many(|r| Ok((r.value()?, r.inst()?)))?,
            term: self.term()?,
        })
    }

    fn inst(&mut self) -> Result<Inst, Wrong> {
        Ok(match self.byte()? {
            0 => Inst::ConstI64(self.i64()?),
            1 => Inst::ConstBool(match self.byte()? {
                0 => false,
                1 => true,
                code => return Err(Wrong::Unknown { what: "a bool", code }),
            }),
            2 => Inst::ConstHandle(self.u32()?),
            3 => Inst::ConstFloat(self.u64()?),
            4 => Inst::ConstText(self.u32()?),
            5 => Inst::Bin { op: self.bin()?, lhs: self.value()?, rhs: self.value()? },
            6 => Inst::Cmp { op: self.cmp()?, lhs: self.value()?, rhs: self.value()? },
            7 => Inst::FCmp { op: self.cmp()?, lhs: self.value()?, rhs: self.value()? },
            8 => Inst::Not(self.value()?),
            9 => Inst::Call { func: FuncId(self.u32()?), args: self.many(|r| r.value())? },
            10 => Inst::CallHost { host: self.host()?, args: self.many(|r| r.value())? },
            code => return Err(Wrong::Unknown { what: "an instruction", code }),
        })
    }

    fn term(&mut self) -> Result<Term, Wrong> {
        Ok(match self.byte()? {
            0 => Term::Ret(self.value()?),
            1 => Term::Jump { to: BlockId(self.u32()?), args: self.many(|r| r.value())? },
            2 => Term::BrIf {
                cond: self.value()?,
                then: self.target()?,
                otherwise: self.target()?,
            },
            code => return Err(Wrong::Unknown { what: "an ending", code }),
        })
    }

    fn target(&mut self) -> Result<Target, Wrong> {
        Ok(Target { block: BlockId(self.u32()?), args: self.many(|r| r.value())? })
    }

    fn bin(&mut self) -> Result<BinOp, Wrong> {
        Ok(match self.byte()? {
            0 => BinOp::Add,
            1 => BinOp::Sub,
            2 => BinOp::Mul,
            3 => BinOp::AddTrapping,
            4 => BinOp::SubTrapping,
            5 => BinOp::MulTrapping,
            6 => BinOp::DivTruncated,
            7 => BinOp::RemTruncated,
            8 => BinOp::DivFloored,
            9 => BinOp::RemFloored,
            10 => BinOp::FAdd,
            11 => BinOp::FSub,
            12 => BinOp::FMul,
            13 => BinOp::FDiv,
            14 => BinOp::FAddChecked,
            15 => BinOp::FSubChecked,
            16 => BinOp::FMulChecked,
            17 => BinOp::FDivChecked,
            18 => BinOp::And,
            19 => BinOp::Or,
            code => return Err(Wrong::Unknown { what: "an operation", code }),
        })
    }

    fn cmp(&mut self) -> Result<CmpOp, Wrong> {
        Ok(match self.byte()? {
            0 => CmpOp::Eq,
            1 => CmpOp::Ne,
            2 => CmpOp::Lt,
            3 => CmpOp::Le,
            4 => CmpOp::Gt,
            5 => CmpOp::Ge,
            code => return Err(Wrong::Unknown { what: "a comparison", code }),
        })
    }

    fn host(&mut self) -> Result<Host, Wrong> {
        Ok(match self.byte()? {
            0 => Host::PrintText,
            1 => Host::PrintI64,
            2 => Host::PrintBool,
            3 => Host::ArrayNew,
            4 => Host::ArraySet,
            5 => Host::ArrayGet,
            6 => Host::ArrayLen,
            7 => Host::ArrayPush,
            8 => Host::ArrayCopy,
            9 => Host::ArrayEqual,
            10 => Host::PrintArray,
            11 => Host::ExactRead,
            12 => Host::ExactAdd,
            13 => Host::ExactSub,
            14 => Host::ExactMul,
            15 => Host::ExactDiv,
            16 => Host::ExactCompare,
            17 => Host::TextJoin,
            18 => Host::TextCompare,
            19 => Host::PrintExact,
            20 => Host::PrintFloat,
            24 => Host::ToB16,
            21 => Host::ExactPow,
            22 => Host::PowI64,
            23 => Host::PowI64Trapping,
            code => return Err(Wrong::Unknown { what: "a runtime call", code }),
        })
    }
}
