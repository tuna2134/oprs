//! Rust twin of `compat/c/probe_mathopscheck.c`.

use oprs::celt::arch::*;
use oprs::celt::mathops::*;
use oprs::types::*;

const NIN: usize = 512;

struct Rng(u32);
impl Rng {
    fn rnd(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345);
        self.0
    }
}

fn main() {
    let mut rng = Rng(0x2545F491);
    let mut pos_in = [0f32; NIN];
    for i in 0..NIN {
        let raw: u32 = if i < 8 {
            const SPECIAL: [u32; 8] = [
                0x00000001, 0x00800000, 0x3F800000, 0x40000000, 0x7F7FFFFF, 0x00000000, 0x3F000000,
                0x4B000000,
            ];
            SPECIAL[i]
        } else {
            let e = 1 + rng.rnd() % 254;
            let m = rng.rnd() & 0x7FFFFF;
            (e << 23) | m
        };
        let mut v = f32::from_bits(raw);
        if !(v > 0.0) {
            v = 1e-6;
        }
        pos_in[i] = v;
    }

    println!("[celt_rsqrt_norm]");
    for i in 0..NIN {
        println!("{} {:08x}", i, celt_rsqrt_norm(pos_in[i]).to_bits());
    }
    println!("[celt_rsqrt]");
    for i in 0..NIN {
        println!("{} {:08x}", i, celt_rsqrt(pos_in[i]).to_bits());
    }
    println!("[celt_sqrt]");
    for i in 0..NIN {
        println!("{} {:08x}", i, celt_sqrt(pos_in[i]).to_bits());
    }
    println!("[celt_rcp]");
    for i in 0..NIN {
        println!("{} {:08x}", i, celt_rcp(pos_in[i]).to_bits());
    }
    println!("[celt_log2]");
    for i in 0..NIN {
        println!("{} {:08x}", i, celt_log2(pos_in[i]).to_bits());
    }
    println!("[celt_exp2]");
    for i in 0..NIN {
        let x = ((rng.rnd() % 2001) as i32 - 1000) as f32 / 100.0;
        println!("{} {:08x}", i, celt_exp2(x).to_bits());
    }
    println!("[celt_cos_norm]");
    for i in 0..NIN {
        let x = (rng.rnd() % 40001) as f32 / 10000.0 - 2.0;
        println!("{} {:08x}", i, celt_cos_norm(x).to_bits());
    }
    println!("[celt_div]");
    for i in 0..NIN {
        let a = pos_in[i];
        let b = pos_in[(i * 7 + 3) % NIN];
        println!("{} {:08x}", i, celt_div(a, b).to_bits());
    }
    println!("[frac_div32]");
    for i in 0..NIN {
        let a = pos_in[i];
        let b = pos_in[(i * 11 + 5) % NIN];
        println!("{} {:08x}", i, frac_div32(a, b).to_bits());
    }
    println!("[isqrt32]");
    for i in 0..NIN {
        let v = rng.rnd();
        println!("{} {} {}", i, v, isqrt32(v));
    }
    println!("[bitexact_cos]");
    for i in 0..NIN {
        let x = (rng.rnd() & 0x7FFF) as i32;
        println!("{} {} {}", i, x, bitexact_cos(x as i16));
    }
    println!("[bitexact_log2tan]");
    for i in 0..NIN {
        let isin = (rng.rnd() % 32768) as i32;
        let icos = 32768 - isin;
        println!("{} {} {}", i, isin, bitexact_log2tan(isin, icos));
    }
    println!("[celt_lcg_rand]");
    {
        let mut seed: opus_uint32 = 0x12345678;
        for i in 0..NIN {
            seed = celt_lcg_rand(seed);
            println!("{} {}", i, seed as i32);
        }
    }
}
