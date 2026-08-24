//! bug-452: the ALSA `audio` backend (`src/codegen/builtins/audio/gen_alsa_*.rs`)
//! drives the dlopen'd libasound through a raw indirect `blr` (a `dlsym`'d function
//! pointer, via `emit_call_fnptr`) and then reads each call's result from
//! `abi::return_register()` — the aligned MFB-return bank. On x86-64 SysV that bank
//! is `rdi`, but a C function returns in `rax` (`c_return(0)`); a raw `blr` (unlike
//! the direct-`bl` `emit_external_call` path for `dlopen`/`dlsym`/`mmap`) does not
//! stage `mov rdi,rax`, so every libasound "result" read from `rdi` is the stale
//! first argument. `snd_pcm_open`/`readi`/`writei` and the device enumeration all
//! misbehave. On AArch64/RISC-V the argument and result banks coincide (`x0`), so
//! the identical shape runs correctly there and only x86-64 is broken.
//!
//! The fix reads every raw-`blr` result from `c_return(0)` (`rax`), byte-identical
//! on AArch64 (`x0`) and correct on x86-64. This inspects the `linux-x86_64`
//! lowering and asserts no external-`blr` result is consumed from the aligned bank
//! (`rdi`). Pinned to `linux-x86_64` — the only target the ALSA backend builds for
//! where the arg/result banks differ. The devices open no hardware in the test env,
//! so this codegen invariant is the committed guard (mirrors
//! `codegen_crypto_ec_c_return_x86_64.rs`; the mechanism is box-proven by bug-450).

mod common;
use common::{assert_no_aligned_bank_result_reads, build_ncode, temp_project};

// Exercises every `audio::*` entry point so each ALSA `AbiFunction` body is emitted.
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
fn audio_alsa_reads_external_call_results_from_c_return_on_x86_64() {
    let project = temp_project("codegen_audio_alsa_c_return", SOURCE);
    let ncode = build_ncode(&project, "linux-x86_64", "codegen_audio_alsa_c_return");
    assert_no_aligned_bank_result_reads(&ncode, "rdi", "_mfb_rt_audio_", 8);
}
