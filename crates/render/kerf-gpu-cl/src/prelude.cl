// What every kernel written out by `translate.rs` starts with: a picture as a buffer of its
// texels, numbers read from and written to a buffer's bytes, and the arithmetic WGSL defines
// where OpenCL C leaves it open.
#pragma OPENCL FP_CONTRACT OFF

#define KERF_RGBA32F 0
#define KERF_R32F 1
#define KERF_RG32F 2
#define KERF_RGBA8 3
#define KERF_RGBA16U 4
#define KERF_R8U 5
#define KERF_R16U 6

// OpenCL lets a float's quotient, root and power be a few last bits off. Through doubles the
// quotient and the root are rounded right, and the power is right far past a float's last bit.
#ifdef KERF_EXACT
#pragma OPENCL EXTENSION cl_khr_fp64 : enable
static float kerf_div1(float a, float b) { return (float)((double)a / (double)b); }
static float2 kerf_div2(float2 a, float2 b) { return convert_float2(convert_double2(a) / convert_double2(b)); }
static float3 kerf_div3(float3 a, float3 b) { return convert_float3(convert_double3(a) / convert_double3(b)); }
static float4 kerf_div4(float4 a, float4 b) { return convert_float4(convert_double4(a) / convert_double4(b)); }
static float kerf_sqrt1(float a) { return (float)sqrt((double)a); }
static float2 kerf_sqrt2(float2 a) { return convert_float2(sqrt(convert_double2(a))); }
static float3 kerf_sqrt3(float3 a) { return convert_float3(sqrt(convert_double3(a))); }
static float4 kerf_sqrt4(float4 a) { return convert_float4(sqrt(convert_double4(a))); }
static float kerf_pow1(float a, float b) { return (float)pow((double)a, (double)b); }
static float2 kerf_pow2(float2 a, float2 b) { return convert_float2(pow(convert_double2(a), convert_double2(b))); }
static float3 kerf_pow3(float3 a, float3 b) { return convert_float3(pow(convert_double3(a), convert_double3(b))); }
static float4 kerf_pow4(float4 a, float4 b) { return convert_float4(pow(convert_double4(a), convert_double4(b))); }
#else
static float kerf_div1(float a, float b) { return a / b; }
static float2 kerf_div2(float2 a, float2 b) { return a / b; }
static float3 kerf_div3(float3 a, float3 b) { return a / b; }
static float4 kerf_div4(float4 a, float4 b) { return a / b; }
static float kerf_sqrt1(float a) { return sqrt(a); }
static float2 kerf_sqrt2(float2 a) { return sqrt(a); }
static float3 kerf_sqrt3(float3 a) { return sqrt(a); }
static float4 kerf_sqrt4(float4 a) { return sqrt(a); }
static float kerf_pow1(float a, float b) { return pow(a, b); }
static float2 kerf_pow2(float2 a, float2 b) { return pow(a, b); }
static float3 kerf_pow3(float3 a, float3 b) { return pow(a, b); }
static float4 kerf_pow4(float4 a, float4 b) { return pow(a, b); }
#endif

// A whole number divided by none is itself, and what is left of it is nothing.
static int kerf_idiv1(int a, int b) { return (b == 0 || (a == INT_MIN && b == -1)) ? a : a / b; }
static int kerf_imod1(int a, int b) { return (b == 0 || (a == INT_MIN && b == -1)) ? 0 : a % b; }
static uint kerf_udiv1(uint a, uint b) { return b == 0u ? a : a / b; }
static uint kerf_umod1(uint a, uint b) { return b == 0u ? 0u : a % b; }
static int2 kerf_idiv2(int2 a, int2 b) { return (int2)(kerf_idiv1(a.x, b.x), kerf_idiv1(a.y, b.y)); }
static int3 kerf_idiv3(int3 a, int3 b) { return (int3)(kerf_idiv1(a.x, b.x), kerf_idiv1(a.y, b.y), kerf_idiv1(a.z, b.z)); }
static int4 kerf_idiv4(int4 a, int4 b) { return (int4)(kerf_idiv1(a.x, b.x), kerf_idiv1(a.y, b.y), kerf_idiv1(a.z, b.z), kerf_idiv1(a.w, b.w)); }
static int2 kerf_imod2(int2 a, int2 b) { return (int2)(kerf_imod1(a.x, b.x), kerf_imod1(a.y, b.y)); }
static int3 kerf_imod3(int3 a, int3 b) { return (int3)(kerf_imod1(a.x, b.x), kerf_imod1(a.y, b.y), kerf_imod1(a.z, b.z)); }
static int4 kerf_imod4(int4 a, int4 b) { return (int4)(kerf_imod1(a.x, b.x), kerf_imod1(a.y, b.y), kerf_imod1(a.z, b.z), kerf_imod1(a.w, b.w)); }
static uint2 kerf_udiv2(uint2 a, uint2 b) { return (uint2)(kerf_udiv1(a.x, b.x), kerf_udiv1(a.y, b.y)); }
static uint3 kerf_udiv3(uint3 a, uint3 b) { return (uint3)(kerf_udiv1(a.x, b.x), kerf_udiv1(a.y, b.y), kerf_udiv1(a.z, b.z)); }
static uint4 kerf_udiv4(uint4 a, uint4 b) { return (uint4)(kerf_udiv1(a.x, b.x), kerf_udiv1(a.y, b.y), kerf_udiv1(a.z, b.z), kerf_udiv1(a.w, b.w)); }
static uint2 kerf_umod2(uint2 a, uint2 b) { return (uint2)(kerf_umod1(a.x, b.x), kerf_umod1(a.y, b.y)); }
static uint3 kerf_umod3(uint3 a, uint3 b) { return (uint3)(kerf_umod1(a.x, b.x), kerf_umod1(a.y, b.y), kerf_umod1(a.z, b.z)); }
static uint4 kerf_umod4(uint4 a, uint4 b) { return (uint4)(kerf_umod1(a.x, b.x), kerf_umod1(a.y, b.y), kerf_umod1(a.z, b.z), kerf_umod1(a.w, b.w)); }

// A picture: its texels row by row in a buffer, how many across and down, and what a texel is.
typedef struct {
    __global uchar* p;
    int w;
    int h;
    int fmt;
} kerf_tex;

static kerf_tex kerf_tex_of(__global uchar* p, int4 info) {
    kerf_tex t;
    t.p = p;
    t.w = info.x;
    t.h = info.y;
    t.fmt = info.z;
    return t;
}

// The texel at `c`, the nearest inside the picture where `c` is outside it.
static size_t kerf_texel(kerf_tex t, int2 c) {
    c = clamp(c, (int2)(0, 0), (int2)(t.w - 1, t.h - 1));
    return (size_t)c.y * (size_t)t.w + (size_t)c.x;
}

static float4 kerf_load_f(kerf_tex t, int2 c) {
    const size_t i = kerf_texel(t, c);
    switch (t.fmt) {
    case KERF_RGBA32F: return vload4(i, (__global const float*)t.p);
    case KERF_R32F: return (float4)(((__global const float*)t.p)[i], 0.0f, 0.0f, 1.0f);
    case KERF_RG32F: return (float4)(vload2(i, (__global const float*)t.p), 0.0f, 1.0f);
    case KERF_RGBA8: return convert_float4(vload4(i, (__global const uchar*)t.p)) / 255.0f;
    default: return (float4)(0.0f);
    }
}

static uint4 kerf_load_u(kerf_tex t, int2 c) {
    const size_t i = kerf_texel(t, c);
    switch (t.fmt) {
    case KERF_RGBA16U: return convert_uint4(vload4(i, (__global const ushort*)t.p));
    case KERF_R8U: return (uint4)((uint)((__global const uchar*)t.p)[i], 0u, 0u, 1u);
    case KERF_R16U: return (uint4)((uint)((__global const ushort*)t.p)[i], 0u, 0u, 1u);
    default: return (uint4)(0u);
    }
}

// A store outside the picture, or into one of another kind of texel, is not made.
static bool kerf_holds(kerf_tex t, int2 c, int fmt) {
    return t.fmt == fmt && c.x >= 0 && c.y >= 0 && c.x < t.w && c.y < t.h;
}

static void kerf_store_rgba32float(kerf_tex t, int2 c, float4 v) {
    if (kerf_holds(t, c, KERF_RGBA32F)) vstore4(v, (size_t)c.y * (size_t)t.w + (size_t)c.x, (__global float*)t.p);
}

static void kerf_store_r32float(kerf_tex t, int2 c, float4 v) {
    if (kerf_holds(t, c, KERF_R32F)) ((__global float*)t.p)[(size_t)c.y * (size_t)t.w + (size_t)c.x] = v.x;
}

static void kerf_store_rg32float(kerf_tex t, int2 c, float4 v) {
    if (kerf_holds(t, c, KERF_RG32F)) vstore2(v.xy, (size_t)c.y * (size_t)t.w + (size_t)c.x, (__global float*)t.p);
}

// To the nearest of 256 steps, a half going up, as a GPU stores into such a texture.
static void kerf_store_rgba8unorm(kerf_tex t, int2 c, float4 v) {
    if (kerf_holds(t, c, KERF_RGBA8)) {
        const float4 scaled = clamp(v, 0.0f, 1.0f) * 255.0f + 0.5f;
        vstore4(convert_uchar4_sat(scaled), (size_t)c.y * (size_t)t.w + (size_t)c.x, (__global uchar*)t.p);
    }
}

static void kerf_store_rgba16uint(kerf_tex t, int2 c, uint4 v) {
    if (kerf_holds(t, c, KERF_RGBA16U)) vstore4(convert_ushort4(v), (size_t)c.y * (size_t)t.w + (size_t)c.x, (__global ushort*)t.p);
}

// Numbers in a buffer's bytes, `o` bytes in.
static float kerf_rd_f1(__global const uchar* p, uint o) { return *(__global const float*)(p + o); }
static float2 kerf_rd_f2(__global const uchar* p, uint o) { return vload2(0, (__global const float*)(p + o)); }
static float3 kerf_rd_f3(__global const uchar* p, uint o) { return vload3(0, (__global const float*)(p + o)); }
static float4 kerf_rd_f4(__global const uchar* p, uint o) { return vload4(0, (__global const float*)(p + o)); }
static int kerf_rd_i1(__global const uchar* p, uint o) { return *(__global const int*)(p + o); }
static int2 kerf_rd_i2(__global const uchar* p, uint o) { return vload2(0, (__global const int*)(p + o)); }
static int3 kerf_rd_i3(__global const uchar* p, uint o) { return vload3(0, (__global const int*)(p + o)); }
static int4 kerf_rd_i4(__global const uchar* p, uint o) { return vload4(0, (__global const int*)(p + o)); }
static uint kerf_rd_u1(__global const uchar* p, uint o) { return *(__global const uint*)(p + o); }
static uint2 kerf_rd_u2(__global const uchar* p, uint o) { return vload2(0, (__global const uint*)(p + o)); }
static uint3 kerf_rd_u3(__global const uchar* p, uint o) { return vload3(0, (__global const uint*)(p + o)); }
static uint4 kerf_rd_u4(__global const uchar* p, uint o) { return vload4(0, (__global const uint*)(p + o)); }
static void kerf_wr_f1(__global uchar* p, uint o, float v) { *(__global float*)(p + o) = v; }
static void kerf_wr_f2(__global uchar* p, uint o, float2 v) { vstore2(v, 0, (__global float*)(p + o)); }
static void kerf_wr_f3(__global uchar* p, uint o, float3 v) { vstore3(v, 0, (__global float*)(p + o)); }
static void kerf_wr_f4(__global uchar* p, uint o, float4 v) { vstore4(v, 0, (__global float*)(p + o)); }
static void kerf_wr_i1(__global uchar* p, uint o, int v) { *(__global int*)(p + o) = v; }
static void kerf_wr_i2(__global uchar* p, uint o, int2 v) { vstore2(v, 0, (__global int*)(p + o)); }
static void kerf_wr_i3(__global uchar* p, uint o, int3 v) { vstore3(v, 0, (__global int*)(p + o)); }
static void kerf_wr_i4(__global uchar* p, uint o, int4 v) { vstore4(v, 0, (__global int*)(p + o)); }
static void kerf_wr_u1(__global uchar* p, uint o, uint v) { *(__global uint*)(p + o) = v; }
static void kerf_wr_u2(__global uchar* p, uint o, uint2 v) { vstore2(v, 0, (__global uint*)(p + o)); }
static void kerf_wr_u3(__global uchar* p, uint o, uint3 v) { vstore3(v, 0, (__global uint*)(p + o)); }
static void kerf_wr_u4(__global uchar* p, uint o, uint4 v) { vstore4(v, 0, (__global uint*)(p + o)); }
