//! Typed SSA graph IR for [`Num`] kernels with algebraic simplification, color-gauge
//! cancellation (`linear_to_srgb(srgb_to_linear(x)) → x`), hash-consed CSE, and dead-code
//! elimination before WGSL code generation.

use crate::Num;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::{Add, Div, Mul, Neg, Sub};

/// A symbolic SSA node handle implementing [`Num`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Sym(pub u32);

/// Statistics reported by [`Graph::emit_wgsl_body`] so tests and diagnostics can observe
/// algebraic/gauge reductions mechanically.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraphStats {
    pub raw_ops: usize,
    pub folded_constants: usize,
    pub algebraic_rewrites: usize,
    pub gauge_cancellations: usize,
    pub cse_hits: usize,
    pub live_nodes: usize,
}

#[derive(Clone, Debug, PartialEq)]
enum Op {
    Lit(f32),
    Input(String),
    Add(u32, u32),
    Sub(u32, u32),
    Mul(u32, u32),
    Div(u32, u32),
    Neg(u32),
    Min(u32, u32),
    Max(u32, u32),
    Sqrt(u32),
    Abs(u32),
    Hypot(u32, u32),
    PowConst(u32, f32),
    Pow(u32, u32),
    Exp(u32),
    Log10(u32),
    RemEuclid(u32, u32),
    LinearToSrgb(u32),
    SrgbToLinear(u32),
    SelectAtMost {
        lhs: u32,
        rhs: u32,
        yes: u32,
        no: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum OpKey {
    Lit(u32),
    Input(String),
    Add(u32, u32),
    Sub(u32, u32),
    Mul(u32, u32),
    Div(u32, u32),
    Neg(u32),
    Min(u32, u32),
    Max(u32, u32),
    Sqrt(u32),
    Abs(u32),
    Hypot(u32, u32),
    PowConst(u32, u32),
    Pow(u32, u32),
    Exp(u32),
    Log10(u32),
    RemEuclid(u32, u32),
    LinearToSrgb(u32),
    SrgbToLinear(u32),
    SelectAtMost(u32, u32, u32, u32),
}

fn canon_bits(v: f32) -> u32 {
    if v == 0.0 {
        0.0f32.to_bits()
    } else {
        v.to_bits()
    }
}

fn op_key(op: &Op) -> OpKey {
    match op {
        Op::Lit(v) => OpKey::Lit(canon_bits(*v)),
        Op::Input(name) => OpKey::Input(name.clone()),
        Op::Add(a, b) => OpKey::Add(*a, *b),
        Op::Sub(a, b) => OpKey::Sub(*a, *b),
        Op::Mul(a, b) => OpKey::Mul(*a, *b),
        Op::Div(a, b) => OpKey::Div(*a, *b),
        Op::Neg(a) => OpKey::Neg(*a),
        Op::Min(a, b) => OpKey::Min(*a, *b),
        Op::Max(a, b) => OpKey::Max(*a, *b),
        Op::Sqrt(a) => OpKey::Sqrt(*a),
        Op::Abs(a) => OpKey::Abs(*a),
        Op::Hypot(a, b) => OpKey::Hypot(*a, *b),
        Op::PowConst(a, p) => OpKey::PowConst(*a, canon_bits(*p)),
        Op::Pow(a, b) => OpKey::Pow(*a, *b),
        Op::Exp(a) => OpKey::Exp(*a),
        Op::Log10(a) => OpKey::Log10(*a),
        Op::RemEuclid(a, b) => OpKey::RemEuclid(*a, *b),
        Op::LinearToSrgb(a) => OpKey::LinearToSrgb(*a),
        Op::SrgbToLinear(a) => OpKey::SrgbToLinear(*a),
        Op::SelectAtMost { lhs, rhs, yes, no } => OpKey::SelectAtMost(*lhs, *rhs, *yes, *no),
    }
}

/// Format an `f32` literal safely for WGSL (always has a decimal point or exponent, never NaN/inf).
pub(crate) fn fmt_f32(v: f32) -> String {
    let v = if v == 0.0 { 0.0 } else { v };
    let text = format!("{v:?}");
    if text.contains('.') || text.contains('e') || text.contains('E') {
        text
    } else {
        format!("{text}.0")
    }
}

/// SSA computation graph built by tracing generic [`Num`] functions over [`Sym`].
#[derive(Default)]
pub struct Graph {
    nodes: Vec<Op>,
    intern: HashMap<OpKey, u32>,
    stats: GraphStats,
}

thread_local! {
    static ACTIVE: RefCell<Graph> = RefCell::new(Graph::default());
}

impl Graph {
    /// Trace `body` into a fresh optimized SSA [`Graph`].
    pub fn trace<T>(body: impl FnOnce() -> T) -> (Self, T) {
        ACTIVE.with(|cell| *cell.borrow_mut() = Graph::default());
        let out = body();
        let graph = ACTIVE.with(|cell| std::mem::take(&mut *cell.borrow_mut()));
        (graph, out)
    }

    /// Reference an external WGSL expression (e.g., `"c.x"` or `"u.p[0u].x"`).
    pub fn input(expr: impl Into<String>) -> Sym {
        ACTIVE.with(|cell| cell.borrow_mut().intern_node(Op::Input(expr.into())))
    }

    pub fn stats(&self) -> GraphStats {
        self.stats
    }

    fn as_lit(&self, id: u32) -> Option<f32> {
        match self.nodes[id as usize] {
            Op::Lit(v) => Some(v),
            _ => None,
        }
    }

    fn is_non_negative(&self, id: u32) -> bool {
        match &self.nodes[id as usize] {
            Op::Lit(v) => *v >= 0.0,
            Op::Abs(_) | Op::Sqrt(_) | Op::Hypot(_, _) | Op::Exp(_) => true,
            Op::Max(a, b) => self.is_non_negative(*a) || self.is_non_negative(*b),
            Op::Min(a, b) => self.is_non_negative(*a) && self.is_non_negative(*b),
            Op::Add(a, b) | Op::Mul(a, b) | Op::Div(a, b) => {
                self.is_non_negative(*a) && self.is_non_negative(*b)
            }
            Op::PowConst(a, p) => self.is_non_negative(*a) || *p == 2.0,
            Op::Pow(a, _) => self.is_non_negative(*a),
            Op::LinearToSrgb(a) | Op::SrgbToLinear(a) => self.is_non_negative(*a),
            Op::SelectAtMost { yes, no, .. } => {
                self.is_non_negative(*yes) && self.is_non_negative(*no)
            }
            _ => false,
        }
    }

    fn is_at_most_one(&self, id: u32) -> bool {
        match &self.nodes[id as usize] {
            Op::Lit(v) => *v <= 1.0,
            Op::Min(a, b) => self.is_at_most_one(*a) || self.is_at_most_one(*b),
            Op::Max(a, b) => self.is_at_most_one(*a) && self.is_at_most_one(*b),
            Op::LinearToSrgb(a) | Op::SrgbToLinear(a) => self.is_at_most_one(*a),
            Op::SelectAtMost { yes, no, .. } => {
                self.is_at_most_one(*yes) && self.is_at_most_one(*no)
            }
            _ => false,
        }
    }

    fn intern_node(&mut self, op: Op) -> Sym {
        let key = op_key(&op);
        if let Some(&existing) = self.intern.get(&key) {
            if !matches!(op, Op::Lit(_) | Op::Input(_)) {
                self.stats.cse_hits += 1;
            }
            return Sym(existing);
        }
        let id = self.nodes.len() as u32;
        self.nodes.push(op);
        self.intern.insert(key, id);
        Sym(id)
    }

    fn lit(&mut self, value: f32) -> Sym {
        self.intern_node(Op::Lit(if value == 0.0 { 0.0 } else { value }))
    }

    fn add(&mut self, a: u32, b: u32) -> Sym {
        self.stats.raw_ops += 1;
        if let (Some(x), Some(y)) = (self.as_lit(a), self.as_lit(b)) {
            self.stats.folded_constants += 1;
            return self.lit(x + y);
        }
        if self.as_lit(a) == Some(0.0) {
            self.stats.algebraic_rewrites += 1;
            return Sym(b);
        }
        if self.as_lit(b) == Some(0.0) {
            self.stats.algebraic_rewrites += 1;
            return Sym(a);
        }
        let (x, y) = if a <= b { (a, b) } else { (b, a) };
        self.intern_node(Op::Add(x, y))
    }

    fn sub(&mut self, a: u32, b: u32) -> Sym {
        self.stats.raw_ops += 1;
        if let (Some(x), Some(y)) = (self.as_lit(a), self.as_lit(b)) {
            self.stats.folded_constants += 1;
            return self.lit(x - y);
        }
        if a == b {
            self.stats.algebraic_rewrites += 1;
            return self.lit(0.0);
        }
        if self.as_lit(b) == Some(0.0) {
            self.stats.algebraic_rewrites += 1;
            return Sym(a);
        }
        if self.as_lit(a) == Some(0.0) {
            self.stats.algebraic_rewrites += 1;
            return self.neg(b);
        }
        self.intern_node(Op::Sub(a, b))
    }

    fn mul(&mut self, a: u32, b: u32) -> Sym {
        self.stats.raw_ops += 1;
        if let (Some(x), Some(y)) = (self.as_lit(a), self.as_lit(b)) {
            self.stats.folded_constants += 1;
            return self.lit(x * y);
        }
        if self.as_lit(a) == Some(0.0) || self.as_lit(b) == Some(0.0) {
            self.stats.algebraic_rewrites += 1;
            return self.lit(0.0);
        }
        if self.as_lit(a) == Some(1.0) {
            self.stats.algebraic_rewrites += 1;
            return Sym(b);
        }
        if self.as_lit(b) == Some(1.0) {
            self.stats.algebraic_rewrites += 1;
            return Sym(a);
        }
        let (x, y) = if a <= b { (a, b) } else { (b, a) };
        self.intern_node(Op::Mul(x, y))
    }

    fn div(&mut self, a: u32, b: u32) -> Sym {
        self.stats.raw_ops += 1;
        if let (Some(x), Some(y)) = (self.as_lit(a), self.as_lit(b))
            && y != 0.0
        {
            self.stats.folded_constants += 1;
            return self.lit(x / y);
        }
        if self.as_lit(b) == Some(1.0) {
            self.stats.algebraic_rewrites += 1;
            return Sym(a);
        }
        if self.as_lit(a) == Some(0.0)
            && let Some(y) = self.as_lit(b)
            && y != 0.0
        {
            self.stats.algebraic_rewrites += 1;
            return self.lit(0.0);
        }
        self.intern_node(Op::Div(a, b))
    }

    fn neg(&mut self, a: u32) -> Sym {
        self.stats.raw_ops += 1;
        if let Some(x) = self.as_lit(a) {
            self.stats.folded_constants += 1;
            return self.lit(-x);
        }
        if let Op::Neg(inner) = self.nodes[a as usize] {
            self.stats.algebraic_rewrites += 1;
            return Sym(inner);
        }
        self.intern_node(Op::Neg(a))
    }

    fn min(&mut self, a: u32, b: u32) -> Sym {
        self.stats.raw_ops += 1;
        if let (Some(x), Some(y)) = (self.as_lit(a), self.as_lit(b)) {
            self.stats.folded_constants += 1;
            return self.lit(x.min(y));
        }
        if a == b {
            self.stats.algebraic_rewrites += 1;
            return Sym(a);
        }
        if self.as_lit(b) == Some(1.0) && self.is_at_most_one(a) {
            self.stats.algebraic_rewrites += 1;
            return Sym(a);
        }
        if self.as_lit(a) == Some(1.0) && self.is_at_most_one(b) {
            self.stats.algebraic_rewrites += 1;
            return Sym(b);
        }
        let (x, y) = if a <= b { (a, b) } else { (b, a) };
        self.intern_node(Op::Min(x, y))
    }

    fn max(&mut self, a: u32, b: u32) -> Sym {
        self.stats.raw_ops += 1;
        if let (Some(x), Some(y)) = (self.as_lit(a), self.as_lit(b)) {
            self.stats.folded_constants += 1;
            return self.lit(x.max(y));
        }
        if a == b {
            self.stats.algebraic_rewrites += 1;
            return Sym(a);
        }
        if self.as_lit(b) == Some(0.0) && self.is_non_negative(a) {
            self.stats.algebraic_rewrites += 1;
            return Sym(a);
        }
        if self.as_lit(a) == Some(0.0) && self.is_non_negative(b) {
            self.stats.algebraic_rewrites += 1;
            return Sym(b);
        }
        let (x, y) = if a <= b { (a, b) } else { (b, a) };
        self.intern_node(Op::Max(x, y))
    }

    fn sqrt(&mut self, a: u32) -> Sym {
        self.stats.raw_ops += 1;
        if let Some(x) = self.as_lit(a)
            && x >= 0.0
        {
            self.stats.folded_constants += 1;
            return self.lit(x.sqrt());
        }
        self.intern_node(Op::Sqrt(a))
    }

    fn abs(&mut self, a: u32) -> Sym {
        self.stats.raw_ops += 1;
        if let Some(x) = self.as_lit(a) {
            self.stats.folded_constants += 1;
            return self.lit(x.abs());
        }
        if self.is_non_negative(a) {
            self.stats.algebraic_rewrites += 1;
            return Sym(a);
        }
        self.intern_node(Op::Abs(a))
    }

    fn hypot(&mut self, a: u32, b: u32) -> Sym {
        self.stats.raw_ops += 1;
        if let (Some(x), Some(y)) = (self.as_lit(a), self.as_lit(b)) {
            self.stats.folded_constants += 1;
            return self.lit(x.hypot(y));
        }
        let (x, y) = if a <= b { (a, b) } else { (b, a) };
        self.intern_node(Op::Hypot(x, y))
    }

    fn pow_const(&mut self, a: u32, p: f32) -> Sym {
        self.stats.raw_ops += 1;
        if let Some(x) = self.as_lit(a) {
            self.stats.folded_constants += 1;
            return self.lit(x.powf(p));
        }
        if p == 0.0 {
            self.stats.algebraic_rewrites += 1;
            return self.lit(1.0);
        }
        if p == 1.0 {
            self.stats.algebraic_rewrites += 1;
            return Sym(a);
        }
        if p == 2.0 {
            self.stats.algebraic_rewrites += 1;
            return self.mul(a, a);
        }
        self.intern_node(Op::PowConst(a, p))
    }

    fn pow(&mut self, a: u32, b: u32) -> Sym {
        self.stats.raw_ops += 1;
        if let Some(p) = self.as_lit(b) {
            return self.pow_const(a, p);
        }
        self.intern_node(Op::Pow(a, b))
    }

    fn exp(&mut self, a: u32) -> Sym {
        self.stats.raw_ops += 1;
        if let Some(x) = self.as_lit(a) {
            self.stats.folded_constants += 1;
            return self.lit(x.exp());
        }
        self.intern_node(Op::Exp(a))
    }

    fn log10(&mut self, a: u32) -> Sym {
        self.stats.raw_ops += 1;
        if let Some(x) = self.as_lit(a)
            && x > 0.0
        {
            self.stats.folded_constants += 1;
            return self.lit(x.log10());
        }
        self.intern_node(Op::Log10(a))
    }

    fn rem_euclid(&mut self, a: u32, b: u32) -> Sym {
        self.stats.raw_ops += 1;
        if let (Some(x), Some(y)) = (self.as_lit(a), self.as_lit(b))
            && y > 0.0
        {
            self.stats.folded_constants += 1;
            return self.lit(x.rem_euclid(y));
        }
        self.intern_node(Op::RemEuclid(a, b))
    }

    fn linear_to_srgb(&mut self, a: u32) -> Sym {
        self.stats.raw_ops += 1;
        if let Some(x) = self.as_lit(a) {
            self.stats.folded_constants += 1;
            return self.lit(x.linear_to_srgb());
        }
        // Color-gauge cancellation: linear_to_srgb(srgb_to_linear(x)) == x for x >= 0.
        if let Op::SrgbToLinear(inner) = self.nodes[a as usize]
            && self.is_non_negative(inner)
        {
            self.stats.gauge_cancellations += 1;
            return Sym(inner);
        }
        self.intern_node(Op::LinearToSrgb(a))
    }

    fn srgb_to_linear(&mut self, a: u32) -> Sym {
        self.stats.raw_ops += 1;
        if let Some(x) = self.as_lit(a) {
            self.stats.folded_constants += 1;
            return self.lit(x.srgb_to_linear());
        }
        // Color-gauge cancellation: srgb_to_linear(linear_to_srgb(x)) == x for x >= 0.
        if let Op::LinearToSrgb(inner) = self.nodes[a as usize]
            && self.is_non_negative(inner)
        {
            self.stats.gauge_cancellations += 1;
            return Sym(inner);
        }
        self.intern_node(Op::SrgbToLinear(a))
    }

    fn select_at_most(&mut self, lhs: u32, rhs: u32, yes: u32, no: u32) -> Sym {
        self.stats.raw_ops += 1;
        if yes == no {
            self.stats.algebraic_rewrites += 1;
            return Sym(yes);
        }
        if let (Some(l), Some(r)) = (self.as_lit(lhs), self.as_lit(rhs)) {
            self.stats.folded_constants += 1;
            return Sym(if l <= r { yes } else { no });
        }
        self.intern_node(Op::SelectAtMost { lhs, rhs, yes, no })
    }

    /// Emit dead-code-eliminated WGSL statements computing `roots`, returning `(body, root_exprs, stats)`.
    /// Expects `kpow`, `lin_to_srgb`, and `srgb_to_lin` helpers in scope.
    pub fn emit_wgsl_body(&self, roots: &[Sym], indent: &str) -> (String, Vec<String>, GraphStats) {
        let mut live = vec![false; self.nodes.len()];
        let mut stack: Vec<u32> = roots.iter().map(|s| s.0).collect();
        while let Some(id) = stack.pop() {
            let idx = id as usize;
            if live[idx] {
                continue;
            }
            live[idx] = true;
            match &self.nodes[idx] {
                Op::Lit(_) | Op::Input(_) => {}
                Op::Neg(a)
                | Op::Sqrt(a)
                | Op::Abs(a)
                | Op::PowConst(a, _)
                | Op::Exp(a)
                | Op::Log10(a)
                | Op::LinearToSrgb(a)
                | Op::SrgbToLinear(a) => stack.push(*a),
                Op::Add(a, b)
                | Op::Sub(a, b)
                | Op::Mul(a, b)
                | Op::Div(a, b)
                | Op::Min(a, b)
                | Op::Max(a, b)
                | Op::Hypot(a, b)
                | Op::Pow(a, b)
                | Op::RemEuclid(a, b) => {
                    stack.push(*a);
                    stack.push(*b);
                }
                Op::SelectAtMost { lhs, rhs, yes, no } => {
                    stack.push(*lhs);
                    stack.push(*rhs);
                    stack.push(*yes);
                    stack.push(*no);
                }
            }
        }

        let atom = |id: u32| -> String {
            match &self.nodes[id as usize] {
                Op::Lit(v) => fmt_f32(*v),
                Op::Input(name) => name.clone(),
                _ => format!("s{id}"),
            }
        };

        let mut body = String::new();
        let mut live_nodes = 0usize;
        for (idx, op) in self.nodes.iter().enumerate() {
            if !live[idx] {
                continue;
            }
            let expr = match op {
                Op::Lit(_) | Op::Input(_) => continue,
                Op::Add(a, b) => format!("{} + {}", atom(*a), atom(*b)),
                Op::Sub(a, b) => format!("{} - {}", atom(*a), atom(*b)),
                Op::Mul(a, b) => format!("{} * {}", atom(*a), atom(*b)),
                Op::Div(a, b) => format!("{} / {}", atom(*a), atom(*b)),
                Op::Neg(a) => format!("-({})", atom(*a)),
                Op::Min(a, b) => format!("min({}, {})", atom(*a), atom(*b)),
                Op::Max(a, b) => format!("max({}, {})", atom(*a), atom(*b)),
                Op::Sqrt(a) => format!("sqrt({})", atom(*a)),
                Op::Abs(a) => format!("abs({})", atom(*a)),
                Op::Hypot(a, b) => format!("length(vec2({}, {}))", atom(*a), atom(*b)),
                Op::PowConst(a, p) => format!("kpow({}, {})", atom(*a), fmt_f32(*p)),
                Op::Pow(a, b) => format!("kpow({}, {})", atom(*a), atom(*b)),
                Op::Exp(a) => format!("exp({})", atom(*a)),
                Op::Log10(a) => {
                    format!(
                        "log2({}) * {}",
                        atom(*a),
                        fmt_f32(std::f32::consts::LOG10_2)
                    )
                }
                Op::RemEuclid(a, b) => {
                    let (sa, sb) = (atom(*a), atom(*b));
                    format!("{sa} - {sb} * floor({sa} / {sb})")
                }
                Op::LinearToSrgb(a) => format!("lin_to_srgb({})", atom(*a)),
                Op::SrgbToLinear(a) => format!("srgb_to_lin({})", atom(*a)),
                Op::SelectAtMost { lhs, rhs, yes, no } => format!(
                    "select({}, {}, {} <= {})",
                    atom(*no),
                    atom(*yes),
                    atom(*lhs),
                    atom(*rhs)
                ),
            };
            live_nodes += 1;
            body.push_str(indent);
            body.push_str(&format!("let s{idx}: f32 = {expr};\n"));
        }
        let root_exprs = roots.iter().map(|s| atom(s.0)).collect();
        let mut stats = self.stats;
        stats.live_nodes = live_nodes;
        (body, root_exprs, stats)
    }
}

impl Add for Sym {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().add(self.0, rhs.0))
    }
}

impl Sub for Sym {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().sub(self.0, rhs.0))
    }
}

impl Mul for Sym {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().mul(self.0, rhs.0))
    }
}

impl Div for Sym {
    type Output = Self;
    fn div(self, rhs: Self) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().div(self.0, rhs.0))
    }
}

impl Neg for Sym {
    type Output = Self;
    fn neg(self) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().neg(self.0))
    }
}

impl Num for Sym {
    fn lit(value: f32) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().lit(value))
    }
    fn min(self, other: Self) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().min(self.0, other.0))
    }
    fn max(self, other: Self) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().max(self.0, other.0))
    }
    fn sqrt(self) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().sqrt(self.0))
    }
    fn abs(self) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().abs(self.0))
    }
    fn hypot(self, other: Self) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().hypot(self.0, other.0))
    }
    fn pow(self, power: f32) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().pow_const(self.0, power))
    }
    fn powf(self, power: Self) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().pow(self.0, power.0))
    }
    fn exp(self) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().exp(self.0))
    }
    fn log10(self) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().log10(self.0))
    }
    fn rem_euclid(self, by: Self) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().rem_euclid(self.0, by.0))
    }
    fn linear_to_srgb(self) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().linear_to_srgb(self.0))
    }
    fn srgb_to_linear(self) -> Self {
        ACTIVE.with(|cell| cell.borrow_mut().srgb_to_linear(self.0))
    }
    fn if_at_most<const N: usize>(
        self,
        limit: Self,
        yes: impl FnOnce() -> [Self; N],
        no: impl FnOnce() -> [Self; N],
    ) -> [Self; N] {
        let (yes, no) = (yes(), no());
        ACTIVE.with(|cell| {
            let mut graph = cell.borrow_mut();
            std::array::from_fn(|at| graph.select_at_most(self.0, limit.0, yes[at].0, no[at].0))
        })
    }
}
