# oprs — 純Rust実装の Opus コーデック（audiopus 0.3.0 互換API）

`oprs` は **libopus 1.6.1 を C に依存せず Rust だけで実装した Opus コーデック**です。
公開APIは [`audiopus`](https://crates.io/crates/audiopus) 0.3.0-rc.0 と**ソース互換**で、
`audiopus` 向けに書かれたコードはそのままこのクレートに対して再コンパイルできます。

- **ビット一致**: インストール済みの libopus-dev 1.6.1 とパケット出力・復号PCMが完全一致
- **C ライブラリ不要**: リンクするものは何もありません（`[dependencies]` は空）
- **構成**: SILK（float解析系含む）＋ CELT ＋ Opusレイヤ ＋ パケット/repacketizer/softclip

現在の検証状況（`bash compat/scripts/run-acceptance.sh` で再現可能）:

| 検証 | 結果 |
|---|---|
| 16構成オラクル（システム libopus 1.6.1 との diff） | **BIT-EXACT**（パケットも復号PCMも一致） |
| C↔Rust プローブ | **25本すべてビット一致** |
| 復号PCM半分（16構成） | BIT-EXACT |
| multistream エンコーダ／デコーダ（8構成、CBR/DTX/FEC込み） | BIT-EXACT |
| `compat/apicompat`（audiopus と同一スイートを両クレートに適用） | 両方 all checks passed |
| `cargo test --lib --release` | 315 passed / 0 failed |

## 動作要件

- Rust **edition 2024**（rustc 1.85 以降）。開発・検証は rustc 1.98.1 で実施
- 依存クレートなし
- 検証スクリプトを使う場合のみ `clang`・`pkg-config`（システム libopus）・Python 3 が必要

## インストール

現時点で crates.io には未公開のため、パス指定で依存に加えます:

```toml
[dependencies]
oprs = { path = "../oprs" }
```

`audiopus` から乗り換える場合は、`use audiopus::...` を `use oprs::...` に置き換えるだけです
（`oprs` はフィーチャフラグを持たず、全モジュールが常に有効です）。

## クイックスタート（エンコード → デコード）

`audiopus` とまったく同じ形で使えます。`encode_float` / `decode` が `&self` / `&mut self` を取る
内部可変性の流儀も `audiopus` に合わせてあります。

そのまま実行できる実例を `examples/` に置いてあります（README のコードはこれと同じです）:

```bash
cargo run --release --example quickstart             # ステレオ encode → decode
cargo run --release --example multistream_quickstart # 5.1 surround multistream
```

```rust
use oprs::coder::{Decoder, Encoder};
use oprs::packet::Packet;
use oprs::{Application, Bandwidth, Bitrate, Channels, MutSignals, SampleRate, Signal};

fn main() -> oprs::Result<()> {
    let rate = SampleRate::Hz48000;
    let channels = Channels::Stereo;          // Mono = 1, Stereo = 2
    let frame_samples = 960;                  // 20 ms @ 48 kHz
    let frame_len = frame_samples * channels as usize;

    // ---- エンコーダ ----
    let mut enc = Encoder::new(rate, channels, Application::Audio)?;
    enc.set_bitrate(Bitrate::BitsPerSecond(64000))?;
    enc.set_complexity(10)?;
    enc.set_vbr(true)?;
    enc.set_vbr_constraint(false)?;           // CVBR
    enc.set_dtx(false)?;
    enc.set_inband_fec(true)?;                // FEC（LBRR）
    enc.set_packet_loss_perc(10)?;
    enc.set_bandwidth(Bandwidth::Auto)?;
    enc.set_signal(Signal::Auto)?;
    enc.set_lsb_depth(16)?;

    // 入力はインターリーブ f32（-1.0 〜 1.0）。i16 なら `encode` を使う
    let pcm: Vec<f32> = (0..frame_len)
        .map(|i| (i as f32 * 0.01).sin() * 0.5)
        .collect();

    let mut packet = vec![0u8; 1500];         // 最大パケット長
    let n = enc.encode_float(&pcm, &mut packet)?;
    packet.truncate(n);
    println!("encoded {n} bytes");

    // ---- デコーダ ----
    let mut dec = Decoder::new(rate, channels)?;
    let mut out = vec![0i16; frame_len];
    let pkt = Packet::try_from(packet.as_slice())?;
    let samples = dec.decode(Some(pkt), MutSignals::try_from(&mut out[..])?, false)?;
    println!("decoded {samples} samples/channel");

    // f32 が欲しいとき: dec.decode_float(Some(pkt), MutSignals::try_from(&mut out_f32[..])?, false)
    Ok(())
}
```

パケットロス時は `dec.decode(None, ..., false)`（PLCによる隠蔽）、
直前パケットからの FEC 復元は `dec.decode(Some(next_pkt), ..., true)` です。

## 公開APIの地図

| モジュール | 内容 |
|---|---|
| `oprs::coder` | `Encoder` / `Decoder` / `size()` / `GenericCtl`（`final_range`, `phase_inversion_disabled`, `sample_rate`, `reset_state`）— `audiopus::coder` 互換 |
| `oprs::packet` | `Packet` と `bandwidth` / `samples_per_frame` / `nb_samples` / `nb_channels` / `nb_frames` などのパケット解析 |
| `oprs::repacketizer` | `Repacketizer` と `packet_pad` / `packet_unpad` / `multistream_packet_pad` / `multistream_packet_unpad` |
| `oprs::softclip` | `SoftClip::new(channels).apply(MutSignals<f32>)` |
| `oprs::error` | `Error` / `ErrorCode` / `Result` |
| ルート | `Signal` / `Bitrate` / `SampleRate` / `Application` / `Channels` / `Bandwidth` / `MutSignals` / `version()` |

`encoder_ctl_request` / `set_encoder_ctl_request`（`audiopus` の汎用 ctl パススルー）も利用できます。
`version()` は追従対象と同じ `"libopus 1.6.1"` を返します。

## multistream（3ch以上／サラウンド）

`audiopus` 0.3.0-rc.0 には multistream の公開APIが存在しないため（`multistream` フィーチャは
コードを持たない空のフィーチャです）、これは **`oprs` の拡張**です。C の `opus_multistream_*` と
同じ形にしてあります。

```rust
use oprs::ffi;
use oprs::opus::multistream_decoder::OpusMSDecoder;
use oprs::opus::multistream_encoder::{
    opus_multistream_encode_float, opus_multistream_encoder_ctl,
    opus_multistream_surround_encoder_create,
};

let channels = 6;                       // 5.1
let frame = 960;                        // 20 ms @ 48 kHz
let mut mapping = [0u8; 256];           // create がストリーム→チャネル対応を書き込む
let mut streams = 0;
let mut coupled = 0;
let mut err = 0;

let mut enc = opus_multistream_surround_encoder_create(
    48000, channels, 1,                 // mapping_family = 1（surround）
    &mut streams, &mut coupled, &mut mapping,
    ffi::OPUS_APPLICATION_AUDIO, &mut err,
).expect("encoder create failed");

let mut bitrate = 128000;
assert_eq!(opus_multistream_encoder_ctl(&mut enc, ffi::OPUS_SET_BITRATE_REQUEST, &mut bitrate), 0);

// インターリーブ f32（無音だと数バイトのパケットになります）
let pcm: Vec<f32> = (0..frame * channels as usize)
    .map(|i| (i as f32 * 0.02).sin() * 0.3)
    .collect();
let mut packet = vec![0u8; 1500];
let n = opus_multistream_encode_float(&mut enc, &pcm, frame as i32, &mut packet, 1500);
assert!(n > 0);

let mut dec = OpusMSDecoder::create(
    48000, channels, streams, coupled, &mapping[..channels as usize], &mut err,
).expect("decoder create failed");
let mut out = vec![0i16; frame * channels as usize];
let samples = dec.decode(Some(&packet[..n as usize]), n, &mut out, frame as i32, 0);
assert!(samples > 0);
```

- 明示マッピングで作る場合は `opus_multistream_encoder_create(Fs, channels, streams, coupled_streams, &mapping, application, &mut err)`（`oprs::opus::multistream_encoder`）
- `mapping_family` は 0（Vorbis 順）・1（surround）・2（ambisonics）に対応し、マッピング表
  （`vorbis_mappings`）と ambisonics の検証（`validate_ambisonics`）も移植済みです
- ストリーム単位で内部状態を触りたい場合は `OpusMSEncoder::get_encoder_state(stream_id)` / `OpusMSDecoder::get_decoder_state(stream_id)`
- `get_size` 系（`opus_multistream_encoder_get_size` / `..._surround_encoder_get_size` / `opus_multistream_decoder_get_size`）も移植済みです
- 検証プローブは 68 行のエンコード構成表＋ `sizes`／`errors`／`msctl`／`dec`／`bad` の各セクションで、
  `probe_multistream: IDENTICAL (1754 lines)` を確認しています

## audiopus との差異・既知の制限

- **フィーチャフラグなし**: `audiopus` の `coder` / `packet` / `repacketizer` などのフィーチャは
  ありません（常に全部有効）。`default-features = false` を指定していた場合は記述を外してください。
- **multistream は拡張**: `audiopus` には存在しないため、上記の `opus_multistream_*` 形式の API を
  追加しています（`audiopus::coder::MultistreamEncoder` のような型はありません）。
- **`size()` の値**: `Decoder::size()` / `coder::size()` と multistream のデコーダ `get_size` は
  **検証に使っている参照ビルド（autotools 構成、`ENABLE_OSCE_BWE` あり）の値**を返します。
  インストール済みの meson ビルドは当該フィールドを持たないため数値が異なります
  （例: 1ch で参照 191,856 / システム 155,104）。コーデックの出力には影響しません。
- **未実装（スコープ外）**:
  - `dnn/` 以下のニューラル機能（**DRED / LPCNet 深層PLC / OSCE / LossGen / blind BWE**）。
    `dnn/` は約 47 万行（大半が学習済み重み）で、本移植では対象外です。
    デコーダ側の深層PLC・OSCE・blind BWE は `audiopus` の公開APIからは**到達できません**
    （`Decoder` に複雑度などの ctl パススルーが無いため）。
    ただし **DRED は `Encoder::set_encoder_ctl_request(ffi::OPUS_SET_DRED_DURATION_REQUEST, n)`
    で到達可能**で、有効にすると学習済み重みが無いため libopus と出力が一致しません。
  - `projection` API（`opus_projection_*` / `mapping_matrix.c`）: `audiopus` にも存在しません。
- **C参照実装側が非決定的な箇所**: SILK の `sLTP`/`sLTP_Q15` は C では未初期化の `VARDECL` で、
  フレーム途中でピッチラグが大きく上昇すると古いスタック値を読みます。これは参照実装の性質で、
  Rust 側では再現できません（プローブはラグを固定して決定化しています）。

### 意図的に C と異なるが、出力に影響しない点

いずれも「C 側が未定義動作になる、またはポインタを引数に取る」ため公開APIからは到達できない
箇所で、コーデックの出力（パケット／復号PCM）は一致します。

1. **状態の持ち方**: C は `opus_encoder_get_size()` バイトの生メモリブロブに内部状態を配置しますが、
   Rust では `Vec<OpusEncoder>` / `Vec<OpusDecoder>` と作業用 `Vec` を所有します。
   `get_size()` は C の計算式を再現しています（プローブと単体テストで確認）。
2. **`OPUS_MULTISTREAM_GET_ENCODER_STATE` / `GET_DECODER_STATE`**: ポインタ引数のため
   スカラな `ctl(request, &mut i32)` では表現できず、その request id に対しては `OPUS_UNIMPLEMENTED`
   を返します。代わりに型付きの `OpusMSEncoder::get_encoder_state(stream_id)` /
   `OpusMSDecoder::get_decoder_state(stream_id)` を提供しています。
3. **短すぎる／空のマッピング**（チャネル数・ストリーム数は妥当な場合）: C は `mapping[0..channels]`
   を無条件に読むため未定義動作（範囲外読み）になりますが、本実装は `OPUS_BAD_ARG` を返します。
4. **`OPUS_ALLOC_FAIL`** は Rust では到達不能（`Box` の確保は失敗しない）で、`create` の
   ドキュメントに明記しています。
5. **負の長さを返す `repacketizer_out_range`** は、C では出力ポインタが負方向に進む未定義動作ですが、
   本実装はその値をそのまま返します（バイト予約の不変条件により到達不能）。

## 性能

`audiopus`（ネイティブ libopus）と `oprs` を**同一のアプリケーションコード**で両方にリンクして比べる
A/B ベンチマークを `compat/bench` に用意しています（`cd compat/bench && cargo run --release --offline`）。
エンコード/デコード（SILK・hybrid・CELT の3経路、複数レート/フレーム長、VBR/CBR、複雑度、FEC/PLC）と
パケット解析・repacketizer・softclip を、同一入力・交互計測・中央値で比較し `oprs/audiopus` 比を
降順に出力します。

実測から分かっていること:

- **`RUSTFLAGS="-C target-cpu=native"` が効きます。** ホストCPU向けにビルドすると LLVM が AVX2/FMA 等で
  自動ベクトル化するため、エンコードが大きく速くなります（本リポジトリの環境で 48 kHz stereo 42倍→56倍、
  48 kHz mono VOIP 57倍→93倍 リアルタイム）。浮動小数点は既定で再結合されないため**ビット一致は保たれます**
  （`probe_celt_encoder`/`probe_celt`/`probe_vq`/`probe_kernels`/`probe_silkenc` で確認）。C 側はこの
  フラグの影響を受けないので比較の公平性も保たれます。
- **ヒープ確保の削減は速度に効きません。** CELT エンコーダの1フレームあたりの確保を 999 回（473 KB）から
  384 回（363 KB）に減らしてもスループットは誤差範囲でした。差は確保ではなく計算そのものに由来します。
- 参考値（48 kHz、complexity 10）: エンコード 37〜93倍、デコード 155〜4000倍 リアルタイム。
  2.5 ms フレームの低遅延（CELT-only）がエンコードでは最も軽くなります。

## 自分で検証する

`reference/` と `target/` は gitignore されているため、クローン直後は参照ビルドを作る必要があります
（クレート本体のビルドには不要です）:

```bash
bash compat/scripts/build-reference.sh     # reference/lib-ref/libopus.a を作成（clang が必要）

bash compat/scripts/run-acceptance.sh      # これ1本で全ゲート（推奨）
bash compat/scripts/run-oracle.sh          # 16構成オラクル vs システム libopus（受け入れ基準）
bash compat/scripts/run-oracle.sh --reference   # 同じものを参照ビルドに対して
bash compat/scripts/run-all-probes.sh      # C↔Rust プローブ 25本
bash compat/scripts/run-probe.sh celt      # 個別プローブ（例: celt）
bash compat/scripts/run-dec-half.sh        # 復号PCM半分（16構成）
bash compat/scripts/port-status.sh         # 移植状況（コンパイル対象100ファイル）
bash compat/scripts/multistream-enc-check.sh   # multistream エンコーダ（8構成）
bash compat/scripts/multistream-dec-check.sh   # multistream デコーダ（8構成）
bash compat/scripts/multistream-system-check.sh # 参照ビルド == システム libopus
```

`run-acceptance.sh` が実行するゲートは次のとおりです:
ビルド → 単体テスト → 全プローブ → 復号半分 → multistream ベースライン／エンコーダ／デコーダ →
**16構成オラクル diff（= 受け入れ基準）**。

## リポジトリ構成

| パス | 内容 |
|---|---|
| `src/celt/`, `src/silk/`, `src/opus/` | コーデック本体（libopus の C ファイル単位で対応） |
| `src/coder.rs`, `src/coder/{encoder,decoder}.rs`, `src/packet.rs`, `src/repacketizer.rs`, `src/softclip.rs`, `src/error.rs`, `src/types.rs`, `src/ffi.rs` | `audiopus` 互換の公開API層 |
| `compat/c/probe_*.c`, `examples/probe_*.rs` | C↔Rust ビット一致プローブ（25本） |
| `compat/rustoracle/`, `reference/tools/oracle.c` | 16構成の受け入れオラクル（Rust双子／C本体） |
| `compat/scripts/` | ビルド・検証・表生成スクリプト |
| `docs/progress.md` | 移植の経緯・検証記録（durableな作業ログ） |
| `docs/porting-conventions.md`, `docs/rtcd-kernels.md`, `docs/libopus-map.md` ほか | 移植規約・SIMDエミュレーション・Cファイル対応表 |

## ライセンス

移植元の libopus 1.6.1 は BSD-3-Clause で、**各ソースファイルは元の著作権表示
（CSIRO / Xiph.Org / Skype ほか）と BSD-3-Clause の条件文をそのまま保持**しています
（翻訳であることの注記を追加）。本リポジトリにはまだトップレベルの `LICENSE` ファイルが
無いため、配布する場合は libopus と同じ BSD-3-Clause の `LICENSE` を追加してください。
