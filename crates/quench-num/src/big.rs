//! Whole numbers with no width.
//!
//! A sign and a magnitude in little-endian 64-bit limbs, with no leading zero limb, and
//! zero always positive. Those two invariants are what make comparison and equality
//! cheap and unambiguous: there is one representation of every number, so two of these
//! are equal exactly when they are the same number.
//!
//! # Which algorithms, and why in this order
//!
//! Luarust wrote this once already, for numbers that stay small, and its choices show
//! it: schoolbook multiplication, a division that shifts and subtracts one bit at a
//! time, and a Euclid gcd built on that division. All correct, and the wrong shape for
//! a type whose purpose is numbers that are absurdly large.
//!
//! The order they were fixed in here is deliberate and is not the obvious one:
//!
//! 1. **gcd is binary**, not Euclid. An exact rational is kept in lowest terms, so a gcd
//!    runs after *every* arithmetic operation — which put the slowest routine in the
//!    crate on the hot path of everything. Binary gcd needs no division at all, only
//!    shifts and subtraction, so it takes the worst thing off the busiest path. Largest
//!    win available, and the least famous.
//! 2. **Division is Knuth's algorithm D**, long division in base 2⁶⁴, rather than one
//!    bit at a time. A 10,000-digit division stops being a walk over 30,000 bits.
//! 3. **Multiplication is still schoolbook**, O(n²). It is the famous one and it is
//!    third, because it was never the thing on the hot path. Karatsuba above a crossover
//!    is the next thing to add here, and nothing else depends on it being done first.

use std::cmp::Ordering;


/// How many limbs a number keeps without asking for memory. Six is three hundred and
/// eighty-four bits, which covers a `b64`'s mantissa many times over, every decimal
/// coefficient anyone writes, and the first two working widths the transcendentals use —
/// so the arithmetic on the hot path never touches the heap at all.
const INLINE: usize = 6;

/// Limbs, kept on the stack while there are few of them.
///
/// This is the whole of the difference between arithmetic that costs forty nanoseconds
/// and arithmetic that costs four hundred. Every operation here builds a new number, and
/// when a number is a `Vec` that means asking the allocator — for three limbs. Measured
/// on the transcendentals, allocation was the cost and the arithmetic was the rounding
/// error: a multiply of two two-hundred-bit numbers is a handful of instructions and was
/// spending most of its time in `malloc`.
#[derive(Clone, Debug)]
enum Store {
    Few { len: u8, data: [u64; INLINE] },
    Many(Vec<u64>),
}

impl Store {
    fn clear(&mut self) {
        *self = Store::new();
    }

    /// Drop the first `n` limbs, which is a shift down by whole limbs.
    fn drop_front(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        if n >= self.len() {
            self.clear();
            return;
        }
        let kept = Store::from_slice(&self[n..]);
        *self = kept;
    }

    fn new() -> Store {
        Store::Few { len: 0, data: [0; INLINE] }
    }

    fn with_capacity(n: usize) -> Store {
        if n <= INLINE {
            Store::new()
        } else {
            Store::Many(Vec::with_capacity(n))
        }
    }

    fn from_slice(limbs: &[u64]) -> Store {
        let mut out = Store::with_capacity(limbs.len());
        out.extend_from_slice(limbs);
        out
    }

    fn push(&mut self, limb: u64) {
        match self {
            Store::Few { len, data } if (*len as usize) < INLINE => {
                data[*len as usize] = limb;
                *len += 1;
            }
            // Outgrown the stack: everything moves to the heap, once.
            Store::Few { len, data } => {
                let mut heap = Vec::with_capacity(INLINE * 2);
                heap.extend_from_slice(&data[..*len as usize]);
                heap.push(limb);
                *self = Store::Many(heap);
            }
            Store::Many(heap) => heap.push(limb),
        }
    }

    fn pop(&mut self) {
        match self {
            Store::Few { len, .. } => *len = len.saturating_sub(1),
            Store::Many(heap) => {
                heap.pop();
            }
        }
    }

    fn extend_from_slice(&mut self, limbs: &[u64]) {
        for limb in limbs {
            self.push(*limb);
        }
    }

    fn resize_zero(&mut self, n: usize) {
        while self.len() < n {
            self.push(0);
        }
    }
}

impl std::ops::Deref for Store {
    type Target = [u64];

    fn deref(&self) -> &[u64] {
        match self {
            Store::Few { len, data } => &data[..*len as usize],
            Store::Many(heap) => heap,
        }
    }
}

impl std::ops::DerefMut for Store {
    fn deref_mut(&mut self) -> &mut [u64] {
        match self {
            Store::Few { len, data } => &mut data[..*len as usize],
            Store::Many(heap) => heap,
        }
    }
}

// By what they hold, never by where they hold it: the same number inline and on the heap
// is the same number, and nothing outside this file can tell which it is.
impl PartialEq for Store {
    fn eq(&self, other: &Store) -> bool {
        **self == **other
    }
}

impl Eq for Store {}

impl std::hash::Hash for Store {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

impl Default for Store {
    fn default() -> Store {
        Store::new()
    }
}

/// A whole number of any size.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Big {
    /// Always `false` for zero, so a number has one representation and not two.
    negative: bool,
    /// Little-endian, and never ending in a zero limb.
    limbs: Store,
}

impl Big {
    pub fn zero() -> Self {
        Self::default()
    }

    pub fn from_u64(n: u64) -> Self {
        Big { negative: false, limbs: if n == 0 { Store::new() } else { Store::from_slice(&[n]) } }
    }

    pub fn from_i64(n: i64) -> Self {
        Big { negative: n < 0, limbs: if n == 0 { Store::new() } else { Store::from_slice(&[n.unsigned_abs()]) } }
    }

    pub fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    pub fn is_negative(&self) -> bool {
        self.negative
    }

    /// `-1`, `0` or `1`.
    pub fn signum(&self) -> i32 {
        if self.is_zero() {
            0
        } else if self.negative {
            -1
        } else {
            1
        }
    }

    pub fn negated(&self) -> Big {
        if self.is_zero() { self.clone() } else { Big { negative: !self.negative, limbs: self.limbs.clone() } }
    }

    pub fn abs(&self) -> Big {
        Big { negative: false, limbs: self.limbs.clone() }
    }

    /// How many limbs the magnitude needs. Zero needs none.
    pub fn limb_count(&self) -> usize {
        self.limbs.len()
    }

    /// The number as a `u64`, if it fits and is not negative.
    pub fn to_u64(&self) -> Option<u64> {
        match &*self.limbs {
            [] => Some(0),
            [one] if !self.negative => Some(*one),
            _ => None,
        }
    }

    fn of(negative: bool, mut limbs: Store) -> Big {
        trim(&mut limbs);
        Big { negative: negative && !limbs.is_empty(), limbs }
    }

    // --- comparison ---------------------------------------------------------------

    /// Compare magnitudes, ignoring both signs.
    /// How many bits it takes to write, ignoring the sign. Nought takes none.
    pub fn bits(&self) -> u64 {
        match self.limbs.last() {
            None => 0,
            Some(top) => (self.limbs.len() as u64 - 1) * 64 + (64 - top.leading_zeros() as u64),
        }
    }

    /// Whether the bit at this place is set, counting from the bottom. A binary float
    /// asks this to find out which way a rounding goes.
    pub fn bit(&self, at: u64) -> bool {
        let limb = (at / 64) as usize;
        match self.limbs.get(limb) {
            None => false,
            Some(word) => word >> (at % 64) & 1 == 1,
        }
    }

    /// Multiplied by two that many times. Whole limbs first, then the bits left over,
    /// because shifting by a whole limb is a move rather than an arithmetic.
    pub fn shifted_up(&self, by: u64) -> Big {
        if self.is_zero() {
            return Big::zero();
        }
        let (limbs, bits) = ((by / 64) as usize, (by % 64) as u32);
        let mut out = Store::with_capacity(self.limbs.len() + limbs + 1);
        out.resize_zero(limbs);
        if bits == 0 {
            out.extend_from_slice(&self.limbs);
        } else {
            let mut carry = 0u64;
            for limb in self.limbs.iter() {
                out.push(limb << bits | carry);
                carry = limb >> (64 - bits);
            }
            if carry != 0 {
                out.push(carry);
            }
        }
        trim(&mut out);
        Big { negative: self.negative, limbs: out }
    }

    /// Divided by two that many times, rounding toward zero — the bits that fall off the
    /// bottom are gone, which is what a shift means and is why a caller that cares about
    /// them asks [`Big::bit`] first.
    pub fn shifted_down(&self, by: u64) -> Big {
        let (limbs, bits) = ((by / 64) as usize, (by % 64) as u32);
        if limbs >= self.limbs.len() {
            return Big::zero();
        }
        let kept = &self.limbs[limbs..];
        let mut out = Store::with_capacity(kept.len());
        if bits == 0 {
            out.extend_from_slice(kept);
        } else {
            for (n, limb) in kept.iter().enumerate() {
                let above = kept.get(n + 1).copied().unwrap_or(0);
                out.push(limb >> bits | above << (64 - bits));
            }
        }
        trim(&mut out);
        Big { negative: self.negative && !out.is_empty(), limbs: out }
    }

    /// Whether any bit below this place is set, which is what says a shift lost
    /// something — the sticky bit, in the language of rounding.
    pub fn any_below(&self, at: u64) -> bool {
        let whole = (at / 64) as usize;
        if self.limbs.iter().take(whole).any(|limb| *limb != 0) {
            return true;
        }
        let bits = at % 64;
        bits != 0 && self.limbs.get(whole).is_some_and(|limb| limb & ((1 << bits) - 1) != 0)
    }

    /// `(self << places) / n`, and whether anything was left over.
    ///
    /// The shift and the division are one pass rather than two, which matters more than
    /// it sounds: every series in `transcend` divides by a term index, and doing it as a
    /// shift and then a division builds a second whole number in between. One pass is one
    /// allocation, and allocation is what these cost — the arithmetic itself is a
    /// multiply and a remainder per limb.
    pub fn shifted_then_divided(&self, places: u64, n: u64) -> (Big, bool) {
        if self.is_zero() || n == 0 {
            return (Big::zero(), false);
        }
        let whole = (places / 64) as usize;
        let bits = (places % 64) as u32;
        // The shifted number, written straight into the buffer the quotient will use.
        let mut limbs = Store::with_capacity(self.limbs.len() + whole + 1);
        limbs.resize_zero(whole);
        if bits == 0 {
            limbs.extend_from_slice(&self.limbs);
        } else {
            let mut carry = 0u64;
            for limb in self.limbs.iter() {
                limbs.push(limb << bits | carry);
                carry = limb >> (64 - bits);
            }
            if carry != 0 {
                limbs.push(carry);
            }
        }
        let rest = div_small_in_place(&mut limbs, n);
        (Big { negative: self.negative && !limbs.is_empty(), limbs }, rest != 0)
    }

    /// The whole part of the square root, and whether anything was left over.
    ///
    /// Newton's, from a guess that is deliberately too big: halving the distance to the
    /// answer from above converges without ever undershooting, so the loop can stop the
    /// moment it stops improving rather than needing a bound worked out in advance.
    pub fn sqrt_floor(&self) -> (Big, bool) {
        if self.is_zero() || self.negative {
            return (Big::zero(), false);
        }
        let two = Big::from_u64(2);
        // Two to the half the bit count, rounded up, is above the root and near it.
        let mut root = Big::from_u64(1).shifted_up(self.bits().div_ceil(2));
        loop {
            let (divided, _) = self.div_rem(&root).expect("a root above nought");
            let (next, _) = root.add(&divided).div_rem(&two).expect("two is not nought");
            if next.cmp_abs(&root) != Ordering::Less {
                break;
            }
            root = next;
        }
        let square = root.mul(&root);
        (root.clone(), square != *self)
    }

    pub fn cmp_abs(&self, other: &Big) -> Ordering {
        cmp_mag(&self.limbs, &other.limbs)
    }

    // --- arithmetic ---------------------------------------------------------------

    pub fn add(&self, other: &Big) -> Big {
        if self.negative == other.negative {
            Big::of(self.negative, add_mag(&self.limbs, &other.limbs))
        } else {
            match cmp_mag(&self.limbs, &other.limbs) {
                Ordering::Equal => Big::zero(),
                Ordering::Greater => {
                    Big::of(self.negative, sub_mag(&self.limbs, &other.limbs))
                }
                Ordering::Less => Big::of(other.negative, sub_mag(&other.limbs, &self.limbs)),
            }
        }
    }

    pub fn sub(&self, other: &Big) -> Big {
        self.add(&other.negated())
    }

    pub fn mul(&self, other: &Big) -> Big {
        if self.is_zero() || other.is_zero() {
            return Big::zero();
        }
        Big::of(self.negative != other.negative, mul_mag(&self.limbs, &other.limbs))
    }

    /// Truncating division, and a remainder with the sign of the dividend.
    ///
    /// `None` when `other` is zero. Which convention a Quench program actually sees is a
    /// `QNL-Config.toml` setting; this is the one everything else is defined in terms of.
    pub fn div_rem(&self, other: &Big) -> Option<(Big, Big)> {
        if other.is_zero() {
            return None;
        }
        let (q, r) = div_rem_mag(&self.limbs, &other.limbs);
        Some((
            Big::of(self.negative != other.negative, q),
            Big::of(self.negative, r),
        ))
    }

    /// The greatest common divisor of two magnitudes. Signs are ignored; the answer is
    /// never negative, and `gcd(0, 0)` is zero.
    ///
    /// Binary gcd — Stein's — which uses only shifts, comparison and subtraction. This
    /// is on the hot path of every rational operation, so it is the one routine here
    /// that must not call division.
    pub fn gcd(a: &Big, b: &Big) -> Big {
        let mut a = Store::from_slice(&a.limbs);
        let mut b = Store::from_slice(&b.limbs);
        if a.is_empty() {
            return Big::of(false, b);
        }
        if b.is_empty() {
            return Big::of(false, a);
        }

        // Two even numbers share a factor of two for every trailing zero they both have.
        // Take those out now and put them back at the end.
        let shared = trailing_zeros(&a).min(trailing_zeros(&b));
        let odd = trailing_zeros(&a);
        shr_bits_in_place(&mut a, odd);

        loop {
            // `a` is odd here, always. Making `b` odd too means their difference is
            // even, so the next round always makes progress.
            let odd = trailing_zeros(&b);
            shr_bits_in_place(&mut b, odd);
            if cmp_mag(&a, &b) == Ordering::Greater {
                std::mem::swap(&mut a, &mut b);
            }
            b = sub_mag(&b, &a);
            if b.is_empty() {
                break;
            }
        }

        Big::of(false, shl_bits(&a, shared))
    }

    // --- text ---------------------------------------------------------------------

    /// Read a decimal number. Accepts a leading `-`, and nothing else.
    pub fn parse(text: &str) -> Option<Big> {
        let (negative, digits) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // Nineteen digits is the most that always fits in a u64, so the number is built
        // a chunk at a time rather than a digit at a time.
        let mut limbs = Store::new();
        for chunk in (Chunks { rest: digits.as_bytes(), size: 19 }) {
            let value: u64 = std::str::from_utf8(chunk).ok()?.parse().ok()?;
            let scale = 10u64.pow(chunk.len() as u32);
            mul_small_in_place(&mut limbs, scale);
            add_small_in_place(&mut limbs, value);
        }
        Some(Big::of(negative, limbs))
    }
}

/// Decimal digits, taken from the front in groups of at most `size`.
struct Chunks<'a> {
    rest: &'a [u8],
    size: usize,
}

impl<'a> Iterator for Chunks<'a> {
    type Item = &'a [u8];
    fn next(&mut self) -> Option<&'a [u8]> {
        if self.rest.is_empty() {
            return None;
        }
        let take = self.size.min(self.rest.len());
        let (head, tail) = self.rest.split_at(take);
        self.rest = tail;
        Some(head)
    }
}

impl std::fmt::Display for Big {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_zero() {
            return f.write_str("0");
        }
        // Divide by the largest power of ten that fits in a limb, so each step yields
        // nineteen digits rather than one.
        let mut limbs = self.limbs.clone();
        let mut groups: Vec<u64> = Vec::new();
        while !limbs.is_empty() {
            let rest = div_small_in_place(&mut limbs, 10_000_000_000_000_000_000);
            groups.push(rest);
        }
        if self.negative {
            f.write_str("-")?;
        }
        let mut groups = groups.into_iter().rev();
        write!(f, "{}", groups.next().expect("a non-zero number has a group"))?;
        for group in groups {
            write!(f, "{group:019}")?;
        }
        Ok(())
    }
}

impl Ord for Big {
    fn cmp(&self, other: &Big) -> Ordering {
        match (self.signum(), other.signum()) {
            (a, b) if a != b => a.cmp(&b),
            // Same sign: a larger magnitude is further from zero, in whichever direction.
            (-1, _) => cmp_mag(&other.limbs, &self.limbs),
            _ => cmp_mag(&self.limbs, &other.limbs),
        }
    }
}

impl PartialOrd for Big {
    fn partial_cmp(&self, other: &Big) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// --- magnitudes, which know nothing about signs -------------------------------------

fn trim(limbs: &mut Store) {
    while limbs.last() == Some(&0) {
        limbs.pop();
    }
}

fn cmp_mag(a: &[u64], b: &[u64]) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.iter().rev().cmp(b.iter().rev()))
}

fn add_mag(a: &[u64], b: &[u64]) -> Store {
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut out = Store::with_capacity(long.len() + 1);
    let mut carry = 0u128;
    for i in 0..long.len() {
        let sum = long[i] as u128 + *short.get(i).unwrap_or(&0) as u128 + carry;
        out.push(sum as u64);
        carry = sum >> 64;
    }
    if carry != 0 {
        out.push(carry as u64);
    }
    out
}

/// `a - b`, and `a` must be at least `b`.
fn sub_mag(a: &[u64], b: &[u64]) -> Store {
    debug_assert!(cmp_mag(a, b) != Ordering::Less, "sub_mag would go negative");
    let mut out = Store::with_capacity(a.len());
    let mut borrow = 0i128;
    for i in 0..a.len() {
        let diff = a[i] as i128 - *b.get(i).unwrap_or(&0) as i128 - borrow;
        if diff < 0 {
            out.push((diff + (1i128 << 64)) as u64);
            borrow = 1;
        } else {
            out.push(diff as u64);
            borrow = 0;
        }
    }
    debug_assert_eq!(borrow, 0, "sub_mag borrowed off the end");
    trim(&mut out);
    out
}

fn mul_mag(a: &[u64], b: &[u64]) -> Store {
    let mut out = Store::with_capacity(a.len() + b.len());
    out.resize_zero(a.len() + b.len());
    for (i, &x) in a.iter().enumerate() {
        if x == 0 {
            continue;
        }
        let mut carry = 0u128;
        for (j, &y) in b.iter().enumerate() {
            let at = i + j;
            let sum = out[at] as u128 + x as u128 * y as u128 + carry;
            out[at] = sum as u64;
            carry = sum >> 64;
        }
        let mut at = i + b.len();
        while carry != 0 {
            let sum = out[at] as u128 + carry;
            out[at] = sum as u64;
            carry = sum >> 64;
            at += 1;
        }
    }
    trim(&mut out);
    out
}

fn trailing_zeros(limbs: &[u64]) -> usize {
    for (i, &limb) in limbs.iter().enumerate() {
        if limb != 0 {
            return i * 64 + limb.trailing_zeros() as usize;
        }
    }
    0
}

fn shl_bits(limbs: &[u64], bits: usize) -> Store {
    if limbs.is_empty() {
        return Store::new();
    }
    let (whole, part) = (bits / 64, bits % 64);
    let mut out = Store::with_capacity(limbs.len() + whole + 1);
    out.resize_zero(whole);
    if part == 0 {
        out.extend_from_slice(limbs);
    } else {
        let mut carry = 0u64;
        for &limb in limbs {
            out.push((limb << part) | carry);
            carry = limb >> (64 - part);
        }
        if carry != 0 {
            out.push(carry);
        }
    }
    trim(&mut out);
    out
}

fn shr_bits_in_place(limbs: &mut Store, bits: usize) {
    if bits == 0 || limbs.is_empty() {
        return;
    }
    let (whole, part) = (bits / 64, bits % 64);
    if whole >= limbs.len() {
        limbs.clear();
        return;
    }
    limbs.drop_front(whole);
    if part != 0 {
        for i in 0..limbs.len() {
            let high = limbs.get(i + 1).copied().unwrap_or(0);
            limbs[i] = (limbs[i] >> part) | (high << (64 - part));
        }
    }
    trim(limbs);
}

fn mul_small_in_place(limbs: &mut Store, factor: u64) {
    let mut carry = 0u128;
    for limb in limbs.iter_mut() {
        let product = *limb as u128 * factor as u128 + carry;
        *limb = product as u64;
        carry = product >> 64;
    }
    while carry != 0 {
        limbs.push(carry as u64);
        carry >>= 64;
    }
    trim(limbs);
}

fn add_small_in_place(limbs: &mut Store, addend: u64) {
    let mut carry = addend as u128;
    let mut i = 0;
    while carry != 0 {
        if i == limbs.len() {
            limbs.push(0);
        }
        let sum = limbs[i] as u128 + carry;
        limbs[i] = sum as u64;
        carry = sum >> 64;
        i += 1;
    }
    trim(limbs);
}

/// Divide in place by a single limb, returning the remainder.
fn div_small_in_place(limbs: &mut Store, divisor: u64) -> u64 {
    let mut rest = 0u128;
    for limb in limbs.iter_mut().rev() {
        let value = (rest << 64) | *limb as u128;
        *limb = (value / divisor as u128) as u64;
        rest = value % divisor as u128;
    }
    trim(limbs);
    rest as u64
}

/// Long division in base 2⁶⁴ — Knuth's algorithm D.
///
/// The divisor is normalised so its top limb has its high bit set, which is what makes
/// the two-limb estimate of each quotient digit wrong by at most one. Everything after
/// that is a multiply-and-subtract, with an add-back on the rare occasion the estimate
/// was high.
fn div_rem_mag(u: &[u64], v: &[u64]) -> (Store, Store) {
    debug_assert!(!v.is_empty(), "division by zero reaches here");
    if cmp_mag(u, v) == Ordering::Less {
        return (Store::new(), Store::from_slice(u));
    }
    if v.len() == 1 {
        let mut q = Store::from_slice(u);
        let r = div_small_in_place(&mut q, v[0]);
        return (q, if r == 0 { Store::new() } else { Store::from_slice(&[r]) });
    }

    let shift = v[v.len() - 1].leading_zeros() as usize;
    let vn = shl_bits(v, shift);
    let mut un = shl_bits(u, shift);
    un.push(0); // room for the top limb the estimate reads

    let n = vn.len();
    let m = un.len() - 1 - n;
    let mut q = Store::with_capacity(m + 2);
    q.resize_zero(m + 1);

    for j in (0..=m).rev() {
        let top = ((un[j + n] as u128) << 64) | un[j + n - 1] as u128;
        let mut qhat = top / vn[n - 1] as u128;
        let mut rhat = top % vn[n - 1] as u128;
        while qhat >> 64 != 0
            || qhat * vn[n - 2] as u128 > ((rhat << 64) | un[j + n - 2] as u128)
        {
            qhat -= 1;
            rhat += vn[n - 1] as u128;
            if rhat >> 64 != 0 {
                break;
            }
        }

        // Subtract qhat * vn from the window of un it lines up with.
        let mut borrow = 0i128;
        let mut carry = 0u128;
        for i in 0..n {
            let product = qhat * vn[i] as u128 + carry;
            carry = product >> 64;
            let diff = un[i + j] as i128 - (product as u64) as i128 - borrow;
            un[i + j] = diff as u64;
            borrow = if diff < 0 { 1 } else { 0 };
        }
        let diff = un[j + n] as i128 - carry as i128 - borrow;
        un[j + n] = diff as u64;

        if diff < 0 {
            // The estimate was one too high, which the normalisation guarantees is the
            // most it can ever be. Give one back and add the divisor in again.
            q[j] = (qhat - 1) as u64;
            let mut carry = 0u128;
            for i in 0..n {
                let sum = un[i + j] as u128 + vn[i] as u128 + carry;
                un[i + j] = sum as u64;
                carry = sum >> 64;
            }
            un[j + n] = (un[j + n] as u128 + carry) as u64;
        } else {
            q[j] = qhat as u64;
        }
    }

    trim(&mut q);
    let mut r = Store::from_slice(&un[..n]);
    trim(&mut r);
    shr_bits_in_place(&mut r, shift);
    (q, r)
}
