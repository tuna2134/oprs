//! Rust side of the CELT band-energy bit-exactness probe.
//!
//! Mirrors `compat/c/probe_quant_bands.c` operation for operation: a real
//! `CELTMode` (48000 Hz, 960-sample frames) and the same table of
//! `quant_coarse_energy` -> `quant_fine_energy` -> `quant_energy_finalise`
//! round trips through a real `ec_enc`/`ec_dec`, with every array printed as
//! raw `f32` bits and the complete range-coder state after every stage.
//!
//! `bash compat/scripts/run-probe.sh quant_bands` builds both sides and diffs
//! their output; the only acceptable result is `IDENTICAL`.

use oprs::celt::entcode::ec_ctx;
use oprs::celt::entdec::{ec_dec_bit_logp, ec_dec_bits, ec_dec_init};
use oprs::celt::entenc::{ec_enc_bits, ec_enc_done, ec_enc_init};
use oprs::celt::modes::{opus_custom_mode_create, CELTMode};
use oprs::celt::quant_bands::*;

const MAXBANDS: usize = 64;
const MAXN: usize = 2 * MAXBANDS;

struct Combo {
    LM: i32,
    C: i32,
    start: i32,
    end: i32,
    effEnd: i32,
    force_intra: i32,
    two_pass: i32,
    nbBytes: i32,
    loss_rate: i32,
    lfe: i32,
    pre_bits: i32,
    use_prev: i32,
    fin_null: i32,
}

/* One row per scenario; identical table to the C probe. */
static COMBOS: [Combo; 18] = [
    Combo { LM: 0, C: 1, start: 0, end: 21, effEnd: 21, force_intra: 0, two_pass: 0, nbBytes: 200, loss_rate: 0, lfe: 0, pre_bits: 0, use_prev: 0, fin_null: 0 },
    Combo { LM: 0, C: 1, start: 0, end: 21, effEnd: 21, force_intra: 1, two_pass: 0, nbBytes: 200, loss_rate: 0, lfe: 0, pre_bits: 0, use_prev: 0, fin_null: 0 },
    Combo { LM: 0, C: 2, start: 0, end: 21, effEnd: 21, force_intra: 0, two_pass: 1, nbBytes: 200, loss_rate: 0, lfe: 0, pre_bits: 0, use_prev: 1, fin_null: 0 },
    Combo { LM: 1, C: 1, start: 0, end: 21, effEnd: 10, force_intra: 0, two_pass: 1, nbBytes: 100, loss_rate: 0, lfe: 0, pre_bits: 0, use_prev: 0, fin_null: 0 },
    Combo { LM: 1, C: 2, start: 3, end: 18, effEnd: 18, force_intra: 0, two_pass: 1, nbBytes: 60, loss_rate: 5, lfe: 0, pre_bits: 0, use_prev: 1, fin_null: 1 },
    Combo { LM: 2, C: 1, start: 0, end: 21, effEnd: 21, force_intra: 0, two_pass: 1, nbBytes: 40, loss_rate: 0, lfe: 0, pre_bits: 0, use_prev: 0, fin_null: 0 },
    Combo { LM: 2, C: 2, start: 0, end: 12, effEnd: 12, force_intra: 1, two_pass: 0, nbBytes: 30, loss_rate: 3, lfe: 0, pre_bits: 0, use_prev: 1, fin_null: 0 },
    Combo { LM: 3, C: 1, start: 0, end: 21, effEnd: 21, force_intra: 0, two_pass: 1, nbBytes: 8, loss_rate: 0, lfe: 0, pre_bits: 0, use_prev: 0, fin_null: 0 },
    Combo { LM: 3, C: 2, start: 0, end: 21, effEnd: 21, force_intra: 0, two_pass: 0, nbBytes: 3, loss_rate: 0, lfe: 0, pre_bits: 0, use_prev: 1, fin_null: 1 },
    Combo { LM: 3, C: 1, start: 0, end: 21, effEnd: 21, force_intra: 0, two_pass: 0, nbBytes: 1, loss_rate: 0, lfe: 0, pre_bits: 0, use_prev: 0, fin_null: 0 },
    Combo { LM: 3, C: 2, start: 0, end: 21, effEnd: 21, force_intra: 0, two_pass: 1, nbBytes: 200, loss_rate: 0, lfe: 1, pre_bits: 0, use_prev: 1, fin_null: 0 },
    Combo { LM: 0, C: 2, start: 0, end: 21, effEnd: 21, force_intra: 0, two_pass: 1, nbBytes: 20, loss_rate: 10, lfe: 0, pre_bits: 0, use_prev: 0, fin_null: 1 },
    Combo { LM: 1, C: 1, start: 5, end: 21, effEnd: 21, force_intra: 1, two_pass: 1, nbBytes: 80, loss_rate: 0, lfe: 0, pre_bits: 0, use_prev: 1, fin_null: 0 },
    Combo { LM: 2, C: 2, start: 0, end: 21, effEnd: 21, force_intra: 0, two_pass: 0, nbBytes: 2, loss_rate: 0, lfe: 0, pre_bits: 0, use_prev: 0, fin_null: 0 },
    Combo { LM: 0, C: 1, start: 0, end: 21, effEnd: 21, force_intra: 0, two_pass: 0, nbBytes: 1, loss_rate: 0, lfe: 1, pre_bits: 0, use_prev: 1, fin_null: 1 },
    Combo { LM: 3, C: 1, start: 0, end: 3, effEnd: 3, force_intra: 0, two_pass: 1, nbBytes: 150, loss_rate: 0, lfe: 0, pre_bits: 0, use_prev: 0, fin_null: 0 },
    Combo { LM: 1, C: 1, start: 0, end: 21, effEnd: 21, force_intra: 0, two_pass: 1, nbBytes: 1, loss_rate: 0, lfe: 0, pre_bits: 5, use_prev: 1, fin_null: 0 },
    Combo { LM: 2, C: 2, start: 0, end: 21, effEnd: 21, force_intra: 0, two_pass: 1, nbBytes: 3, loss_rate: 0, lfe: 0, pre_bits: 10, use_prev: 0, fin_null: 1 },
];

struct Rng(u32);
impl Rng {
    fn rnd(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345);
        self.0
    }
}

fn parr(name: &str, a: &[f32]) {
    print!("{}:", name);
    for x in a {
        print!(" {:08x}", x.to_bits());
    }
    println!();
}

fn parri(name: &str, a: &[i32]) {
    print!("{}:", name);
    for x in a {
        print!(" {}", x);
    }
    println!();
}

/// `error[]` is only ever written for `i in [start,end)` of each channel; the
/// rest is uninitialised memory in C (quant_coarse_energy() copies the whole
/// `error_intra` scratch buffer back through OPUS_COPY) and is never read by
/// the codec, so only the defined range is printed.
fn perr(name: &str, a: &[f32], start: i32, end: i32, len: i32, C: i32) {
    print!("{}:", name);
    for c in 0..C {
        for i in start..end {
            print!(" {:08x}", a[(i + c * len) as usize].to_bits());
        }
    }
    println!();
}

/// The encoder sets `ext` in `ec_enc_init()`, so it is always defined there.
fn pstate_enc(tag: &str, e: &ec_ctx) {
    println!(
        "{} tell={} rng={:08x} val={:08x} ext={:08x} rem={} offs={} nbits_total={} \
         end_offs={} end_window={:08x} nend_bits={} error={}",
        tag,
        e.ec_tell(),
        e.rng,
        e.val,
        e.ext,
        e.rem,
        e.offs,
        e.nbits_total,
        e.end_offs,
        e.end_window,
        e.nend_bits,
        e.error
    );
}

/// `ec_dec_init()` leaves `ext` uninitialised in C (only `ec_decode*()` writes
/// it) while the Rust port zeroes it, so the decoder's `ext` is deliberately
/// not printed; see the "Coordination notes" in docs/progress.md.
fn pstate_dec(tag: &str, e: &ec_ctx) {
    println!(
        "{} tell={} rng={:08x} val={:08x} rem={} offs={} nbits_total={} \
         end_offs={} end_window={:08x} nend_bits={} error={}",
        tag,
        e.ec_tell(),
        e.rng,
        e.val,
        e.rem,
        e.offs,
        e.nbits_total,
        e.end_offs,
        e.end_window,
        e.nend_bits,
        e.error
    );
}

fn run(m: &CELTMode, idx: usize, co: &Combo) {
    let mut eBands = [0f32; MAXN];
    let mut oldE = [0f32; MAXN];
    let mut error = [0f32; MAXN];
    let mut oldLogE = [0f32; MAXN];
    let mut extra_quant = [0i32; MAXBANDS];
    let mut prev_quant = [0i32; MAXBANDS];
    let mut fine_priority = [0i32; MAXBANDS];
    let nb = m.nbEBands as usize;
    let n = (co.C * m.nbEBands) as usize;
    let budget: i32 = co.nbBytes * 8;
    let mut delayed_intra: f32 = 0.0f32;

    let mut rng = Rng(0x12345678u32.wrapping_add((idx as u32).wrapping_mul(7919u32)));
    for i in 0..n {
        eBands[i] = (rng.rnd() % 24000) as f32 / 1000.0f32 - 12.0f32;
        oldE[i] = (rng.rnd() % 24000) as f32 / 1000.0f32 - 12.0f32;
    }
    for i in 0..nb {
        extra_quant[i] = (rng.rnd() % 5) as i32;
    }
    for i in 0..nb {
        prev_quant[i] = (rng.rnd() % 5) as i32;
    }
    for i in 0..nb {
        fine_priority[i] = (rng.rnd() % 2) as i32;
    }
    oldLogE[..n].copy_from_slice(&oldE[..n]);

    println!(
        "== combo {}: LM={} C={} start={} end={} effEnd={} force_intra={} \
         two_pass={} nbBytes={} loss_rate={} lfe={} pre_bits={} use_prev={} \
         fin_null={} ==",
        idx, co.LM, co.C, co.start, co.end, co.effEnd, co.force_intra, co.two_pass,
        co.nbBytes, co.loss_rate, co.lfe, co.pre_bits, co.use_prev, co.fin_null
    );
    parri("extra_quant", &extra_quant[..nb]);
    parri("prev_quant", &prev_quant[..nb]);
    parri("fine_priority", &fine_priority[..nb]);
    parr("eBands_in", &eBands[..n]);
    parr("oldE_in", &oldE[..n]);

    let mut enc = ec_enc_init(co.nbBytes as u32);
    if co.pre_bits != 0 {
        let mask: u32 = (1u32 << co.pre_bits) - 1u32;
        ec_enc_bits(&mut enc, 0x5A5A5A5Au32 & mask, co.pre_bits as u32);
    }
    pstate_enc("enc_pre", &enc);

    /* ---- coarse ---- */
    quant_coarse_energy(
        m,
        co.start,
        co.end,
        co.effEnd,
        &eBands[..n],
        &mut oldE[..n],
        budget as u32,
        &mut error[..n],
        &mut enc,
        co.C,
        co.LM,
        co.nbBytes,
        co.force_intra,
        &mut delayed_intra,
        co.two_pass,
        co.loss_rate,
        co.lfe,
    );
    pstate_enc("enc_coarse", &enc);
    println!("delayedIntra={:08x}", delayed_intra.to_bits());
    parr("oldE_coarse", &oldE[..n]);
    perr("error_coarse", &error[..n], co.start, co.end, nb as i32, co.C);

    /* ---- fine ---- */
    quant_fine_energy(
        m,
        co.start,
        co.end,
        &mut oldE[..n],
        &mut error[..n],
        if co.use_prev != 0 {
            Some(&prev_quant[..nb])
        } else {
            None
        },
        &extra_quant[..nb],
        &mut enc,
        co.C,
    );
    pstate_enc("enc_fine", &enc);
    parr("oldE_fine", &oldE[..n]);
    perr("error_fine", &error[..n], co.start, co.end, nb as i32, co.C);

    /* ---- finalise ---- */
    {
        let bits_left = budget - enc.ec_tell();
        println!("finalise_bits_left={}", bits_left);
        quant_energy_finalise(
            m,
            co.start,
            co.end,
            if co.fin_null != 0 {
                None
            } else {
                Some(&mut oldE[..n])
            },
            &mut error[..n],
            &extra_quant[..nb],
            &fine_priority[..nb],
            bits_left,
            &mut enc,
            co.C,
        );
    }
    pstate_enc("enc_finalise", &enc);
    parr("oldE_finalise", &oldE[..n]);
    perr("error_finalise", &error[..n], co.start, co.end, nb as i32, co.C);

    ec_enc_done(&mut enc);
    pstate_enc("enc_done", &enc);
    print!("buf:");
    for b in &enc.buf[..co.nbBytes as usize] {
        print!(" {:02x}", b);
    }
    println!();

    /* ---- decode ---- */
    let mut dec = ec_dec_init(&enc.buf[..co.nbBytes as usize]);
    if co.pre_bits != 0 {
        let _ = ec_dec_bits(&mut dec, co.pre_bits as u32);
    }
    let intra: i32;
    {
        let t = dec.ec_tell();
        intra = if t + 3 <= budget {
            ec_dec_bit_logp(&mut dec, 3)
        } else {
            0
        };
    }
    println!("intra={}", intra);
    pstate_dec("dec_intra", &dec);

    unquant_coarse_energy(m, co.start, co.end, &mut oldLogE[..n], intra, &mut dec, co.C, co.LM);
    pstate_dec("dec_coarse", &dec);
    parr("oldLogE_coarse", &oldLogE[..n]);

    unquant_fine_energy(
        m,
        co.start,
        co.end,
        &mut oldLogE[..n],
        if co.use_prev != 0 {
            Some(&prev_quant[..nb])
        } else {
            None
        },
        &extra_quant[..nb],
        &mut dec,
        co.C,
    );
    pstate_dec("dec_fine", &dec);
    parr("oldLogE_fine", &oldLogE[..n]);

    {
        let bits_left = budget - dec.ec_tell();
        unquant_energy_finalise(
            m,
            co.start,
            co.end,
            if co.fin_null != 0 {
                None
            } else {
                Some(&mut oldLogE[..n])
            },
            &extra_quant[..nb],
            &fine_priority[..nb],
            bits_left,
            &mut dec,
            co.C,
        );
        println!("dec_finalise_bits_left={}", bits_left);
    }
    pstate_dec("dec_finalise", &dec);
    parr("oldLogE_finalise", &oldLogE[..n]);

    /* encoder and decoder must have produced bit-identical energies */
    {
        let mut eq = 1;
        for i in 0..n {
            if oldE[i].to_bits() != oldLogE[i].to_bits() {
                eq = 0;
            }
        }
        println!("eq_oldE={}", eq);
    }
}

fn main() {
    let mut err = 0;
    let m = opus_custom_mode_create(48000, 960, &mut err).unwrap();
    println!(
        "mode err={} Fs={} nbEBands={} shortMdctSize={} maxLM={} effEBands={}",
        err, m.Fs, m.nbEBands, m.shortMdctSize, m.maxLM, m.effEBands
    );
    parr("eMeans", &eMeans);
    for (i, co) in COMBOS.iter().enumerate() {
        run(&m, i, co);
    }
}
