//! Rust twin of `compat/c/probe_celt_decoder.c`.
//!
//! Run with `bash compat/scripts/run-probe.sh celt_decoder`; the outputs must
//! be byte-identical.
//!
//! The C probe is the driver: it encodes deterministic packets with the
//! reference `celt_encode_with_ec` and writes a "record script" (containing
//! both the decoder configuration and the packets) to
//! `compat/out/celt_decoder_packets.bin`.  This probe reads that script and
//! replays the *decode* half with the Rust `celt_decode_with_ec`, printing the
//! decoded PCM (raw IEEE-754 bits + FNV-1a hash) and the full decoder state
//! after every frame, exactly like the C side.

use oprs::celt::celt_decoder::{
    celt_decoder_ctl, celt_decoder_init, celt_decode_with_ec, opus_custom_decode,
    opus_custom_decode24, opus_custom_decoder_get_size, opus_custom_decoder_init, CELTDecoder,
    DECODE_BUFFER_SIZE,
};
use oprs::celt::celt::{
    CELT_SET_CHANNELS_REQUEST, CELT_SET_END_BAND_REQUEST, CELT_SET_START_BAND_REQUEST,
};
use oprs::celt::celt_lpc::CELT_LPC_ORDER;
use oprs::celt::modes::opus_custom_mode_create;
use oprs::celt::modes::CELTMode;
use oprs::ffi;

const REC_INIT: u32 = 1;
const REC_FRAME: u32 = 2;
const REC_LOSS: u32 = 3;
const REC_RESET: u32 = 4;
const REC_ACCUM: u32 = 5;
const REC_ICUSTOM: u32 = 6;
const REC_KCUSTOM: u32 = 7;

// ---- hashing / printing (must match the C probe bit for bit) --------------

fn f2u(f: f32) -> u32 {
    f.to_bits()
}

fn fnv_add(mut h: u64, v: u32) -> u64 {
    for b in 0..4 {
        h ^= ((v >> (8 * b)) & 0xff) as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn fnv_f32(p: &[f32]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &v in p {
        h = fnv_add(h, f2u(v));
    }
    h
}

fn fnv_i16(p: &[i16]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &v in p {
        h = fnv_add(h, (v as u16) as u32);
    }
    h
}

fn fnv_i32(p: &[i32]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &v in p {
        h = fnv_add(h, v as u32);
    }
    h
}

fn acc_pcm(f: i32, k: i32) -> f32 {
    let mut x = (f as u32)
        .wrapping_mul(2654435761)
        .wrapping_add((k as u32).wrapping_mul(2246822519));
    x ^= x >> 15;
    x = x.wrapping_mul(2246822519);
    x ^= x >> 13;
    ((x & 0xFFFF) as f32) / 65536.0 - 0.5
}

fn print_pcm(pcm: &[f32], ret: i32, c: i32) {
    let n = if ret > 0 { (c * ret) as usize } else { 0 };
    print!("pcm n={} hash={:016x}:", n, fnv_f32(&pcm[..n]));
    if n <= 8 {
        for i in 0..n {
            print!(" {:08x}", f2u(pcm[i]));
        }
    } else {
        for i in 0..4 {
            print!(" {:08x}", f2u(pcm[i]));
        }
        print!(" ..");
        for i in n - 4..n {
            print!(" {:08x}", f2u(pcm[i]));
        }
    }
    println!();
}

fn print_state(st: &CELTDecoder) {
    let nbe = st.mode.nbEBands;
    let dmem_n = (st.channels * (DECODE_BUFFER_SIZE + st.mode.overlap)) as usize;
    println!(
        "  st: rng={} err={} lpi={} loss={} plc={} lft={} skip={} pf={} pp={} ppo={} \
         pg={:08x} pgo={:08x} pt={} pto={} prem={:08x},{:08x} plcf={} plcm={:08x} dwn={} \
         end={} strch={} ch={} comp={} inv={} sign={} ovl={}",
        st.rng,
        st.error,
        st.last_pitch_index,
        st.loss_duration,
        st.plc_duration,
        st.last_frame_type,
        st.skip_plc,
        st.prefilter_and_fold,
        st.postfilter_period,
        st.postfilter_period_old,
        f2u(st.postfilter_gain),
        f2u(st.postfilter_gain_old),
        st.postfilter_tapset,
        st.postfilter_tapset_old,
        f2u(st.preemph_memD[0]),
        f2u(st.preemph_memD[1]),
        st.plc_fill,
        f2u(st.plc_preemphasis_mem),
        st.downsample,
        st.end,
        st.stream_channels,
        st.channels,
        st.complexity,
        st.disable_inv,
        st.signalling,
        st.mode.overlap,
    );
    let n2 = (2 * nbe) as usize;
    print!("  oldE:");
    for i in 0..n2 {
        print!(" {:08x}", f2u(st.oldEBands[i]));
    }
    println!();
    print!("  oldL:");
    for i in 0..n2 {
        print!(" {:08x}", f2u(st.oldLogE[i]));
    }
    println!();
    print!("  oldL2:");
    for i in 0..n2 {
        print!(" {:08x}", f2u(st.oldLogE2[i]));
    }
    println!();
    println!(
        "  bg={:016x} lpc={:016x} dmem={:016x}",
        fnv_f32(&st.backgroundLogE[..n2]),
        fnv_f32(&st.lpc[..(st.channels * CELT_LPC_ORDER) as usize]),
        fnv_f32(&st.decode_mem[..dmem_n]),
    );
}

// ---- record reader --------------------------------------------------------

struct Reader {
    buf: Vec<u8>,
    pos: usize,
}

impl Reader {
    fn u32(&mut self) -> u32 {
        let v = u32::from_le_bytes([
            self.buf[self.pos],
            self.buf[self.pos + 1],
            self.buf[self.pos + 2],
            self.buf[self.pos + 3],
        ]);
        self.pos += 4;
        v
    }
    fn bytes(&mut self, n: usize) -> &[u8] {
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        s
    }
    fn eof(&self) -> bool {
        self.pos >= self.buf.len()
    }
}

fn make_decoder(fs: i32, channels: i32, custom: i32, custom_frame: i32) -> CELTDecoder {
    let mut st = CELTDecoder::new();
    if custom != 0 {
        let mut err = 0;
        let mode: Box<CELTMode> =
            opus_custom_mode_create(fs, custom_frame, &mut err).expect("custom mode create");
        assert_eq!(
            opus_custom_decoder_init(&mut st, mode, channels),
            ffi::OPUS_OK
        );
    } else {
        assert_eq!(celt_decoder_init(&mut st, fs, channels), ffi::OPUS_OK);
    }
    st
}

fn main() {
    let path = std::env::var("CELT_DEC_PROBE_FILE")
        .unwrap_or_else(|_| "compat/out/celt_decoder_packets.bin".to_string());
    let buf = std::fs::read(&path).unwrap_or_else(|e| {
        eprintln!("probe: cannot read {}: {}", path, e);
        std::process::exit(2);
    });
    assert_eq!(&buf[0..8], b"OPCELTD1", "bad record script magic");

    /* The two `layout:` lines are also emitted by the C probe. */
    {
        let mut err = 0;
        let mode = opus_custom_mode_create(48000, 960, &mut err).unwrap();
        println!(
            "layout: struct=1408 get48_1={} get48_2={}",
            opus_custom_decoder_get_size(&mode, 1),
            opus_custom_decoder_get_size(&mode, 2)
        );
        let cm = opus_custom_mode_create(16000, 320, &mut err).unwrap();
        println!(
            "layout: custom_nbe={} custom_ov={} getc_1={} getc_2={}",
            cm.nbEBands,
            cm.overlap,
            opus_custom_decoder_get_size(&cm, 1),
            opus_custom_decoder_get_size(&cm, 2)
        );
    }

    let mut rd = Reader {
        buf,
        pos: 8 + 4, /* magic + record count */
    };
    let mut st: Option<CELTDecoder> = None;
    let mut frame_size = 0i32;
    let mut channels = 0i32;
    let mut evi = 0i32;
    let mut fi = 0i32;
    let mut ci = 0i32;

    while !rd.eof() {
        let ty = rd.u32();
        match ty {
            REC_INIT => {
                let rci = rd.u32() as i32;
                let fs = rd.u32() as i32;
                let frame = rd.u32() as i32;
                let ch = rd.u32() as i32;
                let _enc_ch = rd.u32() as i32;
                let start = rd.u32() as i32;
                let end = rd.u32() as i32;
                let stream_channels = rd.u32() as i32;
                let nbytes = rd.u32() as i32;
                let custom = rd.u32() as i32;
                let custom_frame = rd.u32() as i32;
                println!(
                    "== case {} fs={} frame={} CC={} encCC={} C={} start={} end={} bytes={} custom={}",
                    rci, fs, frame, ch, _enc_ch, stream_channels, start, end, nbytes, custom
                );
                let mut d = make_decoder(fs, ch, custom, custom_frame);
                let mut v = stream_channels;
                celt_decoder_ctl(&mut d, CELT_SET_CHANNELS_REQUEST, &mut v);
                let mut v = start;
                celt_decoder_ctl(&mut d, CELT_SET_START_BAND_REQUEST, &mut v);
                let mut v = end;
                celt_decoder_ctl(&mut d, CELT_SET_END_BAND_REQUEST, &mut v);
                st = Some(d);
                frame_size = frame;
                channels = ch;
                evi = 0;
                fi = 0;
                ci = rci;
                continue;
            }
            REC_FRAME | REC_ACCUM | REC_ICUSTOM | REC_KCUSTOM => {
                let len = rd.u32() as i32;
                let packet = rd.bytes(len as usize).to_vec();
                let d = st.as_mut().unwrap();
                if ty == REC_ICUSTOM {
                    let mut p16 = vec![0i16; (channels * frame_size) as usize];
                    let ret =
                        opus_custom_decode(d, Some(&packet), len, &mut p16, frame_size);
                    let n = if ret > 0 { (channels * ret) as usize } else { 0 };
                    println!("f{} iret={}", fi, ret);
                    print!("ipcm n={} hash={:016x}:", n, fnv_i16(&p16[..n]));
                    if n <= 8 {
                        for q in 0..n {
                            print!(" {:08x}", (p16[q] as u32) & 0xffff);
                        }
                    } else {
                        for q in 0..4 {
                            print!(" {:08x}", (p16[q] as u32) & 0xffff);
                        }
                        print!(" ..");
                        for q in n - 4..n {
                            print!(" {:08x}", (p16[q] as u32) & 0xffff);
                        }
                    }
                    println!();
                    print_state(d);
                } else if ty == REC_KCUSTOM {
                    let mut p32 = vec![0i32; (channels * frame_size) as usize];
                    let ret =
                        opus_custom_decode24(d, Some(&packet), len, &mut p32, frame_size);
                    let n = if ret > 0 { (channels * ret) as usize } else { 0 };
                    println!("f{} kret={}", fi, ret);
                    print!("kpcm n={} hash={:016x}:", n, fnv_i32(&p32[..n]));
                    if n <= 8 {
                        for q in 0..n {
                            print!(" {:08x}", p32[q] as u32);
                        }
                    } else {
                        for q in 0..4 {
                            print!(" {:08x}", p32[q] as u32);
                        }
                        print!(" ..");
                        for q in n - 4..n {
                            print!(" {:08x}", p32[q] as u32);
                        }
                    }
                    println!();
                    print_state(d);
                } else {
                    let mut pcm = vec![0f32; (channels * frame_size) as usize];
                    let ret = if ty == REC_ACCUM {
                        for k in 0..(channels * frame_size) as i32 {
                            pcm[k as usize] = acc_pcm(ci * 37 + evi, k);
                        }
                        celt_decode_with_ec(d, Some(&packet), len, &mut pcm, frame_size, None, 1)
                    } else {
                        celt_decode_with_ec(d, Some(&packet), len, &mut pcm, frame_size, None, 0)
                    };
                    println!("f{} ret={}", fi, ret);
                    print_pcm(&pcm, ret, channels);
                    print_state(d);
                }
            }
            REC_LOSS => {
                let frame = rd.u32() as i32;
                let d = st.as_mut().unwrap();
                let mut pcm = vec![0f32; (channels * frame_size) as usize];
                let ret = celt_decode_with_ec(d, None, 0, &mut pcm, frame, None, 0);
                println!("f{} loss ret={}", fi, ret);
                print_pcm(&pcm, ret, channels);
                print_state(d);
            }
            REC_RESET => {
                let d = st.as_mut().unwrap();
                let mut v = 0;
                celt_decoder_ctl(d, ffi::OPUS_RESET_STATE, &mut v);
                println!("-- reset");
            }
            other => panic!("probe: bad record type {}", other),
        }
        evi += 1;
        fi += 1;
    }
}
