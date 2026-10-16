//! Rust side of the `celt/cwrs.c` bit-exactness probe.
//!
//! Mirrors `compat/c/probe_cwrs.c` operation for operation, including the LCG
//! (`state = state*1103515245 + 12345`) and the order in which it is drawn:
//! `log2_frac()`, `get_required_bits()` for N = 1..=176, and range-coder
//! round trips for N = 2..=176 / K = 0..=min(N, k_bound(N)) (the `_n >= 2`
//! and `_k > 0` domains the reference cwrs.c asserts).  Run
//! `bash compat/scripts/run-probe.sh cwrs` to build both sides and diff their
//! output.

use oprs::celt::cwrs::{decode_pulses, encode_pulses, get_required_bits, log2_frac};
use oprs::celt::entdec::*;
use oprs::celt::entenc::*;

const STORAGE: u32 = 256;
const MAXN: usize = 176;
const MAXK: usize = 208; /* k_bound() is at most 207; holds bits[..=nb] */

struct Rng(u32);
impl Rng {
    fn rnd(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345);
        self.0
    }
}

fn fnv_upd(h: &mut u64, b: u8) {
    *h ^= b as u64;
    *h = h.wrapping_mul(1099511628211);
}
fn fnv_u32(h: &mut u64, v: u32) {
    for b in v.to_le_bytes() {
        fnv_upd(h, b);
    }
}
fn fnv_i32(h: &mut u64, v: i32) {
    fnv_u32(h, v as u32);
}
fn fnv_f32(h: &mut u64, f: f32) {
    fnv_u32(h, f.to_bits());
}

/* K range of each row of CELT_PVQ_U_DATA, from the comments in celt/cwrs.c. */
static ROW_KMAX: [i32; 15] = [208, 208, 208, 208, 208, 208, 109, 60, 40, 29, 24, 20, 18, 16, 14];
/* celt/rate.c fits_in32() tables. */
static FITS_MAXN: [i32; 15] = [
    32767, 32767, 32767, 1476, 283, 109, 60, 40, 29, 24, 20, 18, 16, 14, 13,
];
static FITS_MAXK: [i32; 15] = [
    32767, 32767, 32767, 32767, 1172, 238, 95, 53, 36, 27, 22, 18, 16, 15, 13,
];

/* Is U(n,k) addressable in the table? */
fn vtab_ok(n: i32, k: i32) -> bool {
    let (m, mmax) = if n < k { (n, k) } else { (k, n) };
    if m > 14 {
        return false;
    }
    mmax <= ROW_KMAX[m as usize]
}
/* Is V(n,k) = U(n,k)+U(n,k+1) addressable? */
fn v_ok(n: i32, k: i32) -> bool {
    vtab_ok(n, k) && vtab_ok(n, k + 1)
}
/* celt/rate.c: does V(N,K) fit in a 32-bit unsigned integer? */
fn fits_in32(n: i32, k: i32) -> bool {
    if n >= 14 {
        if k >= 14 {
            return false;
        }
        return n <= FITS_MAXN[k as usize];
    }
    k <= FITS_MAXK[n as usize]
}
fn k_bound(n: i32) -> i32 {
    let mut k = 1;
    while v_ok(n, k) && fits_in32(n, k) {
        k += 1;
    }
    k - 1
}

/* Deterministic pulse vector with sum(|y[i]|) == k. */
fn make_vec(rng: &mut Rng, y: &mut [i32], n: i32, k: i32) {
    let mut rem = k;
    for i in 0..n as usize {
        y[i] = 0;
    }
    for i in 0..(n - 1) as usize {
        let v = (rng.rnd() % (rem + 1) as u32) as i32;
        let s = rng.rnd() & 1;
        y[i] = if s != 0 { -v } else { v };
        rem -= v;
    }
    y[(n - 1) as usize] = if rng.rnd() & 1 != 0 { -rem } else { rem };
}

/* Encode and decode one pulse vector.  Returns the (tell, bytes, yy, ok)
   values and a per-case FNV-1a hash of the decoded vector. */
#[allow(clippy::too_many_arguments)]
fn run_case(
    n: i32,
    k: i32,
    y: &[i32],
    global_hash: &mut u64,
    tell: &mut i32,
    bytes: &mut i32,
    yy_out: &mut f32,
    ok_out: &mut i32,
    chash: &mut u64,
) {
    let mut h: u64 = 1469598103934665603;
    let yy: f32;
    if k > 0 {
        let mut y2 = [0i32; MAXN];
        let mut enc = ec_enc_init(STORAGE);
        encode_pulses(&y[..n as usize], n, k, &mut enc);
        ec_enc_done(&mut enc);
        *tell = enc.ec_tell();
        *bytes = enc.ec_range_bytes() as i32;
        /* See probe_cwrs.c: decode with the encoder's full storage so that the
           ec_enc_bits() payload parked at the end lines up. */
        let mut dec = ec_dec_init(&enc.buf[..STORAGE as usize]);
        yy = decode_pulses(&mut y2[..n as usize], n, k, &mut dec);
        let mut ok = 1;
        let mut sq: f32 = 0.;
        for i in 0..n as usize {
            if y2[i] != y[i] {
                ok = 0;
            }
            sq += (y[i] as f32) * (y[i] as f32);
            fnv_i32(&mut h, y2[i]);
        }
        if sq != yy {
            ok = 0;
        }
        fnv_f32(&mut h, yy);
        *ok_out = ok;
    } else {
        /* V(n,0) == 1: the all-zero vector, nothing is coded. */
        *tell = 0;
        *bytes = 0;
        yy = 0.;
        *ok_out = 1;
        for _ in 0..n {
            fnv_i32(&mut h, 0);
        }
        fnv_f32(&mut h, 0.);
    }
    *yy_out = yy;
    *chash = h;
    fnv_u32(global_hash, (h >> 32) as u32);
    fnv_u32(global_hash, h as u32);
}

fn main() {
    let mut rng = Rng(0x9E3779B9);
    let mut global_hash: u64 = 1469598103934665603;
    let mut y = [0i32; MAXN];
    let mut bits = [0i16; MAXK + 2];

    println!("== cwrs probe ==");

    /* ---------------- log2_frac ---------------- */
    println!("-- log2_frac --");
    {
        let edge: [u32; 13] = [
            1, 2, 3, 0xFFFF, 0x10000, 0x10001, 0x0000FFFF, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFE,
            0xFFFFFFFF, 0x12345678, 0x00010000,
        ];
        let efrac: [i32; 5] = [0, 1, 3, 4, 6];
        let mut h: u64 = 1469598103934665603;
        for &v in edge.iter() {
            for &f in efrac.iter() {
                let r = log2_frac(v, f);
                fnv_u32(&mut h, v);
                fnv_i32(&mut h, f);
                fnv_i32(&mut h, r);
                fnv_i32(&mut global_hash, r);
                println!("log2_frac({:10},{})={}", v, f, r);
            }
        }
        for _ in 0..256 {
            /* val == 0 is undefined for EC_ILOG(); force the low bit. */
            let v = rng.rnd() | 1;
            let r = log2_frac(v, 4);
            fnv_u32(&mut h, v);
            fnv_i32(&mut h, r);
            fnv_i32(&mut global_hash, r);
            println!("log2_frac({:10},4)={}", v, r);
        }
        println!("log2_frac hash={:016x}", h);
    }

    /* ---------------- get_required_bits ---------------- */
    println!("-- get_required_bits --");
    {
        let fracs: [i32; 3] = [0, 3, 4];
        for n in 1..=MAXN as i32 {
            let nb = k_bound(n);
            if nb < 1 {
                println!("grb N={:3} maxk={:3} (none)", n, nb);
                continue;
            }
            for &frac in fracs.iter() {
                let mut h: u64 = 1469598103934665603;
                for b in bits.iter_mut() {
                    *b = 0;
                }
                get_required_bits(&mut bits, n, nb, frac);
                for j in 0..=nb as usize {
                    fnv_u32(&mut h, bits[j] as u16 as u32);
                }
                fnv_u32(&mut global_hash, (h >> 32) as u32);
                fnv_u32(&mut global_hash, h as u32);
                println!(
                    "grb N={:3} maxk={:3} frac={} hash={:016x} head={} {} {}",
                    n, nb, frac, h, bits[0], bits[1], bits[2]
                );
            }
        }
    }

    /* ---------------- encode/decode: K = 0..=min(N, k_bound(N)) ------- */
    println!("-- encode/decode K<=min(N,k_bound) --");
    /* encode_pulses() asserts N>=2 (N==1 bands are handled outside cwrs) and
       we only call it for K>0. */
    for n in 2..=MAXN as i32 {
        let nb = k_bound(n);
        let km = if n < nb { n } else { nb };
        for k in 0..=km {
            let (mut tell, mut bytes, mut yy, mut ok, mut ch) = (0, 0, 0f32, 0, 0u64);
            make_vec(&mut rng, &mut y, n, k);
            run_case(
                n,
                k,
                &y,
                &mut global_hash,
                &mut tell,
                &mut bytes,
                &mut yy,
                &mut ok,
                &mut ch,
            );
            println!(
                "N={:3} K={:3} tell={:3} bytes={:3} code={} yy={:08x} ok={} hash={:016x}",
                n,
                k,
                tell,
                bytes,
                (k > 0) as i32,
                yy.to_bits(),
                ok,
                ch
            );
        }
    }

    /* ---------------- encode/decode: large K, N = 1..=6 --------------- */
    println!("-- encode/decode large K --");
    for n in 2..=6i32 {
        let nb = k_bound(n);
        for k in (n + 1)..=nb {
            let (mut tell, mut bytes, mut yy, mut ok, mut ch) = (0, 0, 0f32, 0, 0u64);
            make_vec(&mut rng, &mut y, n, k);
            run_case(
                n,
                k,
                &y,
                &mut global_hash,
                &mut tell,
                &mut bytes,
                &mut yy,
                &mut ok,
                &mut ch,
            );
            println!(
                "N={:3} K={:3} tell={:3} bytes={:3} code={} yy={:08x} ok={} hash={:016x}",
                n,
                k,
                tell,
                bytes,
                (k > 0) as i32,
                yy.to_bits(),
                ok,
                ch
            );
        }
    }

    println!("-- summary --");
    println!(
        "rng_state={} global_hash={:016x}",
        rng.0, global_hash
    );
    println!("done");
}
