//! A WGSL kernel written out as OpenCL C: one entry point of a module naga has read and checked,
//! with the functions it reaches.
//!
//! Only what Kerf's kernels use is written. Anything else is [`Unsupported`], said by name, and
//! the kernel then does not build, which sends its frames to the CPU as a kernel that failed to
//! build on any GPU does.
//!
//! A texture is a buffer of its texels and a few numbers ([`crate::format`]), so a load is an
//! index into it, held inside the picture, and a store outside it is not made. A uniform or a
//! storage buffer is its bytes, read at the offsets naga lays its type out at, so nothing rests on
//! how OpenCL C would pack a struct.

use crate::format::Texel;
use naga::proc::{Layouter, TypeResolution};
use naga::valid::{FunctionInfo, ModuleInfo};
use naga::{
    AddressSpace, ArraySize, BinaryOperator, Binding, Block, BuiltIn, Expression, Function,
    GlobalVariable, Handle, ImageClass, ImageDimension, ImageQuery, Literal, MathFunction, Module,
    RelationalFunction, Scalar, ScalarKind, Statement, StorageFormat, SwitchValue, Type, TypeInner,
    UnaryOperator, VectorSize,
};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// What a kernel uses that is not written out here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported(pub String);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "not written out as OpenCL C: {}", self.0)
    }
}

impl std::error::Error for Unsupported {}

type Result<T> = std::result::Result<T, Unsupported>;

fn unsupported<T>(what: impl Into<String>) -> Result<T> {
    Err(Unsupported(what.into()))
}

/// What a kernel reads or writes that is bound to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    /// A uniform or storage buffer: its bytes, and how many there are. It holds at least `least`.
    Buffer { least: u32 },
    /// A texture: its texels, and its width, height and kind of texel.
    Texture,
}

/// One of a kernel's bindings, in the order its arguments take them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resource {
    pub group: u32,
    pub binding: u32,
    pub bound: Bound,
}

/// A kernel as OpenCL C.
#[derive(Debug, Clone)]
pub struct Translated {
    pub source: String,
    /// The kernel's name in `source`.
    pub kernel: String,
    pub workgroup: [u32; 3],
    /// Two arguments each: a buffer's bytes and their count, or a texture's texels and its
    /// `(width, height, texel, 0)`.
    pub resources: Vec<Resource>,
}

/// `entry` of `module` as OpenCL C. With `exact`, quotients, roots and powers go through doubles.
pub fn translate(
    module: &Module,
    info: &ModuleInfo,
    entry: &str,
    exact: bool,
) -> Result<Translated> {
    let Some(index) = module
        .entry_points
        .iter()
        .position(|point| point.name == entry)
    else {
        return unsupported(format!("no entry point `{entry}`"));
    };
    let point = &module.entry_points[index];
    if point.stage != naga::ShaderStage::Compute {
        return unsupported("an entry point that is no compute kernel");
    }
    let mut layouter = Layouter::default();
    layouter
        .update(module.to_ctx())
        .map_err(|error| Unsupported(format!("a type with no layout: {error}")))?;
    let mut writer = Writer {
        module,
        info,
        layouter,
        out: String::new(),
        gates: 0,
    };
    let resources = writer.resources()?;
    let mut source = String::new();
    if exact {
        source.push_str("#define KERF_EXACT 1\n");
    }
    source.push_str(include_str!("prelude.cl"));
    writer.types()?;
    writer.context(&resources);
    let reached = reached(module, &point.function);
    for &handle in &reached {
        let function = &module.functions[handle];
        let head = writer.head(handle, function)?;
        let _ = writeln!(writer.out, "{head};");
    }
    for &handle in &reached {
        writer.function(handle)?;
    }
    writer.entry(index, &resources)?;
    source.push_str(&writer.out);
    Ok(Translated {
        source,
        kernel: "kerf_kernel".to_owned(),
        workgroup: point.workgroup_size,
        resources: resources.iter().map(|(_, resource)| *resource).collect(),
    })
}

/// The functions `from` calls, and those they call, each after what it calls.
fn reached(module: &Module, from: &Function) -> Vec<Handle<Function>> {
    fn calls(block: &Block, found: &mut Vec<Handle<Function>>) {
        for statement in block.iter() {
            match statement {
                Statement::Call { function, .. } => found.push(*function),
                Statement::Block(block) => calls(block, found),
                Statement::If { accept, reject, .. } => {
                    calls(accept, found);
                    calls(reject, found);
                }
                Statement::Switch { cases, .. } => {
                    for case in cases {
                        calls(&case.body, found);
                    }
                }
                Statement::Loop {
                    body, continuing, ..
                } => {
                    calls(body, found);
                    calls(continuing, found);
                }
                _ => {}
            }
        }
    }
    fn visit(
        module: &Module,
        function: &Function,
        seen: &mut BTreeSet<usize>,
        order: &mut Vec<Handle<Function>>,
    ) {
        let mut found = Vec::new();
        calls(&function.body, &mut found);
        for handle in found {
            if seen.insert(handle.index()) {
                visit(module, &module.functions[handle], seen, order);
                order.push(handle);
            }
        }
    }
    let mut order = Vec::new();
    visit(module, from, &mut BTreeSet::new(), &mut order);
    order
}

/// Where a pointer points.
enum Place {
    /// A variable of the function, or a part of one: what C calls it.
    Variable(String),
    /// Bytes of a buffer: the buffer, how many bytes it holds, and how far in, as C.
    Bytes {
        buffer: String,
        offset: String,
        writable: bool,
    },
}

/// A type as a value is of it: one in the module, one an expression has without the module
/// holding it, or a number or a vector of them, as a part of a vector or of a matrix is.
#[derive(Clone)]
enum Shape<'a> {
    Of(Handle<Type>),
    Inner(&'a TypeInner),
    Number(Scalar, Option<VectorSize>),
}

struct Writer<'a> {
    module: &'a Module,
    info: &'a ModuleInfo,
    layouter: Layouter,
    out: String,
    /// How many loops have been written, each with a name of its own for what it is told by.
    gates: usize,
}

/// A function being written.
struct Scope<'a> {
    function: &'a Function,
    info: &'a FunctionInfo,
    /// The expressions that have a name of their own by now.
    baked: Vec<bool>,
}

fn size_of(size: VectorSize) -> usize {
    size as usize
}

const PARTS: [char; 4] = ['x', 'y', 'z', 'w'];

fn scalar_name(scalar: Scalar) -> Result<&'static str> {
    match (scalar.kind, scalar.width) {
        (ScalarKind::Float, 4) => Ok("float"),
        (ScalarKind::Sint, 4) => Ok("int"),
        (ScalarKind::Uint, 4) => Ok("uint"),
        (ScalarKind::Bool, _) => Ok("bool"),
        (kind, width) => unsupported(format!("a number of kind {kind:?}, {width} bytes wide")),
    }
}

/// A vector's type. One of booleans is one of whole numbers, all of whose bits are set where it
/// is true, as OpenCL C compares vectors.
fn vector_name(size: VectorSize, scalar: Scalar) -> Result<String> {
    let base = match scalar.kind {
        ScalarKind::Bool => "int",
        _ => scalar_name(scalar)?,
    };
    Ok(format!("{base}{}", size_of(size)))
}

/// The letter a number's kind has in the prelude's names.
fn kind_letter(scalar: Scalar) -> Result<char> {
    match (scalar.kind, scalar.width) {
        (ScalarKind::Float, 4) => Ok('f'),
        (ScalarKind::Sint, 4) => Ok('i'),
        (ScalarKind::Uint, 4) => Ok('u'),
        (kind, width) => unsupported(format!(
            "a number of kind {kind:?}, {width} bytes wide, in a buffer"
        )),
    }
}

fn float_literal(value: f32) -> String {
    if value.is_nan() {
        "NAN".to_owned()
    } else if value.is_infinite() {
        if value > 0.0 {
            "INFINITY"
        } else {
            "(-INFINITY)"
        }
        .to_owned()
    } else {
        format!("{value:?}f")
    }
}

fn literal(value: Literal) -> Result<String> {
    Ok(match value {
        Literal::F32(value) => float_literal(value),
        Literal::I32(i32::MIN) => "(-2147483647 - 1)".to_owned(),
        Literal::I32(value) => format!("{value}"),
        Literal::U32(value) => format!("{value}u"),
        Literal::Bool(value) => format!("{value}"),
        other => return unsupported(format!("the literal {other:?}")),
    })
}

fn storage_texel(format: StorageFormat) -> Result<(Texel, &'static str)> {
    Ok(match format {
        StorageFormat::Rgba32Float => (Texel::Rgba32Float, "rgba32float"),
        StorageFormat::R32Float => (Texel::R32Float, "r32float"),
        StorageFormat::Rg32Float => (Texel::Rg32Float, "rg32float"),
        StorageFormat::Rgba8Unorm => (Texel::Rgba8Unorm, "rgba8unorm"),
        StorageFormat::Rgba16Uint => (Texel::Rgba16Uint, "rgba16uint"),
        other => return unsupported(format!("a storage texture of {other:?}")),
    })
}

impl<'a> Writer<'a> {
    fn inner(&self, shape: &Shape<'a>) -> Cow<'a, TypeInner> {
        match shape {
            Shape::Of(handle) => Cow::Borrowed(&self.module.types[*handle].inner),
            Shape::Inner(inner) => Cow::Borrowed(inner),
            Shape::Number(scalar, None) => Cow::Owned(TypeInner::Scalar(*scalar)),
            Shape::Number(scalar, Some(size)) => Cow::Owned(TypeInner::Vector {
                size: *size,
                scalar: *scalar,
            }),
        }
    }

    fn shape_of(&self, resolution: &'a TypeResolution) -> Shape<'a> {
        match resolution {
            TypeResolution::Handle(handle) => Shape::Of(*handle),
            TypeResolution::Value(inner) => Shape::Inner(inner),
        }
    }

    /// What C calls a value's type.
    fn c_type(&self, shape: &Shape<'a>) -> Result<String> {
        match (&*self.inner(shape), shape) {
            (TypeInner::Scalar(scalar), _) => Ok(scalar_name(*scalar)?.to_owned()),
            (TypeInner::Vector { size, scalar }, _) => vector_name(*size, *scalar),
            (TypeInner::Struct { .. }, Shape::Of(handle)) if self.has_value(*handle) => {
                Ok(format!("struct kerf_s{}", handle.index()))
            }
            (TypeInner::Array { .. }, Shape::Of(handle)) if self.has_value(*handle) => {
                Ok(format!("struct kerf_a{}", handle.index()))
            }
            (TypeInner::Image { .. }, _) => Ok("kerf_tex".to_owned()),
            (other, _) => unsupported(format!("a value of type {other:?}")),
        }
    }

    /// Whether a type is one a C variable can be of: numbers, and structs and arrays of a known
    /// length made of them.
    fn has_value(&self, handle: Handle<Type>) -> bool {
        match &self.module.types[handle].inner {
            TypeInner::Scalar(scalar) => scalar_name(*scalar).is_ok(),
            TypeInner::Vector { scalar, .. } => scalar_name(*scalar).is_ok(),
            TypeInner::Struct { members, .. } => {
                members.iter().all(|member| self.has_value(member.ty))
            }
            TypeInner::Array {
                base,
                size: ArraySize::Constant(_),
                ..
            } => self.has_value(*base),
            _ => false,
        }
    }

    /// The module's bindings, by group and binding, each with the global it is.
    fn resources(&self) -> Result<Vec<(Handle<GlobalVariable>, Resource)>> {
        let mut found = Vec::new();
        for (handle, global) in self.module.global_variables.iter() {
            let Some(binding) = &global.binding else {
                match global.space {
                    AddressSpace::Private | AddressSpace::WorkGroup => {
                        return unsupported("a variable of the module's own");
                    }
                    _ => continue,
                }
            };
            let bound = match (&global.space, &self.module.types[global.ty].inner) {
                (AddressSpace::Uniform | AddressSpace::Storage { .. }, _) => Bound::Buffer {
                    least: self.layouter[global.ty].size,
                },
                (
                    AddressSpace::Handle,
                    TypeInner::Image {
                        dim: ImageDimension::D2,
                        arrayed: false,
                        class,
                    },
                ) => {
                    match class {
                        ImageClass::Sampled { multi: false, .. } => {}
                        ImageClass::Storage { format, .. } => {
                            storage_texel(*format)?;
                        }
                        other => return unsupported(format!("a texture of class {other:?}")),
                    }
                    Bound::Texture
                }
                (space, inner) => {
                    return unsupported(format!("a binding in {space:?} of type {inner:?}"));
                }
            };
            found.push((
                handle,
                Resource {
                    group: binding.group,
                    binding: binding.binding,
                    bound,
                },
            ));
        }
        found.sort_by_key(|(_, resource)| (resource.group, resource.binding));
        Ok(found)
    }

    /// The module's structs and arrays that a value can be of, each as a C struct.
    fn types(&mut self) -> Result<()> {
        for (handle, ty) in self.module.types.iter() {
            if !self.has_value(handle) {
                continue;
            }
            match &ty.inner {
                TypeInner::Struct { members, .. } => {
                    let _ = writeln!(self.out, "struct kerf_s{} {{", handle.index());
                    for (index, member) in members.iter().enumerate() {
                        let of = self.c_type(&Shape::Of(member.ty))?;
                        let _ = writeln!(self.out, "    {of} m{index};");
                    }
                    self.out.push_str("};\n");
                }
                TypeInner::Array {
                    base,
                    size: ArraySize::Constant(count),
                    ..
                } => {
                    let of = self.c_type(&Shape::Of(*base))?;
                    let _ = writeln!(
                        self.out,
                        "struct kerf_a{} {{ {of} a[{count}]; }};",
                        handle.index()
                    );
                }
                _ => {}
            }
        }
        // And each as its bytes in a buffer, read and written.
        for (handle, ty) in self.module.types.iter() {
            if !self.has_value(handle) || !self.in_buffers(handle) {
                continue;
            }
            let (index, name) = (handle.index(), self.c_type(&Shape::Of(handle))?);
            match &ty.inner {
                TypeInner::Struct { members, .. } => {
                    let _ = writeln!(
                        self.out,
                        "static {name} kerf_ld{index}(__global const uchar* p, uint o) {{\n    {name} v;"
                    );
                    for (at, member) in members.iter().enumerate() {
                        let read = self.read(
                            &Shape::Of(member.ty),
                            "p",
                            &format!("o + {}u", member.offset),
                        )?;
                        let _ = writeln!(self.out, "    v.m{at} = {read};");
                    }
                    self.out.push_str("    return v;\n}\n");
                    let _ = writeln!(
                        self.out,
                        "static void kerf_st{index}(__global uchar* p, uint o, {name} v) {{"
                    );
                    for (at, member) in members.iter().enumerate() {
                        let write = self.write(
                            &Shape::Of(member.ty),
                            "p",
                            &format!("o + {}u", member.offset),
                            &format!("v.m{at}"),
                        )?;
                        let _ = writeln!(self.out, "    {write};");
                    }
                    self.out.push_str("}\n");
                }
                TypeInner::Array {
                    base,
                    size: ArraySize::Constant(count),
                    stride,
                } => {
                    let read = self.read(&Shape::Of(*base), "p", &format!("o + i * {stride}u"))?;
                    let _ = writeln!(
                        self.out,
                        "static {name} kerf_ld{index}(__global const uchar* p, uint o) {{\n    {name} v;\n    for (uint i = 0u; i < {count}u; i++) v.a[i] = {read};\n    return v;\n}}"
                    );
                    let write = self.write(
                        &Shape::Of(*base),
                        "p",
                        &format!("o + i * {stride}u"),
                        "v.a[i]",
                    )?;
                    let _ = writeln!(
                        self.out,
                        "static void kerf_st{index}(__global uchar* p, uint o, {name} v) {{\n    for (uint i = 0u; i < {count}u; i++) {write};\n}}"
                    );
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Whether a type is one a buffer holds: no boolean is.
    fn in_buffers(&self, handle: Handle<Type>) -> bool {
        match &self.module.types[handle].inner {
            TypeInner::Scalar(scalar) | TypeInner::Vector { scalar, .. } => {
                kind_letter(*scalar).is_ok()
            }
            TypeInner::Struct { members, .. } => {
                members.iter().all(|member| self.in_buffers(member.ty))
            }
            TypeInner::Array { base, .. } => self.in_buffers(*base),
            _ => false,
        }
    }

    /// A value of `shape` read from `buffer`, `offset` bytes in.
    fn read(&self, shape: &Shape<'a>, buffer: &str, offset: &str) -> Result<String> {
        match (&*self.inner(shape), shape) {
            (TypeInner::Scalar(scalar), _) => Ok(format!(
                "kerf_rd_{}1({buffer}, {offset})",
                kind_letter(*scalar)?
            )),
            (TypeInner::Vector { size, scalar }, _) => Ok(format!(
                "kerf_rd_{}{}({buffer}, {offset})",
                kind_letter(*scalar)?,
                size_of(*size)
            )),
            (TypeInner::Struct { .. } | TypeInner::Array { .. }, Shape::Of(handle))
                if self.has_value(*handle) && self.in_buffers(*handle) =>
            {
                Ok(format!("kerf_ld{}({buffer}, {offset})", handle.index()))
            }
            (other, _) => unsupported(format!("reading {other:?} from a buffer")),
        }
    }

    /// `value` of `shape` written into `buffer`, `offset` bytes in.
    fn write(&self, shape: &Shape<'a>, buffer: &str, offset: &str, value: &str) -> Result<String> {
        match (&*self.inner(shape), shape) {
            (TypeInner::Scalar(scalar), _) => Ok(format!(
                "kerf_wr_{}1({buffer}, {offset}, {value})",
                kind_letter(*scalar)?
            )),
            (TypeInner::Vector { size, scalar }, _) => Ok(format!(
                "kerf_wr_{}{}({buffer}, {offset}, {value})",
                kind_letter(*scalar)?,
                size_of(*size)
            )),
            (TypeInner::Struct { .. } | TypeInner::Array { .. }, Shape::Of(handle))
                if self.has_value(*handle) && self.in_buffers(*handle) =>
            {
                Ok(format!(
                    "kerf_st{}({buffer}, {offset}, {value})",
                    handle.index()
                ))
            }
            (other, _) => unsupported(format!("writing {other:?} into a buffer")),
        }
    }

    /// What every function is handed: the kernel's bindings.
    fn context(&mut self, resources: &[(Handle<GlobalVariable>, Resource)]) {
        self.out.push_str("typedef struct {\n");
        for (handle, resource) in resources {
            let index = handle.index();
            match resource.bound {
                Bound::Buffer { .. } => {
                    let _ = writeln!(
                        self.out,
                        "    __global uchar* g{index};\n    uint g{index}_len;"
                    );
                }
                Bound::Texture => {
                    let _ = writeln!(self.out, "    kerf_tex g{index};");
                }
            }
        }
        // C has no struct of nothing.
        self.out.push_str("    int none;\n} kerf_ctx;\n");
    }

    fn head(&self, handle: Handle<Function>, function: &Function) -> Result<String> {
        let result = match &function.result {
            Some(result) => self.c_type(&Shape::Of(result.ty))?,
            None => "void".to_owned(),
        };
        let mut head = format!(
            "static {result} kerf_f{}(const kerf_ctx* ctx",
            handle.index()
        );
        for (index, argument) in function.arguments.iter().enumerate() {
            if matches!(
                self.module.types[argument.ty].inner,
                TypeInner::Pointer { .. } | TypeInner::ValuePointer { .. }
            ) {
                return unsupported("a function that takes a pointer");
            }
            let of = self.c_type(&Shape::Of(argument.ty))?;
            let _ = write!(head, ", {of} a{index}");
        }
        head.push(')');
        Ok(head)
    }

    fn function(&mut self, handle: Handle<Function>) -> Result<()> {
        let function = &self.module.functions[handle];
        let head = self.head(handle, function)?;
        let name = function.name.as_deref().unwrap_or("");
        let _ = writeln!(self.out, "// {name}\n{head} {{");
        let mut scope = Scope {
            function,
            info: &self.info[handle],
            baked: vec![false; function.expressions.len()],
        };
        self.locals(&mut scope)?;
        self.block(&mut scope, &function.body, 1)?;
        self.out.push_str("}\n");
        Ok(())
    }

    fn entry(
        &mut self,
        index: usize,
        resources: &[(Handle<GlobalVariable>, Resource)],
    ) -> Result<()> {
        let point = &self.module.entry_points[index];
        let function = &point.function;
        self.out.push_str("__kernel void kerf_kernel(");
        let mut arguments = Vec::new();
        for (handle, resource) in resources {
            let index = handle.index();
            arguments.push(match resource.bound {
                Bound::Buffer { .. } => format!("__global uchar* g{index}, uint g{index}_len"),
                Bound::Texture => format!("__global uchar* g{index}, int4 g{index}_info"),
            });
        }
        if arguments.is_empty() {
            arguments.push("int none".to_owned());
        }
        self.out.push_str(&arguments.join(", "));
        self.out
            .push_str(") {\n    kerf_ctx context;\n    context.none = 0;\n");
        for (handle, resource) in resources {
            let index = handle.index();
            match resource.bound {
                Bound::Buffer { .. } => {
                    let _ = writeln!(
                        self.out,
                        "    context.g{index} = g{index};\n    context.g{index}_len = g{index}_len;"
                    );
                }
                Bound::Texture => {
                    let _ = writeln!(
                        self.out,
                        "    context.g{index} = kerf_tex_of(g{index}, g{index}_info);"
                    );
                }
            }
        }
        self.out.push_str("    const kerf_ctx* ctx = &context;\n");
        for (index, argument) in function.arguments.iter().enumerate() {
            let value = match argument.binding {
                Some(Binding::BuiltIn(BuiltIn::GlobalInvocationId)) => {
                    "(uint3)((uint)get_global_id(0), (uint)get_global_id(1), (uint)get_global_id(2))"
                }
                Some(Binding::BuiltIn(BuiltIn::LocalInvocationId)) => {
                    "(uint3)((uint)get_local_id(0), (uint)get_local_id(1), (uint)get_local_id(2))"
                }
                Some(Binding::BuiltIn(BuiltIn::WorkGroupId)) => {
                    "(uint3)((uint)get_group_id(0), (uint)get_group_id(1), (uint)get_group_id(2))"
                }
                Some(Binding::BuiltIn(BuiltIn::NumWorkGroups)) => {
                    "(uint3)((uint)get_num_groups(0), (uint)get_num_groups(1), (uint)get_num_groups(2))"
                }
                ref other => return unsupported(format!("a kernel handed {other:?}")),
            };
            let _ = writeln!(self.out, "    const uint3 a{index} = {value};");
        }
        let mut scope = Scope {
            function,
            info: self.info.get_entry_point(index),
            baked: vec![false; function.expressions.len()],
        };
        self.locals(&mut scope)?;
        self.block(&mut scope, &function.body, 1)?;
        self.out.push_str("}\n");
        Ok(())
    }

    fn locals(&mut self, scope: &mut Scope<'a>) -> Result<()> {
        for (handle, local) in scope.function.local_variables.iter() {
            let shape = Shape::Of(local.ty);
            let of = self.c_type(&shape)?;
            let init = match local.init {
                Some(init) => self.value(scope, init)?,
                None => self.zero(&shape)?,
            };
            let _ = writeln!(self.out, "    {of} l{} = {init};", handle.index());
        }
        Ok(())
    }

    fn zero(&self, shape: &Shape<'a>) -> Result<String> {
        Ok(match &*self.inner(shape) {
            TypeInner::Scalar(scalar) => match scalar.kind {
                ScalarKind::Float => "0.0f".to_owned(),
                ScalarKind::Uint => "0u".to_owned(),
                ScalarKind::Bool => "false".to_owned(),
                _ => "0".to_owned(),
            },
            TypeInner::Vector { size, scalar } => {
                let of = vector_name(*size, *scalar)?;
                let zero = match scalar.kind {
                    ScalarKind::Float => "0.0f",
                    ScalarKind::Uint => "0u",
                    _ => "0",
                };
                format!("(({of})({zero}))")
            }
            TypeInner::Struct { .. } | TypeInner::Array { .. } => {
                format!("(({}){{0}})", self.c_type(shape)?)
            }
            other => return unsupported(format!("nothing of type {other:?}")),
        })
    }

    fn indent(&mut self, depth: usize) {
        for _ in 0..depth {
            self.out.push_str("    ");
        }
    }

    fn block(&mut self, scope: &mut Scope<'a>, block: &'a Block, depth: usize) -> Result<()> {
        for statement in block.iter() {
            self.statement(scope, statement, depth)?;
        }
        Ok(())
    }

    fn statement(
        &mut self,
        scope: &mut Scope<'a>,
        statement: &'a Statement,
        depth: usize,
    ) -> Result<()> {
        match statement {
            Statement::Emit(range) => {
                for handle in range.clone() {
                    self.bake(scope, handle, depth)?;
                }
            }
            Statement::Block(block) => {
                self.indent(depth);
                self.out.push_str("{\n");
                self.block(scope, block, depth + 1)?;
                self.indent(depth);
                self.out.push_str("}\n");
            }
            Statement::If {
                condition,
                accept,
                reject,
            } => {
                let condition = self.value(scope, *condition)?;
                self.indent(depth);
                let _ = writeln!(self.out, "if ({condition}) {{");
                self.block(scope, accept, depth + 1)?;
                if !reject.is_empty() {
                    self.indent(depth);
                    self.out.push_str("} else {\n");
                    self.block(scope, reject, depth + 1)?;
                }
                self.indent(depth);
                self.out.push_str("}\n");
            }
            Statement::Switch { selector, cases } => {
                let selector = self.value(scope, *selector)?;
                self.indent(depth);
                let _ = writeln!(self.out, "switch ({selector}) {{");
                for case in cases {
                    self.indent(depth);
                    match case.value {
                        SwitchValue::I32(value) => {
                            let _ =
                                writeln!(self.out, "case {}: {{", literal(Literal::I32(value))?);
                        }
                        SwitchValue::U32(value) => {
                            let _ = writeln!(self.out, "case {value}u: {{");
                        }
                        SwitchValue::Default => self.out.push_str("default: {\n"),
                    }
                    self.block(scope, &case.body, depth + 1)?;
                    if !case.fall_through && !ends(&case.body) {
                        self.indent(depth + 1);
                        self.out.push_str("break;\n");
                    }
                    self.indent(depth);
                    self.out.push_str("}\n");
                }
                self.indent(depth);
                self.out.push_str("}\n");
            }
            Statement::Loop {
                body,
                continuing,
                break_if,
            } => {
                // What follows each turn of the loop runs before every turn but the first, so
                // that a turn cut short still runs it.
                let gate = format!("kerf_turn{}", self.gates);
                self.gates += 1;
                self.indent(depth);
                let _ = writeln!(self.out, "for (bool {gate} = false;; {gate} = true) {{");
                if !continuing.is_empty() || break_if.is_some() {
                    self.indent(depth + 1);
                    let _ = writeln!(self.out, "if ({gate}) {{");
                    self.block(scope, continuing, depth + 2)?;
                    if let Some(condition) = break_if {
                        let condition = self.value(scope, *condition)?;
                        self.indent(depth + 2);
                        let _ = writeln!(self.out, "if ({condition}) break;");
                    }
                    self.indent(depth + 1);
                    self.out.push_str("}\n");
                }
                self.block(scope, body, depth + 1)?;
                self.indent(depth);
                self.out.push_str("}\n");
            }
            Statement::Break => {
                self.indent(depth);
                self.out.push_str("break;\n");
            }
            Statement::Continue => {
                self.indent(depth);
                self.out.push_str("continue;\n");
            }
            Statement::Return { value } => {
                let value = match value {
                    Some(value) => format!(" {}", self.value(scope, *value)?),
                    None => String::new(),
                };
                self.indent(depth);
                let _ = writeln!(self.out, "return{value};");
            }
            Statement::Store { pointer, value } => {
                let value = self.value(scope, *value)?;
                let line = match self.place(scope, *pointer)? {
                    Place::Variable(name) => format!("{name} = {value}"),
                    Place::Bytes {
                        buffer,
                        offset,
                        writable: true,
                    } => {
                        let pointee = self.pointee(scope, *pointer)?;
                        self.write(&pointee, &buffer, &offset, &value)?
                    }
                    Place::Bytes { .. } => return unsupported("a store into what is only read"),
                };
                self.indent(depth);
                let _ = writeln!(self.out, "{line};");
            }
            Statement::ImageStore {
                image,
                coordinate,
                array_index: None,
                value,
            } => {
                let TypeInner::Image {
                    class: ImageClass::Storage { format, .. },
                    ..
                } = self.type_of(scope, *image)
                else {
                    return unsupported("a store into a texture that is not a storage texture");
                };
                let (_, name) = storage_texel(*format)?;
                let texture = self.value(scope, *image)?;
                let at = self.coordinate(scope, *coordinate)?;
                let value = self.value(scope, *value)?;
                self.indent(depth);
                let _ = writeln!(self.out, "kerf_store_{name}({texture}, {at}, {value});");
            }
            Statement::Call {
                function,
                arguments,
                result,
            } => {
                let mut call = format!("kerf_f{}(ctx", function.index());
                for argument in arguments {
                    let _ = write!(call, ", {}", self.value(scope, *argument)?);
                }
                call.push(')');
                self.indent(depth);
                match result {
                    Some(result) => {
                        let of = self.c_type(&self.shape(scope, *result))?;
                        let _ = writeln!(self.out, "const {of} _e{} = {call};", result.index());
                        scope.baked[result.index()] = true;
                    }
                    None => {
                        let _ = writeln!(self.out, "{call};");
                    }
                }
            }
            other => return unsupported(format!("the statement {other:?}")),
        }
        Ok(())
    }

    fn type_of(&self, scope: &Scope<'a>, handle: Handle<Expression>) -> &'a TypeInner {
        scope.info[handle].ty.inner_with(&self.module.types)
    }

    fn shape(&self, scope: &Scope<'a>, handle: Handle<Expression>) -> Shape<'a> {
        self.shape_of(&scope.info[handle].ty)
    }

    /// What a pointer points at.
    fn pointee(&self, scope: &Scope<'a>, pointer: Handle<Expression>) -> Result<Shape<'a>> {
        match self.type_of(scope, pointer) {
            TypeInner::Pointer { base, .. } => Ok(Shape::Of(*base)),
            // A part of a vector, or a vector of a matrix, has no type of its own in the module.
            TypeInner::ValuePointer { size, scalar, .. } => Ok(Shape::Number(*scalar, *size)),
            other => unsupported(format!("a load through {other:?}")),
        }
    }

    /// Give an expression a name of its own, where it is a value.
    fn bake(
        &mut self,
        scope: &mut Scope<'a>,
        handle: Handle<Expression>,
        depth: usize,
    ) -> Result<()> {
        let inner = self.type_of(scope, handle);
        if matches!(
            inner,
            TypeInner::Pointer { .. } | TypeInner::ValuePointer { .. } | TypeInner::Image { .. }
        ) {
            return Ok(());
        }
        let of = self.c_type(&self.shape(scope, handle))?;
        let value = self.inline(scope, handle)?;
        self.indent(depth);
        let _ = writeln!(self.out, "const {of} _e{} = {value};", handle.index());
        scope.baked[handle.index()] = true;
        Ok(())
    }

    /// An expression's value as C: its name where it has one.
    fn value(&mut self, scope: &Scope<'a>, handle: Handle<Expression>) -> Result<String> {
        if scope.baked[handle.index()] {
            return Ok(format!("_e{}", handle.index()));
        }
        self.inline(scope, handle)
    }

    /// A texture's coordinate as two whole numbers.
    fn coordinate(&mut self, scope: &Scope<'a>, handle: Handle<Expression>) -> Result<String> {
        let value = self.value(scope, handle)?;
        match self.type_of(scope, handle) {
            TypeInner::Vector {
                size: VectorSize::Bi,
                scalar,
            } => match scalar.kind {
                ScalarKind::Sint => Ok(value),
                ScalarKind::Uint => Ok(format!("convert_int2_sat({value})")),
                other => unsupported(format!("a texture coordinate of {other:?}")),
            },
            other => unsupported(format!("a texture coordinate of type {other:?}")),
        }
    }

    /// Where a pointer points, as C.
    fn place(&mut self, scope: &Scope<'a>, handle: Handle<Expression>) -> Result<Place> {
        match &scope.function.expressions[handle] {
            Expression::LocalVariable(local) => Ok(Place::Variable(format!("l{}", local.index()))),
            Expression::GlobalVariable(global) => {
                let variable = &self.module.global_variables[*global];
                match variable.space {
                    AddressSpace::Uniform => Ok(Place::Bytes {
                        buffer: format!("ctx->g{}", global.index()),
                        offset: "0u".to_owned(),
                        writable: false,
                    }),
                    AddressSpace::Storage { access } => Ok(Place::Bytes {
                        buffer: format!("ctx->g{}", global.index()),
                        offset: "0u".to_owned(),
                        writable: access.contains(naga::StorageAccess::STORE),
                    }),
                    other => unsupported(format!("a pointer into {other:?}")),
                }
            }
            Expression::AccessIndex { base, index } => {
                let within = self.pointee(scope, *base)?;
                let place = self.place(scope, *base)?;
                self.part(place, &within, Index::Known(*index))
            }
            Expression::Access { base, index } => {
                let within = self.pointee(scope, *base)?;
                let place = self.place(scope, *base)?;
                let index = self.value(scope, *index)?;
                self.part(place, &within, Index::Told(index))
            }
            other => unsupported(format!("a pointer made by {other:?}")),
        }
    }

    /// A part of what `place` holds, which is of `within`.
    fn part(&self, place: Place, within: &Shape<'a>, index: Index) -> Result<Place> {
        let inner = self.inner(within);
        match place {
            Place::Variable(name) => Ok(Place::Variable(match (&*inner, &index) {
                (TypeInner::Struct { .. }, Index::Known(index)) => format!("{name}.m{index}"),
                (TypeInner::Vector { .. }, Index::Known(index)) => {
                    format!("{name}.{}", PARTS[*index as usize])
                }
                (
                    TypeInner::Array {
                        size: ArraySize::Constant(count),
                        ..
                    },
                    index,
                ) => format!("{name}.a[{}]", index.held(count.get())),
                (other, _) => return unsupported(format!("a part of a variable of {other:?}")),
            })),
            Place::Bytes {
                buffer,
                offset,
                writable,
            } => {
                let further = match (&*inner, &index) {
                    (TypeInner::Struct { members, .. }, Index::Known(index)) => {
                        format!("{}u", members[*index as usize].offset)
                    }
                    (TypeInner::Vector { scalar, .. }, index) => {
                        format!("{} * {}u", index.told(), scalar.width)
                    }
                    (
                        TypeInner::Array {
                            size: ArraySize::Constant(count),
                            stride,
                            ..
                        },
                        index,
                    ) => format!("{} * {stride}u", index.held(count.get())),
                    (other, _) => return unsupported(format!("a part of a buffer's {other:?}")),
                };
                Ok(Place::Bytes {
                    buffer,
                    offset: format!("({offset} + {further})"),
                    writable,
                })
            }
        }
    }

    /// An expression as C, written where it is used.
    fn inline(&mut self, scope: &Scope<'a>, handle: Handle<Expression>) -> Result<String> {
        let expression = &scope.function.expressions[handle];
        Ok(match expression {
            Expression::Literal(value) => literal(*value)?,
            Expression::Constant(constant) => {
                self.constant(self.module.constants[*constant].init)?
            }
            Expression::ZeroValue(ty) => self.zero(&Shape::Of(*ty))?,
            Expression::Compose { ty, components } => {
                let mut parts = Vec::with_capacity(components.len());
                for component in components {
                    let value = self.value(scope, *component)?;
                    parts.push((value, self.type_of(scope, *component)));
                }
                self.compose(*ty, &parts)?
            }
            Expression::Splat { size, value } => {
                let TypeInner::Scalar(scalar) = self.type_of(scope, *value) else {
                    return unsupported("a vector made of what is no number");
                };
                let of = vector_name(*size, *scalar)?;
                let value = self.value(scope, *value)?;
                match scalar.kind {
                    ScalarKind::Bool => format!("(({of})(({value}) ? -1 : 0))"),
                    _ => format!("(({of})({value}))"),
                }
            }
            Expression::Swizzle {
                size,
                vector,
                pattern,
            } => {
                let vector = self.value(scope, *vector)?;
                let parts: String = pattern[..size_of(*size)]
                    .iter()
                    .map(|part| PARTS[*part as usize])
                    .collect();
                format!("({vector}).{parts}")
            }
            Expression::FunctionArgument(index) => format!("a{index}"),
            Expression::GlobalVariable(global) => {
                let variable = &self.module.global_variables[*global];
                match variable.space {
                    AddressSpace::Handle => format!("ctx->g{}", global.index()),
                    other => return unsupported(format!("a global in {other:?} taken as a value")),
                }
            }
            Expression::Load { pointer } => match self.place(scope, *pointer)? {
                Place::Variable(name) => name,
                Place::Bytes { buffer, offset, .. } => {
                    let pointee = self.pointee(scope, *pointer)?;
                    self.read(&pointee, &buffer, &offset)?
                }
            },
            Expression::AccessIndex { base, index } => {
                let value = self.value(scope, *base)?;
                match self.type_of(scope, *base) {
                    TypeInner::Vector { .. } => format!("({value}).{}", PARTS[*index as usize]),
                    TypeInner::Struct { .. } => format!("({value}).m{index}"),
                    TypeInner::Array { .. } => format!("({value}).a[{index}]"),
                    other => return unsupported(format!("a part of a value of {other:?}")),
                }
            }
            Expression::Access { base, index } => {
                let value = self.value(scope, *base)?;
                let index = self.value(scope, *index)?;
                match self.type_of(scope, *base) {
                    TypeInner::Array {
                        size: ArraySize::Constant(count),
                        ..
                    } => format!("({value}).a[{}]", Index::Told(index).held(count.get())),
                    other => return unsupported(format!("a part, told when run, of {other:?}")),
                }
            }
            Expression::ImageLoad {
                image,
                coordinate,
                array_index: None,
                sample: None,
                level: _,
            } => {
                let texture = self.value(scope, *image)?;
                let at = self.coordinate(scope, *coordinate)?;
                match self.type_of(scope, *image) {
                    TypeInner::Image {
                        class: ImageClass::Sampled { kind, .. },
                        ..
                    } => match kind {
                        ScalarKind::Float => format!("kerf_load_f({texture}, {at})"),
                        ScalarKind::Uint => format!("kerf_load_u({texture}, {at})"),
                        other => return unsupported(format!("a texture of {other:?}")),
                    },
                    other => return unsupported(format!("a load from {other:?}")),
                }
            }
            Expression::ImageQuery {
                image,
                query: ImageQuery::Size { level: _ },
            } => {
                let texture = self.value(scope, *image)?;
                format!("((uint2)((uint)({texture}).w, (uint)({texture}).h))")
            }
            Expression::Unary { op, expr } => {
                let value = self.value(scope, *expr)?;
                match op {
                    UnaryOperator::Negate => format!("(-({value}))"),
                    UnaryOperator::LogicalNot => format!("(!({value}))"),
                    UnaryOperator::BitwiseNot => format!("(~({value}))"),
                }
            }
            Expression::Binary { op, left, right } => self.binary(scope, *op, *left, *right)?,
            Expression::Select {
                condition,
                accept,
                reject,
            } => {
                let (accept, reject) = (self.value(scope, *accept)?, self.value(scope, *reject)?);
                let told = self.value(scope, *condition)?;
                match self.type_of(scope, *condition) {
                    TypeInner::Scalar(_) => format!("(({told}) ? ({accept}) : ({reject}))"),
                    _ => format!("select({reject}, {accept}, {told})"),
                }
            }
            Expression::Relational { fun, argument } => {
                let value = self.value(scope, *argument)?;
                let vector = matches!(self.type_of(scope, *argument), TypeInner::Vector { .. });
                match (fun, vector) {
                    (RelationalFunction::All, true) => format!("(all({value}) != 0)"),
                    (RelationalFunction::Any, true) => format!("(any({value}) != 0)"),
                    (RelationalFunction::All | RelationalFunction::Any, false) => value,
                    (RelationalFunction::IsNan, true) => format!("isnan({value})"),
                    (RelationalFunction::IsNan, false) => format!("(isnan({value}) != 0)"),
                    (RelationalFunction::IsInf, true) => format!("isinf({value})"),
                    (RelationalFunction::IsInf, false) => format!("(isinf({value}) != 0)"),
                }
            }
            Expression::Math {
                fun,
                arg,
                arg1,
                arg2,
                arg3: _,
            } => self.math(scope, *fun, *arg, *arg1, *arg2)?,
            Expression::As {
                expr,
                kind,
                convert,
            } => self.cast(scope, *expr, *kind, *convert)?,
            other => return unsupported(format!("the expression {other:?}")),
        })
    }

    /// A value of `ty` made of `parts`, each with its type.
    fn compose(&self, ty: Handle<Type>, parts: &[(String, &TypeInner)]) -> Result<String> {
        let shape = Shape::Of(ty);
        Ok(match &*self.inner(&shape) {
            TypeInner::Vector { size, scalar } => {
                let of = vector_name(*size, *scalar)?;
                let parts: Vec<String> = parts
                    .iter()
                    .map(|(value, inner)| match (scalar.kind, inner) {
                        (ScalarKind::Bool, TypeInner::Scalar(_)) => format!("(({value}) ? -1 : 0)"),
                        _ => value.clone(),
                    })
                    .collect();
                format!("(({of})({}))", parts.join(", "))
            }
            TypeInner::Struct { .. } => {
                let parts: Vec<&str> = parts.iter().map(|(value, _)| value.as_str()).collect();
                format!("(({}){{{}}})", self.c_type(&shape)?, parts.join(", "))
            }
            TypeInner::Array { .. } => {
                let parts: Vec<&str> = parts.iter().map(|(value, _)| value.as_str()).collect();
                format!("(({}){{{{{}}}}})", self.c_type(&shape)?, parts.join(", "))
            }
            other => return unsupported(format!("a value of {other:?} put together")),
        })
    }

    /// One of the module's constant expressions as C.
    fn constant(&self, handle: Handle<Expression>) -> Result<String> {
        Ok(match &self.module.global_expressions[handle] {
            Expression::Literal(value) => literal(*value)?,
            Expression::Constant(constant) => {
                self.constant(self.module.constants[*constant].init)?
            }
            Expression::ZeroValue(ty) => self.zero(&Shape::Of(*ty))?,
            Expression::Compose { ty, components } => {
                let mut parts = Vec::with_capacity(components.len());
                for component in components {
                    let inner = self.constant_type(*component)?;
                    parts.push((self.constant(*component)?, inner));
                }
                self.compose(*ty, &parts)?
            }
            Expression::Splat { size, value } => {
                let TypeInner::Scalar(scalar) = self.constant_type(*value)? else {
                    return unsupported("a constant vector made of what is no number");
                };
                let of = vector_name(*size, *scalar)?;
                format!("(({of})({}))", self.constant(*value)?)
            }
            other => return unsupported(format!("the constant {other:?}")),
        })
    }

    /// The type of one of the module's constant expressions.
    fn constant_type(&self, handle: Handle<Expression>) -> Result<&'a TypeInner> {
        static FLOAT: TypeInner = TypeInner::Scalar(Scalar::F32);
        static INT: TypeInner = TypeInner::Scalar(Scalar::I32);
        static UINT: TypeInner = TypeInner::Scalar(Scalar::U32);
        static BOOL: TypeInner = TypeInner::Scalar(Scalar::BOOL);
        Ok(match &self.module.global_expressions[handle] {
            Expression::Literal(Literal::F32(_)) => &FLOAT,
            Expression::Literal(Literal::I32(_)) => &INT,
            Expression::Literal(Literal::U32(_)) => &UINT,
            Expression::Literal(Literal::Bool(_)) => &BOOL,
            Expression::Constant(constant) => {
                &self.module.types[self.module.constants[*constant].ty].inner
            }
            Expression::ZeroValue(ty) | Expression::Compose { ty, .. } => {
                &self.module.types[*ty].inner
            }
            other => return unsupported(format!("the type of the constant {other:?}")),
        })
    }

    fn binary(
        &mut self,
        scope: &Scope<'a>,
        op: BinaryOperator,
        left: Handle<Expression>,
        right: Handle<Expression>,
    ) -> Result<String> {
        let (l, r) = (self.value(scope, left)?, self.value(scope, right)?);
        let (left_type, right_type) = (self.type_of(scope, left), self.type_of(scope, right));
        let numbers = |inner: &TypeInner| match inner {
            TypeInner::Scalar(scalar) => Some((*scalar, 1)),
            TypeInner::Vector { size, scalar } => Some((*scalar, size_of(*size))),
            _ => None,
        };
        let (Some((scalar, left_count)), Some((_, right_count))) =
            (numbers(left_type), numbers(right_type))
        else {
            return unsupported(format!("{op:?} of {left_type:?} and {right_type:?}"));
        };
        let count = left_count.max(right_count);
        // A number beside a vector is that number in each of its places.
        let wide = |value: &str, own: usize| -> Result<String> {
            if own == count {
                return Ok(value.to_owned());
            }
            let of = match count {
                2 => vector_name(VectorSize::Bi, scalar)?,
                3 => vector_name(VectorSize::Tri, scalar)?,
                _ => vector_name(VectorSize::Quad, scalar)?,
            };
            Ok(format!("(({of})({value}))"))
        };
        let plain = |sign: &str| format!("(({l}) {sign} ({r}))");
        Ok(match op {
            BinaryOperator::Add => plain("+"),
            BinaryOperator::Subtract => plain("-"),
            BinaryOperator::Multiply => plain("*"),
            BinaryOperator::Divide | BinaryOperator::Modulo => {
                let (l, r) = (wide(&l, left_count)?, wide(&r, right_count)?);
                let divide = op == BinaryOperator::Divide;
                match (scalar.kind, divide) {
                    (ScalarKind::Float, true) => format!("kerf_div{count}({l}, {r})"),
                    (ScalarKind::Float, false) => format!("fmod({l}, {r})"),
                    (ScalarKind::Sint, true) => format!("kerf_idiv{count}({l}, {r})"),
                    (ScalarKind::Sint, false) => format!("kerf_imod{count}({l}, {r})"),
                    (ScalarKind::Uint, true) => format!("kerf_udiv{count}({l}, {r})"),
                    (ScalarKind::Uint, false) => format!("kerf_umod{count}({l}, {r})"),
                    (other, _) => return unsupported(format!("{op:?} of {other:?}")),
                }
            }
            BinaryOperator::Equal => plain("=="),
            BinaryOperator::NotEqual => plain("!="),
            BinaryOperator::Less => plain("<"),
            BinaryOperator::LessEqual => plain("<="),
            BinaryOperator::Greater => plain(">"),
            BinaryOperator::GreaterEqual => plain(">="),
            BinaryOperator::And => plain("&"),
            BinaryOperator::ExclusiveOr => plain("^"),
            BinaryOperator::InclusiveOr => plain("|"),
            BinaryOperator::LogicalAnd => plain("&&"),
            BinaryOperator::LogicalOr => plain("||"),
            BinaryOperator::ShiftLeft => plain("<<"),
            BinaryOperator::ShiftRight => plain(">>"),
        })
    }

    fn math(
        &mut self,
        scope: &Scope<'a>,
        fun: MathFunction,
        arg: Handle<Expression>,
        arg1: Option<Handle<Expression>>,
        arg2: Option<Handle<Expression>>,
    ) -> Result<String> {
        let a = self.value(scope, arg)?;
        let b = match arg1 {
            Some(arg) => self.value(scope, arg)?,
            None => String::new(),
        };
        let c = match arg2 {
            Some(arg) => self.value(scope, arg)?,
            None => String::new(),
        };
        let (scalar, count) = match self.type_of(scope, arg) {
            TypeInner::Scalar(scalar) => (*scalar, 1),
            TypeInner::Vector { size, scalar } => (*scalar, size_of(*size)),
            other => return unsupported(format!("{fun:?} of {other:?}")),
        };
        let float = scalar.kind == ScalarKind::Float;
        let of = match count {
            1 => scalar_name(scalar)?.to_owned(),
            2 => vector_name(VectorSize::Bi, scalar)?,
            3 => vector_name(VectorSize::Tri, scalar)?,
            _ => vector_name(VectorSize::Quad, scalar)?,
        };
        // A number beside a vector, as `pow(v, 2.4)` has one, is that number in each place.
        let beside =
            |scope: &Scope<'a>, this: &Self, other: Option<Handle<Expression>>, value: &str| {
                match other.map(|other| this.type_of(scope, other)) {
                    Some(TypeInner::Scalar(_)) if count > 1 => format!("(({of})({value}))"),
                    _ => value.to_owned(),
                }
            };
        let simple = |name: &str| format!("{name}({a})");
        Ok(match fun {
            MathFunction::Abs if float => simple("fabs"),
            MathFunction::Abs if scalar.kind == ScalarKind::Sint && count == 1 => {
                format!("((int)abs({a}))")
            }
            MathFunction::Abs if scalar.kind == ScalarKind::Sint => format!("as_{of}(abs({a}))"),
            MathFunction::Abs => a,
            MathFunction::Min => format!("min({a}, {b})"),
            MathFunction::Max => format!("max({a}, {b})"),
            MathFunction::Clamp => format!("clamp({a}, {b}, {c})"),
            MathFunction::Saturate => format!("clamp({a}, 0.0f, 1.0f)"),
            MathFunction::Floor => simple("floor"),
            MathFunction::Ceil => simple("ceil"),
            MathFunction::Trunc => simple("trunc"),
            MathFunction::Round => simple("rint"),
            MathFunction::Fract => format!("(({a}) - floor({a}))"),
            MathFunction::Sqrt => format!("kerf_sqrt{count}({a})"),
            MathFunction::InverseSqrt => {
                let one = if count == 1 {
                    "1.0f".to_owned()
                } else {
                    format!("(({of})(1.0f))")
                };
                format!("kerf_div{count}({one}, kerf_sqrt{count}({a}))")
            }
            MathFunction::Pow => {
                format!("kerf_pow{count}({a}, {})", beside(scope, self, arg1, &b))
            }
            MathFunction::Exp => simple("exp"),
            MathFunction::Exp2 => simple("exp2"),
            MathFunction::Log => simple("log"),
            MathFunction::Log2 => simple("log2"),
            MathFunction::Sin => simple("sin"),
            MathFunction::Cos => simple("cos"),
            MathFunction::Tan => simple("tan"),
            MathFunction::Asin => simple("asin"),
            MathFunction::Acos => simple("acos"),
            MathFunction::Atan => simple("atan"),
            MathFunction::Atan2 => format!("atan2({a}, {b})"),
            MathFunction::Sinh => simple("sinh"),
            MathFunction::Cosh => simple("cosh"),
            MathFunction::Tanh => simple("tanh"),
            MathFunction::Sign if float => simple("sign"),
            MathFunction::Sign => format!("clamp({a}, -1, 1)"),
            MathFunction::Dot if float => format!("dot({a}, {b})"),
            MathFunction::Length if float => simple("length"),
            MathFunction::Distance if float => format!("distance({a}, {b})"),
            MathFunction::Normalize if float => simple("normalize"),
            MathFunction::Cross if float => format!("cross({a}, {b})"),
            MathFunction::Mix => format!("mix({a}, {b}, {c})"),
            MathFunction::Step => format!("step({a}, {b})"),
            MathFunction::Fma => format!("((({a}) * ({b})) + ({c}))"),
            MathFunction::CountOneBits => simple("popcount"),
            other => return unsupported(format!("the function {other:?}")),
        })
    }

    fn cast(
        &mut self,
        scope: &Scope<'a>,
        expr: Handle<Expression>,
        kind: ScalarKind,
        convert: Option<u8>,
    ) -> Result<String> {
        let value = self.value(scope, expr)?;
        let (from, count) = match self.type_of(scope, expr) {
            TypeInner::Scalar(scalar) => (*scalar, 1),
            TypeInner::Vector { size, scalar } => (*scalar, size_of(*size)),
            other => return unsupported(format!("a cast of {other:?}")),
        };
        let to = Scalar {
            kind,
            width: convert.unwrap_or(from.width),
        };
        let of = match count {
            1 => scalar_name(to)?.to_owned(),
            2 => vector_name(VectorSize::Bi, to)?,
            3 => vector_name(VectorSize::Tri, to)?,
            _ => vector_name(VectorSize::Quad, to)?,
        };
        if convert.is_none() {
            // The same bits read as another kind of number.
            return Ok(if from.kind == kind {
                value
            } else {
                format!("as_{of}({value})")
            });
        }
        let one = match kind {
            ScalarKind::Float => "1.0f",
            ScalarKind::Uint => "1u",
            _ => "1",
        };
        Ok(match (from.kind, kind, count) {
            (from, to, _) if from == to => value,
            (ScalarKind::Bool, _, 1) => format!("(({value}) ? {one} : ({of})0)"),
            (ScalarKind::Bool, _, _) => {
                format!("select(({of})(0), ({of})({one}), {value})")
            }
            (_, ScalarKind::Bool, _) => format!("(({value}) != 0)"),
            (ScalarKind::Float, ScalarKind::Sint | ScalarKind::Uint, _) => {
                format!("convert_{of}_sat({value})")
            }
            (ScalarKind::Sint | ScalarKind::Uint, ScalarKind::Float, 1) => {
                format!("(({of})({value}))")
            }
            (ScalarKind::Sint | ScalarKind::Uint, ScalarKind::Float, _) => {
                format!("convert_{of}({value})")
            }
            (ScalarKind::Sint | ScalarKind::Uint, ScalarKind::Sint | ScalarKind::Uint, 1) => {
                format!("(({of})({value}))")
            }
            (ScalarKind::Sint | ScalarKind::Uint, ScalarKind::Sint | ScalarKind::Uint, _) => {
                format!("as_{of}({value})")
            }
            (from, to, _) => return unsupported(format!("a cast from {from:?} to {to:?}")),
        })
    }
}

/// Which part: one the kernel names, or one it works out when run.
enum Index {
    Known(u32),
    Told(String),
}

impl Index {
    /// The part as C, no further than the last of `count`.
    fn held(&self, count: u32) -> String {
        match self {
            Index::Known(index) => format!("{index}"),
            Index::Told(index) => format!("min((uint)({index}), {}u)", count.saturating_sub(1)),
        }
    }

    fn told(&self) -> String {
        match self {
            Index::Known(index) => format!("{index}u"),
            Index::Told(index) => format!("min((uint)({index}), 3u)"),
        }
    }
}

/// Whether a block ends by leaving it, so that nothing written after it would be reached.
fn ends(block: &Block) -> bool {
    matches!(
        block.last(),
        Some(Statement::Break | Statement::Continue | Statement::Return { .. })
    )
}
