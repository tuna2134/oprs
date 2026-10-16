//! Rust side of the SILK-foundation bit-exactness probe.
//!
//! Mirrors `compat/c/probe_silkbase.c` line for line: the integer primitives of
//! `silk/macros.h` / `silk/SigProc_FIX.h` / `silk/Inlines.h`, an FNV-1a hash of
//! every constant table, and the sizes/offsets of the pointer-free state
//! structures.  Run `bash compat/scripts/run-probe.sh silkbase`.

use oprs::silk::define::*;
use oprs::silk::fixed::structs_fix::*;
use oprs::silk::inlines::*;
use oprs::silk::macros::*;
use oprs::silk::pitch_est_tables::*;
use oprs::silk::resampler_rom::*;
use oprs::silk::sigproc_fix::*;
use oprs::silk::structs::*;
use oprs::silk::table_lsf_cos::*;
use oprs::silk::tables_gain::*;
use oprs::silk::tables_ltp::*;
use oprs::silk::tables_nlsf_cb_nb_mb::*;
use oprs::silk::tables_nlsf_cb_wb::*;
use oprs::silk::tables_other::*;
use oprs::silk::tables_pitch_lag::*;
use oprs::silk::tables_pulses_per_block::*;
use oprs::silk::control::silk_EncControlStruct;
use std::mem::{offset_of, size_of};

/* ------------------------------------------------------------------ */
/* Deterministic input set                                             */
/* ------------------------------------------------------------------ */

const NTV: usize = 64;

static TV_INIT: [i32; 48] = [
    0, 1, -1, 2, -2, 3, -3, 7, -7, 15, -15, 16, -16, 127, -127, 128, -128, 255, -255, 256, -256,
    32767, -32768, 32768, 65535, -65536, 1000000, -1000000, 0x7FFFFFFF, -0x80000000, 0x7FFFFFFE,
    -0x7FFFFFFF, 0x55555555, -0x55555556, 0x0000FFFF, -0x00010000, 0x00FF00FF, -0x00FF0100,
    0x12345678, -0x789ABCDF, 0x40000000, -0x40000000, 1664525, -1664525, 46340, -46341,
    -0x21524111, 0x0BADF00D,
];

fn init_tv() -> [i32; NTV] {
    let mut tv = [0i32; NTV];
    let mut state: u32 = 12345;
    let n = TV_INIT.len();
    tv[..n].copy_from_slice(&TV_INIT);
    for i in n..NTV {
        state = state.wrapping_mul(1103515245).wrapping_add(12345);
        tv[i] = state as i32;
    }
    tv
}

const NU: usize = 32;

fn pv(tv: &[i32; NTV], i: usize) -> i32 {
    tv[i % NTV]
}
fn pw(tv: &[i32; NTV], i: usize) -> i32 {
    tv[(i * 5 + 7) % NTV]
}
/* See probe_silkbase.c: `silk_int32_MIN` normalises to zero and traps. */
fn div_b(tv: &[i32; NTV], i: usize) -> i32 {
    let mut b = pw(tv, i);
    if b == 0 {
        b = 3;
    }
    if b == i32::MIN {
        b = -3;
    }
    b
}

macro_rules! u1 {
    ($tv:expr, $name:expr, $f:expr) => {
        for i in 0..NU {
            let x = pv($tv, i);
            println!("{}({}) -> {}", $name, x, $f(x));
        }
    };
}
macro_rules! u1_64 {
    ($tv:expr, $name:expr, $f:expr) => {
        for i in 0..NU {
            let x = pv($tv, i);
            println!("{}({}) -> {}", $name, x, $f(x));
        }
    };
}
macro_rules! b2 {
    ($tv:expr, $name:expr, $f:expr) => {
        for i in 0..NU {
            let (a, b) = (pv($tv, i), pw($tv, i));
            println!("{}({},{}) -> {}", $name, a, b, $f(a, b));
        }
    };
}
macro_rules! b2i {
    ($tv:expr, $name:expr, $f:expr) => {
        for i in 0..NU {
            let (a, b) = (pv($tv, i), pw($tv, i));
            println!("{}({},{}) -> {}", $name, a, b, $f(a, b, i));
        }
    };
}
macro_rules! b2_64 {
    ($tv:expr, $name:expr, $f:expr) => {
        for i in 0..NU {
            let (a, b) = (pv($tv, i), pw($tv, i));
            println!("{}({},{}) -> {}", $name, a, b, $f(a, b));
        }
    };
}
macro_rules! b2i_64 {
    ($tv:expr, $name:expr, $f:expr) => {
        for i in 0..NU {
            let (a, b) = (pv($tv, i), pw($tv, i));
            println!("{}({},{}) -> {}", $name, a, b, $f(a, b, i));
        }
    };
}
macro_rules! divb2 {
    ($tv:expr, $name:expr, $f:expr) => {
        for i in 0..NU {
            let (a, b) = (pv($tv, i), div_b($tv, i));
            println!("{}({},{}) -> {}", $name, a, b, $f(a, b));
        }
    };
}
macro_rules! sh32 {
    ($tv:expr, $name:expr, $f:expr) => {
        for i in 0..12usize {
            let x = pv($tv, i * 5);
            for s in [0, 1, 3, 7, 15, 30] {
                println!("{}({},{}) -> {}", $name, x, s, $f(x, s));
            }
        }
    };
}
macro_rules! sh32i {
    ($tv:expr, $name:expr, $f:expr) => {
        for i in 0..12usize {
            let x = pv($tv, i * 5);
            for s in [0, 1, 3, 7, 15, 30] {
                println!("{}({},{}) -> {}", $name, x, s, $f(x, s, i));
            }
        }
    };
}
macro_rules! sh32_64 {
    ($tv:expr, $name:expr, $f:expr) => {
        for i in 0..12usize {
            let x = pv($tv, i * 5);
            for s in [0, 1, 15, 31, 32, 63] {
                println!("{}({},{}) -> {}", $name, x, s, $f(x, s));
            }
        }
    };
}
macro_rules! sh16 {
    ($tv:expr, $name:expr, $f:expr) => {
        for i in 0..12usize {
            let x = pv($tv, i * 5) as i16;
            for s in [0, 1, 3, 7, 15] {
                println!("{}({},{}) -> {}", $name, x, s, $f(x, s));
            }
        }
    };
}
macro_rules! sh8 {
    ($tv:expr, $name:expr, $f:expr) => {
        for i in 0..12usize {
            let x = pv($tv, i * 5) as i8;
            for s in [0, 1, 3, 7] {
                println!("{}({},{}) -> {}", $name, x, s, $f(x, s));
            }
        }
    };
}
macro_rules! rot {
    ($tv:expr, $name:expr, $f:expr) => {
        for i in 0..12usize {
            let x = pv($tv, i * 5);
            for r in [-31, -16, -1, 0, 1, 16, 31] {
                println!("{}({},{}) -> {}", $name, x, r, $f(x, r));
            }
        }
    };
}
macro_rules! b2_u32i {
    ($tv:expr, $name:expr, $f:expr) => {
        for i in 0..NU {
            let (a, b) = (pv($tv, i) as u32, pw($tv, i) as u32);
            println!("{}({},{}) -> {}", $name, a, b, $f(a, b, i));
        }
    };
}
macro_rules! u32_1 {
    ($tv:expr, $name:expr, $f:expr) => {
        for i in 0..NU {
            let x = pv($tv, i) as u32;
            println!("{}({}) -> {}", $name, x, $f(x));
        }
    };
}
macro_rules! b2_u32 {
    ($tv:expr, $name:expr, $f:expr) => {
        for i in 0..NU {
            let (a, b) = (pv($tv, i) as u32, pw($tv, i) as u32);
            println!("{}({},{}) -> {}", $name, a, b, $f(a, b));
        }
    };
}

fn probe_macros(tv: &[i32; NTV]) {
    println!("=== macros ===");

    /* --- simple 32-bit unary --- */
    u1!(tv, "silk_abs", silk_abs);
    u1!(tv, "silk_abs_int", silk_abs_int);
    u1!(tv, "silk_abs_int32", silk_abs_int32);
    u1!(tv, "silk_sign", silk_sign);
    u1!(tv, "silk_SAT8", silk_SAT8);
    u1!(tv, "silk_SAT16", silk_SAT16);
    u1!(tv, "silk_SAT32", silk_SAT32);
    u1!(tv, "silk_POS_SAT32", silk_POS_SAT32);
    u1!(tv, "silk_CHECK_FIT8", silk_CHECK_FIT8);
    u1!(tv, "silk_CHECK_FIT16", silk_CHECK_FIT16);
    u1!(tv, "silk_CHECK_FIT32", silk_CHECK_FIT32);
    u1!(tv, "silk_CLZ32", silk_CLZ32);
    u1!(tv, "silk_min_int", |x| silk_min_int(x, 17));
    u1!(tv, "silk_max_int", |x| silk_max_int(x, -17));
    u1!(tv, "silk_LIMIT", |x| silk_LIMIT(x, -1000, 1000));
    u1!(tv, "silk_RAND", silk_RAND);

    /* --- 64-bit unary --- */
    u1_64!(tv, "silk_abs_int64", |x: i32| silk_abs_int64(x as i64));
    u1_64!(tv, "silk_CLZ64", |x: i32| silk_CLZ64(x as i64));

    /* --- 16-bit unary --- */
    for i in 0..NU {
        let x16 = pv(tv, i) as i16;
        let other = if x16 > 0 { -3i16 } else { 3i16 };
        println!("silk_CLZ16({}) -> {}", x16, silk_CLZ16(x16));
        println!("silk_min_16({},{}) -> {}", x16, other, silk_min_16(x16, other));
        println!("silk_max_16({},{}) -> {}", x16, other, silk_max_16(x16, other));
    }

    b2!(tv, "silk_min_32", silk_min_32);
    b2!(tv, "silk_max_32", silk_max_32);

    /* --- 16-bit-intermediate multiplies --- */
    b2!(tv, "silk_SMULWB", silk_SMULWB);
    b2i!(tv, "silk_SMLAWB", |a, b, i| silk_SMLAWB(a, b, pv(tv, i * 3)));
    b2!(tv, "silk_SMULWT", silk_SMULWT);
    b2i!(tv, "silk_SMLAWT", |a, b, i| silk_SMLAWT(a, b, pv(tv, i * 3)));
    b2!(tv, "silk_SMULBB", silk_SMULBB);
    b2i!(tv, "silk_SMLABB", |a, b, i| silk_SMLABB(a, b, pv(tv, i * 3)));
    b2!(tv, "silk_SMULBT", silk_SMULBT);
    b2i!(tv, "silk_SMLABT", |a, b, i| silk_SMLABT(a, b, pv(tv, i * 3)));
    b2!(tv, "silk_SMULTT", silk_SMULTT);
    b2i!(tv, "silk_SMLATT", |a, b, i| silk_SMLATT(a, b, pv(tv, i * 3)));
    b2!(tv, "silk_SMULWW", silk_SMULWW);
    b2i!(tv, "silk_SMLAWW", |a, b, i| silk_SMLAWW(a, b, pv(tv, i * 3)));
    b2!(tv, "silk_SMMUL", silk_SMMUL);
    b2!(tv, "silk_MUL", silk_MUL);
    b2i!(tv, "silk_MLA", |a, b, i| silk_MLA(a, b, pv(tv, i * 3)));
    b2i!(tv, "silk_MLA_ovflw", |a, b, i| silk_MLA_ovflw(a, b, pv(tv, i * 3)));
    b2i!(tv, "silk_SMLABB_ovflw", |a, b, i| silk_SMLABB_ovflw(a, b, pv(tv, i * 3)));
    b2!(tv, "silk_ADD32", silk_ADD32);
    b2!(tv, "silk_SUB32", silk_SUB32);
    b2!(tv, "silk_ADD32_ovflw", silk_ADD32_ovflw);
    b2!(tv, "silk_SUB32_ovflw", silk_SUB32_ovflw);
    b2!(tv, "silk_ADD_SAT32", silk_ADD_SAT32);
    b2!(tv, "silk_SUB_SAT32", silk_SUB_SAT32);
    b2!(tv, "silk_ADD_POS_SAT32", silk_ADD_POS_SAT32);
    b2!(tv, "silk_ADD_POS_SAT16", silk_ADD_POS_SAT16);
    b2!(tv, "silk_ADD_POS_SAT8", silk_ADD_POS_SAT8);
    b2!(tv, "silk_NSHIFT_MUL_32_32", silk_NSHIFT_MUL_32_32);

    /* --- 16/8-bit operands --- */
    b2!(tv, "silk_ADD16", |a, b| silk_ADD16(a as i16, b as i16));
    b2!(tv, "silk_SUB16", |a, b| silk_SUB16(a as i16, b as i16));
    b2!(tv, "silk_ADD_SAT16", |a, b| silk_ADD_SAT16(a as i16, b as i16));
    b2!(tv, "silk_SUB_SAT16", |a, b| silk_SUB_SAT16(a as i16, b as i16));
    b2!(tv, "silk_NSHIFT_MUL_16_16", |a, b| silk_NSHIFT_MUL_16_16(a as i16, b as i16));

    /* --- 64-bit arithmetic --- */
    b2_64!(tv, "silk_ADD64", |a: i32, b: i32| silk_ADD64(a as i64, b as i64));
    b2_64!(tv, "silk_SUB64", |a: i32, b: i32| silk_SUB64(a as i64, b as i64));
    b2_64!(tv, "silk_ADD_SAT64", |a: i32, b: i32| silk_ADD_SAT64(a as i64, b as i64));
    b2_64!(tv, "silk_SUB_SAT64", |a: i32, b: i32| silk_SUB_SAT64(a as i64, b as i64));
    b2_64!(tv, "silk_SMULL", |a: i32, b: i32| silk_SMULL(a, b));
    b2i_64!(tv, "silk_SMLAL", |a: i32, b: i32, i| silk_SMLAL(a as i64, b, pv(tv, i * 3)));
    b2i_64!(tv, "silk_SMLALBB", |a: i32, b: i32, i| silk_SMLALBB(
        a as i64,
        b as i16,
        pv(tv, i * 3) as i16
    ));
    b2_64!(tv, "silk_min_64", |a: i32, b: i32| silk_min_64(a as i64, b as i64));
    b2_64!(tv, "silk_max_64", |a: i32, b: i32| silk_max_64(a as i64, b as i64));

    /* --- unsigned --- */
    u32_1!(tv, "silk_LSHIFT_uint", |x| silk_LSHIFT_uint(x, 3));
    u32_1!(tv, "silk_RSHIFT_uint", |x| silk_RSHIFT_uint(x, 3));
    b2_u32!(tv, "silk_MUL_uint", silk_MUL_uint);
    b2_u32i!(tv, "silk_MLA_uint", |a, b, i| silk_MLA_uint(a, b, pv(tv, i * 3) as u32));
    b2_u32!(tv, "silk_ADD_LSHIFT_uint", |a, b| silk_ADD_LSHIFT_uint(a, b, 3));
    b2_u32!(tv, "silk_ADD_RSHIFT_uint", |a, b| silk_ADD_RSHIFT_uint(a, b, 3));

    /* --- shifts --- */
    sh32!(tv, "silk_LSHIFT", |x, s| silk_LSHIFT(x, s));
    sh32!(tv, "silk_LSHIFT32", |x, s| silk_LSHIFT32(x, s));
    sh32!(tv, "silk_LSHIFT_ovflw", |x, s| silk_LSHIFT_ovflw(x, s));
    sh32!(tv, "silk_RSHIFT", |x, s| silk_RSHIFT(x, s));
    sh32!(tv, "silk_RSHIFT32", |x, s| silk_RSHIFT32(x, s));
    sh32!(tv, "silk_LSHIFT_SAT32", |x, s| silk_LSHIFT_SAT32(x, s));
    sh32!(tv, "silk_RSHIFT_ROUND", |x, s| silk_RSHIFT_ROUND(x, if s > 0 { s } else { 1 }));
    sh32i!(tv, "silk_ADD_LSHIFT", |x, s, i| silk_ADD_LSHIFT(x, pv(tv, i * 3), s));
    sh32i!(tv, "silk_ADD_LSHIFT32", |x, s, i| silk_ADD_LSHIFT32(x, pv(tv, i * 3), s));
    sh32i!(tv, "silk_ADD_RSHIFT", |x, s, i| silk_ADD_RSHIFT(x, pv(tv, i * 3), s));
    sh32i!(tv, "silk_ADD_RSHIFT32", |x, s, i| silk_ADD_RSHIFT32(x, pv(tv, i * 3), s));
    sh32i!(tv, "silk_SUB_LSHIFT32", |x, s, i| silk_SUB_LSHIFT32(x, pv(tv, i * 3), s));
    sh32i!(tv, "silk_SUB_RSHIFT32", |x, s, i| silk_SUB_RSHIFT32(x, pv(tv, i * 3), s));
    sh32_64!(tv, "silk_LSHIFT64", |x, s| silk_LSHIFT64(x as i64, s));
    sh32_64!(tv, "silk_RSHIFT64", |x, s| silk_RSHIFT64(x as i64, s));
    sh32_64!(tv, "silk_RSHIFT_ROUND64", |x, s| silk_RSHIFT_ROUND64(
        x as i64,
        if s > 0 { s } else { 1 }
    ));
    sh16!(tv, "silk_LSHIFT16", |x, s| silk_LSHIFT16(x, s));
    sh16!(tv, "silk_RSHIFT16", |x, s| silk_RSHIFT16(x, s));
    sh8!(tv, "silk_LSHIFT8", |x, s| silk_LSHIFT8(x, s));
    sh8!(tv, "silk_RSHIFT8", |x, s| silk_RSHIFT8(x, s));
    rot!(tv, "silk_ROR32", |x, r| silk_ROR32(x, r));

    /* --- division --- */
    divb2!(tv, "silk_DIV32_16", silk_DIV32_16);
    divb2!(tv, "silk_DIV32", silk_DIV32);

    {
        const BVALS: [i32; 6] = [1, -1, 1000, -3, 0x12345678, 0x7FFFFFFF];
        const QVALS: [i32; 4] = [0, 7, 16, 31];
        for i in 0..NU / 2 {
            for &bv in BVALS.iter() {
                for &qv in QVALS.iter() {
                    println!(
                        "silk_DIV32_varQ({},{},{}) -> {}",
                        pv(tv, i),
                        bv,
                        qv,
                        silk_DIV32_varQ(pv(tv, i), bv, qv)
                    );
                }
                for &qv in QVALS[1..].iter() {
                    println!(
                        "silk_INVERSE32_varQ({},{}) -> {}",
                        bv,
                        qv,
                        silk_INVERSE32_varQ(bv, qv)
                    );
                }
            }
        }
    }

    /* --- square root over a wide range --- */
    for i in 0..NU {
        let x = ((pv(tv, i) as u32) >> 8) as i32;
        println!("silk_SQRT_APPROX({}) -> {}", x, silk_SQRT_APPROX(x));
    }
    let mut i = 1;
    while i < 2200 {
        println!("silk_SQRT_APPROX({}) -> {}", i, silk_SQRT_APPROX(i));
        i += 97;
    }
    println!("silk_SQRT_APPROX({}) -> {}", 0x7FFFFFFF, silk_SQRT_APPROX(0x7FFFFFFF));

    /* --- CLZ_FRAC (two results) --- */
    for i in 0..NU {
        let x = pv(tv, i);
        for s in 0..2 {
            let v = if s == 0 { x } else { ((x as u32) >> 1) as i32 };
            let (lz, frac) = silk_CLZ_FRAC(v);
            println!("silk_CLZ_FRAC({}) -> {} {}", v, lz, frac);
        }
    }

    /* --- matrix accessors (row based N=7, column based M=5) --- */
    {
        let mut m = [[0i32; 7]; 5];
        let mut mc = [[0i32; 5]; 7];
        for r in 0..5i32 {
            for c in 0..7i32 {
                *matrix_ptr_mut(m.as_flattened_mut(), r, c, 7) = (r * 7 + c) * 3 - 11;
            }
        }
        for c in 0..7i32 {
            for r in 0..5i32 {
                *matrix_c_ptr_mut(mc.as_flattened_mut(), r, c, 5) = (r * 31 + c) * 5 + 1;
            }
        }
        let mut sum = 0i32;
        for r in 0..5i32 {
            for c in 0..7i32 {
                sum += matrix_ptr(m.as_flattened(), r, c, 7) * 2 + *matrix_adr(mc.as_flattened(), c, r, 5);
            }
        }
        println!("matrix sum -> {}", sum);
    }
}

/* ------------------------------------------------------------------ */
/* 2. tables                                                           */
/* ------------------------------------------------------------------ */

fn fnv_upd(h: &mut u64, byte: u8) {
    *h ^= byte as u64;
    *h = h.wrapping_mul(0x100000001b3);
}

const FNV_OFFSET: u64 = 0xcbf29ce484222325;

fn fnv_u8(s: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    for &v in s {
        fnv_upd(&mut h, v);
    }
    h
}
fn fnv_i8(s: &[i8]) -> u64 {
    let mut h = FNV_OFFSET;
    for &v in s {
        fnv_upd(&mut h, v as u8);
    }
    h
}
fn fnv_i16(s: &[i16]) -> u64 {
    let mut h = FNV_OFFSET;
    for &v in s {
        for b in v.to_le_bytes() {
            fnv_upd(&mut h, b);
        }
    }
    h
}
fn fnv_i32(s: &[i32]) -> u64 {
    let mut h = FNV_OFFSET;
    for &v in s {
        for b in v.to_le_bytes() {
            fnv_upd(&mut h, b);
        }
    }
    h
}

macro_rules! tbl {
    ($name:expr, $data:expr, $count:expr, $f:ident) => {
        println!("table {} n={} fnv={:016x}", $name, $count, $f($data));
    };
}

fn probe_tables() {
    println!("=== tables ===");

    tbl!("silk_gain_iCDF", silk_gain_iCDF.as_flattened(), 3 * 8, fnv_u8);
    tbl!("silk_delta_gain_iCDF", &silk_delta_gain_iCDF, 41, fnv_u8);
    tbl!("silk_pitch_lag_iCDF", &silk_pitch_lag_iCDF, 32, fnv_u8);
    tbl!("silk_pitch_delta_iCDF", &silk_pitch_delta_iCDF, 21, fnv_u8);
    tbl!("silk_pitch_contour_iCDF", &silk_pitch_contour_iCDF, 34, fnv_u8);
    tbl!("silk_pitch_contour_NB_iCDF", &silk_pitch_contour_NB_iCDF, 11, fnv_u8);
    tbl!(
        "silk_pitch_contour_10_ms_iCDF",
        &silk_pitch_contour_10_ms_iCDF,
        12,
        fnv_u8
    );
    tbl!(
        "silk_pitch_contour_10_ms_NB_iCDF",
        &silk_pitch_contour_10_ms_NB_iCDF,
        3,
        fnv_u8
    );
    tbl!("silk_max_pulses_table", &silk_max_pulses_table, 4, fnv_u8);
    tbl!(
        "silk_pulses_per_block_iCDF",
        silk_pulses_per_block_iCDF.as_flattened(),
        10 * 18,
        fnv_u8
    );
    tbl!(
        "silk_pulses_per_block_BITS_Q5",
        silk_pulses_per_block_BITS_Q5.as_flattened(),
        9 * 18,
        fnv_u8
    );
    tbl!("silk_rate_levels_iCDF", silk_rate_levels_iCDF.as_flattened(), 2 * 9, fnv_u8);
    tbl!(
        "silk_rate_levels_BITS_Q5",
        silk_rate_levels_BITS_Q5.as_flattened(),
        2 * 9,
        fnv_u8
    );
    tbl!("silk_shell_code_table0", &silk_shell_code_table0, 152, fnv_u8);
    tbl!("silk_shell_code_table1", &silk_shell_code_table1, 152, fnv_u8);
    tbl!("silk_shell_code_table2", &silk_shell_code_table2, 152, fnv_u8);
    tbl!("silk_shell_code_table3", &silk_shell_code_table3, 152, fnv_u8);
    tbl!(
        "silk_shell_code_table_offsets",
        &silk_shell_code_table_offsets,
        17,
        fnv_u8
    );
    tbl!("silk_sign_iCDF", &silk_sign_iCDF, 42, fnv_u8);

    tbl!("silk_LTP_per_index_iCDF", &silk_LTP_per_index_iCDF, 3, fnv_u8);
    for i in 0..NB_LTP_CBKS as usize {
        let n = 8usize << i;
        tbl!(
            format!("silk_LTP_gain_iCDF_{}", i),
            silk_LTP_gain_iCDF_ptrs[i],
            n,
            fnv_u8
        );
        tbl!(
            format!("silk_LTP_gain_BITS_Q5_{}", i),
            silk_LTP_gain_BITS_Q5_ptrs[i],
            n,
            fnv_u8
        );
        tbl!(
            format!("silk_LTP_gain_vq_{}", i),
            silk_LTP_vq_ptrs_Q7[i],
            n * 5,
            fnv_i8
        );
        tbl!(
            format!("silk_LTP_gain_vq_{}_gain", i),
            silk_LTP_vq_gain_ptrs_Q7[i],
            n,
            fnv_u8
        );
    }
    tbl!("silk_LTP_vq_sizes", &silk_LTP_vq_sizes, 3, fnv_i8);

    tbl!("silk_NLSF_CB1_NB_MB_Q8", &silk_NLSF_CB1_NB_MB_Q8, 320, fnv_u8);
    tbl!("silk_NLSF_CB1_Wght_Q9", &silk_NLSF_CB1_Wght_Q9, 320, fnv_i16);
    tbl!("silk_NLSF_CB1_iCDF_NB_MB", &silk_NLSF_CB1_iCDF_NB_MB, 64, fnv_u8);
    tbl!(
        "silk_NLSF_CB2_SELECT_NB_MB",
        &silk_NLSF_CB2_SELECT_NB_MB,
        160,
        fnv_u8
    );
    tbl!("silk_NLSF_CB2_iCDF_NB_MB", &silk_NLSF_CB2_iCDF_NB_MB, 72, fnv_u8);
    tbl!(
        "silk_NLSF_CB2_BITS_NB_MB_Q5",
        &silk_NLSF_CB2_BITS_NB_MB_Q5,
        72,
        fnv_u8
    );
    tbl!("silk_NLSF_PRED_NB_MB_Q8", &silk_NLSF_PRED_NB_MB_Q8, 18, fnv_u8);
    tbl!(
        "silk_NLSF_DELTA_MIN_NB_MB_Q15",
        &silk_NLSF_DELTA_MIN_NB_MB_Q15,
        11,
        fnv_i16
    );
    println!(
        "nlsfcb silk_NLSF_CB_NB_MB nVectors={} order={} quantStepSize_Q16={} invQuantStepSize_Q6={}",
        silk_NLSF_CB_NB_MB.nVectors,
        silk_NLSF_CB_NB_MB.order,
        silk_NLSF_CB_NB_MB.quantStepSize_Q16,
        silk_NLSF_CB_NB_MB.invQuantStepSize_Q6
    );

    tbl!("silk_NLSF_CB1_WB_Q8", &silk_NLSF_CB1_WB_Q8, 512, fnv_u8);
    tbl!("silk_NLSF_CB1_WB_Wght_Q9", &silk_NLSF_CB1_WB_Wght_Q9, 512, fnv_i16);
    tbl!("silk_NLSF_CB1_iCDF_WB", &silk_NLSF_CB1_iCDF_WB, 64, fnv_u8);
    tbl!("silk_NLSF_CB2_SELECT_WB", &silk_NLSF_CB2_SELECT_WB, 256, fnv_u8);
    tbl!("silk_NLSF_CB2_iCDF_WB", &silk_NLSF_CB2_iCDF_WB, 72, fnv_u8);
    tbl!("silk_NLSF_CB2_BITS_WB_Q5", &silk_NLSF_CB2_BITS_WB_Q5, 72, fnv_u8);
    tbl!("silk_NLSF_PRED_WB_Q8", &silk_NLSF_PRED_WB_Q8, 30, fnv_u8);
    tbl!(
        "silk_NLSF_DELTA_MIN_WB_Q15",
        &silk_NLSF_DELTA_MIN_WB_Q15,
        17,
        fnv_i16
    );
    println!(
        "nlsfcb silk_NLSF_CB_WB nVectors={} order={} quantStepSize_Q16={} invQuantStepSize_Q6={}",
        silk_NLSF_CB_WB.nVectors,
        silk_NLSF_CB_WB.order,
        silk_NLSF_CB_WB.quantStepSize_Q16,
        silk_NLSF_CB_WB.invQuantStepSize_Q6
    );

    tbl!(
        "silk_stereo_pred_quant_Q13",
        &silk_stereo_pred_quant_Q13,
        16,
        fnv_i16
    );
    tbl!(
        "silk_stereo_pred_joint_iCDF",
        &silk_stereo_pred_joint_iCDF,
        25,
        fnv_u8
    );
    tbl!(
        "silk_stereo_only_code_mid_iCDF",
        &silk_stereo_only_code_mid_iCDF,
        2,
        fnv_u8
    );
    tbl!("silk_LBRR_flags_2_iCDF", silk_LBRR_flags_iCDF_ptr[0], 3, fnv_u8);
    tbl!("silk_LBRR_flags_3_iCDF", silk_LBRR_flags_iCDF_ptr[1], 7, fnv_u8);
    tbl!("silk_lsb_iCDF", &silk_lsb_iCDF, 2, fnv_u8);
    tbl!("silk_LTPscale_iCDF", &silk_LTPscale_iCDF, 3, fnv_u8);
    tbl!("silk_type_offset_VAD_iCDF", &silk_type_offset_VAD_iCDF, 4, fnv_u8);
    tbl!(
        "silk_type_offset_no_VAD_iCDF",
        &silk_type_offset_no_VAD_iCDF,
        2,
        fnv_u8
    );
    tbl!(
        "silk_NLSF_interpolation_factor_iCDF",
        &silk_NLSF_interpolation_factor_iCDF,
        5,
        fnv_u8
    );
    tbl!("silk_LTPScales_table_Q14", &silk_LTPScales_table_Q14, 3, fnv_i16);
    tbl!("silk_uniform3_iCDF", &silk_uniform3_iCDF, 3, fnv_u8);
    tbl!("silk_uniform4_iCDF", &silk_uniform4_iCDF, 4, fnv_u8);
    tbl!("silk_uniform5_iCDF", &silk_uniform5_iCDF, 5, fnv_u8);
    tbl!("silk_uniform6_iCDF", &silk_uniform6_iCDF, 6, fnv_u8);
    tbl!("silk_uniform8_iCDF", &silk_uniform8_iCDF, 8, fnv_u8);
    tbl!("silk_NLSF_EXT_iCDF", &silk_NLSF_EXT_iCDF, 7, fnv_u8);
    tbl!(
        "silk_Transition_LP_B_Q28",
        silk_Transition_LP_B_Q28.as_flattened(),
        5 * 3,
        fnv_i32
    );
    tbl!(
        "silk_Transition_LP_A_Q28",
        silk_Transition_LP_A_Q28.as_flattened(),
        5 * 2,
        fnv_i32
    );
    tbl!(
        "silk_Quantization_Offsets_Q10",
        silk_Quantization_Offsets_Q10.as_flattened(),
        2 * 2,
        fnv_i16
    );
    tbl!("silk_LSFCosTab_FIX_Q12", &silk_LSFCosTab_FIX_Q12, 129, fnv_i16);

    tbl!("silk_Resampler_3_4_COEFS", &silk_Resampler_3_4_COEFS, 29, fnv_i16);
    tbl!("silk_Resampler_2_3_COEFS", &silk_Resampler_2_3_COEFS, 20, fnv_i16);
    tbl!("silk_Resampler_1_2_COEFS", &silk_Resampler_1_2_COEFS, 14, fnv_i16);
    tbl!("silk_Resampler_1_3_COEFS", &silk_Resampler_1_3_COEFS, 20, fnv_i16);
    tbl!("silk_Resampler_1_4_COEFS", &silk_Resampler_1_4_COEFS, 20, fnv_i16);
    tbl!("silk_Resampler_1_6_COEFS", &silk_Resampler_1_6_COEFS, 20, fnv_i16);
    tbl!(
        "silk_Resampler_2_3_COEFS_LQ",
        &silk_Resampler_2_3_COEFS_LQ,
        6,
        fnv_i16
    );
    tbl!(
        "silk_resampler_frac_FIR_12",
        silk_resampler_frac_FIR_12.as_flattened(),
        12 * 4,
        fnv_i16
    );

    tbl!(
        "silk_CB_lags_stage2_10_ms",
        silk_CB_lags_stage2_10_ms.as_flattened(),
        2 * 3,
        fnv_i8
    );
    tbl!(
        "silk_CB_lags_stage3_10_ms",
        silk_CB_lags_stage3_10_ms.as_flattened(),
        2 * 12,
        fnv_i8
    );
    tbl!(
        "silk_Lag_range_stage3_10_ms",
        silk_Lag_range_stage3_10_ms.as_flattened(),
        2 * 2,
        fnv_i8
    );
    tbl!(
        "silk_CB_lags_stage2",
        silk_CB_lags_stage2.as_flattened(),
        4 * 11,
        fnv_i8
    );
    tbl!(
        "silk_CB_lags_stage3",
        silk_CB_lags_stage3.as_flattened(),
        4 * 34,
        fnv_i8
    );
    tbl!(
        "silk_Lag_range_stage3",
        silk_Lag_range_stage3.as_flattened().as_flattened(),
        3 * 4 * 2,
        fnv_i8
    );
    tbl!(
        "silk_nb_cbk_searchs_stage3",
        &silk_nb_cbk_searchs_stage3,
        3,
        fnv_i8
    );
}

/* ------------------------------------------------------------------ */
/* 3. structure sizes (pointer-free structures only)                   */
/* ------------------------------------------------------------------ */

fn probe_structs() {
    println!("=== structs ===");
    println!("sizeof {} {}", "silk_nsq_state", size_of::<silk_nsq_state>());
    println!("sizeof {} {}", "silk_VAD_state", size_of::<silk_VAD_state>());
    println!("sizeof {} {}", "silk_LP_state", size_of::<silk_LP_state>());
    println!("sizeof {} {}", "stereo_enc_state", size_of::<stereo_enc_state>());
    println!("sizeof {} {}", "stereo_dec_state", size_of::<stereo_dec_state>());
    println!("sizeof {} {}", "SideInfoIndices", size_of::<SideInfoIndices>());
    println!("sizeof {} {}", "silk_PLC_struct", size_of::<silk_PLC_struct>());
    println!("sizeof {} {}", "silk_CNG_struct", size_of::<silk_CNG_struct>());
    println!(
        "sizeof {} {}",
        "silk_decoder_control",
        size_of::<silk_decoder_control>()
    );
    println!(
        "sizeof {} {}",
        "silk_shape_state_FIX",
        size_of::<silk_shape_state_FIX>()
    );
    println!(
        "sizeof {} {}",
        "silk_encoder_control_FIX",
        size_of::<silk_encoder_control_FIX>()
    );
    println!(
        "sizeof {} {}",
        "silk_EncControlStruct",
        size_of::<silk_EncControlStruct>()
    );

    println!(
        "offsetof {} {} {}",
        "silk_nsq_state",
        "sAR2_Q14",
        offset_of!(silk_nsq_state, sAR2_Q14)
    );
    println!(
        "offsetof {} {} {}",
        "silk_nsq_state",
        "rand_seed",
        offset_of!(silk_nsq_state, rand_seed)
    );
    println!(
        "offsetof {} {} {}",
        "silk_VAD_state",
        "counter",
        offset_of!(silk_VAD_state, counter)
    );
    println!(
        "offsetof {} {} {}",
        "silk_LP_state",
        "saved_fs_kHz",
        offset_of!(silk_LP_state, saved_fs_kHz)
    );
    println!(
        "offsetof {} {} {}",
        "stereo_enc_state",
        "predIx",
        offset_of!(stereo_enc_state, predIx)
    );
    println!(
        "offsetof {} {} {}",
        "SideInfoIndices",
        "Seed",
        offset_of!(SideInfoIndices, Seed)
    );
    println!(
        "offsetof {} {} {}",
        "silk_PLC_struct",
        "enable_deep_plc",
        offset_of!(silk_PLC_struct, enable_deep_plc)
    );
    println!(
        "offsetof {} {} {}",
        "silk_CNG_struct",
        "fs_kHz",
        offset_of!(silk_CNG_struct, fs_kHz)
    );
    println!(
        "offsetof {} {} {}",
        "silk_decoder_control",
        "PredCoef_Q12",
        offset_of!(silk_decoder_control, PredCoef_Q12)
    );
    println!(
        "offsetof {} {} {}",
        "silk_encoder_control_FIX",
        "lastGainIndexPrev",
        offset_of!(silk_encoder_control_FIX, lastGainIndexPrev)
    );
    println!(
        "offsetof {} {} {}",
        "silk_EncControlStruct",
        "offset",
        offset_of!(silk_EncControlStruct, offset)
    );
}

fn main() {
    let tv = init_tv();
    probe_macros(&tv);
    probe_tables();
    probe_structs();
}
