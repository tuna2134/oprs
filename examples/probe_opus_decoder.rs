//! Rust twin of `compat/c/probe_opus_decoder.c`.
//!
//! Run with `bash compat/scripts/run-probe.sh opus_decoder`; the outputs must
//! be byte-identical.
//!
//! The C probe is the driver: it encodes deterministic packets with the
//! reference `opus_encode()` and writes a "record script" (the exact sequence
//! of decoder calls, packets inlined) to
//! `compat/out/opus_decoder_packets.bin`.  This probe reads that script and
//! replays it with the Rust port, printing the decoded PCM sample by sample
//! and the full decoder state after every operation, exactly like the C side.
//!
//! The layout line mirrors the C one.  `silk` is the reference
//! `sizeof(silk_decoder)` (112624), i.e. the value `opus_decoder_get_size()`
//! uses; the port's `silk_Get_Decoder_Size()` returns a smaller number because
//! the Rust `silk_decoder` drops the `ENABLE_OSCE_BWE` fields and widens three
//! pointer members to slices (see `SILK_DECODER_SIZE` in `src/opus/decoder.rs`
//! and the accompanying report).

use oprs::celt::celt_decoder::celt_decoder_get_size;
use oprs::ffi;
use oprs::opus::decoder::{
    opus_decoder_get_nb_samples, OpusDecoder, OPUS_DECODER_STRUCT_SIZE, SILK_DECODER_SIZE,
};
use oprs::opus::packet::opus_packet_has_lbrr;

const REC_INIT: u32 = 1;
const REC_DECODE: u32 = 2;
const REC_DECODE_F: u32 = 3;
const REC_DECODE24: u32 = 4;
const REC_LOSS: u32 = 5;
const REC_FEC: u32 = 6;
const REC_RESET: u32 = 7;
const REC_GAIN: u32 = 8;
const REC_COMPLEX: u32 = 9;
const REC_CTL: u32 = 10;
const REC_NBSAMPLES: u32 = 11;
const REC_LBRR: u32 = 12;
const REC_STATE: u32 = 13;
const REC_IGNORE: u32 = 14;
const REC_PHASE: u32 = 15;

// ---- printing (must match the C probe bit for bit) ------------------------

fn print_i16(tag: &str, p: &[i16], n: usize) {
    print!("{} n={}:", tag, n);
    for i in 0..n {
        if i % 16 == 0 {
            print!("\n  {:06}", i);
        }
        print!(" {:04x}", p[i] as u16);
    }
    println!();
}

fn print_f32(tag: &str, p: &[f32], n: usize) {
    print!("{} n={}:", tag, n);
    for i in 0..n {
        if i % 16 == 0 {
            print!("\n  {:06}", i);
        }
        print!(" {:08x}", p[i].to_bits());
    }
    println!();
}

fn print_i32(tag: &str, p: &[i32], n: usize) {
    print!("{} n={}:", tag, n);
    for i in 0..n {
        if i % 16 == 0 {
            print!("\n  {:06}", i);
        }
        print!(" {:08x}", p[i] as u32);
    }
    println!();
}

fn print_state(st: &OpusDecoder) {
    println!(
        "state: ch={} Fs={} gain={} cx={} iext={} arch={} \
         | nci={} isr={} psms={} ppitch={} deep={} osce={} bwe={} oext={} poext={} \
         | stream={} bw={} mode={} prevmode={} fsz={} prevred={} lpd={} range={:08x} sc0={:08x} sc1={:08x}",
        st.channels,
        st.Fs,
        st.decode_gain,
        st.complexity,
        st.ignore_extensions,
        st.arch,
        st.DecControl.nChannelsInternal,
        st.DecControl.internalSampleRate,
        st.DecControl.payloadSize_ms,
        st.DecControl.prevPitchLag,
        st.DecControl.enable_deep_plc,
        st.DecControl.osce_method,
        st.enable_osce_bwe,
        st.osce_extended_mode,
        st.prev_osce_extended_mode,
        st.stream_channels,
        st.bandwidth,
        st.mode,
        st.prev_mode,
        st.frame_size,
        st.prev_redundancy,
        st.last_packet_duration,
        st.rangeFinal,
        st.softclip_mem[0].to_bits(),
        st.softclip_mem[1].to_bits(),
    );
}

// ---- record reader --------------------------------------------------------

struct Reader {
    buf: Vec<u8>,
    pos: usize,
}

impl Reader {
    fn u32(&mut self) -> u32 {
        let v = u32::from_le_bytes(self.buf[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        v
    }

    fn bytes(&mut self, n: usize) -> &[u8] {
        let v = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        v
    }
}

fn main() {
    let path = std::env::var("OPUS_DEC_PROBE_FILE")
        .unwrap_or_else(|_| "compat/out/opus_decoder_packets.bin".to_string());
    let buf = std::fs::read(&path).unwrap_or_else(|e| {
        eprintln!("probe: cannot read {}: {}", path, e);
        std::process::exit(2);
    });
    assert_eq!(&buf[0..8], b"OPDECD01", "bad record script magic");

    println!(
        "layout: sizeof_opusdecoder={} get_size_1={} get_size_2={} silk_ref={} celt_1={} celt_2={}",
        OPUS_DECODER_STRUCT_SIZE,
        OpusDecoder::get_size(1),
        OpusDecoder::get_size(2),
        SILK_DECODER_SIZE,
        celt_decoder_get_size(1),
        celt_decoder_get_size(2)
    );

    let mut rd = Reader {
        buf,
        pos: 8, /* magic, then the record count */
    };
    let mut dec: Option<Box<OpusDecoder>> = None;
    let mut cur_ch = 1i32;
    let mut case_no = 0i32;

    let ipcm_cap = 5760 * 2;
    let mut ipcm = vec![0i16; ipcm_cap];
    let mut fpcm = vec![0f32; ipcm_cap];
    let mut i32pcm = vec![0i32; ipcm_cap];

    let nrec = rd.u32();
    for _ in 0..nrec {
        let t = rd.u32();
        match t {
            REC_INIT => {
                let fs = rd.u32() as i32;
                let ch = rd.u32() as i32;
                let mut err = 0i32;
                dec = OpusDecoder::create(fs, ch, &mut err);
                cur_ch = ch;
                println!(
                    "== case {} fs={} ch={} create_ret={} size={}",
                    case_no,
                    fs,
                    ch,
                    err,
                    OpusDecoder::get_size(ch)
                );
                case_no += 1;
            }
            REC_DECODE | REC_DECODE_F | REC_DECODE24 | REC_FEC => {
                let len = rd.u32() as i32;
                let fs = rd.u32() as i32;
                let pkt = rd.bytes(len.max(0) as usize).to_vec();
                let st = dec.as_mut().unwrap();
                let data = if len == 0 { None } else { Some(&pkt[..]) };
                match t {
                    REC_DECODE => {
                        let ret = st.decode(data, len, &mut ipcm, fs, 0);
                        println!("op decode len={} fs={} ret={}", len, fs, ret);
                        print_i16("pcm", &ipcm, if ret > 0 { (ret * cur_ch) as usize } else { 0 });
                    }
                    REC_DECODE_F => {
                        let ret = st.decode_float(data, len, &mut fpcm, fs, 0);
                        println!("op decode_float len={} fs={} ret={}", len, fs, ret);
                        print_f32("pcmf", &fpcm, if ret > 0 { (ret * cur_ch) as usize } else { 0 });
                    }
                    REC_DECODE24 => {
                        let ret = st.decode24(data, len, &mut i32pcm, fs, 0);
                        println!("op decode24 len={} fs={} ret={}", len, fs, ret);
                        print_i32("pcm24", &i32pcm, if ret > 0 { (ret * cur_ch) as usize } else { 0 });
                    }
                    _ => {
                        let ret = st.decode(data, len, &mut ipcm, fs, 1);
                        println!("op decode_fec len={} fs={} ret={}", len, fs, ret);
                        print_i16("pcm", &ipcm, if ret > 0 { (ret * cur_ch) as usize } else { 0 });
                    }
                }
            }
            REC_LOSS => {
                let fs = rd.u32() as i32;
                let st = dec.as_mut().unwrap();
                let ret = st.decode(None, 0, &mut ipcm, fs, 0);
                println!("op loss fs={} ret={}", fs, ret);
                print_i16("pcm", &ipcm, if ret > 0 { (ret * cur_ch) as usize } else { 0 });
            }
            REC_RESET => {
                let mut v = 0i32;
                let ret = dec.as_mut().unwrap().ctl(ffi::OPUS_RESET_STATE, &mut v);
                println!("op reset ret={}", ret);
            }
            REC_GAIN => {
                let v = rd.u32() as i32;
                let ret = dec
                    .as_mut()
                    .unwrap()
                    .ctl(ffi::OPUS_SET_GAIN_REQUEST, &mut { v });
                println!("op set_gain {} ret={}", v, ret);
            }
            REC_COMPLEX => {
                let v = rd.u32() as i32;
                let ret = dec
                    .as_mut()
                    .unwrap()
                    .ctl(ffi::OPUS_SET_COMPLEXITY_REQUEST, &mut { v });
                println!("op set_complexity {} ret={}", v, ret);
            }
            REC_PHASE => {
                let v = rd.u32() as i32;
                let ret = dec
                    .as_mut()
                    .unwrap()
                    .ctl(ffi::OPUS_SET_PHASE_INVERSION_DISABLED_REQUEST, &mut { v });
                println!("op set_phase_inv {} ret={}", v, ret);
            }
            REC_IGNORE => {
                let v = rd.u32() as i32;
                let ret = dec
                    .as_mut()
                    .unwrap()
                    .ctl(ffi::OPUS_SET_IGNORE_EXTENSIONS_REQUEST, &mut { v });
                println!("op set_ignore_ext {} ret={}", v, ret);
            }
            REC_CTL => {
                let req = rd.u32() as i32;
                let mut v = 0i32;
                let ret = dec.as_mut().unwrap().ctl(req, &mut v);
                println!("op ctl req={} ret={} val={}", req, ret, v);
            }
            REC_NBSAMPLES => {
                let len = rd.u32() as i32;
                let pkt = rd.bytes(len.max(0) as usize).to_vec();
                let ret = opus_decoder_get_nb_samples(dec.as_ref().unwrap(), &pkt, len);
                println!("op nb_samples len={} ret={}", len, ret);
            }
            REC_LBRR => {
                let len = rd.u32() as i32;
                let pkt = rd.bytes(len.max(0) as usize).to_vec();
                let ret = opus_packet_has_lbrr(&pkt, len);
                println!("op has_lbrr len={} ret={}", len, ret);
            }
            REC_STATE => print_state(dec.as_ref().unwrap()),
            other => {
                eprintln!("probe: bad record type {}", other);
                std::process::exit(2);
            }
        }
    }
}
