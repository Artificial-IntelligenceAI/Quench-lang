//! The meaning of a program, turned into the IR every engine runs.
//!
//! There are no decisions here. [`quench_check`] resolved every name, settled every
//! type, joined every piece of text and refused everything that did not make sense — so
//! what is left is a transliteration, and that is the point of doing the checking first.
//! Anything in this file that started to look like a judgement would belong further up.

use quench_check::{Arm, Checked, OpKind, Place, Printed, Stmt, Ty, Value};
use quench_conf::{Division, Overflow, Settings};
use quench_diag::{Diagnostic, Span};
use quench_qir as qir;

/// What a file became, and everything wrong with it.
pub struct Lowered {
    /// Absent when the file could not be turned into a program at all.
    pub module: Option<qir::Module>,
    pub errors: Vec<Diagnostic>,
}

impl Lowered {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Read a whole file and turn it into something that can run, under the project's
/// default settings.
pub fn lower(source: &str) -> Lowered {
    lower_under(source, Settings::default())
}

/// The same, under settings of your choosing.
///
/// Which matters for exactly one thing so far, and it is the thing the two-piles note
/// warned about: `[defaults] division` decides whether `/` and `mod` round toward zero
/// or toward negative infinity, so the same source is two different programs under the
/// two settings. The choice is made here and written into the IR as an instruction, so
/// nothing below this has to know a setting existed.
pub fn lower_under(source: &str, settings: Settings) -> Lowered {
    let mut checked = quench_check::check(source);

    if !checked.has_start {
        // Not something the parser could report: a file of declarations is a fine thing
        // to parse and a useless thing to run, and only something trying to run it knows
        // which was wanted.
        checked.errors.push(
            Diagnostic::new("E0301", "this file has no `START`, so there is nothing to run.")
                .primary(Span::at(source.len()), "the file ends here")
                .rule("a program begins at `START`, and a file without one is not a program")
                .tip("a file may hold declarations and no `START`. It just cannot be run.")
                .fix("add `START { … }`"),
        );
        return Lowered { module: None, errors: checked.errors };
    }
    if !checked.errors.is_empty() {
        // Lowering a program that did not check out would build nonsense out of it.
        return Lowered { module: None, errors: checked.errors };
    }

    Lowered { module: Some(build(&checked, settings)), errors: checked.errors }
}

fn build(checked: &Checked, settings: Settings) -> qir::Module {
    let mut module = qir::Module::new();
    let mut b = qir::Builder::new(qir::ENTRY, &[], qir::Ty::I64);

    // Where each variable's value ended up. A declaration fills one in; a use reads it;
    // a join replaces the ones that could have come from either side.
    let mut held: Vec<Option<qir::Value>> = vec![None; checked.locals.len()];

    lower_body(&mut b, &mut module, &checked.body, &mut held, checked, settings);

    // A program that says nothing about how it ended, ended fine.
    let nothing = b.const_i64(0);
    b.ret(nothing);

    let id = module.add(b.finish());
    module.set_entry(id);
    module
}

/// What a QIR value of this type looks like.
fn qir_ty(ty: &Ty) -> qir::Ty {
    match ty {
        Ty::I64 => qir::Ty::I64,
        Ty::Bool => qir::Ty::Bool,
        Ty::Str => qir::Ty::Text,
        Ty::Arr { .. } => qir::Ty::Handle,
    }
}

fn lower_body(
    b: &mut qir::Builder,
    module: &mut qir::Module,
    body: &[Stmt],
    held: &mut Vec<Option<qir::Value>>,
    checked: &Checked,
    settings: Settings,
) {
    for stmt in body {
        match stmt {
            Stmt::Declare { local, value } => {
                let value = emit(b, module, value, held, settings);
                held[local.0 as usize] = Some(value);
            }
            // Changing a variable is naming a new value for it. Inside an arm that is
            // still just a write here -- what makes it correct is that the join below
            // takes whichever value the branch that ran left behind.
            Stmt::Assign { to, value } => {
                let value = emit(b, module, value, held, settings);
                match to {
                    Place::Local(local) => held[local.0 as usize] = Some(value),
                    Place::Element { local, indices, shape } => {
                        let handle = held[local.0 as usize].expect("declared before used");
                        let at = flat_index(b, module, indices, shape, held, settings);
                        b.call_host(qir::Host::ArraySet, &[handle, at, value]);
                    }
                }
            }
            Stmt::If { arms, otherwise, live } => {
                lower_if(b, module, arms, otherwise.as_deref(), *live, held, checked, settings);
            }
            Stmt::Print(pieces) => {
                for piece in pieces {
                    match piece {
                        Printed::Text(text) => {
                            let at = module.intern(text);
                            let value = b.const_text(at);
                            b.call_host(qir::Host::PrintText, &[value]);
                        }
                        Printed::Value { value, ty } => {
                            let value = emit(b, module, value, held, settings);
                            let host = match ty {
                                Ty::Str => qir::Host::PrintText,
                                Ty::I64 => qir::Host::PrintI64,
                                Ty::Bool => qir::Host::PrintBool,
                                Ty::Arr { .. } => {
                                    unreachable!("refused by the checker: an array is not printed")
                                }
                            };
                            b.call_host(host, &[value]);
                        }
                    }
                }
            }
        }
    }
}

/// `if` — arms asked in order, and the values they leave behind carried to one place.
///
/// This is where the lowering stops being a transliteration. Up to here a variable was
/// one QIR value and `set` overwrote which; a variable changed inside an arm is a
/// *different* value depending on which arm ran, and that is exactly what a block
/// parameter is for. So the join takes one parameter per variable that existed before
/// the `if`, and every path hands it whatever it ended up with.
///
/// Anything declared *inside* an arm is not carried. It is gone at the closing brace,
/// which the checker already enforced by scope and which `live` records.
fn lower_if(
    b: &mut qir::Builder,
    module: &mut qir::Module,
    arms: &[Arm],
    otherwise: Option<&[Stmt]>,
    live: u32,
    held: &mut Vec<Option<qir::Value>>,
    checked: &Checked,
    settings: Settings,
) {
    let carried: Vec<usize> = (0..live as usize).filter(|i| held[*i].is_some()).collect();
    let types: Vec<qir::Ty> =
        carried.iter().map(|i| qir_ty(&checked.locals[*i].ty)).collect();
    let join = b.block(&types);

    // What each path hands the join: everything it is holding by then.
    let handed = |held: &[Option<qir::Value>]| -> Vec<qir::Value> {
        carried.iter().map(|i| held[*i].expect("checked above")).collect()
    };

    let before = held.clone();
    for arm in arms {
        let condition = emit(b, module, &arm.condition, held, settings);
        let taken = b.block(&[]);
        let next = b.block(&[]);
        b.br_if(condition, (taken, &[]), (next, &[]));

        b.switch_to(taken);
        // Each arm starts from what was true before the `if`, not from what the arm
        // before it did -- only one of them ever runs.
        *held = before.clone();
        lower_body(b, module, &arm.body, held, checked, settings);
        let leaving = handed(held);
        b.jump(join, &leaving);

        b.switch_to(next);
    }

    // Nothing held. Whatever the `else` says, or nothing at all.
    *held = before.clone();
    if let Some(body) = otherwise {
        lower_body(b, module, body, held, checked, settings);
    }
    let leaving = handed(held);
    b.jump(join, &leaving);

    b.switch_to(join);
    for (n, i) in carried.iter().enumerate() {
        held[*i] = Some(b.block_param(join, n));
    }
}

/// Where element (i, j, …) sits in a block laid out row by row.
///
/// Counting from one, so each index is shifted down before it is scaled. This is the
/// whole of what one `arr` link costs to index: no handle is followed on the way.
fn flat_index(
    b: &mut qir::Builder,
    module: &mut qir::Module,
    indices: &[Value],
    shape: &[usize],
    held: &[Option<qir::Value>],
    settings: Settings,
) -> qir::Value {
    let mut flat = None;
    for (n, index) in indices.iter().enumerate() {
        let this = emit(b, module, index, held, settings);
        flat = Some(match flat {
            None => this,
            Some(so_far) => {
                let one = b.const_i64(1);
                let zeroed = b.sub(so_far, one);
                let stride = b.const_i64(shape[n] as i64);
                let scaled = b.mul(zeroed, stride);
                b.add(scaled, this)
            }
        });
    }
    flat.expect("the checker refused an index with no numbers in it")
}

/// One value, put into the IR.
fn emit(
    b: &mut qir::Builder,
    module: &mut qir::Module,
    value: &Value,
    held: &[Option<qir::Value>],
    settings: Settings,
) -> qir::Value {
    match value {
        Value::Text(text) => {
            let at = module.intern(text);
            b.const_text(at)
        }
        Value::Number(n) => b.const_i64(*n),
        Value::Bool(yes) => b.const_bool(*yes),
        // Values do not change, so copying one is naming the same value again rather
        // than doing anything.
        Value::Copy(from) => held[from.0 as usize].expect("declared before used"),
        // The array is made, then filled one element at a time. Both are host calls:
        // asking for memory is a runtime service, and this is the first time Quench
        // asks. Nothing frees it yet, which is the first stage of the collector.
        Value::Array(elements) => {
            let len = b.const_i64(elements.len() as i64);
            let handle = b.call_host(qir::Host::ArrayNew, &[len]);
            for (n, element) in elements.iter().enumerate() {
                let at = b.const_i64(n as i64 + 1); // counted from one
                let value = emit(b, module, element, held, settings);
                b.call_host(qir::Host::ArraySet, &[handle, at, value]);
            }
            handle
        }
        // Row by row: element (i, j) of a (2 3) is at (i - 1) x 3 + j, counting from one.
        // Which is why one `arr` link is one allocation -- the whole address is
        // arithmetic, and no handle is followed on the way.
        Value::At { array, indices, shape } => {
            let handle = emit(b, module, array, held, settings);
            let at = flat_index(b, module, indices, shape, held, settings);
            b.call_host(qir::Host::ArrayGet, &[handle, at])
        }
        Value::Binary { op, lhs, rhs } => {
            let l = emit(b, module, lhs, held, settings);
            let r = emit(b, module, rhs, held, settings);
            let floored = settings.division == Division::Floored;
            match op {
                // Whether a sum that does not fit rounds or stops is the project's
                // decision, written down here as an instruction.
                OpKind::Add if settings.overflow == Overflow::Trap => b.add_trapping(l, r),
                OpKind::Sub if settings.overflow == Overflow::Trap => b.sub_trapping(l, r),
                OpKind::Mul if settings.overflow == Overflow::Trap => b.mul_trapping(l, r),
                OpKind::Add => b.add(l, r),
                OpKind::Sub => b.sub(l, r),
                OpKind::Mul => b.mul(l, r),
                // The project's decision, written down here as an instruction so that no
                // backend has to know a setting was ever involved.
                OpKind::Div => {
                    if floored { b.div_floored(l, r) } else { b.div(l, r) }
                }
                OpKind::Mod => {
                    if floored { b.rem_floored(l, r) } else { b.rem(l, r) }
                }
                OpKind::Lt => b.cmp(qir::CmpOp::Lt, l, r),
                OpKind::Gt => b.cmp(qir::CmpOp::Gt, l, r),
                OpKind::Le => b.cmp(qir::CmpOp::Le, l, r),
                OpKind::Ge => b.cmp(qir::CmpOp::Ge, l, r),
                OpKind::Eq => b.cmp(qir::CmpOp::Eq, l, r),
                OpKind::Ne => b.cmp(qir::CmpOp::Ne, l, r),
                OpKind::Pow | OpKind::And | OpKind::Or => {
                    unreachable!("refused by the checker as not built yet")
                }
            }
        }
    }
}
