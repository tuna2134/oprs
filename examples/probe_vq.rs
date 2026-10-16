//! Twin of `compat/c/probe_vq.c` — bit-exactness probe for `celt/vq.c`
//! (libopus 1.6.1, float build, arch-4 SIMD).
//!
//! Covers `kernels::op_pvq_search` (the `op_pvq_search_sse2` kernel the
//! reference executes at arch 4), `vq::alg_quant` / `vq::alg_unquant` round
//! trips through a real `ec_enc`/`ec_dec`, and `renormalise_vector`,
//! `stereo_itheta`, `exp_rotation` over deterministic and boundary inputs.
//!
//! Prints the identical output format as the C probe so that
//! `bash compat/scripts/run-probe.sh vq` can diff the two; all floats are
//! printed as raw IEEE-754 bits.

use oprs::celt::entcode::{ec_ctx, ec_tell_frac};
use oprs::celt::entdec::ec_dec_init;
use oprs::celt::entenc::{ec_enc_done, ec_enc_init};
use oprs::celt::kernels::op_pvq_search;
use oprs::celt::vq::{alg_quant, alg_unquant, exp_rotation, renormalise_vector, stereo_itheta};

const MAXN: usize = 176;
const PKT: usize = 256;

// ---- deterministic input generation (mirrors the C) ----------------------

/// `celt/mathops.c:388 celt_lcg_rand()`: `state = state*1103515245 + 12345`.
struct Rng {
    state: u32,
}
impl Rng {
    fn new(state: u32) -> Self {
        Rng { state }
    }
    fn rnd(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(1103515245).wrapping_add(12345);
        self.state
    }
    /// A raw 32-bit LCG value mapped into `[-scale, scale)`.
    fn rndf(&mut self, scale: f32) -> f32 {
        let v = ((self.rnd() >> 8) as i32) - 8388608;
        (v as f32) / 8388608.0f32 * scale
    }
}

/// Raw bit patterns used as boundary inputs: 0, +-denormal-min, +-0.5, +-1,
/// +-1e10, +-~3e-23 (all finite).
static BFLOATS: [u32; 12] = [
    0x0000_0000, 0x8000_0000, /* +-0 */
    0x0000_0001, 0x8000_0001, /* +-denormal min */
    0x3f00_0000, 0xbf00_0000, /* +-0.5 */
    0x3f80_0000, 0xbf80_0000, /* +-1 */
    0x5015_02f9, 0xd015_02f9, /* +-1e10 */
    0x1a56_cb90, 0x9a56_cb90, /* +-~3e-23 */
];

/// The special patterns used only by the `op_pvq_search` section.
static SPEC: [u32; 10] = [
    0x0000_0000, 0x8000_0000, 0x3f80_0000, 0xbf80_0000, 0x0000_0001, 0x8000_0001, 0x007f_ffff,
    0x3dcc_cccc, 0xbdcc_cccc, 0x7fc0_0000,
];

fn fill_rand(x: &mut [f32], scale: f32, rng: &mut Rng) {
    for v in x.iter_mut() {
        *v = rng.rndf(scale);
    }
}
fn fill_bits(x: &mut [f32], off: i32) {
    for (i, v) in x.iter_mut().enumerate() {
        *v = f32::from_bits(BFLOATS[(i as i32 + off).rem_euclid(12) as usize]);
    }
}
fn fill_const(x: &mut [f32], v: f32) {
    for e in x.iter_mut() {
        *e = v;
    }
}
fn fill_spec(x: &mut [f32], seed: i32, rng: &mut Rng) {
    for (i, v) in x.iter_mut().enumerate() {
        let i = i as i32;
        if (i + seed) % 3 == 0 {
            *v = f32::from_bits(SPEC[((i + seed) % 10) as usize]);
        } else {
            *v = rng.rndf(1.0);
        }
    }
}

// ---- hashing (FNV-1a 64) --------------------------------------------------

const FNV_INIT: u64 = 1469598103934665603;

fn fnv_u32(h: &mut u64, v: u32) {
    for i in 0..4 {
        *h ^= ((v >> (8 * i)) & 0xff) as u64;
        *h = h.wrapping_mul(1099511628211);
    }
}
fn fnv_bytes(h: &mut u64, p: &[u8]) {
    for &b in p {
        *h ^= b as u64;
        *h = h.wrapping_mul(1099511628211);
    }
}
fn hash_f32(p: &[f32]) -> u64 {
    let mut h = FNV_INIT;
    for &f in p {
        fnv_u32(&mut h, f.to_bits());
    }
    h
}
fn hash_i32(p: &[i32]) -> u64 {
    let mut h = FNV_INIT;
    for &v in p {
        fnv_u32(&mut h, v as u32);
    }
    h
}

// ---- celt/cwrs.c domain, identical to compat/c/probe_cwrs.c ---------------

static ROW_KMAX: [i32; 15] = [208, 208, 208, 208, 208, 208, 109, 60, 40, 29, 24, 20, 18, 16, 14];
static FITS_MAXN: [i32; 15] = [
    32767, 32767, 32767, 1476, 283, 109, 60, 40, 29, 24, 20, 18, 16, 14, 13,
];
static FITS_MAXK: [i32; 15] = [
    32767, 32767, 32767, 32767, 1172, 238, 95, 53, 36, 27, 22, 18, 16, 15, 13,
];
fn vtab_ok(n: i32, k: i32) -> bool {
    let (m, mmax) = if n < k { (n, k) } else { (k, n) };
    if m > 14 {
        return false;
    }
    mmax <= ROW_KMAX[m as usize]
}
fn v_ok(n: i32, k: i32) -> bool {
    vtab_ok(n, k) && vtab_ok(n, k + 1)
}
fn fits_in32(n: i32, k: i32) -> bool {
    if n >= 14 {
        if k >= 14 {
            return false;
        }
        return n <= FITS_MAXN[k as usize];
    }
    k <= FITS_MAXK[n as usize]
}

static NCAND: [i32; 24] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 20, 24, 28, 31, 32, 33, 48,
];
static KCAND: [i32; 9] = [0, 1, 2, 3, 5, 8, 16, 32, 64];
static SPREADS: [i32; 4] = [0, 1, 2, 3];
static BS: [i32; 4] = [1, 2, 4, 8];
static GAINS: [f32; 4] = [1.0, 0.5, 2.0, 37.5];

// ---------------- op_pvq_search (arch = 4) ----------------
fn section_opq() {
    let mut x = vec![0f32; MAXN + 8];
    let mut iy = vec![0i32; MAXN + 8];
    let mut hin = FNV_INIT;
    let mut hout = FNV_INIT;

    println!("== op_pvq_search_sse2 (arch=4) ==");
    for &n in NCAND.iter() {
        for &k in KCAND.iter() {
            if !(v_ok(n, k) && fits_in32(n, k)) {
                continue;
            }
            let mut rng = Rng::new(0x9E37_79B9u32 ^ ((n * 1000 + k) as u32));
            fill_rand(&mut x[..n as usize], 1.0, &mut rng);
            for e in iy.iter_mut() {
                *e = 0;
            }
            let input = hash_f32(&x[..n as usize]);
            let gain = op_pvq_search(&mut x[..n as usize], &mut iy, k, n, 4);
            let out = hash_i32(&iy[..(n + 3) as usize]);
            print!(
                "opq N={:3} K={:3} mode={} in={:016x} xh={:016x} gain={:08x}",
                n,
                k,
                0,
                input,
                hash_f32(&x[..n as usize]),
                gain.to_bits()
            );
            print!(" iy=");
            for j in 0..(n + 3) as usize {
                print!(" {}", iy[j]);
            }
            println!(" out={:016x}", out);
            fnv_u32(&mut hin, input as u32);
            fnv_u32(&mut hout, out as u32);
        }
    }
    // Special / boundary inputs on a few shapes.
    let sn: [i32; 10] = [1, 2, 3, 4, 5, 7, 8, 9, 16, 33];
    let sk: [i32; 10] = [1, 2, 3, 5, 8, 16, 1, 4, 16, 32];
    for i in 0..10 {
        let n = sn[i];
        let k = sk[i];
        if !(v_ok(n, k) && fits_in32(n, k)) {
            continue;
        }
        for mode in 1..=5 {
            let mut rng = Rng::new(0x0123_4567u32.wrapping_add((n * 31 + k * 7 + mode) as u32));
            if mode == 1 {
                fill_rand(&mut x[..n as usize], 0.03125, &mut rng);
            } else if mode == 2 {
                fill_rand(&mut x[..n as usize], 100.0, &mut rng);
            } else if mode == 3 {
                fill_const(&mut x[..n as usize], 0.0);
            } else if mode == 4 {
                fill_bits(&mut x[..n as usize], (n + k) % 12);
            } else {
                fill_spec(&mut x[..n as usize], n + k, &mut rng);
            }
            for e in iy.iter_mut() {
                *e = 0;
            }
            let input = hash_f32(&x[..n as usize]);
            let gain = op_pvq_search(&mut x[..n as usize], &mut iy, k, n, 4);
            let out = hash_i32(&iy[..(n + 3) as usize]);
            print!(
                "opq N={:3} K={:3} mode={} in={:016x} xh={:016x} gain={:08x}",
                n,
                k,
                mode,
                input,
                hash_f32(&x[..n as usize]),
                gain.to_bits()
            );
            print!(" iy=");
            for j in 0..(n + 3) as usize {
                print!(" {}", iy[j]);
            }
            println!(" out={:016x}", out);
            fnv_u32(&mut hin, input as u32);
            fnv_u32(&mut hout, out as u32);
        }
    }
    println!("opq summary in={:016x} out={:016x}", hin, hout);
}

// ---------------- alg_quant / alg_unquant round trip ----------------
fn section_quant() {
    static QN: [i32; 23] = [
        2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 16, 17, 20, 24, 31, 32, 48, 64, 96, 128,
    ];
    static QK: [i32; 7] = [1, 2, 3, 5, 8, 16, 32];
    let mut x0 = vec![0f32; MAXN + 8];
    let mut xe = vec![0f32; MAXN + 8];
    let mut xd = vec![0f32; MAXN + 8];
    let mut henc = FNV_INIT;
    let mut hdec = FNV_INIT;
    let mut idx = 0usize;

    println!("== alg_quant / alg_unquant ==");
    for &n in QN.iter() {
        for &k in QK.iter() {
            if !(v_ok(n, k) && fits_in32(n, k)) {
                continue;
            }
            let spread = SPREADS[idx & 3];
            let b = BS[(idx >> 2) & 3];
            let gain = GAINS[(idx >> 4) & 3];
            idx += 1;

            // --- encoder, resynth = 1 ---
            let mut rng =
                Rng::new(0x00C0_FFEEu32.wrapping_add((n * 65536 + k * 256 + spread * 16 + b) as u32));
            fill_rand(&mut x0[..n as usize], 1.0, &mut rng);
            xe[..n as usize].copy_from_slice(&x0[..n as usize]);
            let mut enc: ec_ctx = ec_enc_init(PKT as u32);
            let collapse = alg_quant(
                &mut xe[..n as usize],
                n,
                k,
                spread,
                b,
                &mut enc,
                gain,
                1,
                4,
            );
            ec_enc_done(&mut enc);
            let mut henc2 = FNV_INIT;
            fnv_bytes(&mut henc2, &enc.buf);
            println!(
                "q N={:3} K={:3} spread={} B={} gain={:08x} resynth=1 in={:016x} enc={:08x} x={:016x} pkt={:016x} offs={} end_offs={} nbits={} error={}",
                n,
                k,
                spread,
                b,
                gain.to_bits(),
                hash_f32(&x0[..n as usize]),
                collapse,
                hash_f32(&xe[..n as usize]),
                henc2,
                enc.offs,
                enc.end_offs,
                enc.nbits_total,
                enc.error
            );
            // --- decoder ---
            for e in xd.iter_mut() {
                *e = 0.0;
            }
            let mut dec = ec_dec_init(&enc.buf);
            let collapse2 = alg_unquant(&mut xd[..n as usize], n, k, spread, b, &mut dec, gain);
            let tell = ec_tell_frac(&dec) as i32;
            let hdec2 = hash_f32(&xd[..n as usize]);
            println!(
                "  d collapse={:08x} x={:016x} tell_frac={} offs={} end_offs={} nbits={} error={}",
                collapse2, hdec2, tell, dec.offs, dec.end_offs, dec.nbits_total, dec.error
            );
            // The full decoded X[] as raw bits.
            print!("  xbits=");
            for j in 0..n as usize {
                print!(" {:08x}", xd[j].to_bits());
            }
            println!();

            // --- encoder, resynth = 0 (no normalise / no inverse rotation) ---
            xe[..n as usize].copy_from_slice(&x0[..n as usize]);
            let mut enc: ec_ctx = ec_enc_init(PKT as u32);
            let collapse = alg_quant(
                &mut xe[..n as usize],
                n,
                k,
                spread,
                b,
                &mut enc,
                gain,
                0,
                4,
            );
            ec_enc_done(&mut enc);
            let mut henc0 = FNV_INIT;
            fnv_bytes(&mut henc0, &enc.buf);
            println!(
                "q N={:3} K={:3} spread={} B={} gain={:08x} resynth=0 in={:016x} enc={:08x} x={:016x} pkt={:016x} offs={} end_offs={} nbits={} error={}",
                n,
                k,
                spread,
                b,
                gain.to_bits(),
                hash_f32(&x0[..n as usize]),
                collapse,
                hash_f32(&xe[..n as usize]),
                henc0,
                enc.offs,
                enc.end_offs,
                enc.nbits_total,
                enc.error
            );

            fnv_u32(&mut henc, henc2 as u32);
            fnv_u32(&mut hdec, hdec2 as u32);
        }
    }
    println!("q summary cases={} enc={:016x} dec={:016x}", idx, henc, hdec);
}

// ---------------- renormalise_vector ----------------
fn section_renorm() {
    static RN: [i32; 16] = [1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 24, 32, 33, 48, 64];
    let mut x = vec![0f32; MAXN + 8];
    for &n in RN.iter() {
        for mode in 0..4 {
            for g in 0..4 {
                let gain = GAINS[g];
                let mut rng =
                    Rng::new(0xBEEF_0000u32.wrapping_add((n * 97 + mode * 13 + g as i32) as u32));
                if mode == 0 {
                    fill_rand(&mut x[..n as usize], 1.0, &mut rng);
                } else if mode == 1 {
                    fill_rand(&mut x[..n as usize], 0.001, &mut rng);
                } else if mode == 2 {
                    fill_bits(&mut x[..n as usize], (n + g as i32) % 12);
                } else {
                    fill_const(&mut x[..n as usize], -1.0 + 0.25 * (g as f32));
                }
                let input = hash_f32(&x[..n as usize]);
                renormalise_vector(&mut x[..n as usize], n, gain, 4);
                print!(
                    "rn N={:3} mode={} gain={:08x} in={:016x} out={:016x}",
                    n,
                    mode,
                    gain.to_bits(),
                    input,
                    hash_f32(&x[..n as usize])
                );
                if n <= 24 {
                    print!(" x=");
                    for j in 0..n as usize {
                        print!(" {:08x}", x[j].to_bits());
                    }
                }
                println!();
            }
        }
    }
}

// ---------------- stereo_itheta ----------------
fn section_itheta() {
    static SN: [i32; 9] = [1, 2, 3, 4, 8, 16, 17, 32, 64];
    let mut x = vec![0f32; MAXN + 8];
    let mut y = vec![0f32; MAXN + 8];
    for &n in SN.iter() {
        for stereo in 0..2 {
            for mode in 0..4 {
                let mut rng = Rng::new(
                    0x5EED_0000u32.wrapping_add((n * 131 + stereo * 17 + mode) as u32),
                );
                if mode == 0 {
                    fill_rand(&mut x[..n as usize], 1.0, &mut rng);
                    fill_rand(&mut y[..n as usize], 1.0, &mut rng);
                } else if mode == 1 {
                    fill_rand(&mut x[..n as usize], 0.001, &mut rng);
                    fill_rand(&mut y[..n as usize], 0.001, &mut rng);
                } else if mode == 2 {
                    fill_bits(&mut x[..n as usize], n % 12);
                    fill_bits(&mut y[..n as usize], (n + 5) % 12);
                } else {
                    fill_rand(&mut x[..n as usize], 1.0, &mut rng);
                    y[..n as usize].copy_from_slice(&x[..n as usize]);
                }
                let it = stereo_itheta(&x[..n as usize], &y[..n as usize], stereo, n, 4);
                println!(
                    "it stereo={} N={:3} mode={} inx={:016x} iny={:016x} itheta={}",
                    stereo,
                    n,
                    mode,
                    hash_f32(&x[..n as usize]),
                    hash_f32(&y[..n as usize]),
                    it
                );
            }
        }
    }
}

// ---------------- exp_rotation ----------------
fn section_rotation() {
    static LEN: [i32; 13] = [2, 4, 6, 8, 12, 16, 24, 32, 48, 64, 96, 128, 176];
    static STR: [i32; 5] = [1, 2, 3, 4, 8];
    let mut x = vec![0f32; MAXN + 8];
    let mut idx = 0i32;

    for &len in LEN.iter() {
        for &stride in STR.iter() {
            let ks = [1i32, len / 4, len / 2, len];
            for &k0 in ks.iter() {
                for spread in 0..4 {
                    for dir in [-1i32, 1i32] {
                        let mut k = k0;
                        if k < 1 {
                            k = 1;
                        }
                        if len < 2 * stride {
                            continue;
                        }
                        idx += 1;
                        let mode = idx & 3;
                        let mut rng = Rng::new(
                            0xFEED_0000u32
                                .wrapping_add((len * 7 + stride * 3 + k + spread) as u32),
                        );
                        if mode == 0 {
                            fill_rand(&mut x[..len as usize], 1.0, &mut rng);
                        } else if mode == 1 {
                            fill_rand(&mut x[..len as usize], 0.01, &mut rng);
                        } else if mode == 2 {
                            fill_bits(&mut x[..len as usize], (len + k) % 12);
                        } else {
                            fill_const(&mut x[..len as usize], 0.5);
                        }
                        let input = hash_f32(&x[..len as usize]);
                        exp_rotation(&mut x[..len as usize], len, dir, stride, k, spread);
                        print!(
                            "rot len={:3} dir={:2} stride={} K={:3} spread={} in={:016x} out={:016x}",
                            len,
                            dir,
                            stride,
                            k,
                            spread,
                            input,
                            hash_f32(&x[..len as usize])
                        );
                        if len <= 24 {
                            print!(" x=");
                            for j in 0..len as usize {
                                print!(" {:08x}", x[j].to_bits());
                            }
                        }
                        println!();
                    }
                }
            }
        }
    }
    println!("rot cases={}", idx);
}

fn main() {
    println!("vq probe");
    section_opq();
    section_quant();
    section_renorm();
    section_itheta();
    section_rotation();
    println!("done");
}
