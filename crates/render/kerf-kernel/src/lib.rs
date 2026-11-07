//! One source for a kernel. Written once in Rust over [`Num`], a kernel runs on the CPU as plain `f32`,
//! and run over [`Traced`] it writes itself out as WGSL, one line for each thing it does.

use std::cell::{Cell, RefCell};
use std::ops::{Add, Div, Mul, Neg, Sub};

/// What a kernel does with a number.
pub trait Num:
    Copy
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Neg<Output = Self>
{
    fn lit(value: f32) -> Self;
    fn min(self, other: Self) -> Self;
    fn max(self, other: Self) -> Self;
    fn sqrt(self) -> Self;
    fn pow(self, power: f32) -> Self;
    /// `yes` where this is at most `limit`, else `no`. The CPU works out the one it takes, and WGSL both.
    fn if_at_most<const N: usize>(
        self,
        limit: Self,
        yes: impl FnOnce() -> [Self; N],
        no: impl FnOnce() -> [Self; N],
    ) -> [Self; N];

    fn clamp(self, low: Self, high: Self) -> Self {
        self.max(low).min(high)
    }
}

impl Num for f32 {
    #[inline(always)]
    fn lit(value: f32) -> f32 {
        value
    }
    #[inline(always)]
    fn min(self, other: f32) -> f32 {
        f32::min(self, other)
    }
    #[inline(always)]
    fn max(self, other: f32) -> f32 {
        f32::max(self, other)
    }
    #[inline(always)]
    fn sqrt(self) -> f32 {
        f32::sqrt(self)
    }
    #[inline(always)]
    fn pow(self, power: f32) -> f32 {
        self.powf(power)
    }
    #[inline(always)]
    fn if_at_most<const N: usize>(
        self,
        limit: f32,
        yes: impl FnOnce() -> [f32; N],
        no: impl FnOnce() -> [f32; N],
    ) -> [f32; N] {
        if self <= limit {
            yes()
        } else {
            no()
        }
    }
}

thread_local! {
    static LINES: RefCell<String> = const { RefCell::new(String::new()) };
    static NEXT: Cell<u32> = const { Cell::new(0) };
}

fn next() -> u32 {
    NEXT.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    })
}

fn line(text: &str) {
    LINES.with(|lines| {
        let mut lines = lines.borrow_mut();
        lines.push_str("    ");
        lines.push_str(text);
        lines.push('\n');
    });
}

/// Run `body`, and what it did as WGSL.
pub fn written(body: impl FnOnce()) -> String {
    LINES.with(|lines| lines.borrow_mut().clear());
    NEXT.with(|next| next.set(0));
    body();
    LINES.with(|lines| std::mem::take(&mut *lines.borrow_mut()))
}

/// A number in the WGSL being written.
#[derive(Clone, Copy, Debug)]
pub struct Traced(u32);

impl Traced {
    fn of(expression: &str) -> Traced {
        let id = next();
        line(&format!("let v{id}: f32 = {expression};"));
        Traced(id)
    }

    /// A number the kernel reads, such as a uniform.
    pub fn named(expression: &str) -> Traced {
        Traced::of(expression)
    }

    pub fn name(self) -> String {
        format!("v{}", self.0)
    }
}

macro_rules! traced_op {
    ($trait:ident, $method:ident, $op:literal) => {
        impl $trait for Traced {
            type Output = Traced;
            fn $method(self, other: Traced) -> Traced {
                Traced::of(&format!(
                    concat!("{} ", $op, " {}"),
                    self.name(),
                    other.name()
                ))
            }
        }
    };
}
traced_op!(Add, add, "+");
traced_op!(Sub, sub, "-");
traced_op!(Mul, mul, "*");
traced_op!(Div, div, "/");

impl Neg for Traced {
    type Output = Traced;
    fn neg(self) -> Traced {
        Traced::of(&format!("-{}", self.name()))
    }
}

impl Num for Traced {
    fn lit(value: f32) -> Traced {
        Traced::of(&format!("{value:?}"))
    }
    fn min(self, other: Traced) -> Traced {
        Traced::of(&format!("min({}, {})", self.name(), other.name()))
    }
    fn max(self, other: Traced) -> Traced {
        Traced::of(&format!("max({}, {})", self.name(), other.name()))
    }
    fn sqrt(self) -> Traced {
        Traced::of(&format!("sqrt({})", self.name()))
    }
    fn pow(self, power: f32) -> Traced {
        Traced::of(&format!("pow({}, {power:?})", self.name()))
    }
    fn if_at_most<const N: usize>(
        self,
        limit: Traced,
        yes: impl FnOnce() -> [Traced; N],
        no: impl FnOnce() -> [Traced; N],
    ) -> [Traced; N] {
        let (yes, no) = (yes(), no());
        std::array::from_fn(|at| {
            Traced::of(&format!(
                "select({}, {}, {} <= {})",
                no[at].name(),
                yes[at].name(),
                self.name(),
                limit.name()
            ))
        })
    }
}

// ------------------------------------------------------------------------------------------ A grade

fn linear_to_srgb<N: Num>(v: N) -> N {
    let [out] = v.if_at_most(
        N::lit(0.003_130_8),
        || [v * N::lit(12.92)],
        || [N::lit(1.055) * v.pow(1.0 / 2.4) - N::lit(0.055)],
    );
    out
}

fn srgb_to_linear<N: Num>(v: N) -> N {
    let [out] = v.if_at_most(
        N::lit(0.04045),
        || [v / N::lit(12.92)],
        || [((v + N::lit(0.055)) / N::lit(1.055)).pow(2.4)],
    );
    out
}

/// What a color balance is told.
#[derive(Clone, Copy, Debug)]
pub struct Balance<N> {
    pub shadows: [N; 3],
    pub middles: [N; 3],
    pub highlights: [N; 3],
    /// 1 to keep each pixel's brightness, 0 not to.
    pub keep: N,
}

/// Color Balance of a premultiplied pixel.
#[inline(always)]
pub fn color_balance<N: Num>(p: [N; 4], b: &Balance<N>) -> [N; 4] {
    let a = p[3];
    a.if_at_most(
        N::lit(1e-6),
        || p,
        || {
            let zero = N::lit(0.0);
            let encode = |at: usize| linear_to_srgb((p[at] / a).max(zero));
            let e = [encode(0), encode(1), encode(2)];
            let luma =
                |c: [N; 3]| N::lit(0.2126) * c[0] + N::lit(0.7152) * c[1] + N::lit(0.0722) * c[2];
            let l = luma(e);
            let one = N::lit(1.0);
            let ws = (one - l) * (one - l);
            let wh = l * l;
            let wm = (one - ws - wh).max(zero);
            let o: [N; 3] = std::array::from_fn(|k| {
                e[k] + b.shadows[k] * ws + b.middles[k] * wm + b.highlights[k] * wh
            });
            let d = (l - luma(o)) * b.keep;
            let decode = |at: usize| srgb_to_linear((o[at] + d).clamp(zero, one)) * a;
            [decode(0), decode(1), decode(2), a]
        },
    )
}

// ------------------------------------------------------------------------------------------- A wipe

/// `smoothstep` of the transitions, which is defined where its two edges meet.
fn sstep<N: Num>(e0: N, e1: N, x: N) -> N {
    let (zero, one) = (N::lit(0.0), N::lit(1.0));
    let [out] = e1.if_at_most(
        e0,
        || e0.if_at_most(x, || [one], || [zero]),
        || {
            let t = ((x - e0) / (e1 - e0)).clamp(zero, one);
            [t * t * (N::lit(3.0) - N::lit(2.0) * t)]
        },
    );
    out
}

/// What the edge of a wipe is told.
#[derive(Clone, Copy, Debug)]
pub struct WipeEdge<N> {
    pub feather: N,
    pub border: N,
    pub border_color: [N; 4],
    pub aa: N,
}

/// A pixel of a wipe, `d` from the edge, inside below 0.
#[inline(always)]
pub fn wipe<N: Num>(e: &WipeEdge<N>, a: [N; 4], b: [N; 4], d: N) -> [N; 4] {
    let (zero, one) = (N::lit(0.0), N::lit(1.0));
    let s = (e.feather + e.aa) * N::lit(0.5);
    let kb = one - sstep(zero - s, s, d);
    let [ko] = e.border.if_at_most(
        zero,
        || [kb],
        || [one - sstep(e.border - s, e.border + s, d)],
    );
    let ko = ko.max(kb);
    std::array::from_fn(|k| a[k] * (one - ko) + e.border_color[k] * (ko - kb) + b[k] * kb)
}

/// An iris opening from a point.
#[inline(always)]
pub fn iris_round<N: Num>(x: N, y: N, cx: N, cy: N) -> N {
    let (dx, dy) = (x - cx, y - cy);
    (dx * dx + dy * dy).sqrt()
}

// ------------------------------------------------------------------------------------------- A blur

/// A row or column of pixels, read and written by a kernel that goes along it.
pub trait Line {
    type N: Num;
    type I: Copy + Add<Output = Self::I> + Sub<Output = Self::I>;
    type Sum;
    fn index(value: i32) -> Self::I;
    fn length(&self) -> Self::I;
    fn radius(&self) -> Self::I;
    fn inverse(&self) -> Self::N;
    /// Each of `from` up to `to`.
    fn each(&mut self, from: Self::I, to: Self::I, body: impl FnMut(&mut Self, Self::I));
    /// Whether a pixel counts: all do when ends repeat, else those on the line.
    fn when_counted(&mut self, at: Self::I, body: impl FnOnce(&mut Self));
    /// A pixel, its place held to the ends.
    fn read(&self, at: Self::I) -> [Self::N; 4];
    fn write(&mut self, at: Self::I, value: [Self::N; 4]);
    fn sum(&mut self) -> Self::Sum;
    fn add(&mut self, sum: &Self::Sum, value: [Self::N; 4]);
    fn take(&mut self, sum: &Self::Sum, value: [Self::N; 4]);
    fn value(&self, sum: &Self::Sum) -> [Self::N; 4];
}

/// A box blur along a line, for any radius at the cost of two pixels for each: a running sum.
#[inline(always)]
pub fn box_line<L: Line>(line: &mut L) {
    let r = line.radius();
    let n = line.length();
    let inverse = line.inverse();
    let one = L::index(1);
    let sum = line.sum();
    line.each(L::index(0) - r, r + one, |line, at| {
        line.when_counted(at, |line| {
            let value = line.read(at);
            line.add(&sum, value);
        });
    });
    line.each(L::index(0), n, |line, x| {
        let [a, b, c, d] = line.value(&sum);
        line.write(x, [a * inverse, b * inverse, c * inverse, d * inverse]);
        let leaving = x - r;
        line.when_counted(leaving, |line| {
            let value = line.read(leaving);
            line.take(&sum, value);
        });
        let coming = x + r + one;
        line.when_counted(coming, |line| {
            let value = line.read(coming);
            line.add(&sum, value);
        });
    });
}

/// A row of a picture on the CPU.
pub struct Row<'a> {
    pub from: &'a [f32],
    pub to: &'a mut [f32],
    pub width: isize,
    pub radius: isize,
    pub repeat: bool,
    /// The running sum: a kernel here has one, and on a plain array it goes four values at a time.
    pub sum: [f32; 4],
}

impl Line for Row<'_> {
    type N = f32;
    type I = isize;
    type Sum = ();
    #[inline(always)]
    fn index(value: i32) -> isize {
        value as isize
    }
    #[inline(always)]
    fn length(&self) -> isize {
        self.width
    }
    #[inline(always)]
    fn radius(&self) -> isize {
        self.radius
    }
    #[inline(always)]
    fn inverse(&self) -> f32 {
        1.0 / (2 * self.radius + 1) as f32
    }
    #[inline(always)]
    fn each(&mut self, from: isize, to: isize, mut body: impl FnMut(&mut Self, isize)) {
        for at in from..to {
            body(self, at);
        }
    }
    #[inline(always)]
    fn when_counted(&mut self, at: isize, body: impl FnOnce(&mut Self)) {
        if self.repeat || (at >= 0 && at < self.width) {
            body(self);
        }
    }
    #[inline(always)]
    fn read(&self, at: isize) -> [f32; 4] {
        let j = at.clamp(0, self.width - 1) as usize;
        match self.from.as_chunks::<4>().0.get(j) {
            Some(pixel) => *pixel,
            None => [0.0; 4],
        }
    }
    #[inline(always)]
    fn write(&mut self, at: isize, value: [f32; 4]) {
        if let Some(pixel) = self.to.as_chunks_mut::<4>().0.get_mut(at as usize) {
            *pixel = value;
        }
    }
    #[inline(always)]
    fn sum(&mut self) {
        self.sum = [0.0; 4];
    }
    #[inline(always)]
    fn add(&mut self, _: &(), value: [f32; 4]) {
        for (sum, value) in self.sum.iter_mut().zip(value) {
            *sum += value;
        }
    }
    #[inline(always)]
    fn take(&mut self, _: &(), value: [f32; 4]) {
        for (sum, value) in self.sum.iter_mut().zip(value) {
            *sum -= value;
        }
    }
    #[inline(always)]
    fn value(&self, _: &()) -> [f32; 4] {
        self.sum
    }
}

/// A line of a picture in the WGSL being written, reading `ld`, `line`, `n`, `r`, `repeat`, and `vertical`
/// from the entry point around it.
pub struct TracedLine;

/// A whole number in the WGSL being written.
#[derive(Clone, Copy, Debug)]
pub struct TracedIndex(u32);

impl TracedIndex {
    fn of(expression: &str) -> TracedIndex {
        let id = next();
        line(&format!("let k{id}: i32 = {expression};"));
        TracedIndex(id)
    }
}

impl Add for TracedIndex {
    type Output = TracedIndex;
    fn add(self, other: TracedIndex) -> TracedIndex {
        TracedIndex::of(&format!("k{} + k{}", self.0, other.0))
    }
}

impl Sub for TracedIndex {
    type Output = TracedIndex;
    fn sub(self, other: TracedIndex) -> TracedIndex {
        TracedIndex::of(&format!("k{} - k{}", self.0, other.0))
    }
}

impl Line for TracedLine {
    type N = Traced;
    type I = TracedIndex;
    type Sum = u32;
    fn index(value: i32) -> TracedIndex {
        TracedIndex::of(&value.to_string())
    }
    fn length(&self) -> TracedIndex {
        TracedIndex::of("n")
    }
    fn radius(&self) -> TracedIndex {
        TracedIndex::of("r")
    }
    fn inverse(&self) -> Traced {
        Traced::named("1.0 / f32(2 * r + 1)")
    }
    fn each(
        &mut self,
        from: TracedIndex,
        to: TracedIndex,
        mut body: impl FnMut(&mut Self, TracedIndex),
    ) {
        let id = next();
        line(&format!(
            "for (var c{id} = k{}; c{id} < k{}; c{id}++) {{",
            from.0, to.0
        ));
        let at = TracedIndex::of(&format!("c{id}"));
        body(self, at);
        line("}");
    }
    fn when_counted(&mut self, at: TracedIndex, body: impl FnOnce(&mut Self)) {
        line(&format!("if repeat || (k{0} >= 0 && k{0} < n) {{", at.0));
        body(self);
        line("}");
    }
    fn read(&self, at: TracedIndex) -> [Traced; 4] {
        let id = next();
        line(&format!(
            "let j{id} = clamp(k{}, 0, n - 1); let p{id} = ld(select(vec2(j{id}, line), vec2(line, j{id}), vertical));",
            at.0
        ));
        ["x", "y", "z", "w"].map(|field| Traced::named(&format!("p{id}.{field}")))
    }
    fn write(&mut self, at: TracedIndex, value: [Traced; 4]) {
        line(&format!(
            "textureStore(dst, select(vec2(k{0}, line), vec2(line, k{0}), vertical), vec4({1}, {2}, {3}, {4}));",
            at.0,
            value[0].name(),
            value[1].name(),
            value[2].name(),
            value[3].name()
        ));
    }
    fn sum(&mut self) -> u32 {
        let id = next();
        line(&format!("var s{id} = vec4(0.0);"));
        id
    }
    fn add(&mut self, sum: &u32, value: [Traced; 4]) {
        line(&format!(
            "s{sum} += vec4({}, {}, {}, {});",
            value[0].name(),
            value[1].name(),
            value[2].name(),
            value[3].name()
        ));
    }
    fn take(&mut self, sum: &u32, value: [Traced; 4]) {
        line(&format!(
            "s{sum} -= vec4({}, {}, {}, {});",
            value[0].name(),
            value[1].name(),
            value[2].name(),
            value[3].name()
        ));
    }
    fn value(&self, sum: &u32) -> [Traced; 4] {
        ["x", "y", "z", "w"].map(|field| Traced::named(&format!("s{sum}.{field}")))
    }
}

// --------------------------------------------------------------------------------------- Written out

/// What a kernel makes, returned from a function of WGSL.
fn returned(result: [Traced; 4]) -> String {
    format!(
        "    return vec4({}, {}, {}, {});\n",
        result[0].name(),
        result[1].name(),
        result[2].name(),
        result[3].name()
    )
}

/// The four numbers of a `vec4` in WGSL.
fn parts(vector: &str) -> [Traced; 4] {
    ["x", "y", "z", "w"].map(|field| Traced::named(&format!("{vector}.{field}")))
}

/// `color_balance` as `fn color_balance_px`, for `fx.wgsl`: it takes a premultiplied pixel, and its
/// numbers are the op's, shadows, middles, and highlights in `u.p0`, `u.p1`, and `u.p2`, and whether to
/// keep brightness in `u.i1.x`.
pub fn color_balance_wgsl() -> String {
    let mut result = None;
    let body = written(|| {
        let v = Traced::named;
        let balance = Balance {
            shadows: [v("u.p0.x"), v("u.p0.y"), v("u.p0.z")],
            middles: [v("u.p1.x"), v("u.p1.y"), v("u.p1.z")],
            highlights: [v("u.p2.x"), v("u.p2.y"), v("u.p2.z")],
            keep: v("f32(u.i1.x)"),
        };
        result = Some(color_balance(parts("q"), &balance));
    });
    format!(
        "fn color_balance_px(q: vec4<f32>) -> vec4<f32> {{\n{body}{}}}\n",
        returned(result.expect("written"))
    )
}

/// `box_line` as `fn fx_run`, for `fx.wgsl`: one row (`u.i1.z` 0) or column (1) for each invocation, its
/// radius in `u.i1.x`, and whether its ends repeat in `u.i1.y`.
pub fn box_line_wgsl() -> String {
    let body = written(|| box_line(&mut TracedLine));
    format!(
        "@compute @workgroup_size(64)\nfn fx_run(@builtin(global_invocation_id) id: vec3<u32>) {{\n    let s = size();\n    let vertical = u.i1.z != 0u;\n    let n = select(s.x, s.y, vertical);\n    let lines = select(s.y, s.x, vertical);\n    let line = i32(id.x);\n    if line >= lines {{\n        return;\n    }}\n    let r = i32(u.i1.x);\n    let repeat = u.i1.y != 0u;\n{body}}}\n"
    )
}

/// `wipe` and `iris_round` as functions of the same names, for the transitions' `kernels.wgsl`: the edge's
/// numbers are `n(0)`, the feather, `n(1)`, the border, `n(2)`, the anti-aliasing, and its color from `n(4)`.
pub fn wipe_wgsl() -> String {
    let mut result = None;
    let body = written(|| {
        let v = Traced::named;
        let edge = WipeEdge {
            feather: v("n(0u)"),
            border: v("n(1u)"),
            aa: v("n(2u)"),
            border_color: [v("n(4u)"), v("n(5u)"), v("n(6u)"), v("n(7u)")],
        };
        result = Some(wipe(&edge, parts("a"), parts("b"), v("d")));
    });
    let wipe_px = format!(
        "fn wipe_px(a: vec4<f32>, b: vec4<f32>, d: f32) -> vec4<f32> {{\n{body}{}}}\n",
        returned(result.expect("written"))
    );
    let mut result = None;
    let body = written(|| {
        let v = Traced::named;
        result = Some(iris_round(v("x"), v("y"), v("cx"), v("cy")));
    });
    format!(
        "{wipe_px}fn iris_round(x: f32, y: f32, cx: f32, cy: f32) -> f32 {{\n{body}    return {};\n}}\n",
        result.expect("written").name()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A number between `low` and `high` that is the same each time.
    fn scatter(seed: u32, low: f32, high: f32) -> f32 {
        let mut h = seed.wrapping_mul(0x9E37_79B9);
        h ^= h >> 15;
        h = h.wrapping_mul(0x846C_A68B);
        h ^= h >> 13;
        low + (high - low) * (h as f32 / u32::MAX as f32)
    }

    fn pixels(count: u32, seed: u32) -> Vec<[f32; 4]> {
        (0..count)
            .map(|at| {
                let k = at.wrapping_mul(7).wrapping_add(seed);
                // Some transparent, some clear, some over 1.
                let a = match at % 7 {
                    0 => 0.0,
                    1 => 1e-7,
                    2 => 0.5,
                    _ => 1.0,
                };
                let c = |i: u32| scatter(k + i, -0.2, 1.6) * a;
                [c(1), c(2), c(3), a]
            })
            .collect()
    }

    // ---- Each as it was written by hand for the CPU, down to its bits.

    fn srgb_decode(v: f32) -> f32 {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }

    fn srgb_encode(v: f32) -> f32 {
        if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    }

    fn balance_by_hand(
        p: [f32; 4],
        sh: [f32; 3],
        md: [f32; 3],
        hi: [f32; 3],
        preserve: bool,
    ) -> [f32; 4] {
        let alpha = p[3];
        if alpha <= 1e-6 {
            return p;
        }
        let c = [p[0] / alpha, p[1] / alpha, p[2] / alpha].map(|v| srgb_encode(v.max(0.0)));
        let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let ws = (1.0 - l).powi(2);
        let wh = l.powi(2);
        let wm = 1.0 - ws - wh;
        let mut o = [0.0; 3];
        for k in 0..3 {
            o[k] = c[k] + sh[k] * ws + md[k] * wm.max(0.0) + hi[k] * wh;
        }
        if preserve {
            let l2 = 0.2126 * o[0] + 0.7152 * o[1] + 0.0722 * o[2];
            let d = l - l2;
            o = o.map(|v| v + d);
        }
        let [r, g, b] = o.map(|v| srgb_decode(v.clamp(0.0, 1.0)));
        [r * alpha, g * alpha, b * alpha, alpha]
    }

    fn smoothstep_by_hand(e0: f32, e1: f32, x: f32) -> f32 {
        if e1 <= e0 {
            return if x < e0 { 0.0 } else { 1.0 };
        }
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    fn wipe_by_hand(e: &WipeEdge<f32>, a: [f32; 4], b: [f32; 4], d: f32) -> [f32; 4] {
        let s = (e.feather + e.aa) * 0.5;
        let kb = 1.0 - smoothstep_by_hand(-s, s, d);
        let ko = if e.border > 0.0 {
            1.0 - smoothstep_by_hand(e.border - s, e.border + s, d)
        } else {
            kb
        };
        let ko = ko.max(kb);
        let mut o = [0.0; 4];
        for k in 0..4 {
            o[k] = a[k] * (1.0 - ko) + e.border_color[k] * (ko - kb) + b[k] * kb;
        }
        o
    }

    fn box_by_hand(from: &[f32], w: usize, r: usize, repeat: bool) -> Vec<f32> {
        let mut row = vec![0.0; from.len()];
        let inv = 1.0 / (2 * r + 1) as f32;
        let fetch = |i: isize| {
            if repeat {
                i.clamp(0, w as isize - 1) as usize
            } else {
                i as usize
            }
        };
        let mut acc = [0f32; 4];
        for i in -(r as isize)..=(r as isize) {
            if repeat || (i >= 0 && (i as usize) < w) {
                let j = fetch(i);
                for k in 0..4 {
                    acc[k] += from[j * 4 + k];
                }
            }
        }
        for x in 0..w {
            for k in 0..4 {
                row[x * 4 + k] = acc[k] * inv;
            }
            let out_i = x as isize - r as isize;
            let in_i = x as isize + r as isize + 1;
            if repeat || out_i >= 0 {
                let j = fetch(out_i);
                for k in 0..4 {
                    acc[k] -= from[j * 4 + k];
                }
            }
            if repeat || (in_i as usize) < w {
                let j = fetch(in_i);
                for k in 0..4 {
                    acc[k] += from[j * 4 + k];
                }
            }
        }
        row
    }

    #[test]
    fn a_grade_is_on_the_cpu_what_it_was_by_hand() {
        for (seed, preserve) in [(1, true), (2, false), (3, true), (4, false)] {
            let n = |at: u32| scatter(seed * 100 + at, -0.3, 0.3);
            let (sh, md, hi) = ([n(0), n(1), n(2)], [n(3), n(4), n(5)], [n(6), n(7), n(8)]);
            let told = Balance {
                shadows: sh,
                middles: md,
                highlights: hi,
                keep: if preserve { 1.0 } else { 0.0 },
            };
            for pixel in pixels(5000, seed) {
                let now = color_balance(pixel, &told);
                let was = balance_by_hand(pixel, sh, md, hi, preserve);
                // (equal rather than the same bits: a zero may come out with another sign)
                assert_eq!(now, was, "{pixel:?}");
            }
        }
    }

    #[test]
    fn a_wipe_is_on_the_cpu_what_it_was_by_hand() {
        for (feather, border, aa) in [
            (0.0, 0.0, 0.0),
            (0.0, 0.0, 1.0),
            (8.0, 0.0, 1.0),
            (4.0, 6.0, 2.5),
            (0.0, 3.0, 0.0),
        ] {
            let edge = WipeEdge {
                feather,
                border,
                border_color: [0.9, 0.5, 0.1, 1.0],
                aa,
            };
            let (a, b) = (pixels(2000, 11), pixels(2000, 12));
            for (at, (a, b)) in a.iter().zip(&b).enumerate() {
                let d = at as f32 / 100.0 - 10.0;
                let now = wipe(&edge, *a, *b, d);
                let was = wipe_by_hand(&edge, *a, *b, d);
                assert_eq!(
                    now.map(f32::to_bits),
                    was.map(f32::to_bits),
                    "{feather} {border} {aa} at {d}"
                );
            }
        }
        assert_eq!(iris_round(7.0f32, 9.0, 4.0, 5.0), 5.0);
    }

    #[test]
    fn a_blur_is_on_the_cpu_what_it_was_by_hand() {
        let w = 37;
        let from: Vec<f32> = pixels(w as u32, 21).into_iter().flatten().collect();
        for r in [0, 1, 3, 8, 36, 50] {
            for repeat in [true, false] {
                let mut to = vec![0.0; from.len()];
                let mut row = Row {
                    from: &from,
                    to: &mut to,
                    width: w as isize,
                    radius: r as isize,
                    repeat,
                    sum: [0.0; 4],
                };
                box_line(&mut row);
                let was = box_by_hand(&from, w, r, repeat);
                let bits = |values: &[f32]| values.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
                assert_eq!(bits(&to), bits(&was), "radius {r}, repeat {repeat}");
            }
        }
    }

    /// What `fx.wgsl` gives what is written out for it.
    const FX: &str = "
struct U { i0: vec4<u32>, i1: vec4<u32>, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba32float, write>;
fn size() -> vec2<i32> { return vec2<i32>(textureDimensions(src)); }
fn ld(p: vec2<i32>) -> vec4<f32> { return textureLoad(src, p, 0); }
";

    /// And what the transitions' `kernels.wgsl` does.
    const TRANSITIONS: &str = "
struct U { v: array<vec4<f32>, 200> };
@group(0) @binding(0) var<uniform> u: U;
fn n(i: u32) -> f32 { return u.v[i / 4u][i % 4u]; }
";

    fn valid(source: &str) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|error| panic!("{source}\n{}", error.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|error| panic!("{source}\n{error:?}"));
    }

    /// No GPU is needed to know that what is written out is WGSL, and that it reads what it says.
    #[test]
    fn what_is_written_out_is_wgsl() {
        valid(&format!("{FX}{}{}", color_balance_wgsl(), box_line_wgsl()));
        valid(&format!("{TRANSITIONS}{}", wipe_wgsl()));
    }

    #[test]
    fn writing_out_the_same_twice_comes_to_the_same() {
        assert_eq!(color_balance_wgsl(), color_balance_wgsl());
        assert_eq!(box_line_wgsl(), box_line_wgsl());
        assert_eq!(wipe_wgsl(), wipe_wgsl());
    }

    /// A kernel's names may not be taken for WGSL's own, as `i32` once was.
    #[test]
    fn names_written_out_are_no_type_of_wgsl() {
        for written in [color_balance_wgsl(), box_line_wgsl(), wipe_wgsl()] {
            for shadowed in ["let i32", "let f32", "let u32", "var i32", "var f32"] {
                assert!(!written.contains(shadowed), "{shadowed}");
            }
        }
    }
}
