//! Per-kernel timing harness for the opt-in `fast` build.
//!
//! Measures the CELT kernels that `--features fast` swaps for AVX2+FMA variants,
//! at sizes representative of the 16 oracle configurations, and prints ns/call.
//! Build and run the *same* source under both feature sets, alternating:
//!
//! ```text
//! cargo build --release --example kernel_ab
//! cargo build --release --features fast --example kernel_ab
//! ```
//!
//! The harness only calls the public `oprs::celt::kernels` entry points, so the
//! exact and fast builds exercise the dispatcher (and its run-time AVX2+FMA
//! guard) exactly as the codec does.  Kernels that are already 4-lane SSE in the
//! bit-exact build (`celt_inner_prod`, `dual_inner_prod`) or already 8-lane
//! AVX2+FMA (`celt_pitch_xcorr`) are included so the per-kernel gain (or the
//! absence of one) is visible rather than inferred.
//!
//! Timing method: `REPS` repetitions of a fixed iteration count, keep the
//! fastest, report ns per call.  The final checksum keeps every call live and
//! is printed so the two builds can be checked for sanity (it differs in the
//! fast build: that is the point of the feature).

use oprs::celt::kernels;
use std::time::Instant;

const REPS: usize = 7;

struct Lcg(u64);

impl Lcg {
    fn new() -> Self {
        Lcg(0x2545_F491_4F6C_DD1D)
    }
    fn next_f32(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let v = ((self.0 >> 40) as i32 - 8_388_608) as f32 / 8_388_608.0;
        v * 0.5
    }
}

fn fill(n: usize) -> Vec<f32> {
    let mut rng = Lcg::new();
    (0..n).map(|_| rng.next_f32()).collect()
}

/// Runs `f` `iters` times, `REPS` times, and returns the fastest ns/call.
fn time<F: FnMut() -> f32>(iters: usize, mut f: F) -> (f64, f32) {
    let mut best = f64::INFINITY;
    let mut check = 0.0f32;
    for _ in 0..REPS {
        let t0 = Instant::now();
        let mut acc = 0.0f32;
        for _ in 0..iters {
            acc += f();
        }
        let dt = t0.elapsed().as_secs_f64();
        check += acc;
        best = best.min(dt);
    }
    (best * 1e9 / iters as f64, check)
}

fn main() {
    println!(
        "kernel timing harness — build: {}",
        if cfg!(feature = "fast") {
            "fast (AVX2+FMA variants)"
        } else {
            "default (bit-exact bodies)"
        }
    );
    println!("{:<44} {:>12} {:>10}", "kernel (representative size)", "ns/call", "checksum");

    let mut report = |name: &str, iters: usize, f: &mut dyn FnMut() -> f32| {
        let (ns, check) = time(iters, f);
        println!("{name:<44} {ns:>12.2} {:>10.4}", check);
    };

    /* --- celt_inner_prod: band energy (176), pitch refine (480), frame (960) */
    let x960 = fill(960);
    let y960 = fill(960);
    {
        let (x, y) = (&x960, &y960);
        report("celt_inner_prod n=8", 4_000_000, &mut || {
            kernels::celt_inner_prod(x, y, 8)
        });
        report("celt_inner_prod n=16", 4_000_000, &mut || {
            kernels::celt_inner_prod(x, y, 16)
        });
        report("celt_inner_prod n=32", 2_000_000, &mut || {
            kernels::celt_inner_prod(x, y, 32)
        });
        report("celt_inner_prod n=64", 2_000_000, &mut || {
            kernels::celt_inner_prod(x, y, 64)
        });
        report("celt_inner_prod n=176", 1_000_000, &mut || {
            kernels::celt_inner_prod(x, y, 176)
        });
        report("celt_inner_prod n=480", 400_000, &mut || {
            kernels::celt_inner_prod(x, y, 480)
        });
        report("celt_inner_prod n=960", 200_000, &mut || {
            kernels::celt_inner_prod(x, y, 960)
        });
    }

    /* --- dual_inner_prod (remove_doubling, n = frame/2) --- */
    let y02 = fill(960);
    {
        let (x, y1, y2) = (&x960, &y960, &y02);
        report("dual_inner_prod n=480", 400_000, &mut || {
            let mut a = 0.0f32;
            let mut b = 0.0f32;
            kernels::dual_inner_prod(x, y1, y2, 480, &mut a, &mut b);
            a + b
        });
        report("dual_inner_prod n=960", 200_000, &mut || {
            let mut a = 0.0f32;
            let mut b = 0.0f32;
            kernels::dual_inner_prod(x, y1, y2, 960, &mut a, &mut b);
            a + b
        });
    }

    /* --- xcorr_kernel (celt_fir: ord taps, 4 output lags at a time) --- */
    let xx = fill(24);
    let yx = fill(24 + 3);
    {
        let (x, y) = (&xx, &yx);
        report("xcorr_kernel len=8", 4_000_000, &mut || {
            let mut sum = [0.0f32; 4];
            kernels::xcorr_kernel(x, &y[..11], &mut sum, 8);
            sum[0] + sum[1] + sum[2] + sum[3]
        });
        report("xcorr_kernel len=24", 2_000_000, &mut || {
            let mut sum = [0.0f32; 4];
            kernels::xcorr_kernel(x, y, &mut sum, 24);
            sum[0] + sum[1] + sum[2] + sum[3]
        });
    }

    /* --- celt_pitch_xcorr (coarse pitch search: len=120, 180 lags) --- */
    let xp = fill(120);
    let yp = fill(180 + 120 - 1 + 16);
    let mut xcorr = vec![0.0f32; 192];
    {
        let (x, y, out) = (&xp, &yp, &mut xcorr);
        report("celt_pitch_xcorr len=120 max_pitch=180", 50_000, &mut || {
            kernels::celt_pitch_xcorr(x, y, out, 120, 180);
            out[0] + out[179]
        });
    }

    /* --- comb_filter_const (N=960, T=120; unreachable on real packets but
           still exercised by the CELT comb filter in some paths) --- */
    let mut yc = vec![0.0f32; 960];
    let xc = fill(960 + 122 + 8);
    {
        let (x, y) = (&xc, &mut yc);
        report("comb_filter_const N=960 T=120", 100_000, &mut || {
            kernels::comb_filter_const(y, x, 120, 960, 0.5, 0.25, 0.125);
            y[0]
        });
    }

    /* --- op_pvq_search (encoder-only; K > N/2 selects the projection path,
           which is the part the FMA variant contracts) --- */
    let pvq_x = fill(64);
    let mut pvq_iy = vec![0i32; 64 + 3];
    {
        let (x, iy) = (&pvq_x, &mut pvq_iy);
        report("op_pvq_search N=64 K=48", 20_000, &mut || {
            kernels::op_pvq_search_sse2(x, iy, 48, 64, 4)
        });
    }
    let pvq_x32 = fill(32);
    let mut pvq_iy32 = vec![0i32; 32 + 3];
    {
        let (x, iy) = (&pvq_x32, &mut pvq_iy32);
        report("op_pvq_search N=32 K=20", 50_000, &mut || {
            kernels::op_pvq_search_sse2(x, iy, 20, 32, 4)
        });
    }
}
