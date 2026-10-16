//! Rust side of the range-coder bit-exactness probe.
//!
//! Mirrors `compat/c/probe_entropy.c` operation for operation.  Run
//! `compat/scripts/run-probe.sh` to build both sides and diff their output.

use oprs::celt::{entcode::*, entdec::*, entenc::*};

const NOPS: usize = 4000;
const STORAGE: u32 = 16384;
const SHRINK_AT: usize = 2000;
const SHRINK_BY: u32 = 1000;

struct Rng(u32);
impl Rng {
    fn rnd(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345);
        self.0
    }
}

#[derive(Clone, Copy, Default)]
struct Op {
    r#type: usize,
    ft: u32,
    v: u32,
    logp: u32,
    ftb: u32,
    s: u32,
    bits: u32,
    fl: u32,
    fh: u32,
}

fn fnv_upd(h: &mut u64, byte: u8) {
    *h ^= byte as u64;
    *h = h.wrapping_mul(1099511628211);
}

fn main() {
    /* icdf_tab[ftb][j] = (2^ftb - 1 - j) */
    let mut icdf_tab = [[0u8; 256]; 9];
    for ftb in 1..=8usize {
        let n = 1usize << ftb;
        for j in 0..n {
            icdf_tab[ftb][j] = (n - 1 - j) as u8;
        }
    }

    let mut ops = vec![Op::default(); NOPS];
    let mut rng = Rng(0x9E3779B9);
    for i in 0..NOPS {
        let o = &mut ops[i];
        o.r#type = i % 5;
        match o.r#type {
            0 => {
                o.ft = 2 + rng.rnd() % 2048;
                o.v = rng.rnd() % o.ft;
            }
            1 => {
                o.logp = 1 + rng.rnd() % 14;
                o.v = rng.rnd() & 1;
            }
            2 => {
                o.ftb = 3 + rng.rnd() % 6;
                o.s = rng.rnd() % (1u32 << o.ftb);
            }
            3 => {
                o.bits = 1 + rng.rnd() % 24;
                o.v = rng.rnd() & ((1u32 << o.bits) - 1);
            }
            _ => {
                o.ft = 3 + rng.rnd() % 1000;
                o.fl = rng.rnd() % o.ft;
                o.fh = o.fl + 1 + rng.rnd() % (o.ft - o.fl);
            }
        }
    }

    /* ---------------- encode ---------------- */
    let mut enc = ec_enc_init(STORAGE);
    let mut final_storage = STORAGE;
    for i in 0..NOPS {
        let o = ops[i];
        match o.r#type {
            0 => ec_enc_uint(&mut enc, o.v, o.ft),
            1 => ec_enc_bit_logp(&mut enc, o.v as i32, o.logp),
            2 => ec_enc_icdf(&mut enc, o.s as i32, &icdf_tab[o.ftb as usize], o.ftb),
            3 => ec_enc_bits(&mut enc, o.v, o.bits),
            _ => ec_encode(&mut enc, o.fl, o.fh, o.ft),
        }
        if i == SHRINK_AT {
            final_storage = STORAGE - SHRINK_BY;
            ec_enc_shrink(&mut enc, final_storage);
        }
        if i % 500 == 0 {
            println!(
                "enc tell[{}]={} fracc={}",
                i,
                enc.ec_tell(),
                ec_tell_frac(&enc)
            );
        }
    }
    ec_enc_done(&mut enc);
    println!(
        "enc error={} offs={} end_offs={} storage={} nbits_total={} rng={}",
        enc.ec_get_error(),
        enc.offs,
        enc.end_offs,
        enc.storage,
        enc.nbits_total,
        enc.rng
    );
    let mut buf_hash: u64 = 1469598103934665603;
    for i in 0..final_storage as usize {
        fnv_upd(&mut buf_hash, enc.buf[i]);
    }
    println!("buf_hash=0x{:016x}", buf_hash);
    println!("buffer:");
    for i in 0..final_storage as usize {
        print!("{:02x}", enc.buf[i]);
        if i % 64 == 63 {
            println!();
        }
    }
    println!();

    /* ---------------- decode ---------------- */
    let mut dec = ec_dec_init(&enc.buf[..final_storage as usize]);
    let mut sym_hash: u64 = 1469598103934665603;
    for i in 0..NOPS {
        let o = ops[i];
        let got: u32 = match o.r#type {
            0 => ec_dec_uint(&mut dec, o.ft),
            1 => ec_dec_bit_logp(&mut dec, o.logp) as u32,
            2 => ec_dec_icdf(&mut dec, &icdf_tab[o.ftb as usize], o.ftb) as u32,
            3 => ec_dec_bits(&mut dec, o.bits),
            _ => {
                let s = ec_decode(&mut dec, o.ft);
                ec_dec_update(&mut dec, o.fl, o.fh, o.ft);
                if s < o.fl || s >= o.fh {
                    println!("INTERVAL MISMATCH at {}: s={} [{},{})", i, s, o.fl, o.fh);
                    std::process::exit(1);
                }
                s
            }
        };
        let want = if o.r#type == 2 { o.s } else { o.v };
        if o.r#type != 4 && got != want {
            println!(
                "MISMATCH at {} type={}: got {} want {}",
                i, o.r#type, got, want
            );
            std::process::exit(1);
        }
        for b in got.to_le_bytes() {
            fnv_upd(&mut sym_hash, b);
        }
        if i % 500 == 0 {
            println!(
                "dec tell[{}]={} fracc={} val={} rng={}",
                i,
                dec.ec_tell(),
                ec_tell_frac(&dec),
                dec.val,
                dec.rng
            );
        }
    }
    println!(
        "dec error={} nbits_total={} rng={} sym_hash=0x{:016x}",
        dec.ec_get_error(),
        dec.nbits_total,
        dec.rng,
        sym_hash
    );
}
