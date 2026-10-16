// The probe keeps the C names (`LM`, `C`, `signalBandwidth`, ...) so that it can
// be diffed against `compat/c/probe_modes.c` line by line.
#![allow(non_snake_case)]

//! Rust side of the `celt/modes.{h,c}` + `celt/rate.{h,c}` +
//! `celt/static_modes_float.h` bit-exactness probe.
//!
//! Mirrors `compat/c/probe_modes.c` call for call, byte for byte, including the
//! LCG (`state = state*1103515245 + 12345`) and the order in which it is drawn.
//! Run `bash compat/scripts/run-probe.sh modes` to build both sides and diff
//! their output.
//!
//! `init_caps()` lives in `celt/celt.c` (not part of this port yet), so the
//! probe reproduces it locally; `interp_bits2pulses()` is `static` in C, so the
//! `bits1`/`bits2` it consumes are recomputed by `alloc_prelude()`, a
//! transliteration of the first half of `clt_compute_allocation()`.  Everything
//! else printed comes from the port itself.

use oprs::celt::entdec::ec_dec_init;
use oprs::celt::entenc::{ec_enc_done, ec_enc_init};
use oprs::celt::entcode::ec_tell_frac;
use oprs::celt::kiss_fft::{kiss_twiddle_cpx, MAXFACTORS};
use oprs::celt::modes::{opus_custom_mode_create, CELTMode};
use oprs::celt::rate::{clt_compute_allocation, LOG2_FRAC_TABLE};

/* ------------------------------------------------------------------ */
/* deterministic RNG: state = state*1103515245 + 12345                */
/* ------------------------------------------------------------------ */
static mut RNG_STATE: u32 = 0;
fn rnd_reset(s: u32) {
    unsafe { RNG_STATE = s }
}
fn rnd() -> u32 {
    unsafe {
        RNG_STATE = RNG_STATE.wrapping_mul(1103515245).wrapping_add(12345);
        RNG_STATE
    }
}

/* ------------------------------------------------------------------ */
/* FNV-1a 64 over raw element bytes                                   */
/* ------------------------------------------------------------------ */
struct Fnv(u64);
impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf29ce484222325)
    }
    fn byte(&mut self, b: u8) {
        self.0 ^= b as u64;
        self.0 = self.0.wrapping_mul(1099511628211);
    }
    fn u16(&mut self, v: u16) {
        for b in v.to_le_bytes() {
            self.byte(b);
        }
    }
    fn u32(&mut self, v: u32) {
        for b in v.to_le_bytes() {
            self.byte(b);
        }
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
}
fn fb(v: f32) -> u32 {
    v.to_bits()
}
fn hash_i16(v: &[i16]) -> u64 {
    let mut h = Fnv::new();
    for &x in v {
        h.u16(x as u16);
    }
    h.0
}
fn hash_u8(v: &[u8]) -> u64 {
    let mut h = Fnv::new();
    for &x in v {
        h.byte(x);
    }
    h.0
}
fn hash_f32(v: &[f32]) -> u64 {
    let mut h = Fnv::new();
    for &x in v {
        h.f32(x);
    }
    h.0
}
fn hash_twid(v: &[kiss_twiddle_cpx], len: usize) -> u64 {
    let mut h = Fnv::new();
    for x in v.iter().take(len) {
        h.f32(x.r);
        h.f32(x.i);
    }
    h.0
}

/* ------------------------------------------------------------------ */
/* full dumps                                                         */
/* ------------------------------------------------------------------ */
fn dump_i16(tag: &str, ci: i32, v: &[i16]) {
    println!("full {} {} {}", tag, ci, v.len());
    for (i, &x) in v.iter().enumerate() {
        print!("{}", x);
        if i % 10 == 9 || i == v.len() - 1 {
            println!();
        } else {
            print!(" ");
        }
    }
}
fn dump_u8(tag: &str, ci: i32, v: &[u8]) {
    println!("full {} {} {}", tag, ci, v.len());
    for (i, &x) in v.iter().enumerate() {
        print!("{}", x);
        if i % 10 == 9 || i == v.len() - 1 {
            println!();
        } else {
            print!(" ");
        }
    }
}
fn dump_f32(tag: &str, ci: i32, v: &[f32]) {
    println!("full {} {} {}", tag, ci, v.len());
    for (i, &x) in v.iter().enumerate() {
        print!("{:08x}", fb(x));
        if i % 10 == 9 || i == v.len() - 1 {
            println!();
        } else {
            print!(" ");
        }
    }
}
fn dump_twid(tag: &str, ci: i32, v: &[kiss_twiddle_cpx]) {
    println!("full {} {} {}", tag, ci, v.len());
    for (i, x) in v.iter().enumerate() {
        print!("{:08x} {:08x}", fb(x.r), fb(x.i));
        if i % 4 == 3 || i == v.len() - 1 {
            println!();
        } else {
            print!(" ");
        }
    }
}

/* ------------------------------------------------------------------ */
/* part 1: mode creation                                              */
/* ------------------------------------------------------------------ */
const MODE_CASES: [(i32, i32); 25] = [
    (48000, 960),  /* static mode48000_960_120, frame_size<<0 */
    (48000, 480),  /* static, frame_size<<1 */
    (48000, 240),  /* static, frame_size<<2 */
    (48000, 120),  /* static, frame_size<<3 */
    (24000, 480),  /* computed, standard eBand, effEBands 19 */
    (16000, 320),
    (12000, 240),
    (8000, 160),
    (96000, 960), /* computed, maxLM 2 */
    (44100, 480), /* computed, non-standard eBands (22 bands) */
    (88200, 960),
    (12345, 320), /* odd sampling rate (17 bands) */
    (22050, 480), /* 19 bands */
    (11025, 240), /* 16 bands */
    (40000, 480),
    (32000, 320),
    (64000, 640),
    (44100, 440), /* mdct length 880 has a prime factor > 5: OPUS_ALLOC_FAIL */
    (88200, 880), /* idem (1760 = 2^5*5*11) */
    (48000, 961), /* odd frame_size: OPUS_BAD_ARG */
    (48000, 38),  /* frame_size < 40 */
    (96000, 40),  /* frame shorter than 1ms */
    (7999, 160),  /* Fs < 8000 */
    (12345, 480), /* short block longer than 3.3ms */
    (7350, 160),  /* Fs < 8000 */
];
const NMODES: usize = MODE_CASES.len();

fn probe_mode(ci: i32, m: &CELTMode) {
    println!(
        "mode {} fields Fs={} overlap={} nbEBands={} effEBands={} maxLM={} nbShortMdcts={} shortMdctSize={} nbAllocVectors={} cacheSize={}",
        ci, m.Fs, m.overlap, m.nbEBands, m.effEBands, m.maxLM, m.nbShortMdcts, m.shortMdctSize,
        m.nbAllocVectors, m.cache.size
    );
    println!(
        "mode {} preemph {:08x} {:08x} {:08x} {:08x}",
        ci,
        fb(m.preemph[0]),
        fb(m.preemph[1]),
        fb(m.preemph[2]),
        fb(m.preemph[3])
    );
    println!(
        "mode {} eBands len={} hash={:016x}",
        ci,
        m.nbEBands + 1,
        hash_i16(&m.eBands[..(m.nbEBands + 1) as usize])
    );
    println!(
        "mode {} logN len={} hash={:016x}",
        ci,
        m.nbEBands,
        hash_i16(&m.logN[..m.nbEBands as usize])
    );
    println!(
        "mode {} allocVectors len={} hash={:016x}",
        ci,
        m.nbAllocVectors * m.nbEBands,
        hash_u8(&m.allocVectors[..(m.nbAllocVectors * m.nbEBands) as usize])
    );
    println!(
        "mode {} cacheIndex len={} hash={:016x}",
        ci,
        m.nbEBands * (m.maxLM + 2),
        hash_i16(&m.cache.index[..(m.nbEBands * (m.maxLM + 2)) as usize])
    );
    println!(
        "mode {} cacheBits len={} hash={:016x}",
        ci,
        m.cache.size,
        hash_u8(&m.cache.bits[..m.cache.size as usize])
    );
    println!(
        "mode {} cacheCaps len={} hash={:016x}",
        ci,
        (m.maxLM + 1) * 2 * m.nbEBands,
        hash_u8(&m.cache.caps[..((m.maxLM + 1) * 2 * m.nbEBands) as usize])
    );
    println!(
        "mode {} window len={} hash={:016x}",
        ci,
        m.overlap,
        hash_f32(&m.window[..m.overlap as usize])
    );
    let triglen = m.mdct.n - ((m.mdct.n >> 1) >> m.mdct.maxshift);
    println!(
        "mode {} mdct n={} maxshift={} triglen={} hash={:016x}",
        ci,
        m.mdct.n,
        m.mdct.maxshift,
        triglen,
        hash_f32(&m.mdct.trig[..triglen as usize])
    );
    let base_nfft = m.mdct.kfft[0].as_ref().unwrap().nfft;
    for i in 0..=m.mdct.maxshift {
        let st = m.mdct.kfft[i as usize].as_ref().unwrap();
        println!(
            "mode {} fft {} nfft={} shift={} scale={:08x} bitrevLen={} bitrevHash={:016x} twidLen={} twidHash={:016x}",
            ci,
            i,
            st.nfft,
            st.shift,
            fb(st.scale),
            st.nfft,
            hash_i16(&st.bitrev[..st.nfft as usize]),
            base_nfft,
            hash_twid(&st.twiddles, base_nfft as usize)
        );
        print!("mode {} factors {}", ci, i);
        for j in 0..MAXFACTORS {
            print!(" {}", st.factors[2 * j]);
            if st.factors[2 * j + 1] == 1 {
                break;
            }
        }
        println!();
    }
    if ci == 0 {
        /* Full dumps of the static mode's tables. */
        dump_i16("eBands", ci, &m.eBands[..(m.nbEBands + 1) as usize]);
        dump_i16("logN", ci, &m.logN[..m.nbEBands as usize]);
        dump_i16("cacheIndex", ci, &m.cache.index[..(m.nbEBands * (m.maxLM + 2)) as usize]);
        dump_u8("cacheBits", ci, &m.cache.bits[..m.cache.size as usize]);
        dump_u8("cacheCaps", ci, &m.cache.caps[..((m.maxLM + 1) * 2 * m.nbEBands) as usize]);
        dump_f32("window", ci, &m.window[..m.overlap as usize]);
        dump_f32("trig", ci, &m.mdct.trig[..triglen as usize]);
        dump_twid("twid", ci, &m.mdct.kfft[0].as_ref().unwrap().twiddles);
        dump_i16("bitrev0", ci, &m.mdct.kfft[0].as_ref().unwrap().bitrev);
        dump_i16("bitrev3", ci, &m.mdct.kfft[3].as_ref().unwrap().bitrev);
    }
}

/* ------------------------------------------------------------------ */
/* part 2: rate allocation                                            */
/* ------------------------------------------------------------------ */

/// `init_caps()` (celt/celt.c).
fn init_caps(m: &CELTMode, cap: &mut [i32], LM: i32, C: i32) {
    for i in 0..m.nbEBands as usize {
        let n = (m.eBands[i + 1] as i32 - m.eBands[i] as i32) << LM;
        cap[i] = (m.cache.caps[(m.nbEBands * (2 * LM + C - 1) + i as i32) as usize] as i32 + 64)
            * C
            * n
            >> 2;
    }
}

fn imin(a: i32, b: i32) -> i32 {
    if a < b { a } else { b }
}
fn imax(a: i32, b: i32) -> i32 {
    if a > b { a } else { b }
}

/// Transliteration of the first half of `clt_compute_allocation()`; the function
/// itself is the port's, but `bits1`/`bits2` are internal to the static
/// `interp_bits2pulses()`, so they are recomputed here.
fn alloc_prelude(
    m: &CELTMode,
    start: i32,
    end: i32,
    offsets: &[i32],
    cap: &[i32],
    alloc_trim: i32,
    C: i32,
    LM: i32,
    mut total: i32,
    bits1: &mut [i32],
    bits2: &mut [i32],
    thresh: &mut [i32],
    trim_offset: &mut [i32],
) -> (i32, i32, i32, i32) {
    let len = m.nbEBands;
    let mut skip_start = start;
    total = imax(total, 0);
    let skip_rsv = if total >= 1 << 3 { 1 << 3 } else { 0 };
    total -= skip_rsv;
    let mut intensity_rsv = 0;
    let mut dual_stereo_rsv = 0;
    if C == 2 {
        intensity_rsv = LOG2_FRAC_TABLE[(end - start) as usize] as i32;
        if intensity_rsv > total {
            intensity_rsv = 0;
        } else {
            total -= intensity_rsv;
            dual_stereo_rsv = if total >= 1 << 3 { 1 << 3 } else { 0 };
            total -= dual_stereo_rsv;
        }
    }
    for j in start..end {
        thresh[j as usize] = imax(
            C << 3,
            (3 * (m.eBands[(j + 1) as usize] as i32 - m.eBands[j as usize] as i32) << LM << 3) >> 4,
        );
        trim_offset[j as usize] = C
            * (m.eBands[(j + 1) as usize] as i32 - m.eBands[j as usize] as i32)
            * (alloc_trim - 5 - LM)
            * (end - j - 1)
            * (1 << (LM + 3))
            >> 6;
        if (m.eBands[(j + 1) as usize] as i32 - m.eBands[j as usize] as i32) << LM == 1 {
            trim_offset[j as usize] -= C << 3;
        }
    }
    let mut lo = 1;
    let mut hi = m.nbAllocVectors - 1;
    loop {
        let mut done = 0;
        let mut psum = 0;
        let mid = (lo + hi) >> 1;
        for j in (start..end).rev() {
            let n = m.eBands[(j + 1) as usize] as i32 - m.eBands[j as usize] as i32;
            let mut bitsj =
                (C * n * (m.allocVectors[(mid * len + j) as usize] as i32)) << LM >> 2;
            if bitsj > 0 {
                bitsj = imax(0, bitsj + trim_offset[j as usize]);
            }
            bitsj += offsets[j as usize];
            if bitsj >= thresh[j as usize] || done != 0 {
                done = 1;
                psum += imin(bitsj, cap[j as usize]);
            } else if bitsj >= C << 3 {
                psum += C << 3;
            }
        }
        if psum > total {
            hi = mid - 1;
        } else {
            lo = mid + 1;
        }
        if lo > hi {
            break;
        }
    }
    hi = lo;
    lo -= 1;
    for j in start..end {
        let n = m.eBands[(j + 1) as usize] as i32 - m.eBands[j as usize] as i32;
        let mut bits1j =
            (C * n * (m.allocVectors[(lo * len + j) as usize] as i32)) << LM >> 2;
        let mut bits2j = if hi >= m.nbAllocVectors {
            cap[j as usize]
        } else {
            (C * n * (m.allocVectors[(hi * len + j) as usize] as i32)) << LM >> 2
        };
        if bits1j > 0 {
            bits1j = imax(0, bits1j + trim_offset[j as usize]);
        }
        if bits2j > 0 {
            bits2j = imax(0, bits2j + trim_offset[j as usize]);
        }
        if lo > 0 {
            bits1j += offsets[j as usize];
        }
        bits2j += offsets[j as usize];
        if offsets[j as usize] > 0 {
            skip_start = j;
        }
        bits2j = imax(0, bits2j - bits1j);
        bits1[j as usize] = bits1j;
        bits2[j as usize] = bits2j;
    }
    (skip_start, skip_rsv, intensity_rsv, dual_stereo_rsv)
}

const ENC_STORAGE: usize = 2048;
const RATE_MODES: [usize; 6] = [0, 4, 7, 8, 9, 11];

fn rate_case(rc: i32, midx: i32, m: &CELTMode, LM: i32, C: i32, variant: i32) {
    let len = m.nbEBands;
    let mut offsets = vec![0i32; 512];
    let mut cap = vec![0i32; 512];
    let mut pulses = vec![0i32; 512];
    let mut ebits = vec![0i32; 512];
    let mut fine_priority = vec![0i32; 512];
    let mut bits1 = vec![0i32; 512];
    let mut bits2 = vec![0i32; 512];
    let mut thresh = vec![0i32; 512];
    let mut trim_offset = vec![0i32; 512];
    let mut dec_pulses = vec![0i32; 512];
    let mut dec_ebits = vec![0i32; 512];
    let mut dec_fine = vec![0i32; 512];

    let start = 0;
    let end = if variant == 2 { m.nbEBands } else { m.effEBands };
    for j in 0..len as usize {
        offsets[j] = if rnd() % 4 == 0 { (rnd() % 64) as i32 } else { 0 };
    }
    init_caps(m, &mut cap, LM, C);
    if variant == 2 {
        for j in 0..len as usize {
            if rnd() % 5 == 0 {
                cap[j] = imin(cap[j], (rnd() % 256) as i32);
            }
        }
    }
    let alloc_trim = (rnd() % 11) as i32;
    let total = if variant == 0 {
        8 + (rnd() % 256) as i32
    } else if variant == 1 {
        256 + (rnd() % 1024) as i32
    } else {
        1024 + (rnd() % 3000) as i32
    };
    let prev = if variant == 0 { 0 } else { (rnd() % len as u32) as i32 };
    let signalBandwidth = if variant == 2 {
        (rnd() % len as u32) as i32
    } else {
        end - 1
    };
    let mut intensity = if variant == 2 {
        (rnd() % (len as u32 + 1)) as i32
    } else {
        end
    };
    let mut dual_stereo = (rnd() & 1) as i32;

    let (skip_start, skip_rsv, intensity_rsv, dual_stereo_rsv) = alloc_prelude(
        m,
        start,
        end,
        &offsets,
        &cap,
        alloc_trim,
        C,
        LM,
        total,
        &mut bits1,
        &mut bits2,
        &mut thresh,
        &mut trim_offset,
    );

    println!(
        "rate {} mode={} Fs={} LM={} C={} variant={} start={} end={} total={} trim={} prev={} signalBandwidth={} intensity0={} dual0={} skip_start={} skip_rsv={} intensity_rsv={} dual_stereo_rsv={}",
        rc, midx, m.Fs, LM, C, variant, start, end, total, alloc_trim, prev, signalBandwidth,
        intensity, dual_stereo, skip_start, skip_rsv, intensity_rsv, dual_stereo_rsv
    );
    print!("rate {} offsets", rc);
    for j in 0..len {
        print!(" {}", offsets[j as usize]);
    }
    println!();
    print!("rate {} cap", rc);
    for j in 0..len {
        print!(" {}", cap[j as usize]);
    }
    println!();
    print!("rate {} bits1", rc);
    for j in 0..len {
        print!(" {}", bits1[j as usize]);
    }
    println!();
    print!("rate {} bits2", rc);
    for j in 0..len {
        print!(" {}", bits2[j as usize]);
    }
    println!();
    print!("rate {} thresh", rc);
    for j in 0..len {
        print!(" {}", thresh[j as usize]);
    }
    println!();

    let mut balance = 0i32;
    let mut enc = ec_enc_init(ENC_STORAGE as u32);
    let tell0 = ec_tell_frac(&enc);
    let coded_bands = clt_compute_allocation(
        m,
        start,
        end,
        &offsets,
        &cap,
        alloc_trim,
        &mut intensity,
        &mut dual_stereo,
        total,
        &mut balance,
        &mut pulses,
        &mut ebits,
        &mut fine_priority,
        C,
        LM,
        &mut enc,
        1,
        prev,
        signalBandwidth,
    );
    let tell1 = ec_tell_frac(&enc);
    ec_enc_done(&mut enc);
    let bufhash = hash_u8(&enc.buf);
    println!(
        "rate {} enc codedBands={} balance={} intensity={} dual_stereo={} tell0={} tell1={} offs={} storage={} bufhash={:016x}",
        rc, coded_bands, balance, intensity, dual_stereo, tell0, tell1, enc.offs, enc.storage, bufhash
    );
    print!("rate {} enc pulses", rc);
    for j in 0..len {
        print!(" {}", pulses[j as usize]);
    }
    println!();
    print!("rate {} enc ebits", rc);
    for j in 0..len {
        print!(" {}", ebits[j as usize]);
    }
    println!();
    print!("rate {} enc fine_priority", rc);
    for j in 0..len {
        print!(" {}", fine_priority[j as usize]);
    }
    println!();

    /* Decoder: same inputs, parameters read back from the stream. */
    for j in 0..len as usize {
        dec_pulses[j] = 0;
        dec_ebits[j] = 0;
        dec_fine[j] = 0;
    }
    intensity = 0;
    dual_stereo = 0;
    let mut dec_balance = 0i32;
    let bytes = enc.buf.clone();
    let mut dec = ec_dec_init(&bytes);
    let dec_tell0 = ec_tell_frac(&dec);
    let dec_coded_bands = clt_compute_allocation(
        m,
        start,
        end,
        &offsets,
        &cap,
        alloc_trim,
        &mut intensity,
        &mut dual_stereo,
        total,
        &mut dec_balance,
        &mut dec_pulses,
        &mut dec_ebits,
        &mut dec_fine,
        C,
        LM,
        &mut dec,
        0,
        prev,
        signalBandwidth,
    );
    let dec_tell1 = ec_tell_frac(&dec);
    println!(
        "rate {} dec codedBands={} balance={} intensity={} dual_stereo={} tell0={} tell1={}",
        rc, dec_coded_bands, dec_balance, intensity, dual_stereo, dec_tell0, dec_tell1
    );
    print!("rate {} dec pulses", rc);
    for j in 0..len {
        print!(" {}", dec_pulses[j as usize]);
    }
    println!();
    print!("rate {} dec ebits", rc);
    for j in 0..len {
        print!(" {}", dec_ebits[j as usize]);
    }
    println!();
    print!("rate {} dec fine_priority", rc);
    for j in 0..len {
        print!(" {}", dec_fine[j as usize]);
    }
    println!();
}

fn main() {
    let mut modes: Vec<Option<Box<CELTMode>>> = Vec::new();
    println!("probe_modes: FIXED_POINT=0 FLOAT_APPROX=0 CUSTOM_MODES=1 ENABLE_QEXT=0");
    for ci in 0..NMODES {
        let (fs, frame_size) = MODE_CASES[ci];
        let mut err = 12345;
        let m = opus_custom_mode_create(fs, frame_size, &mut err);
        println!(
            "mode {} req Fs={} frame_size={} err={}",
            ci, fs, frame_size, err
        );
        match &m {
            None => println!("mode {} NULL", ci),
            Some(mm) => probe_mode(ci as i32, mm),
        }
        modes.push(m);
    }

    rnd_reset(0x13579BDF);
    let mut rc = 0i32;
    const LMS: [i32; 3] = [0, 1, 3];
    for &mi in RATE_MODES.iter() {
        let m = match &modes[mi] {
            Some(m) => m,
            None => continue,
        };
        for variant in 0..3 {
            for C in 1..=2 {
                for lm in 0..3 {
                    if LMS[lm] > m.maxLM {
                        continue;
                    }
                    rate_case(rc, mi as i32, m, LMS[lm], C, variant);
                    rc += 1;
                }
            }
        }
    }
}
