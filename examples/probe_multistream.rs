//! Rust side of the multistream bit-exactness probe.
//!
//! Mirrors `compat/c/probe_multistream.c` operation for operation (same row
//! table text, same PCM generator, same output format).  Run
//! `bash compat/scripts/run-probe.sh multistream` to build both sides and diff
//! their output.

use oprs::ffi::*;
use oprs::opus::multistream::*;

/* ------------------------------------------------------------------ */
/* helpers                                                            */
/* ------------------------------------------------------------------ */

fn fnv_bytes(p: &[u8]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &b in p {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn fnv_i16(p: &[i16]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &s in p {
        for b in s.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(1099511628211);
        }
    }
    h
}

fn print_hex(p: &[u8]) {
    for &b in p {
        print!("{:02x}", b);
    }
}

struct Rng {
    s: u32,
}

impl Rng {
    fn next(&mut self) -> u32 {
        self.s = self.s.wrapping_mul(1103515245).wrapping_add(12345);
        self.s
    }
    fn sample(&mut self) -> f32 {
        let v = ((self.next() >> 8) as i32) - 8388608;
        v as f32 / 8388608.0
    }
}

fn envelope(i: u64) -> f32 {
    0.25 + 0.75 * (i % 9973) as f32 / 9973.0
}

/// `gen_pcm()` for `ectype == 1`; the i16/i32 variants convert this `f32`.
fn gen_pcm_f32(out: &mut [f32], f: i32, frame: i32, channels: i32, r: &mut Rng) {
    let base = (f as i64 * frame as i64 * channels as i64) as u64;
    for (i, s) in out.iter_mut().enumerate() {
        *s = r.sample() * envelope(base + i as u64);
    }
}

/* ------------------------------------------------------------------ */
/* row table (identical text to the C probe)                          */
/* ------------------------------------------------------------------ */

struct Row {
    tag: String,
    channels: i32,
    streams: i32,
    coupled: i32,
    map: [u8; 8],
    nmap: usize,
    family: i32,
    app: i32,
    fs: i32,
    msx4: i32,
    br: i32,
    vbr: i32,
    comp: i32,
    dtx: i32,
    fec: i32,
    loss: i32,
    bw: i32,
    maxbw: i32,
    fch: i32,
    vbrc: i32,
    sig: i32,
    lsb: i32,
    pred: i32,
    et: i32,
    nf: i32,
}

static ROWS: &[&str] = &[
    "ex3-2s1c-vbr        3 2 1 0,2,1 -1 2048 48000 80 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-3s0c-audio      3 3 0 0,1,2 -1 2049 48000 80 64000 1 5 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-muted-ch2       3 2 0 0,1,255 -1 2048 48000 80 40000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex4-2s2c-quad       4 2 2 0,1,2,3 -1 2049 48000 80 64000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex4-4s0c-c0         4 4 0 0,1,2,3 -1 2048 48000 80 64000 1 0 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex4-3s1c            4 3 1 0,2,1,3 -1 2048 48000 80 56000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex5-3s2c            5 3 2 0,4,1,2,3 -1 2049 48000 80 80000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex5-4s1c            5 4 1 0,1,2,3,4 -1 2049 48000 80 80000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex6-4s2c            6 4 2 0,4,1,2,3,5 -1 2048 48000 80 64000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex7-4s3c            7 4 3 0,4,1,2,3,5,6 -1 2049 48000 80 96000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex8-5s3c            8 5 3 0,6,1,2,3,4,5,7 -1 2049 48000 80 128000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex8-4s4c            8 4 4 0,1,2,3,4,5,6,7 -1 2049 48000 80 128000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex2-stereo-explicit 2 1 1 0,1 -1 2049 48000 80 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-fs2.5           3 2 1 0,2,1 -1 2048 48000 10 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-fs5             3 2 1 0,2,1 -1 2048 48000 20 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-fs10            3 2 1 0,2,1 -1 2048 48000 40 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-fs40            3 2 1 0,2,1 -1 2048 48000 160 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-fs60            3 2 1 0,2,1 -1 2048 48000 240 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-8k              3 2 1 0,2,1 -1 2048 8000 80 16000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-12k             3 2 1 0,2,1 -1 2048 12000 80 24000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-16k             3 2 1 0,2,1 -1 2048 16000 80 32000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-24k             3 2 1 0,2,1 -1 2048 24000 80 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-audio           3 2 1 0,2,1 -1 2049 48000 80 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-lowdelay        3 2 1 0,2,1 -1 2051 48000 80 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-silk            3 2 1 0,2,1 -1 2052 48000 80 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-celt            3 2 1 0,2,1 -1 2053 48000 80 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-cbr             3 2 1 0,2,1 -1 2048 48000 80 48000 0 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-comp0           3 2 1 0,2,1 -1 2048 48000 80 48000 1 0 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-comp5           3 2 1 0,2,1 -1 2048 48000 80 48000 1 5 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-dtx             3 2 1 0,2,1 -1 2048 48000 80 48000 1 10 1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 8",
    "ex3-fec-loss        3 2 1 0,2,1 -1 2048 48000 80 48000 1 10 -1 1 10 0 0 -9999 -1 -2000 0 -1 1 8",
    "ex3-bw-wb           3 2 1 0,2,1 -1 2048 48000 80 48000 1 10 -1 -1 -1 1103 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-maxbw-nb        3 2 1 0,2,1 -1 2048 48000 80 48000 1 10 -1 -1 -1 0 1101 -9999 -1 -2000 0 -1 1 6",
    "ex3-fch1            3 2 1 0,2,1 -1 2048 48000 80 48000 1 10 -1 -1 -1 0 0 1 -1 -2000 0 -1 1 6",
    "ex3-vbrc0           3 2 1 0,2,1 -1 2048 48000 80 48000 1 10 -1 -1 -1 0 0 -9999 0 -2000 0 -1 1 6",
    "ex3-sig-voice       3 2 1 0,2,1 -1 2048 48000 80 48000 1 10 -1 -1 -1 0 0 -9999 -1 3001 0 -1 1 6",
    "ex3-sig-music       3 2 1 0,2,1 -1 2048 48000 80 48000 1 10 -1 -1 -1 0 0 -9999 -1 3002 0 -1 1 6",
    "ex3-lsb8            3 2 1 0,2,1 -1 2048 48000 80 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 8 -1 1 6",
    "ex3-lsb24           3 2 1 0,2,1 -1 2048 48000 80 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 24 -1 1 6",
    "ex3-pred1           3 2 1 0,2,1 -1 2048 48000 80 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 1 1 6",
    "ex3-br-auto         3 2 1 0,2,1 -1 2048 48000 80 -1000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-br-max          3 2 1 0,2,1 -1 2048 48000 80 -1 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-br-low          3 2 1 0,2,1 -1 2048 48000 80 1000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-br-high         3 2 1 0,2,1 -1 2048 48000 80 1000000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "sur6-5.1-voip-64k   6 0 0 0,0,0,0,0,0 1 2048 48000 80 64000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "sur8-7.1-audio-96k  8 0 0 0,0,0,0,0,0,0,0 1 2049 48000 80 96000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "sur5-5.0-audio-80k  5 0 0 0,0,0,0,0 1 2049 48000 80 80000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "sur3-1d-voip        3 0 0 0,0,0 1 2048 48000 80 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "sur7-6.1-audio      7 0 0 0,0,0,0,0,0,0 1 2049 48000 80 96000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "sur4-quad-audio     4 0 0 0,0,0,0 1 2049 48000 80 64000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "sur6-5.1-dtx-fec    6 0 0 0,0,0,0,0,0 1 2048 48000 80 64000 1 10 1 1 10 0 0 -9999 -1 -2000 0 -1 1 8",
    "sur6-5.1-cbr        6 0 0 0,0,0,0,0,0 1 2048 48000 80 64000 0 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "sur6-5.1-comp0      6 0 0 0,0,0,0,0,0 1 2048 48000 80 64000 1 0 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "sur6-5.1-fs10       6 0 0 0,0,0,0,0,0 1 2048 48000 40 64000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "sur6-5.1-fs40       6 0 0 0,0,0,0,0,0 1 2048 48000 160 64000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "sur6-5.1-8k         6 0 0 0,0,0,0,0,0 1 2048 8000 80 32000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "sur6-5.1-fbw        6 0 0 0,0,0,0,0,0 1 2048 48000 80 64000 1 10 -1 -1 -1 1105 0 -9999 -1 -2000 0 -1 1 6",
    "sur6-5.1-lowdelay   6 0 0 0,0,0,0,0,0 1 2051 48000 80 64000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "fam0-mono           1 0 0 0 0 2048 48000 80 32000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "fam0-stereo         2 0 0 0,1 0 2049 48000 80 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "fam1-mono           1 0 0 0 1 2048 48000 80 32000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "fam255-3mono        3 0 0 0,0,0 255 2049 48000 80 64000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "fam2-4ambi          4 0 0 0,0,0,0 2 2049 48000 80 64000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "fam2-6ambi          6 0 0 0,0,0,0,0,0 2 2049 48000 80 80000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 1 6",
    "ex3-short           3 2 1 0,2,1 -1 2048 48000 80 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 0 6",
    "ex3-int24           3 2 1 0,2,1 -1 2048 48000 80 48000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 2 6",
    "sur6-short          6 0 0 0,0,0,0,0,0 1 2048 48000 80 64000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 0 6",
    "sur6-int24          6 0 0 0,0,0,0,0,0 1 2048 48000 80 64000 1 10 -1 -1 -1 0 0 -9999 -1 -2000 0 -1 2 6",
];

fn parse_row(s: &str) -> Row {
    let mut it = s.split_whitespace();
    let tag = it.next().unwrap().to_string();
    let channels: i32 = it.next().unwrap().parse().unwrap();
    let streams: i32 = it.next().unwrap().parse().unwrap();
    let coupled: i32 = it.next().unwrap().parse().unwrap();
    let mapstr = it.next().unwrap();
    let mut map = [0u8; 8];
    let mut nmap = 0;
    for tok in mapstr.split(',') {
        if nmap < 8 {
            map[nmap] = tok.parse().unwrap();
            nmap += 1;
        }
    }
    let mut p = || it.next().unwrap().parse::<i32>().unwrap();
    let row = Row {
        tag,
        channels,
        streams,
        coupled,
        map,
        nmap,
        family: p(),
        app: p(),
        fs: p(),
        msx4: p(),
        br: p(),
        vbr: p(),
        comp: p(),
        dtx: p(),
        fec: p(),
        loss: p(),
        bw: p(),
        maxbw: p(),
        fch: p(),
        vbrc: p(),
        sig: p(),
        lsb: p(),
        pred: p(),
        et: p(),
        nf: p(),
    };
    row
}

/* ------------------------------------------------------------------ */
/* encode rows                                                        */
/* ------------------------------------------------------------------ */

fn apply_ctls(st: &mut OpusMSEncoder, r: &Row) {
    macro_rules! set {
        ($name:expr, $req:expr, $val:expr) => {{
            let mut v = $val;
            let shown = v;
            let ret = opus_multistream_encoder_ctl(st, $req, &mut v);
            println!("  set {} {} -> {}", $name, shown, ret);
        }};
    }
    if r.br != -999999 {
        set!("BITRATE", OPUS_SET_BITRATE_REQUEST, r.br);
    }
    if r.comp >= 0 {
        set!("COMPLEXITY", OPUS_SET_COMPLEXITY_REQUEST, r.comp);
    }
    if r.vbr >= 0 {
        set!("VBR", OPUS_SET_VBR_REQUEST, r.vbr);
    }
    if r.vbrc >= 0 {
        set!("VBR_CONSTRAINT", OPUS_SET_VBR_CONSTRAINT_REQUEST, r.vbrc);
    }
    if r.dtx >= 0 {
        set!("DTX", OPUS_SET_DTX_REQUEST, r.dtx);
    }
    if r.fec >= 0 {
        set!("INBAND_FEC", OPUS_SET_INBAND_FEC_REQUEST, r.fec);
    }
    if r.loss >= 0 {
        set!("PACKET_LOSS_PERC", OPUS_SET_PACKET_LOSS_PERC_REQUEST, r.loss);
    }
    if r.bw != 0 {
        set!("BANDWIDTH", OPUS_SET_BANDWIDTH_REQUEST, r.bw);
    }
    if r.maxbw != 0 {
        set!("MAX_BANDWIDTH", OPUS_SET_MAX_BANDWIDTH_REQUEST, r.maxbw);
    }
    if r.fch != -9999 {
        set!("FORCE_CHANNELS", OPUS_SET_FORCE_CHANNELS_REQUEST, r.fch);
    }
    if r.sig != -2000 {
        set!("SIGNAL", OPUS_SET_SIGNAL_REQUEST, r.sig);
    }
    if r.lsb != 0 {
        set!("LSB_DEPTH", OPUS_SET_LSB_DEPTH_REQUEST, r.lsb);
    }
    if r.pred >= 0 {
        set!("PREDICTION_DISABLED", OPUS_SET_PREDICTION_DISABLED_REQUEST, r.pred);
    }
}

fn print_get_summary(st: &mut OpusMSEncoder) {
    let mut br = 0;
    let mut vbr = 0;
    let mut comp = 0;
    let mut dtx = 0;
    let mut fec = 0;
    let mut loss = 0;
    let mut bw = 0;
    let mut maxbw = 0;
    let mut fc = 0;
    let mut vbrc = 0;
    let mut sig = 0;
    let mut lsb = 0;
    let mut look = 0;
    let mut fs = 0;
    let mut app = 0;
    let mut pred = 0;
    let mut phinv = 0;
    let mut expdur = 0;
    let mut range = 0;
    opus_multistream_encoder_ctl(st, OPUS_GET_BITRATE_REQUEST, &mut br);
    opus_multistream_encoder_ctl(st, OPUS_GET_VBR_REQUEST, &mut vbr);
    opus_multistream_encoder_ctl(st, OPUS_GET_COMPLEXITY_REQUEST, &mut comp);
    opus_multistream_encoder_ctl(st, OPUS_GET_DTX_REQUEST, &mut dtx);
    opus_multistream_encoder_ctl(st, OPUS_GET_INBAND_FEC_REQUEST, &mut fec);
    opus_multistream_encoder_ctl(st, OPUS_GET_PACKET_LOSS_PERC_REQUEST, &mut loss);
    opus_multistream_encoder_ctl(st, OPUS_GET_BANDWIDTH_REQUEST, &mut bw);
    opus_multistream_encoder_ctl(st, OPUS_GET_MAX_BANDWIDTH_REQUEST, &mut maxbw);
    opus_multistream_encoder_ctl(st, OPUS_GET_FORCE_CHANNELS_REQUEST, &mut fc);
    opus_multistream_encoder_ctl(st, OPUS_GET_VBR_CONSTRAINT_REQUEST, &mut vbrc);
    opus_multistream_encoder_ctl(st, OPUS_GET_SIGNAL_REQUEST, &mut sig);
    opus_multistream_encoder_ctl(st, OPUS_GET_LSB_DEPTH_REQUEST, &mut lsb);
    opus_multistream_encoder_ctl(st, OPUS_GET_LOOKAHEAD_REQUEST, &mut look);
    opus_multistream_encoder_ctl(st, OPUS_GET_SAMPLE_RATE_REQUEST, &mut fs);
    opus_multistream_encoder_ctl(st, OPUS_GET_APPLICATION_REQUEST, &mut app);
    opus_multistream_encoder_ctl(st, OPUS_GET_PREDICTION_DISABLED_REQUEST, &mut pred);
    opus_multistream_encoder_ctl(st, OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST, &mut phinv);
    opus_multistream_encoder_ctl(st, OPUS_GET_EXPERT_FRAME_DURATION_REQUEST, &mut expdur);
    opus_multistream_encoder_ctl(st, OPUS_GET_FINAL_RANGE_REQUEST, &mut range);
    println!(
        "  get br={} vbr={} comp={} dtx={} fec={} loss={} bw={} maxbw={} fc={} vbrc={} sig={} lsb={} look={} Fs={} app={} pred={} phinv={} expdur={} range={}",
        br, vbr, comp, dtx, fec, loss, bw, maxbw, fc, vbrc, sig, lsb, look, fs, app, pred, phinv, expdur, range
    );
    let mut indtx = 0;
    let ret = opus_multistream_encoder_ctl(st, OPUS_GET_IN_DTX_REQUEST, &mut indtx);
    println!("  get_indtx ret={}", ret);
}

fn run_enc_row(idx: i32, r: &Row) {
    let mut mapping = [0u8; 256];
    for i in 0..r.nmap {
        mapping[i] = r.map[i];
    }
    let mut streams = r.streams;
    let mut coupled = r.coupled;
    let mut err = 0;

    println!("enc {} {}", idx, r.tag);
    let st_opt = if r.family >= 0 {
        opus_multistream_surround_encoder_create(
            r.fs, r.channels, r.family, &mut streams, &mut coupled, &mut mapping, r.app, &mut err,
        )
    } else {
        opus_multistream_encoder_create(
            r.fs, r.channels, streams, coupled, &mapping, r.app, &mut err,
        )
    };
    print!("  create err={} streams={} coupled={} map=", err, streams, coupled);
    for i in 0..r.channels {
        print!("{},", mapping[i as usize]);
    }
    println!();
    let mut st = match st_opt {
        Some(s) => s,
        None => {
            println!("  create FAILED");
            panic!("{}: opus_multistream_encoder_create returned None (C succeeds)", r.tag);
        }
    };

    apply_ctls(&mut st, r);
    print_get_summary(&mut st);

    let mut derr = 0;
    let dec_opt = opus_multistream_decoder_create(
        r.fs, r.channels, streams, coupled, &mapping[..r.channels as usize], &mut derr,
    );
    println!("  dec_create err={}", derr);
    let mut dec = match dec_opt {
        Some(d) => d,
        None => {
            println!("  dec_create FAILED");
            opus_multistream_encoder_destroy(st);
            panic!("{}: opus_multistream_decoder_create returned None (C succeeds)", r.tag);
        }
    };

    let frame = r.fs * r.msx4 / 4000;
    let nsamp = (frame * r.channels) as usize;
    let mut sf = vec![0f32; nsamp];
    let mut si16 = vec![0i16; nsamp];
    let mut si32 = vec![0i32; nsamp];
    let mut dpcm = vec![0i16; nsamp];
    let mut rng = Rng {
        s: 0x1234567u32.wrapping_add(idx as u32 * 7919),
    };
    for f in 0..r.nf {
        let mut out = [0u8; 1500];
        gen_pcm_f32(&mut sf, f, frame, r.channels, &mut rng);
        let n = if r.et == 0 {
            for i in 0..nsamp {
                si16[i] = (sf[i] * 32768.0) as i16;
            }
            opus_multistream_encode(&mut st, &si16, frame, &mut out, 1500)
        } else if r.et == 1 {
            opus_multistream_encode_float(&mut st, &sf, frame, &mut out, 1500)
        } else {
            for i in 0..nsamp {
                si32[i] = (sf[i] * 8388608.0) as i32;
            }
            opus_multistream_encode24(&mut st, &si32, frame, &mut out, 1500)
        };
        let eh = fnv_bytes(&out[..n.max(0) as usize]);
        print!("  e {} len={} fnv={:016x} ", f, n, eh);
        if n > 0 {
            print_hex(&out[..n as usize]);
        }
        println!();
        if n <= 0 {
            continue;
        }
        let ret = opus_multistream_decode(&mut dec, Some(&out[..n as usize]), n, &mut dpcm, frame, 0);
        let dh = if ret > 0 {
            fnv_i16(&dpcm[..(ret * r.channels) as usize])
        } else {
            0
        };
        println!("  d {} ret={} fnv={:016x}", f, ret, dh);
    }
    opus_multistream_decoder_destroy(dec);
    opus_multistream_encoder_destroy(st);
}

fn section_encode() {
    println!("== enc ==");
    for (i, row) in ROWS.iter().enumerate() {
        let r = parse_row(row);
        run_enc_row(i as i32, &r);
    }
}

/* ------------------------------------------------------------------ */
/* sizes                                                              */
/* ------------------------------------------------------------------ */

fn section_sizes() {
    println!("== sizes ==");
    for s in 0..=6 {
        for c in -1..=3 {
            println!("encg {} {} -> {}", s, c, opus_multistream_encoder_get_size(s, c));
            println!("decg {} {} -> {}", s, c, opus_multistream_decoder_get_size(s, c));
        }
    }
    println!("encg 255 0 -> {}", opus_multistream_encoder_get_size(255, 0));
    println!("encg 200 100 -> {}", opus_multistream_encoder_get_size(200, 100));
    println!("decg 255 0 -> {}", opus_multistream_decoder_get_size(255, 0));
    println!("decg 200 100 -> {}", opus_multistream_decoder_get_size(200, 100));
    for ch in 0..=9 {
        for fam in [0, 1, 2, 255] {
            let mut streams = -1;
            let mut coupled = -1;
            let mut err = 0;
            let mut mapping = [0u8; 256];
            let size = opus_multistream_surround_encoder_get_size(ch, fam);
            let st = opus_multistream_surround_encoder_create(
                48000, ch, fam, &mut streams, &mut coupled, &mut mapping,
                OPUS_APPLICATION_AUDIO, &mut err,
            );
            print!(
                "sur {} {} -> size={} create_err={} streams={} coupled={} map=",
                ch, fam, size, err, streams, coupled
            );
            match st {
                Some(s) => {
                    for i in 0..ch.min(8) {
                        print!("{},", mapping[i as usize]);
                    }
                    opus_multistream_encoder_destroy(s);
                }
                None => print!("-"),
            }
            println!();
        }
    }
    println!("sur 6 99 -> size={}", opus_multistream_surround_encoder_get_size(6, 99));
}

/* ------------------------------------------------------------------ */
/* create errors                                                      */
/* ------------------------------------------------------------------ */

fn section_errors() {
    println!("== errors ==");

    let mut err = 0;
    let st = opus_multistream_encoder_create(48000, 1, 2, 0, &[], OPUS_APPLICATION_AUDIO, &mut err);
    println!("e_null_map_toomany_streams -> {} err={}", st.is_some() as i32, err);
    err = 0;
    let st = opus_multistream_encoder_create(48000, 3, 1, 2, &[], OPUS_APPLICATION_AUDIO, &mut err);
    println!("e_null_map_coupled_gt_streams -> {} err={}", st.is_some() as i32, err);
    err = 0;
    let st = opus_multistream_encoder_create(48000, 0, 1, 0, &[], OPUS_APPLICATION_AUDIO, &mut err);
    println!("e_ch0 -> {} err={}", st.is_some() as i32, err);
    err = 0;
    let st = opus_multistream_encoder_create(48000, 256, 1, 0, &[], OPUS_APPLICATION_AUDIO, &mut err);
    println!("e_ch256 -> {} err={}", st.is_some() as i32, err);
    err = 0;
    let st = opus_multistream_encoder_create(48000, 2, 0, 0, &[], OPUS_APPLICATION_AUDIO, &mut err);
    println!("e_streams0 -> {} err={}", st.is_some() as i32, err);
    err = 0;
    let st = opus_multistream_encoder_create(44100, 2, 1, 1, &[], OPUS_APPLICATION_AUDIO, &mut err);
    println!("e_fs -> {} err={}", st.is_some() as i32, err);
    err = 0;
    let st = opus_multistream_encoder_create(48000, 2, 1, 1, &[], 123, &mut err);
    println!("e_app -> {} err={}", st.is_some() as i32, err);
    err = 0;
    let st = opus_multistream_encoder_create(48000, 2, 200, 100, &[], OPUS_APPLICATION_AUDIO, &mut err);
    println!("e_streams255 -> {} err={}", st.is_some() as i32, err);
    if let Some(s) = st {
        opus_multistream_encoder_destroy(s);
    }

    let bad1 = [0u8, 3, 1];
    let bad2 = [0u8, 0, 1];
    err = 0;
    let st = opus_multistream_encoder_create(48000, 3, 2, 1, &bad1, OPUS_APPLICATION_AUDIO, &mut err);
    println!("e_map_range -> {} err={}", st.is_some() as i32, err);
    err = 0;
    let st = opus_multistream_encoder_create(48000, 3, 2, 1, &bad2, OPUS_APPLICATION_AUDIO, &mut err);
    println!("e_map_no_right -> {} err={}", st.is_some() as i32, err);

    let mut streams = 0;
    let mut coupled = 0;
    let mut mapping = [0u8; 256];
    err = 0;
    let st = opus_multistream_surround_encoder_create(
        48000, 3, 0, &mut streams, &mut coupled, &mut mapping, OPUS_APPLICATION_AUDIO, &mut err,
    );
    println!("e_fam0_3ch -> {} err={}", st.is_some() as i32, err);
    err = 0;
    let st = opus_multistream_surround_encoder_create(
        48000, 2, 99, &mut streams, &mut coupled, &mut mapping, OPUS_APPLICATION_AUDIO, &mut err,
    );
    println!("e_fam99 -> {} err={}", st.is_some() as i32, err);
    err = 0;
    let st = opus_multistream_surround_encoder_create(
        44100, 6, 1, &mut streams, &mut coupled, &mut mapping, OPUS_APPLICATION_AUDIO, &mut err,
    );
    println!("e_fam1_badfs -> {} err={}", st.is_some() as i32, err);
    err = 0;
    let st = opus_multistream_surround_encoder_create(
        48000, 5, 2, &mut streams, &mut coupled, &mut mapping, OPUS_APPLICATION_AUDIO, &mut err,
    );
    println!("e_fam2_5ch -> {} err={}", st.is_some() as i32, err);
    if let Some(s) = st {
        opus_multistream_encoder_destroy(s);
    }

    let map_stereo_err = [0u8, 1];
    err = 0;
    let dec = opus_multistream_decoder_create(48000, 1, 1, 2, &[], &mut err);
    println!("d_null_map_coupled_gt_streams -> {} err={}", dec.is_some() as i32, err);
    err = 0;
    let dec = opus_multistream_decoder_create(48000, 0, 1, 0, &[], &mut err);
    println!("d_ch0 -> {} err={}", dec.is_some() as i32, err);
    err = 0;
    let dec = opus_multistream_decoder_create(48000, 256, 1, 0, &[], &mut err);
    println!("d_ch256 -> {} err={}", dec.is_some() as i32, err);
    err = 0;
    let dec = opus_multistream_decoder_create(48000, 2, 0, 0, &[], &mut err);
    println!("d_streams0 -> {} err={}", dec.is_some() as i32, err);
    err = 0;
    let dec = opus_multistream_decoder_create(44100, 2, 1, 1, &map_stereo_err, &mut err);
    println!("d_fs -> {} err={}", dec.is_some() as i32, err);
    err = 0;
    let dec = opus_multistream_decoder_create(48000, 2, 200, 100, &[], &mut err);
    println!("d_streams255 -> {} err={}", dec.is_some() as i32, err);
    if let Some(d) = dec {
        opus_multistream_decoder_destroy(d);
    }

    let bad1 = [0u8, 3, 1];
    err = 0;
    let dec = opus_multistream_decoder_create(48000, 3, 2, 1, &bad1, &mut err);
    println!("d_map_range -> {} err={}", dec.is_some() as i32, err);
    if let Some(d) = dec {
        opus_multistream_decoder_destroy(d);
    }
}

/* ------------------------------------------------------------------ */
/* multistream ctl surface                                            */
/* ------------------------------------------------------------------ */

fn section_msctl() {
    let map31 = [0u8, 2, 1];
    println!("== msctl ==");
    let mut err = 0;
    let mut st = match opus_multistream_encoder_create(48000, 3, 2, 1, &map31, OPUS_APPLICATION_VOIP, &mut err) {
        Some(s) => s,
        None => {
            println!("create err={}", err);
            return;
        }
    };
    println!("create err={}", err);

    let mut v = -5;
    println!("set_bitrate_neg -> {}", opus_multistream_encoder_ctl(&mut st, OPUS_SET_BITRATE_REQUEST, &mut v));
    v = 100;
    println!("set_bitrate_low -> {}", opus_multistream_encoder_ctl(&mut st, OPUS_SET_BITRATE_REQUEST, &mut v));
    v = 100000000;
    println!("set_bitrate_high -> {}", opus_multistream_encoder_ctl(&mut st, OPUS_SET_BITRATE_REQUEST, &mut v));
    v = OPUS_AUTO;
    println!("set_bitrate_auto -> {}", opus_multistream_encoder_ctl(&mut st, OPUS_SET_BITRATE_REQUEST, &mut v));
    v = OPUS_BITRATE_MAX;
    println!("set_bitrate_max -> {}", opus_multistream_encoder_ctl(&mut st, OPUS_SET_BITRATE_REQUEST, &mut v));

    v = 0;
    let r = opus_multistream_encoder_ctl(&mut st, OPUS_GET_BITRATE_REQUEST, &mut v);
    println!("get_bitrate -> {} {}", r, v);

    v = 3;
    println!("set_complexity -> {}", opus_multistream_encoder_ctl(&mut st, OPUS_SET_COMPLEXITY_REQUEST, &mut v));
    v = 0;
    let r = opus_multistream_encoder_ctl(&mut st, OPUS_GET_COMPLEXITY_REQUEST, &mut v);
    println!("get_complexity -> {} {}", r, v);
    for s in 0..2 {
        let mut v2 = 0;
        if let Ok(sub) = st.get_encoder_state(s) {
            sub.ctl(OPUS_GET_COMPLEXITY_REQUEST, &mut v2);
            println!("sub {} complexity={} sr={}", s, v2, false as i32);
        }
    }
    println!(
        "get_state_oob -> {}",
        match st.get_encoder_state(2) {
            Ok(_) => 0,
            Err(e) => e,
        }
    );

    v = OPUS_FRAMESIZE_20_MS;
    println!(
        "set_expdur -> {}",
        opus_multistream_encoder_ctl(&mut st, OPUS_SET_EXPERT_FRAME_DURATION_REQUEST, &mut v)
    );
    v = 0;
    let r = opus_multistream_encoder_ctl(&mut st, OPUS_GET_EXPERT_FRAME_DURATION_REQUEST, &mut v);
    println!("get_expdur -> {} {}", r, v);
    v = 12345;
    println!(
        "set_expdur_bad -> {}",
        opus_multistream_encoder_ctl(&mut st, OPUS_SET_EXPERT_FRAME_DURATION_REQUEST, &mut v)
    );

    v = OPUS_APPLICATION_AUDIO;
    println!("set_application -> {}", opus_multistream_encoder_ctl(&mut st, OPUS_SET_APPLICATION_REQUEST, &mut v));

    v = 0;
    println!("unimplemented -> {}", opus_multistream_encoder_ctl(&mut st, 999999, &mut v));

    let mut rv = 0;
    println!("reset -> {}", opus_multistream_encoder_ctl(&mut st, OPUS_RESET_STATE, &mut rv));
    rv = 0;
    let r = opus_multistream_encoder_ctl(&mut st, OPUS_GET_FINAL_RANGE_REQUEST, &mut rv);
    println!("get_range_after_reset -> {} {}", r, rv);

    opus_multistream_encoder_destroy(st);
}

/* ------------------------------------------------------------------ */
/* decoder extras                                                     */
/* ------------------------------------------------------------------ */

fn make_encoder(r: &Row) -> (Option<Box<OpusMSEncoder>>, i32, i32, [u8; 256], i32) {
    let mut mapping = [0u8; 256];
    for i in 0..r.nmap {
        mapping[i] = r.map[i];
    }
    let mut streams = r.streams;
    let mut coupled = r.coupled;
    let mut err = 0;
    let st = if r.family >= 0 {
        opus_multistream_surround_encoder_create(
            r.fs, r.channels, r.family, &mut streams, &mut coupled, &mut mapping, r.app, &mut err,
        )
    } else {
        opus_multistream_encoder_create(r.fs, r.channels, streams, coupled, &mapping, r.app, &mut err)
    };
    (st, streams, coupled, mapping, err)
}

fn run_dec_row(idx: i32, r: &Row) {
    let (st_opt, streams, coupled, mapping, err) = make_encoder(r);
    print!(
        "dec {} {} create={} streams={} coupled={} map=",
        idx, r.tag, err, streams, coupled
    );
    for i in 0..r.channels {
        print!("{},", mapping[i as usize]);
    }
    println!();
    let mut st = match st_opt {
        Some(s) => s,
        None => panic!("{}: opus_multistream_encoder_create returned None (C succeeds)", r.tag),
    };

    let frame = r.fs * r.msx4 / 4000;
    let nsamp = (frame * r.channels) as usize;
    let mut sf = vec![0f32; nsamp];
    let mut d16 = vec![0i16; nsamp];
    let mut df = vec![0f32; nsamp];
    let mut d24 = vec![0i32; nsamp];
    let mut pkts = vec![0u8; r.nf as usize * 1500];
    let mut lens = vec![0i32; r.nf as usize];
    let mut rng = Rng {
        s: 0x1234567u32.wrapping_add(idx as u32 * 7919),
    };
    for f in 0..r.nf {
        gen_pcm_f32(&mut sf, f, frame, r.channels, &mut rng);
        lens[f as usize] = opus_multistream_encode_float(
            &mut st,
            &sf,
            frame,
            &mut pkts[(f as usize) * 1500..],
            1500,
        );
    }

    let mut derr = 0;
    let mut dec = opus_multistream_decoder_create(
        r.fs, r.channels, streams, coupled, &mapping[..r.channels as usize], &mut derr,
    )
    .expect("decoder");
    for f in 0..r.nf {
        let pkt = &pkts[(f as usize) * 1500..(f as usize) * 1500 + lens[f as usize] as usize];
        let ret = opus_multistream_decode(&mut dec, Some(pkt), lens[f as usize], &mut d16, frame, 0);
        let h16 = if ret > 0 { fnv_i16(&d16[..(ret * r.channels) as usize]) } else { 0 };
        let ret = opus_multistream_decode_float(&mut dec, Some(pkt), lens[f as usize], &mut df, frame, 0);
        let hf = if ret > 0 {
            let mut h: u64 = 1469598103934665603;
            for &x in &df[..(ret * r.channels) as usize] {
                for b in x.to_le_bytes() {
                    h ^= b as u64;
                    h = h.wrapping_mul(1099511628211);
                }
            }
            h
        } else {
            0
        };
        let ret = opus_multistream_decode24(&mut dec, Some(pkt), lens[f as usize], &mut d24, frame, 0);
        let h24 = if ret > 0 {
            let mut h: u64 = 1469598103934665603;
            for &x in &d24[..(ret * r.channels) as usize] {
                for b in x.to_le_bytes() {
                    h ^= b as u64;
                    h = h.wrapping_mul(1099511628211);
                }
            }
            h
        } else {
            0
        };
        println!("  plain {} ret={} h16={:016x} hf={:016x} h24={:016x}", f, ret, h16, hf, h24);
    }
    {
        let mut v = 0;
        opus_multistream_decoder_ctl(&mut dec, OPUS_GET_LAST_PACKET_DURATION_REQUEST, &mut v);
        println!("  ctl last_packet_duration={}", v);
        opus_multistream_decoder_ctl(&mut dec, OPUS_GET_FINAL_RANGE_REQUEST, &mut v);
        println!("  ctl final_range={}", v);
        opus_multistream_decoder_ctl(&mut dec, OPUS_GET_BANDWIDTH_REQUEST, &mut v);
        println!("  ctl bandwidth={}", v);
        opus_multistream_decoder_ctl(&mut dec, OPUS_GET_SAMPLE_RATE_REQUEST, &mut v);
        println!("  ctl sample_rate={}", v);
        opus_multistream_decoder_ctl(&mut dec, OPUS_GET_GAIN_REQUEST, &mut v);
        println!("  ctl gain={}", v);
        v = 5;
        opus_multistream_decoder_ctl(&mut dec, OPUS_SET_GAIN_REQUEST, &mut v);
        opus_multistream_decoder_ctl(&mut dec, OPUS_GET_GAIN_REQUEST, &mut v);
        println!("  ctl gain_after_set={}", v);
        opus_multistream_decoder_ctl(&mut dec, OPUS_GET_PITCH_REQUEST, &mut v);
        println!("  ctl pitch={}", v);
        v = 999;
        println!("  ctl set_gain_bad={}", opus_multistream_decoder_ctl(&mut dec, OPUS_SET_GAIN_REQUEST, &mut v));
        v = 3;
        opus_multistream_decoder_ctl(&mut dec, OPUS_SET_COMPLEXITY_REQUEST, &mut v);
        opus_multistream_decoder_ctl(&mut dec, OPUS_GET_COMPLEXITY_REQUEST, &mut v);
        println!("  ctl complexity={}", v);
        v = 1;
        opus_multistream_decoder_ctl(&mut dec, OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST, &mut v);
        opus_multistream_decoder_ctl(&mut dec, OPUS_GET_PHASE_INVERSION_DISABLED_REQUEST, &mut v);
        println!("  ctl phinv={}", v);
        v = 0;
        println!("  ctl unimplemented={}", opus_multistream_decoder_ctl(&mut dec, 999999, &mut v));
        opus_multistream_decoder_ctl(&mut dec, OPUS_RESET_STATE, &mut v);
        opus_multistream_decoder_ctl(&mut dec, OPUS_GET_LAST_PACKET_DURATION_REQUEST, &mut v);
        println!("  ctl last_packet_duration_after_reset={}", v);
        for s in 0..streams {
            let mut fr = 0;
            let r2 = match dec.get_decoder_state(s) {
                Ok(sub) => {
                    sub.ctl(OPUS_GET_FINAL_RANGE_REQUEST, &mut fr);
                    0
                }
                Err(e) => e,
            };
            println!("  state {} ret={} final_range={}", s, r2, fr);
        }
        println!(
            "  state_oob={}",
            match dec.get_decoder_state(streams) {
                Ok(_) => 0,
                Err(e) => e,
            }
        );
    }
    opus_multistream_decoder_destroy(dec);

    /* FEC recovery from the next packet */
    let mut dec = opus_multistream_decoder_create(
        r.fs, r.channels, streams, coupled, &mapping[..r.channels as usize], &mut derr,
    )
    .expect("decoder");
    for f in 0..r.nf {
        let pkt = &pkts[(f as usize) * 1500..(f as usize) * 1500 + lens[f as usize] as usize];
        let ret = opus_multistream_decode(&mut dec, Some(pkt), lens[f as usize], &mut d16, frame, 1);
        let h = if ret > 0 { fnv_i16(&d16[..(ret * r.channels) as usize]) } else { 0 };
        println!("  fec {} ret={} fnv={:016x}", f, ret, h);
    }
    opus_multistream_decoder_destroy(dec);

    /* packet-loss concealment */
    let mut dec = opus_multistream_decoder_create(
        r.fs, r.channels, streams, coupled, &mapping[..r.channels as usize], &mut derr,
    )
    .expect("decoder");
    for f in 0..r.nf {
        let ret = opus_multistream_decode(&mut dec, None, 0, &mut d16, frame, 0);
        let h = if ret > 0 { fnv_i16(&d16[..(ret * r.channels) as usize]) } else { 0 };
        println!("  plc {} ret={} fnv={:016x}", f, ret, h);
    }
    opus_multistream_decoder_destroy(dec);

    opus_multistream_encoder_destroy(st);
}

fn section_decode() {
    let pick = [0usize, 10, 44, 45, 62];
    println!("== dec ==");
    for &i in pick.iter() {
        let r = parse_row(ROWS[i]);
        run_dec_row(i as i32, &r);
    }
}

/* ------------------------------------------------------------------ */
/* malformed packets / small buffers                                  */
/* ------------------------------------------------------------------ */

fn section_bad() {
    let map31 = [0u8, 2, 1];
    let map_stereo = [0u8, 1];
    let mut err = 0;
    println!("== bad ==");
    let mut st = match opus_multistream_encoder_create(48000, 3, 2, 1, &map31, OPUS_APPLICATION_VOIP, &mut err) {
        Some(s) => s,
        None => {
            println!("encoder create failed");
            return;
        }
    };
    let mut rng = Rng { s: 0x1234567 };
    let mut sf = vec![0f32; 960 * 3];
    gen_pcm_f32(&mut sf, 0, 960, 3, &mut rng);
    let mut out = [0u8; 1500];
    let n = opus_multistream_encode_float(&mut st, &sf, 960, &mut out, 1500);
    println!("base_len={}", n);

    let mut copy = [0u8; 1500];
    println!("enc max0={}", opus_multistream_encode_float(&mut st, &sf, 960, &mut copy, 0));
    println!("enc max2={}", opus_multistream_encode_float(&mut st, &sf, 960, &mut copy, 2));
    println!("enc max3={}", opus_multistream_encode_float(&mut st, &sf, 960, &mut copy, 3));
    println!("enc fs0={}", opus_multistream_encode_float(&mut st, &sf, 0, &mut copy, 1500));
    println!("enc fsneg={}", opus_multistream_encode_float(&mut st, &sf, -1, &mut copy, 1500));
    println!("enc fs3={}", opus_multistream_encode_float(&mut st, &sf, 3, &mut copy, 1500));
    opus_multistream_encoder_destroy(st);

    let mut dec = opus_multistream_decoder_create(48000, 3, 2, 1, &map31, &mut err).expect("decoder");
    let mut dpcm = vec![0i16; 960 * 3];
    println!("dec full={}", opus_multistream_decode(&mut dec, Some(&out[..n as usize]), n, &mut dpcm, 960, 0));
    println!("dec len-1={}", opus_multistream_decode(&mut dec, Some(&out[..(n - 1) as usize]), n - 1, &mut dpcm, 960, 0));
    println!("dec len1={}", opus_multistream_decode(&mut dec, Some(&out[..1]), 1, &mut dpcm, 960, 0));
    println!("dec len0={}", opus_multistream_decode(&mut dec, Some(&[]), 0, &mut dpcm, 960, 0));
    println!("dec nulllen0={}", opus_multistream_decode(&mut dec, None, 0, &mut dpcm, 960, 0));
    println!("dec len-1neg={}", opus_multistream_decode(&mut dec, Some(&out[..n as usize]), -1, &mut dpcm, 960, 0));
    println!("dec fec2={}", opus_multistream_decode(&mut dec, Some(&out[..n as usize]), n, &mut dpcm, 960, 2));
    println!("dec fs0={}", opus_multistream_decode(&mut dec, Some(&out[..n as usize]), n, &mut dpcm, 0, 0));
    println!("dec fs1={}", opus_multistream_decode(&mut dec, Some(&out[..n as usize]), n, &mut dpcm, 1, 0));
    println!("dec fsneg={}", opus_multistream_decode(&mut dec, Some(&out[..n as usize]), n, &mut dpcm, -5, 0));
    copy[..n as usize].copy_from_slice(&out[..n as usize]);
    copy[0] ^= 0xFF;
    println!("dec flip0={}", opus_multistream_decode(&mut dec, Some(&copy[..n as usize]), n, &mut dpcm, 960, 0));
    copy[..n as usize].copy_from_slice(&out[..n as usize]);
    copy[n as usize - 1] ^= 0x55;
    println!("dec fliplast={}", opus_multistream_decode(&mut dec, Some(&copy[..n as usize]), n, &mut dpcm, 960, 0));
    copy[..n as usize].copy_from_slice(&out[..n as usize]);
    copy[n as usize / 2] ^= 0x0F;
    println!("dec flipmid={}", opus_multistream_decode(&mut dec, Some(&copy[..n as usize]), n, &mut dpcm, 960, 0));
    opus_multistream_decoder_destroy(dec);

    {
        let mut err = 0;
        let mut st1 = opus_multistream_encoder_create(48000, 2, 1, 1, &map_stereo, OPUS_APPLICATION_VOIP, &mut err).unwrap();
        let mut rng = Rng { s: 0x7654321 };
        let mut sf = vec![0f32; 960 * 2];
        gen_pcm_f32(&mut sf, 0, 960, 2, &mut rng);
        let n = opus_multistream_encode_float(&mut st1, &sf, 960, &mut out, 1500);
        println!("stereo_len={}", n);
        let mut dec = opus_multistream_decoder_create(48000, 3, 2, 1, &map31, &mut err).unwrap();
        println!("mismatch={}", opus_multistream_decode(&mut dec, Some(&out[..n as usize]), n, &mut dpcm, 960, 0));
        opus_multistream_decoder_destroy(dec);
        let mut dec1 = opus_multistream_decoder_create(48000, 2, 1, 1, &map_stereo, &mut err).unwrap();
        for f in 0..3 {
            println!(
                "stereo_dec {}={}",
                f,
                opus_multistream_decode(&mut dec1, Some(&out[..n as usize]), n, &mut dpcm, 960, 0)
            );
        }
        opus_multistream_decoder_destroy(dec1);
        opus_multistream_encoder_destroy(st1);
    }
}

fn main() {
    section_sizes();
    section_errors();
    section_encode();
    section_msctl();
    section_decode();
    section_bad();
}
