//! bug-452: the WASAPI `audio` backend (`src/codegen/builtins/audio/gen_windows*.rs`)
//! drives the Windows audio stack through direct IAT calls (`ole_call`, a `bl`) and
//! COM vtable calls (`com_call`, a `blr`), then reads each HRESULT/DWORD result from
//! `abi::return_register()` — the aligned MFB-return bank (`rcx` on Win64). But a
//! Win64 external call is NOT staged into that bank (the shared emitter skips the
//! `%retC`→aligned `mov` for the Windows target), so the result stays in `rax`
//! (`c_return(0)`). Each call sign-extends its result in place — `sxtw rcx, rcx` —
//! sign-extending the STALE first argument / `this` pointer, so every FAILED(hr)
//! check tests garbage. On AArch64 the arg/result banks coincide (`x0`), which is
//! why the CoreAudio backend runs correctly and only Win64 is broken.
//!
//! The fix reads the result from `c_return(0)` (`sxtw rcx, rax`), byte-identical on
//! AArch64 (`x0`) and correct on Win64. This inspects the `windows-x86_64` lowering
//! and asserts no `sxtw` sign-extends the aligned bank into itself. Pinned to
//! `windows-x86_64` — the only target the WASAPI backend builds for. No Windows
//! audio hardware runs in `cargo test`, so this codegen invariant is the committed
//! guard (mirrors `codegen_crypto_ec_c_return_x86_64.rs`; the mechanism is
//! box-proven by bug-450).

mod common;
use common::{assert_no_inplace_sxtw_of_aligned_bank, build_ncode, temp_project};

// Exercises every `audio::*` entry point so each WASAPI `AbiFunction` body is emitted.
const SOURCE: &str = "\
IMPORT io\n\
IMPORT collections\n\
IMPORT audio\n\
FUNC main() AS Integer\n\
  RES mic AS audio::AudioInput = audio::openInput(48000, 1, 512)\n\
  LET pcm = audio::read(mic, 256)\n\
  io::print(toString(len(pcm)))\n\
  LET pcm2 = audio::read(mic, 256, 100)\n\
  io::print(toString(len(pcm2)))\n\
  io::print(toString(audio::available(mic)))\n\
  io::print(toString(audio::poll(mic)))\n\
  io::print(toString(audio::poll(mic, 50)))\n\
  io::print(toString(audio::xruns(mic)))\n\
  LET devs = audio::devices()\n\
  io::print(toString(len(devs)))\n\
  LET dev = collections::get(devs, 0)\n\
  RES mic2 AS audio::AudioInput = audio::openInput(dev, 48000, 1, 512)\n\
  audio::close(mic2)\n\
  RES spk AS audio::AudioOutput = audio::openOutput(48000, 1, 512)\n\
  audio::write(spk, pcm)\n\
  io::print(toString(audio::available(spk)))\n\
  io::print(toString(audio::poll(spk)))\n\
  io::print(toString(audio::poll(spk, 50)))\n\
  io::print(toString(audio::xruns(spk)))\n\
  RES spk2 AS audio::AudioOutput = audio::openOutput(dev, 48000, 1, 512)\n\
  audio::close(spk2)\n\
  audio::close(mic)\n\
  audio::close(spk)\n\
  RETURN 0\n\
END FUNC\n";

#[test]
fn audio_wasapi_reads_external_call_results_from_c_return_on_win64() {
    let project = temp_project("codegen_audio_wasapi_c_return", SOURCE);
    let ncode = build_ncode(&project, "windows-x86_64", "codegen_audio_wasapi_c_return");
    assert_no_inplace_sxtw_of_aligned_bank(&ncode, "rcx", "_mfb_rt_audio_", 8);
}
