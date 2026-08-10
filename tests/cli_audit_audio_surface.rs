//! bug-278: the hand-maintained audit tables in `src/audit/collect/source.rs`
//! (`builtin_capability`, `is_fallible_call`/`is_fallible_builtin`,
//! `resource_producer`) lagged the actual builtin surface, so a project using
//! `audio` audited as completely empty — 0 permissions, 0 resources, 0 findings,
//! no fallible flow. The tables were re-synced; `audio` now discloses its
//! capability, its `AudioOutput` resource, and its fallible calls.
//!
//! This drives `mfb audit` on an audio-using project and asserts those rows are
//! present in the report — they vanish the moment the `audio` entries are dropped
//! from the audit tables.

use std::process::Command;

mod common;
use common::*;

const SOURCE: &str = "IMPORT audio\n\n\
FUNC main AS Integer\n\
  RES out AS AudioOutput = audio::openOutput(48000, 2, 512)\n\
  audio::close(out)\n\
  RETURN 0\n\
END FUNC\n";

#[test]
fn audit_discloses_audio_capability_resource_and_fallibility() {
    let project = temp_project("bug278_audit_audio", SOURCE);

    let output = Command::new(mfb_exe())
        .arg("audit")
        .arg(&project)
        .output()
        .expect("run mfb audit");
    assert!(
        output.status.success(),
        "mfb audit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Capability disclosure (was entirely absent pre-fix).
    assert!(
        stdout.contains("AUDIT-PERM-AUDIO"),
        "audit did not disclose the audio capability finding (bug-278):\n{stdout}"
    );
    assert!(
        stdout.contains("audio.openOutput"),
        "audit did not list the audio.openOutput permission site (bug-278):\n{stdout}"
    );
    // Resource disclosure: AudioOutput closed by audio.closeOutput.
    assert!(
        stdout.contains("AudioOutput") && stdout.contains("audio.closeOutput"),
        "audit did not disclose the AudioOutput resource / close op (bug-278):\n{stdout}"
    );
    // Fallible control-flow disclosure.
    assert!(
        stdout.contains("fallible call audio.openOutput"),
        "audit did not mark audio.openOutput as a fallible call (bug-278):\n{stdout}"
    );

    let _ = std::fs::remove_dir_all(&project);
}
