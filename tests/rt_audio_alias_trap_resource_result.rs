//! An inline `TRAP` on the device-argument `audio::openOutput`/`openInput`
//! overloads must compile. Those overloads are rewritten at IR level to their
//! runtime `os_alias` names (`audio.openOutputDevice`/`openInputDevice`,
//! `audio::runtime_overload_name`), which are not registry members — so the raw
//! trap path's return-type resolution fell through to the derived runtime spec,
//! whose ABI spelling **bares** a resource name (`abi_return_name`:
//! `audio.AudioOutput` → `AudioOutput`). The bare spelling is invisible to the
//! resource classification, so `materialize_current_result` flat-copied the
//! handle and the build died with "native inlined field size not available for
//! type 'AudioOutput'". The fix resolves the aliased implementation's own
//! package-qualified return type (`registry::alias_call_return_type`) before the
//! spec fallback. The default-device overloads (registry members, resolved
//! qualified all along) were never affected.
//!
//! Build-only: opening a device needs hardware, but the bug was a compile
//! error — a successful build is the regression proof.

mod common;
use common::{build_project, temp_project};

const SOURCE: &str = "\
IMPORT io\n\
IMPORT collections\n\
IMPORT audio\n\
FUNC main() AS Integer\n\
  LET devs = audio::devices()\n\
  LET dev = collections::get(devs, 0)\n\
  RES spk AS audio::AudioOutput = audio::openOutput(dev, 48000, 1, 512) TRAP(e)\n\
    io::print(\"no output device: \" & e.message)\n\
    RETURN 0\n\
  END TRAP\n\
  audio::close(spk)\n\
  RES mic AS audio::AudioInput = audio::openInput(dev, 48000, 1, 512) TRAP(e)\n\
    io::print(\"no input device: \" & e.message)\n\
    RETURN 0\n\
  END TRAP\n\
  audio::close(mic)\n\
  RETURN 0\n\
END FUNC\n";

#[test]
fn device_overload_open_with_inline_trap_compiles() {
    let project = temp_project("rt_audio_alias_trap_resource_result", SOURCE);
    build_project(&project);
}
