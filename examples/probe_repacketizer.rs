//! Rust side of the packet-framing / repacketizer bit-exactness probe.
//!
//! Mirrors `compat/c/probe_repacketizer.c` operation for operation.  Run
//! `bash compat/scripts/run-probe.sh repacketizer` to build both sides and
//! diff their output.

use oprs::opus::packet::*;
use oprs::opus::repacketizer::*;

const MAXPKT: usize = 48;

struct Rng(u32);
impl Rng {
    fn rnd(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345);
        self.0
    }
}

fn print_hex(d: &[u8]) {
    for b in d {
        print!("{:02x}", b);
    }
}

fn enc_size(d: &mut [u8], size: i32) -> i32 {
    if size < 252 {
        d[0] = size as u8;
        1
    } else {
        d[0] = (252 + (size & 3)) as u8;
        d[1] = ((size - d[0] as i32) >> 2) as u8;
        2
    }
}

/// Build a packet from explicit framing parameters, exactly like the C
/// `build_packet()`.
fn build_packet(
    b: &mut [u8],
    toc: u8,
    nframes: i32,
    sizes: &[i32],
    code: i32,
    vbr: i32,
    pad_chunks: i32,
    pad_final: i32,
) -> i32 {
    let mut pos = 0usize;
    if code == 0 {
        b[pos] = toc & 0xFC;
        pos += 1;
        for j in 0..nframes as usize {
            for k in 0..sizes[j] {
                b[pos] = (sizes[j] * 7 + k) as u8;
                pos += 1;
            }
        }
    } else if code == 1 {
        b[pos] = (toc & 0xFC) | 1;
        pos += 1;
        for j in 0..nframes as usize {
            for k in 0..sizes[j] {
                b[pos] = (sizes[j] * 7 + k) as u8;
                pos += 1;
            }
        }
    } else if code == 2 {
        b[pos] = (toc & 0xFC) | 2;
        pos += 1;
        pos += enc_size(&mut b[pos..], sizes[0]) as usize;
        for j in 0..nframes as usize {
            for k in 0..sizes[j] {
                b[pos] = (sizes[j] * 7 + k) as u8;
                pos += 1;
            }
        }
    } else {
        let pad_bytes = 254 * pad_chunks + pad_final;
        let has_pad = pad_chunks > 0 || pad_final > 0;
        b[pos] = (toc & 0xFC) | 3;
        pos += 1;
        b[pos] = (nframes | (if vbr != 0 { 0x80 } else { 0 }) | (if has_pad { 0x40 } else { 0 }))
            as u8;
        pos += 1;
        for _j in 0..pad_chunks {
            b[pos] = 255;
            pos += 1;
        }
        if has_pad {
            b[pos] = pad_final as u8;
            pos += 1;
        }
        if vbr != 0 {
            for j in 0..nframes - 1 {
                pos += enc_size(&mut b[pos..], sizes[j as usize]) as usize;
            }
        }
        for j in 0..nframes as usize {
            for k in 0..sizes[j] {
                b[pos] = (sizes[j] * 7 + k) as u8;
                pos += 1;
            }
        }
        for _j in 0..pad_bytes {
            b[pos] = 0;
            pos += 1;
        }
    }
    pos as i32
}

fn section_corpus(rng: &mut Rng) {
    let mut pkts: Vec<Vec<u8>> = Vec::new();
    let mut pkt_len = vec![0i32; MAXPKT];
    let mut kind = vec![0i32; MAXPKT];

    println!("== corpus ==");
    for i in 0..MAXPKT {
        let mut b = vec![0u8; 512];
        let len;
        kind[i] = (rng.rnd() % 4) as i32;
        if kind[i] == 0 {
            let l = 1 + (rng.rnd() % 40) as i32;
            for j in 0..l as usize {
                b[j] = (rng.rnd() >> 16) as u8;
            }
            len = l;
        } else {
            let nframes = 1 + (rng.rnd() % 4) as i32;
            let toc = (rng.rnd() & 0xFC) as u8;
            let code;
            if nframes == 1 {
                code = 0;
            } else if nframes == 2 {
                code = 1 + (rng.rnd() % 2) as i32;
            } else {
                code = 3;
            }
            let vbr = (rng.rnd() % 2) as i32;
            let mut nb = (rng.rnd() % 3) as i32;
            let mut fin = (rng.rnd() % 255) as i32;
            let mut sizes = [0i32; 4];
            for j in 0..nframes as usize {
                sizes[j] = (rng.rnd() % 64) as i32;
            }
            if code == 1 {
                sizes[1] = sizes[0];
            }
            if vbr == 0 {
                for j in 0..nframes as usize {
                    sizes[j] = sizes[0];
                }
            }
            if code != 3 {
                nb = 0;
                fin = 0;
            }
            let mut l = build_packet(&mut b, toc, nframes, &sizes, code, vbr, nb, fin);
            if rng.rnd() % 3 == 0 {
                l = 1 + (rng.rnd() % l as u32) as i32;
            }
            if rng.rnd() % 4 == 0 {
                let idx = (rng.rnd() % l as u32) as usize;
                let x = (1 + rng.rnd() % 255) as u8;
                b[idx] ^= x;
            }
            len = l;
        }
        pkt_len[i] = len;
        pkts.push(b);
    }

    for i in 0..MAXPKT {
        let b = &pkts[i];
        let len = pkt_len[i];
        let p = &b[..len as usize];

        println!("pkt {} kind {} len {}", i, kind[i], len);
        println!(
            "  q {} {} {} {} {} {}",
            opus_packet_get_bandwidth(p),
            opus_packet_get_samples_per_frame(p, 48000),
            opus_packet_get_nb_channels(p),
            opus_packet_get_nb_frames(p, len),
            opus_packet_get_nb_samples(p, len, 48000),
            opus_packet_has_lbrr(p, len)
        );

        match opus_packet_parse_impl(p, len, false) {
            Ok(parsed) => {
                print!("  parse {}", parsed.count);
                if parsed.count > 0 {
                    print!(
                        " toc {:02x} poff {} pko {} padlen {}",
                        parsed.toc,
                        parsed.payload_offset,
                        parsed.packet_offset,
                        parsed.padding.len()
                    );
                    for j in 0..parsed.count as usize {
                        let off = parsed.frames[j].as_ptr() as usize - b.as_ptr() as usize;
                        print!(" {}:{}", off, parsed.size[j]);
                    }
                }
                println!();
            }
            Err(e) => println!("  parse {}", e),
        }

        let mut toc2 = 0u8;
        let mut frames2: [&[u8]; 48] = [&[]; 48];
        let mut size2 = [0i16; 48];
        let mut payload_offset2 = -999i32;
        let count2 = opus_packet_parse(
            p,
            len,
            &mut toc2,
            &mut frames2,
            &mut size2,
            &mut payload_offset2,
        );
        print!("  parse2 {}", count2);
        if count2 > 0 {
            print!(" toc {:02x} poff {}", toc2, payload_offset2);
        }
        println!();
    }
}

fn section_repacketizer(rng: &mut Rng) {
    println!("== repacketizer ==");
    for trial in 0..12 {
        let mut rp = OpusRepacketizer::default();
        rp.init();
        let mut store: Vec<Vec<u8>> = vec![Vec::new(); 48];
        let mut store_len = [0i32; 48];

        println!("rp {}", trial);
        let nf = 1 + (rng.rnd() % 48) as i32;
        let stereo = (rng.rnd() & 0x4) as u8;
        for f in 0..nf {
            let flen = (rng.rnd() % 16) as usize;
            let mut p = vec![0u8; 1 + flen];
            p[0] = 0x80 | stereo;
            for j in 0..flen {
                p[1 + j] = (rng.rnd() >> 16) as u8;
            }
            store_len[f as usize] = (1 + flen) as i32;
            let r = rp.cat(&p, store_len[f as usize]);
            println!("  cat {} {}", f, r);
            store[f as usize] = p;
        }
        let nfr = rp.get_nb_frames();
        println!("  rpnb {}", nfr);
        if nfr > 0 {
            let begin = (rng.rnd() % nfr as u32) as i32;
            let end = begin + 1 + (rng.rnd() % (nfr - begin) as u32) as i32;
            let maxlen = (rng.rnd() % 400) as i32;
            let mut out = vec![0u8; 4096];
            let r = rp.out_range(begin, end, &mut out, maxlen);
            print!("  out_range {} {} {} {} ", begin, end, maxlen, r);
            if r > 0 {
                print_hex(&out[..r as usize]);
            }
            println!();
            let maxlen = (rng.rnd() % 4096) as i32;
            let r = rp.out(&mut out, maxlen);
            print!("  out {} {} ", maxlen, r);
            if r > 0 {
                print_hex(&out[..r as usize]);
            }
            println!();
        }
    }
}

fn section_pad(rng: &mut Rng) {
    println!("== pad ==");
    for trial in 0..10 {
        println!("pad {}", trial);
        let nframes = 1 + (rng.rnd() % 4) as i32;
        let toc = (0x80 | (rng.rnd() & 0x4)) as u8;
        let vbr = (rng.rnd() % 2) as i32;
        let mut sizes = [0i32; 4];
        for j in 0..nframes as usize {
            sizes[j] = (rng.rnd() % 40) as i32;
        }
        if vbr == 0 {
            for j in 0..nframes as usize {
                sizes[j] = sizes[0];
            }
        }
        let mut pkt = vec![0u8; 512];
        let plen = build_packet(&mut pkt, toc, nframes, &sizes, 3, vbr, 0, 0);
        let newlen = plen + (rng.rnd() % 60) as i32;
        println!("  base {} {}", plen, newlen);

        let mut work = vec![0u8; 512];
        work[..plen as usize].copy_from_slice(&pkt[..plen as usize]);
        for j in plen as usize..newlen as usize {
            work[j] = 0xAA;
        }
        let r = opus_packet_pad(&mut work, plen, newlen);
        print!("  pad {} ", r);
        if r == 0 {
            print_hex(&work[..newlen as usize]);
        }
        println!();

        let r2 = opus_packet_unpad(&mut work, newlen);
        print!("  unpad {} ", r2);
        if r2 > 0 {
            print_hex(&work[..r2 as usize]);
        }
        println!();

        work[..plen as usize].copy_from_slice(&pkt[..plen as usize]);
        let r3 = opus_packet_pad(&mut work, plen, plen);
        let r4 = opus_packet_pad(&mut work, plen, plen - 1);
        println!("  edge {} {}", r3, r4);

        work[..plen as usize].copy_from_slice(&pkt[..plen as usize]);
        let _r = opus_packet_pad(&mut work, plen, newlen);
        let newlen2 = newlen + (rng.rnd() % 30) as i32;
        let r5 = opus_packet_pad(&mut work, newlen, newlen2);
        print!("  padpad {} {} ", newlen2, r5);
        if r5 == 0 {
            print_hex(&work[..newlen2 as usize]);
        }
        println!();
        let r6 = opus_packet_unpad(&mut work, newlen2);
        print!("  unpad2 {} ", r6);
        if r6 > 0 {
            print_hex(&work[..r6 as usize]);
        }
        println!();
    }
}

fn section_multistream(rng: &mut Rng) {
    println!("== multistream ==");
    for trial in 0..8 {
        println!("ms {}", trial);
        let nb_streams = 1 + (rng.rnd() % 3) as i32;
        let mut pkt = vec![0u8; 512];
        let mut len: i32 = 0;
        for s in 0..nb_streams {
            let flen = (rng.rnd() % 12) as i32;
            let toc = (0x80 | (rng.rnd() & 0x4)) as u8;
            pkt[len as usize] = toc;
            len += 1;
            if s != nb_streams - 1 {
                len += enc_size(&mut pkt[len as usize..], flen);
            }
            for _j in 0..flen {
                pkt[len as usize] = (rng.rnd() >> 16) as u8;
                len += 1;
            }
        }
        let newlen = len + (rng.rnd() % 40) as i32;
        println!("  base {} {} {}", nb_streams, len, newlen);

        let mut work = vec![0u8; 512];
        work[..len as usize].copy_from_slice(&pkt[..len as usize]);
        for j in len as usize..newlen as usize {
            work[j] = 0xAA;
        }
        let r = opus_multistream_packet_pad(&mut work, len, newlen, nb_streams);
        print!("  mspad {} ", r);
        if r == 0 {
            print_hex(&work[..newlen as usize]);
        }
        println!();

        let r2 = opus_multistream_packet_unpad(&mut work, newlen, nb_streams);
        print!("  msunpad {} ", r2);
        if r2 > 0 {
            print_hex(&work[..r2 as usize]);
        }
        println!();
    }
}

fn section_big(rng: &mut Rng) {
    println!("== big ==");
    for trial in 0..6 {
        let mut pkt = vec![0u8; 4096];
        let mut work = vec![0u8; 8192];
        let nframes = 2 + (rng.rnd() % 3) as i32;
        let toc = (0x80 | (rng.rnd() & 0x4)) as u8;
        let mut sizes = [0i32; 4];
        for j in 0..nframes as usize {
            sizes[j] = (rng.rnd() % 400) as i32;
        }
        let plen = build_packet(&mut pkt, toc, nframes, &sizes, 3, 1, 0, 0);
        println!("big {} {} {}", trial, nframes, plen);

        let mut rp = OpusRepacketizer::default();
        rp.init();
        let r = rp.cat(&pkt[..plen as usize], plen);
        println!("  cat {}", r);
        let maxlen = (rng.rnd() % 4096) as i32;
        let r = rp.out(&mut work, maxlen);
        print!("  out {} {} ", maxlen, r);
        if r > 0 {
            print_hex(&work[..r as usize]);
        }
        println!();

        let newlen = plen + (rng.rnd() % 100) as i32;
        work[..plen as usize].copy_from_slice(&pkt[..plen as usize]);
        for j in plen as usize..newlen as usize {
            work[j] = 0x55;
        }
        let r = opus_packet_pad(&mut work, plen, newlen);
        print!("  pad {} {} ", newlen, r);
        if r == 0 {
            print_hex(&work[..newlen as usize]);
        }
        println!();
        let r = opus_packet_unpad(&mut work, newlen);
        print!("  unpad {} ", r);
        if r > 0 {
            print_hex(&work[..r as usize]);
        }
        println!();
    }
}

fn section_extensions(rng: &mut Rng) {
    println!("== extensions ==");
    for t in 0..8 {
        let nb_frames = 1 + (rng.rnd() % 4) as i32;
        let ne = 1 + (rng.rnd() % 5) as i32;
        let mut ext: Vec<OpusExtensionData> = Vec::new();
        for _k in 0..ne {
            let id = 3 + (rng.rnd() % 125) as i32;
            let frame = (rng.rnd() % nb_frames as u32) as i32;
            let l = if id < 32 {
                (rng.rnd() % 2) as usize
            } else {
                (rng.rnd() % 300) as usize
            };
            let mut data = vec![0u8; l];
            for j in 0..l {
                data[j] = (rng.rnd() >> 16) as u8;
            }
            ext.push(OpusExtensionData {
                id,
                frame,
                data,
                len: l as i32,
            });
        }
        let genlen = opus_packet_extensions_generate(None, 100000, &ext, ne, nb_frames, 0);
        print!("ext {} {} {} {} ", t, nb_frames, ne, genlen);
        if genlen >= 0 {
            let mut ebuf = vec![0u8; genlen as usize];
            let g2 =
                opus_packet_extensions_generate(Some(&mut ebuf), genlen, &ext, ne, nb_frames, 0);
            print!("{} ", g2);
            print_hex(&ebuf);
            print!(
                " cnt {}",
                opus_packet_extensions_count(&ebuf, genlen, nb_frames)
            );
            let mut parsed = vec![OpusExtensionData::default(); 16];
            let mut cap = 16i32;
            let pr =
                opus_packet_extensions_parse(&ebuf, genlen, &mut parsed, &mut cap, nb_frames);
            print!(" parse {} {}", pr, cap);
            if pr >= 0 {
                for k in 0..cap as usize {
                    print!(
                        " ({},{},{}:",
                        parsed[k].id, parsed[k].frame, parsed[k].len
                    );
                    print_hex(&parsed[k].data[..parsed[k].len as usize]);
                    print!(")");
                }
            }
        }
        println!();
    }
}

fn section_softclip(rng: &mut Rng) {
    println!("== softclip ==");
    for t in 0..8 {
        let c = 1 + (rng.rnd() % 2) as i32;
        let n = 1 + (rng.rnd() % 16) as i32;
        let mut pcm = vec![0f32; (n * c) as usize];
        for i in 0..(n * c) as usize {
            pcm[i] = ((rng.rnd() % 2001) as i32 - 1000) as f32 / 300.0f32;
        }
        let mut mem = vec![0f32; c as usize];
        for i in 0..c as usize {
            mem[i] = ((rng.rnd() % 2001) as i32 - 1000) as f32 / 1000.0f32;
        }
        opus_pcm_soft_clip(&mut pcm, n, c, &mut mem);
        print!("clip {} {} {} ", t, n, c);
        for i in 0..(n * c) as usize {
            print!("{:08x}", pcm[i].to_bits());
        }
        print!(" | ");
        for i in 0..c as usize {
            print!("{:08x}", mem[i].to_bits());
        }
        println!();
    }
}

fn main() {
    let mut rng = Rng(0x12345678);
    section_corpus(&mut rng);
    section_repacketizer(&mut rng);
    section_pad(&mut rng);
    section_multistream(&mut rng);
    section_big(&mut rng);
    section_extensions(&mut rng);
    section_softclip(&mut rng);
}
