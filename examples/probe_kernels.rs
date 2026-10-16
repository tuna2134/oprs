//! Twin of `compat/c/probe_kernels.c` — prints the identical output format so
//! that `bash compat/scripts/run-probe.sh kernels` can diff the two.
//!
//! See `src/celt/kernels.rs` for the kernel specification.

use oprs::celt::kernels::*;

const MAXBUF: usize = 2048 + 64;

const SPECIALS: [u32; 10] = [
    0x0000_0000, 0x8000_0000, 0x3f80_0000, 0xbf80_0000, 0x0000_0001, 0x8000_0001, 0x007f_ffff,
    0x3dcc_cccc, 0xbdcc_cccc, 0x7fc0_0000,
];

/// Deterministic buffer fill; mirrors the C `fill()` exactly.
fn fill(buf: &mut [f32], scale: f32, seed: u32, off: i32, mode: i32) {
    let mut st = seed.wrapping_mul(2654435761).wrapping_add(1);
    for i in 0..buf.len() {
        if mode == 2 {
            buf[i] = 0.0;
        } else if mode == 3 {
            // spiky: a few +/-1 values among tiny ones (see the C probe)
            let v = if i % 37 == 0 { 1.0f32 } else { 1e-3f32 };
            buf[i] = if i % 2 == 1 { v } else { -v };
        } else if mode == 1 || ((i as i32 + off) % 7) == 3 {
            let b = SPECIALS[(((i / 7) + seed as usize) % 10) as usize];
            buf[i] = f32::from_bits(b);
        } else {
            st = st.wrapping_mul(1103515245).wrapping_add(12345);
            let v = ((st >> 8) as i32) - 8388608;
            buf[i] = (v as f32) / 8388608.0 * scale;
        }
    }
}

fn fnv_u32(h: &mut u64, w: u32) {
    for i in 0..4 {
        *h ^= ((w >> (8 * i)) & 0xff) as u64;
        *h = h.wrapping_mul(0x100_0000_01b3);
    }
}

fn fnv_f32s(p: &[f32]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &f in p {
        fnv_u32(&mut h, f.to_bits());
    }
    h
}

fn fnv_i32s(p: &[i32]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &v in p {
        fnv_u32(&mut h, v as u32);
    }
    h
}

fn report(fnname: &str, par: &str, input: u64, out: u64) {
    println!(
        "{:<7} {:<26} in={:016x} out={:016x}",
        fnname, par, input, out
    );
}

const SMALL_N: [i32; 40] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
];
const BIG_N: [i32; 18] = [
    48, 63, 64, 65, 96, 127, 128, 129, 160, 255, 256, 257, 300, 512, 640, 1000, 1024, 2000,
];
const COMB_T: [i32; 6] = [15, 16, 17, 32, 33, 60];
const COMB_BIG: [i32; 10] = [63, 64, 65, 127, 128, 129, 255, 256, 480, 960];
const FIR_ORD: [i32; 7] = [1, 2, 3, 4, 8, 16, 24];
const FIR_BIG: [i32; 9] = [63, 64, 65, 127, 128, 129, 257, 480, 960];
const PXC_LEN: [i32; 22] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 16, 17, 24, 32, 33, 48, 63, 64, 65, 120, 240,
];
const PXC_MP: [i32; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 15, 16, 17, 31, 32, 33];

fn section_special_ops() {
    const ES: [u32; 10] = [0, 1, 2, 3, 126, 127, 128, 129, 254, 255];
    const MS: [u32; 6] = [0, 1, 0x7ff, 0x800, 0x400000, 0x7fffff];
    for k in 0..2u32 {
        for &e in ES.iter() {
            for &m in MS.iter() {
                let mut b = (e << 23) | m;
                if k == 1 {
                    b |= 0x8000_0000;
                }
                let f = f32::from_bits(b);
                println!("rcp    {:08x} -> {:08x}", b, simd_rcp_ps(f).to_bits());
                println!("rsqrt  {:08x} -> {:08x}", b, simd_rsqrt_ps(f).to_bits());
            }
        }
    }
    for i in 0..40u32 {
        let b = 12345u32.wrapping_add(2654435761u32.wrapping_mul(i));
        let f = f32::from_bits(b);
        println!("rcp    {:08x} -> {:08x}", b, simd_rcp_ps(f).to_bits());
        println!("rsqrt  {:08x} -> {:08x}", b, simd_rsqrt_ps(f).to_bits());
    }
}

fn section_xcorr() {
    let mut xbuf = vec![0f32; MAXBUF];
    let mut ybuf = vec![0f32; MAXBUF];
    for idx in 0..40 + 18 {
        let n = if idx < 40 { SMALL_N[idx] } else { BIG_N[idx - 40] };
        let mut sum = [0f32; 4];
        fill(&mut xbuf[..n as usize + 8], 1.0, 11 + n as u32, 0, 0);
        fill(&mut ybuf[..n as usize + 8], 0.75, 23 + n as u32, 1, 0);
        fill(&mut sum, 0.5, 31 + n as u32, 2, 0);
        let mut input = fnv_f32s(&xbuf[..n as usize]);
        input = (input << 1) ^ fnv_f32s(&ybuf[..n as usize]);
        input = (input << 1) ^ fnv_f32s(&sum);
        {
            let mut s = sum;
            xcorr_kernel(&xbuf, &ybuf, &mut s, n);
            report(
                "xcorr",
                &format!("len={} arch=4", n),
                input,
                fnv_f32s(&s),
            );
        }
        if n >= 3 {
            let mut s = sum;
            xcorr_kernel_c(&xbuf, &ybuf, &mut s, n);
            report(
                "xcorr",
                &format!("len={} arch=0", n),
                input,
                fnv_f32s(&s),
            );
        }
    }
}

fn section_inner() {
    let mut xbuf = vec![0f32; MAXBUF];
    let mut ybuf = vec![0f32; MAXBUF];
    let mut zbuf = vec![0f32; MAXBUF];
    for idx in 0..40 + 18 {
        let n = if idx < 40 { SMALL_N[idx] } else { BIG_N[idx - 40] };
        fill(&mut xbuf[..n as usize + 4], 1.0, 41 + n as u32, 0, 0);
        fill(&mut ybuf[..n as usize + 4], 0.5, 43 + n as u32, 1, 0);
        fill(&mut zbuf[..n as usize + 4], 1.5, 47 + n as u32, 2, 0);
        let mut input = fnv_f32s(&xbuf[..n as usize]);
        input = (input << 1) ^ fnv_f32s(&ybuf[..n as usize]);

        let r = celt_inner_prod(&xbuf, &ybuf, n);
        report(
            "iprod",
            &format!("N={} arch=4", n),
            input,
            fnv_f32s(&[r]),
        );
        let r = celt_inner_prod_c(&xbuf, &ybuf, n);
        report(
            "iprod",
            &format!("N={} arch=0", n),
            input,
            fnv_f32s(&[r]),
        );

        let mut input = fnv_f32s(&xbuf[..n as usize]);
        input = (input << 1) ^ fnv_f32s(&ybuf[..n as usize]);
        input = (input << 1) ^ fnv_f32s(&zbuf[..n as usize]);
        let mut a = 0f32;
        let mut b = 0f32;
        dual_inner_prod(&xbuf, &ybuf, &zbuf, n, &mut a, &mut b);
        let mut out = 0xcbf2_9ce4_8422_2325u64;
        fnv_u32(&mut out, a.to_bits());
        fnv_u32(&mut out, b.to_bits());
        report("dprod", &format!("N={} arch=4", n), input, out);
        dual_inner_prod_c(&xbuf, &ybuf, &zbuf, n, &mut a, &mut b);
        let mut out = 0xcbf2_9ce4_8422_2325u64;
        fnv_u32(&mut out, a.to_bits());
        fnv_u32(&mut out, b.to_bits());
        report("dprod", &format!("N={} arch=0", n), input, out);
    }
}

fn section_comb() {
    let mut xbuf = vec![0f32; MAXBUF];
    let mut ybuf = vec![0f32; MAXBUF];
    for &t in COMB_T.iter() {
        for idx in 0..40 + 10 {
            let n = if idx < 40 {
                SMALL_N[idx]
            } else {
                COMB_BIG[idx - 40]
            };
            let len = (n + t + 4 + 4) as usize;
            let seed = 61 + (n + 1000 * t) as u32;
            fill(&mut xbuf[..len], 1.0, seed, 0, 0);
            fill(&mut ybuf[..n as usize + 4], 0.25, 67 + (n + 1000 * t) as u32, 1, 0);
            let g10 = (((n * 7 + t) % 101) - 50) as f32 / 64.0;
            let g11 = (((n * 11 + t) % 101) - 50) as f32 / 128.0;
            let g12 = (((n * 13 + t) % 101) - 50) as f32 / 256.0;
            let mut input = fnv_f32s(&xbuf[..(n + t + 2) as usize]);
            input = (input << 1) ^ fnv_f32s(&ybuf[..n as usize]);

            comb_filter_const(&mut ybuf, &xbuf, t, n, g10, g11, g12);
            report(
                "comb",
                &format!("T={} N={} arch=4", t, n),
                input,
                fnv_f32s(&ybuf[..n as usize]),
            );
            fill(&mut ybuf[..n as usize + 4], 0.25, 67 + (n + 1000 * t) as u32, 1, 0);
            comb_filter_const_c(&mut ybuf, &xbuf, t, n, g10, g11, g12);
            report(
                "comb",
                &format!("T={} N={} arch=0", t, n),
                input,
                fnv_f32s(&ybuf[..n as usize]),
            );
        }
    }
}

fn section_fir() {
    let mut xbuf = vec![0f32; MAXBUF];
    let mut zbuf = vec![0f32; MAXBUF];
    let mut ybuf = vec![0f32; MAXBUF];
    for &ord in FIR_ORD.iter() {
        for idx in 0..40 + 9 {
            let n = if idx < 40 {
                SMALL_N[idx]
            } else {
                FIR_BIG[idx - 40]
            };
            let seed = (n + 1000 * ord) as u32;
            fill(&mut xbuf[..(n + ord + 8) as usize], 1.0, 71 + seed, 0, 0);
            fill(&mut zbuf[..(ord + 4) as usize], 0.5, 73 + seed, 1, 0);
            let mut input = fnv_f32s(&xbuf[..(n + ord) as usize]);
            input = (input << 1) ^ fnv_f32s(&zbuf[..ord as usize]);

            celt_fir(&xbuf, &zbuf, &mut ybuf, n, ord);
            report(
                "fir",
                &format!("ord={} N={} arch=4", ord, n),
                input,
                fnv_f32s(&ybuf[..n as usize]),
            );
            celt_fir(&xbuf, &zbuf, &mut ybuf, n, ord);
            report(
                "fir",
                &format!("ord={} N={} arch=0", ord, n),
                input,
                fnv_f32s(&ybuf[..n as usize]),
            );
        }
    }
}

fn section_pxc() {
    let mut xbuf = vec![0f32; MAXBUF];
    let mut ybuf = vec![0f32; MAXBUF];
    let mut obuf = vec![0f32; MAXBUF];
    for &len in PXC_LEN.iter() {
        for &mp in PXC_MP.iter() {
            let seed = (len + 1000 * mp) as u32;
            fill(&mut xbuf[..len as usize + 8], 1.0, 83 + seed, 0, 0);
            fill(
                &mut ybuf[..(len + mp + 8) as usize],
                0.75,
                89 + seed,
                1,
                0,
            );
            let mut input = fnv_f32s(&xbuf[..len as usize]);
            input = (input << 1) ^ fnv_f32s(&ybuf[..(len + mp - 1) as usize]);
            celt_pitch_xcorr(&xbuf, &ybuf, &mut obuf, len, mp);
            report(
                "pxcorr",
                &format!("len={} mp={} arch=4", len, mp),
                input,
                fnv_f32s(&obuf[..mp as usize]),
            );
            celt_pitch_xcorr_c(&xbuf, &ybuf, &mut obuf, len, mp);
            report(
                "pxcorr",
                &format!("len={} mp={} arch=0", len, mp),
                input,
                fnv_f32s(&obuf[..mp as usize]),
            );
        }
    }
}

fn pvq_case(n: i32, k: i32, seed: u32, mode: i32, arch: i32) {
    let mut xbuf = vec![0f32; MAXBUF];
    let mut xx = vec![0f32; (n + 8) as usize];
    let mut ibuf = vec![0i32; MAXBUF];
    fill(&mut xbuf[..n as usize], 1.0, seed, 0, mode);
    for v in ibuf.iter_mut() {
        *v = 0x5555_5555;
    }
    xx[..n as usize].copy_from_slice(&xbuf[..n as usize]);
    let input = fnv_f32s(&xx[..n as usize]);
    let (par, out) = if arch == 4 {
        let yy = op_pvq_search_sse2(&xx, &mut ibuf, k, n, 4);
        let mut out = 0xcbf2_9ce4_8422_2325u64;
        fnv_u32(&mut out, yy.to_bits());
        out = (out << 1) ^ fnv_i32s(&ibuf[..(n + 3) as usize]);
        out = (out << 1) ^ fnv_f32s(&xx[..n as usize]);
        (format!("N={} K={} m={} arch=4", n, k, mode), out)
    } else {
        let yy = op_pvq_search_c(&mut xx, &mut ibuf, k, n);
        let mut out = 0xcbf2_9ce4_8422_2325u64;
        fnv_u32(&mut out, yy.to_bits());
        out = (out << 1) ^ fnv_i32s(&ibuf[..n as usize]);
        out = (out << 1) ^ fnv_f32s(&xx[..n as usize]);
        (format!("N={} K={} m={} arch=0", n, k, mode), out)
    };
    report("pvq", &par, input, out);
}

fn section_pvq() {
    const EXTRA_N: [i32; 16] = [2, 2, 3, 4, 4, 8, 8, 16, 16, 32, 32, 64, 100, 128, 176, 200];
    const EXTRA_K: [i32; 16] = [1, 2, 1, 1, 3, 1, 8, 4, 33, 10, 80, 32, 50, 64, 88, 101];
    for idx in 0..40 + 5 {
        let n = if idx < 40 {
            SMALL_N[idx]
        } else {
            BIG_N[idx - 40 + 13]
        };
        pvq_case(n, (n >> 1) + 1, 97 + n as u32, 0, 4);
        pvq_case(n, (n >> 1) + 1, 97 + n as u32, 0, 0);
    }
    for idx in 0..16 {
        pvq_case(EXTRA_N[idx], EXTRA_K[idx], 101 + idx as u32, 0, 4);
        pvq_case(EXTRA_N[idx], EXTRA_K[idx], 101 + idx as u32, 0, 0);
    }
    const SP_N: [i32; 4] = [4, 8, 16, 32];
    for si in 0..4 {
        for sm in 1..=2 {
            pvq_case(SP_N[si], SP_N[si] / 2 + 1, 103 + si as u32, sm, 4);
            pvq_case(SP_N[si], SP_N[si] / 2 + 1, 103 + si as u32, sm, 0);
        }
    }
    const DN: [i32; 7] = [8, 16, 32, 64, 100, 256, 1000];
    const EN: [i32; 4] = [64, 100, 256, 1000];
    for &n in DN.iter() {
        pvq_case(n, n / 2 + 2, 107 + n as u32, 3, 4);
        pvq_case(n, n / 2 + 2, 107 + n as u32, 3, 0);
    }
    for &n in EN.iter() {
        pvq_case(n, n / 2 + n / 4, 109 + n as u32, 3, 4);
        pvq_case(n, n / 2 + n / 4, 109 + n as u32, 3, 0);
    }
}

fn main() {
    section_special_ops();
    section_xcorr();
    section_inner();
    section_comb();
    section_fir();
    section_pxc();
    section_pvq();
}
