//! Rust side of the `celt/mathops.{h,c}` bit-exactness probe.
//!
//! Mirrors `compat/c/probe_mathops.c` call for call and print for print.  The
//! inputs are generated with the identical LCG (`state = state*1103515245 +
//! 12345`) and are passed through `std::hint::black_box` so that the optimiser
//! cannot constant-fold a libm call (which could use a compile-time algorithm
//! instead of the system libm the C side calls).
//!
//! Run `bash compat/scripts/run-probe.sh mathops`; the only acceptable result
//! is `probe_mathops: IDENTICAL`.

use oprs::celt::mathops::*;
use std::hint::black_box;

const NRAW: usize = 96;
const NPOS: usize = 96;

struct Rng(u32);
impl Rng {
    fn rnd(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345);
        self.0
    }
}

fn pf(name: &str, i: usize, v: f32) {
    println!("{}[{}]={:08x}", name, i, v.to_bits());
}

fn pii(name: &str, i: usize, x: i32, v: i32) {
    println!("{}[{}]({})={}", name, i, x, v);
}

/* Interesting positive floats, as exact bit patterns. */
const FIXED_POS: [u32; 31] = [
    0x00000000, /* 0.0            */
    0x00000001, /* min denormal   */
    0x00400000, /* mid denormal   */
    0x00800000, /* FLT_MIN        */
    0x00800001, /* just above min */
    0x0d000000, /* ~6.5e-31       */
    0x1a000000, /* ~1.7e-23       */
    0x27800000, /* ~5.7e-15       */
    0x34000000, /* FLT_EPSILON    */
    0x3e800000, /* 0.25           */
    0x3f000000, /* 0.5            */
    0x3f400000, /* 0.75           */
    0x3f7fffff, /* 1-2^-24        */
    0x3f800000, /* 1.0            */
    0x3f800001, /* 1+2^-23        */
    0x3fc00000, /* 1.5            */
    0x40000000, /* 2.0            */
    0x40490fda, /* ~PI            */
    0x40800000, /* 4.0            */
    0x40a00000, /* 5.0            */
    0x41200000, /* 10.0           */
    0x42c80000, /* 100.0          */
    0x447a0000, /* 1000.0         */
    0x461c4000, /* 10000.0        */
    0x501502f9, /* 1e10           */
    0x5f0ac723, /* ~1e19          */
    0x6b5fca6b, /* ~1e26          */
    0x7149f2ca, /* 1e30           */
    0x7f000000, /* ~1.7e38        */
    0x7f7fffff, /* FLT_MAX        */
    0x7f800000, /* +Inf           */
];

struct Inputs {
    raw: [f32; NRAW],
    pos: [f32; NPOS],
}

fn build_inputs(rng: &mut Rng) -> Inputs {
    let mut raw = [0f32; NRAW];
    let mut pos = [0f32; NPOS];
    for i in 0..NRAW {
        raw[i] = f32::from_bits(rng.rnd());
    }
    for i in 0..NPOS {
        match i & 3 {
            0 => {
                /* positive finite, full exponent range (0 .. 0xfe) */
                pos[i] = f32::from_bits(rng.rnd() & 0x7f7fffff);
            }
            1 => {
                let k = (i >> 2) as i32; /* 0 .. NPOS/4-1 */
                let e = -40 + (k * 80) / ((NPOS / 4) as i32 - 1); /* -40 .. 40 */
                pos[i] = f32::from_bits(((e + 127) as u32) << 23);
            }
            2 => {
                pos[i] = f32::from_bits(FIXED_POS[(i >> 2) % FIXED_POS.len()]);
            }
            _ => {
                /* positive, <= ~1 */
                pos[i] = f32::from_bits(rng.rnd() & 0x3f7fffff);
            }
        }
    }
    Inputs { raw, pos }
}

fn print_inputs(inp: &Inputs) {
    for i in 0..NRAW {
        println!("raw[{}]={:08x}", i, inp.raw[i].to_bits());
    }
    for i in 0..NPOS {
        println!("pos[{}]={:08x}", i, inp.pos[i].to_bits());
    }
}

fn print_isqrt32(inp: &Inputs) {
    for i in 0..NRAW {
        let v = inp.raw[i].to_bits() | 1;
        println!("isqrt32[{}]({})={}", i, v, isqrt32(v));
    }
    println!("isqrt32_edge[0]={}", isqrt32(1));
    println!("isqrt32_edge[1]={}", isqrt32(2));
    println!("isqrt32_edge[2]={}", isqrt32(0xffffffff));
    println!("isqrt32_edge[3]={}", isqrt32(0x7fffffff));
    println!("isqrt32_edge[4]={}", isqrt32(0x80000000));
    println!("isqrt32_edge[5]={}", isqrt32(0x40000000));
}

fn print_lcg(rng: &mut Rng) {
    let mut s = rng.rnd();
    for i in 0..16 {
        s = celt_lcg_rand(s);
        println!("celt_lcg_rand[{}]={:08x}", i, s);
    }
}

fn print_bitexact(rng: &mut Rng) {
    for i in 0..NRAW {
        let x = (rng.rnd() & 0x7fff) as i16; /* [0, 32767] */
        pii("bitexact_cos", i, x as i32, bitexact_cos(x) as i32);
    }
    for i in 0..16 {
        let x = rng.rnd() as i16;
        pii("bitexact_cos_neg", i, x as i32, bitexact_cos(x) as i32);
    }
    for i in 0..NRAW {
        let isin = 1 + (rng.rnd() % 32767) as i32;
        let icos = 1 + (rng.rnd() % 32767) as i32;
        println!(
            "bitexact_log2tan[{}]({},{})={}",
            i,
            isin,
            icos,
            bitexact_log2tan(isin, icos)
        );
    }
    println!("bitexact_log2tan_known[0]={}", bitexact_log2tan(32767, 200));
    println!(
        "bitexact_log2tan_known[1]={}",
        bitexact_log2tan(30274, 12540)
    );
    println!(
        "bitexact_log2tan_known[2]={}",
        bitexact_log2tan(23171, 23171)
    );
    println!("bitexact_cos_known[0]={}", bitexact_cos(64));
    println!("bitexact_cos_known[1]={}", bitexact_cos(16320));
    println!("bitexact_cos_known[2]={}", bitexact_cos(8192));
}

fn print_float_raw(inp: &Inputs) {
    for i in 0..NRAW {
        let x = black_box(inp.raw[i]);
        pf("celt_rcp", i, celt_rcp(x));
        pf("celt_div", i, celt_div(x, black_box(inp.pos[i])));
        pf("frac_div32", i, frac_div32(x, black_box(inp.pos[i])));
        pf(
            "frac_div32_q29",
            i,
            frac_div32_q29(x, black_box(inp.pos[i])),
        );
        pf("celt_cos_norm", i, celt_cos_norm(x));
        pf("celt_log2", i, celt_log2(x));
        pf("celt_exp2", i, celt_exp2(x));
        pf("celt_log", i, celt_log(x));
        pf("celt_exp", i, celt_exp(x));
        pf("celt_atan_norm", i, celt_atan_norm(x));
        pf("celt_cos_norm2", i, celt_cos_norm2(x));
        pf("celt_sin", i, celt_sin(x));
        pf(
            "fast_atan2f",
            i,
            fast_atan2f(x, black_box(inp.raw[(i + 13) % NRAW])),
        );
    }
}

fn print_float_pos(inp: &Inputs) {
    for i in 0..NPOS {
        let x = black_box(inp.pos[i]);
        pf("celt_sqrt", i, celt_sqrt(x));
        pf("celt_sqrt32", i, celt_sqrt32(x));
        pf("celt_rsqrt", i, celt_rsqrt(x));
        pf("celt_rsqrt_norm", i, celt_rsqrt_norm(x));
        pf("celt_rsqrt_norm32", i, celt_rsqrt_norm32(x));
        pf("celt_log2_pos", i, celt_log2(x));
        pf("celt_exp2_pos", i, celt_exp2(x));
        pf("celt_exp2_db_pos", i, celt_exp2_db(x));
        pf("celt_log2_db_pos", i, celt_log2_db(x));
        pf(
            "celt_atan2p_norm",
            i,
            celt_atan2p_norm(x, black_box(inp.pos[(i + 7) % NPOS])),
        );
    }
}

fn print_maxabs(inp: &Inputs) {
    const LENS: [i32; 12] = [0, 1, 2, 3, 5, 8, 13, 16, 32, 48, 64, 96];
    for &len in LENS.iter() {
        let mut tmp = [0f32; NRAW];
        for i in 0..NRAW {
            tmp[i] = black_box(inp.raw[i]);
        }
        pf("celt_maxabs16", len as usize, celt_maxabs16(&tmp, len));
        pf("celt_maxabs32", len as usize, celt_maxabs32(&tmp, len));
        pf("celt_maxabs_res", len as usize, celt_maxabs_res(&tmp, len));
    }
}

fn print_float_api(inp: &Inputs) {
    let mut inbuf = [0f32; NRAW];
    let mut outbuf = [0i16; NRAW];
    for i in 0..NRAW {
        inbuf[i] = black_box(inp.raw[i]);
    }
    celt_float2int16_c(&inbuf, &mut outbuf, NRAW as i32);
    for i in 0..NRAW {
        println!("celt_float2int16[{}]={:04x}", i, outbuf[i] as u16);
    }

    let mut lim = [0f32; NRAW];
    for i in 0..NRAW {
        lim[i] = black_box(inp.raw[i]);
    }
    let r = opus_limit2_checkwithin1_c(&mut lim, NRAW as i32);
    println!("opus_limit2_checkwithin1_ret={}", r);
    for i in 0..NRAW {
        pf("opus_limit2_checkwithin1", i, lim[i]);
    }
}

fn main() {
    let mut rng = Rng(0x12345678);
    let inp = build_inputs(&mut rng);
    print_inputs(&inp);
    print_isqrt32(&inp);
    print_lcg(&mut rng);
    print_bitexact(&mut rng);
    print_float_raw(&inp);
    print_float_pos(&inp);
    print_maxabs(&inp);
    print_float_api(&inp);
}
