//! Rust side of the Laplace-coder bit-exactness probe.
//!
//! Mirrors `compat/c/probe_laplace.c` operation for operation.  Run
//! `bash compat/scripts/run-probe.sh laplace` to build both sides and diff
//! their output.
//!
//! The C probe recompiles `celt/laplace.c` (with its public entry points
//! renamed) only to reach the `static` helper `ec_laplace_get_freq1()`; here
//! the same helper is `pub` in `src/celt/laplace.rs`.

use oprs::celt::laplace::*;
use oprs::celt::{entdec::*, entenc::*};

const NVAL: usize = 256;
const NFREQ: usize = 96;
const NP0: usize = 128;
const STORAGE: u32 = 65536;

struct Rng(u32);
impl Rng {
    fn rnd(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345);
        self.0
    }
}

/// The start-frequency helper used by `celt/tests/test_unit_laplace.c`.
fn ec_laplace_get_start_freq(decay: i32) -> i32 {
    let ft: u32 = 32768 - LAPLACE_MINP * (2 * LAPLACE_NMIN + 1);
    let fs: i32 = (ft.wrapping_mul((16384 - decay) as u32) / (16384 + decay) as u32) as i32;
    fs + LAPLACE_MINP as i32
}

fn main() {
    let mut rng = Rng(0x9E3779B9);
    let mut bad: i32 = 0;

    let mut val_a = vec![0i32; NVAL];
    let mut fs_a = vec![0u32; NVAL];
    let mut decay_a = vec![0i32; NVAL];
    let mut state_a = vec![0u32; NVAL];

    let mut val_b = vec![0i32; NVAL];
    let mut fs_b = vec![0u32; NVAL];
    let mut decay_b = vec![0i32; NVAL];
    let mut state_b = vec![0u32; NVAL];

    let mut val_p = vec![0i32; NP0];
    let mut p0_p = vec![0u16; NP0];
    let mut decay_p = vec![0u16; NP0];
    let mut state_p = vec![0u32; NP0];

    println!("== ec_laplace_get_freq1 ==");
    for i in 0..NFREQ {
        let st = rng.rnd();
        let fs0: u32 = st % 33000;
        let st = rng.rnd();
        let decay = (st % 16385) as i32;
        println!(
            "freq1 i={} state={:08x} fs0={} decay={} out={}",
            i,
            st,
            fs0,
            decay,
            ec_laplace_get_freq1(fs0, decay)
        );
    }

    println!("== ec_laplace_encode fs=get_start_freq(decay) ==");
    let mut enc = ec_enc_init(STORAGE);
    for i in 0..NVAL {
        let st = rng.rnd();
        val_a[i] = (st % 41) as i32 - 20;
        let st = rng.rnd();
        decay_a[i] = (st % 11000) as i32 + 5000;
        fs_a[i] = ec_laplace_get_start_freq(decay_a[i]) as u32;
        state_a[i] = st;
        let mut v = val_a[i];
        print!(
            "encA i={} state={:08x} vin={} fs={} decay={}",
            i, st, v, fs_a[i], decay_a[i]
        );
        ec_laplace_encode(&mut enc, &mut v, fs_a[i], decay_a[i]);
        val_a[i] = v;
        println!(
            " vout={} offs={} rng={:08x} val={:08x} rem={} ext={} nbits={} err={} tell={}",
            v,
            enc.offs,
            enc.rng,
            enc.val,
            enc.rem,
            enc.ext,
            enc.nbits_total,
            enc.error,
            enc.ec_tell()
        );
    }
    ec_enc_done(&mut enc);
    println!(
        "encA done offs={} end_offs={} nbits={} err={} tell={} bytes={}",
        enc.offs,
        enc.end_offs,
        enc.nbits_total,
        enc.error,
        enc.ec_tell(),
        enc.ec_range_bytes()
    );
    print!("encA buf=");
    let n = enc.ec_range_bytes() as usize;
    for b in &enc.ec_get_buffer()[..n] {
        print!("{:02x}", b);
    }
    println!();

    println!("== ec_laplace_decode ==");
    let bytes = enc.ec_get_buffer()[..n].to_vec();
    let mut dec = ec_dec_init(&bytes);
    for i in 0..NVAL {
        let d = ec_laplace_decode(&mut dec, fs_a[i], decay_a[i]);
        if d != val_a[i] {
            bad += 1;
        }
        println!(
            "decA i={} state={:08x} fs={} decay={} expect={} got={} offs={} rng={:08x} val={:08x} rem={} ext={} err={} tell={} ok={}",
            i,
            state_a[i],
            fs_a[i],
            decay_a[i],
            val_a[i],
            d,
            dec.offs,
            dec.rng,
            dec.val,
            dec.rem,
            dec.ext,
            dec.error,
            dec.ec_tell(),
            (d == val_a[i]) as i32
        );
    }

    println!("== ec_laplace_encode fs=random ==");
    enc = ec_enc_init(STORAGE);
    for i in 0..NVAL {
        let st = rng.rnd();
        val_b[i] = (st % 41) as i32 - 20;
        let st = rng.rnd();
        fs_b[i] = st % 32000 + 1;
        let st = rng.rnd();
        decay_b[i] = (st % 16000) as i32 + 1;
        state_b[i] = st;
        let mut v = val_b[i];
        print!(
            "encB i={} state={:08x} vin={} fs={} decay={}",
            i, st, v, fs_b[i], decay_b[i]
        );
        ec_laplace_encode(&mut enc, &mut v, fs_b[i], decay_b[i]);
        val_b[i] = v;
        println!(
            " vout={} offs={} rng={:08x} val={:08x} rem={} ext={} nbits={} err={} tell={}",
            v,
            enc.offs,
            enc.rng,
            enc.val,
            enc.rem,
            enc.ext,
            enc.nbits_total,
            enc.error,
            enc.ec_tell()
        );
    }
    ec_enc_done(&mut enc);
    println!(
        "encB done offs={} end_offs={} nbits={} err={} tell={} bytes={}",
        enc.offs,
        enc.end_offs,
        enc.nbits_total,
        enc.error,
        enc.ec_tell(),
        enc.ec_range_bytes()
    );
    print!("encB buf=");
    let n = enc.ec_range_bytes() as usize;
    for b in &enc.ec_get_buffer()[..n] {
        print!("{:02x}", b);
    }
    println!();

    println!("== ec_laplace_decode random fs ==");
    let bytes = enc.ec_get_buffer()[..n].to_vec();
    let mut dec = ec_dec_init(&bytes);
    for i in 0..NVAL {
        let d = ec_laplace_decode(&mut dec, fs_b[i], decay_b[i]);
        if d != val_b[i] {
            bad += 1;
        }
        println!(
            "decB i={} state={:08x} fs={} decay={} expect={} got={} offs={} rng={:08x} val={:08x} rem={} ext={} err={} tell={} ok={}",
            i,
            state_b[i],
            fs_b[i],
            decay_b[i],
            val_b[i],
            d,
            dec.offs,
            dec.rng,
            dec.val,
            dec.rem,
            dec.ext,
            dec.error,
            dec.ec_tell(),
            (d == val_b[i]) as i32
        );
    }

    println!("== ec_laplace_encode_p0 ==");
    enc = ec_enc_init(STORAGE);
    for i in 0..NP0 {
        let st = rng.rnd();
        val_p[i] = (st % 41) as i32 - 20;
        let st = rng.rnd();
        p0_p[i] = (st % 32000 + 1) as u16;
        let st = rng.rnd();
        decay_p[i] = (st % 16000 + 1) as u16;
        state_p[i] = st;
        print!(
            "encP i={} state={:08x} vin={} p0={} decay={}",
            i, st, val_p[i], p0_p[i], decay_p[i]
        );
        ec_laplace_encode_p0(&mut enc, val_p[i], p0_p[i], decay_p[i]);
        println!(
            " offs={} rng={:08x} val={:08x} rem={} ext={} nbits={} err={} tell={}",
            enc.offs,
            enc.rng,
            enc.val,
            enc.rem,
            enc.ext,
            enc.nbits_total,
            enc.error,
            enc.ec_tell()
        );
    }
    ec_enc_done(&mut enc);
    println!(
        "encP done offs={} end_offs={} nbits={} err={} tell={} bytes={}",
        enc.offs,
        enc.end_offs,
        enc.nbits_total,
        enc.error,
        enc.ec_tell(),
        enc.ec_range_bytes()
    );
    print!("encP buf=");
    let n = enc.ec_range_bytes() as usize;
    for b in &enc.ec_get_buffer()[..n] {
        print!("{:02x}", b);
    }
    println!();

    println!("== ec_laplace_decode_p0 ==");
    let bytes = enc.ec_get_buffer()[..n].to_vec();
    let mut dec = ec_dec_init(&bytes);
    for i in 0..NP0 {
        let d = ec_laplace_decode_p0(&mut dec, p0_p[i], decay_p[i]);
        if d != val_p[i] {
            bad += 1;
        }
        /* Note: `ext` is not printed here.  C's ec_dec_init() does not
           initialize it (it is only written by ec_decode*()), so its value
           after a run of ec_laplace_decode_p0() is stale. */
        println!(
            "decP i={} state={:08x} p0={} decay={} expect={} got={} offs={} rng={:08x} val={:08x} rem={} err={} tell={} ok={}",
            i,
            state_p[i],
            p0_p[i],
            decay_p[i],
            val_p[i],
            d,
            dec.offs,
            dec.rng,
            dec.val,
            dec.rem,
            dec.error,
            dec.ec_tell(),
            (d == val_p[i]) as i32
        );
    }

    println!("== summary bad={} ==", bad);
    std::process::exit(if bad != 0 { 1 } else { 0 });
}
