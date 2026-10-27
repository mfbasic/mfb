Probe confirmed: `play` on a 44.1 kHz stereo output returns without any error. I have everything I need. Here are the findings.

### 1. The MML language that `play` accepts is documented nowhere
UNIT:      man-pkg:audio
PAGE:      audio::play (package-wide gap)
CATEGORY:  coverage
CLAIM:     "A track is a string of space-separated tokens (notes A..G with accidentals/length/dots, R/P rests, O/</> octave, L length, T tempo, V volume, I <name> instrument, ( ) legato, [ ] staccato, { }<count> repeat)."
VERDICT:   missing
EVIDENCE:  `ls src/docs/man/` shows no audio or MML guide topic and `grep -rli "mml\|music macro\|instrument" src/docs/man/` prints nothing, so the `play` page is the only MML reference in the product. It never says which instrument names exist, what ranges the commands accept, or what the defaults are. The parser does: `src/codegen/builtins/audio/helper_mml_wave_code.rs` (`__audio_mmlWaveCode`) accepts exactly `sine`, `square`, `triangle`, `saw`, `noise` and anything else raises ErrInvalidArgument "unknown instrument"; `helper_mml_parse.rs` (`__audio_mmlParse`) starts each track at T120, O4, L4, V10, sine, and bounds T to 32..=255, O to 0..=6, L and P to 1..=64, V to 0..=10 (all via `__audio_mmlReqInt`, raising ErrInvalidArgument out of range), while `<`/`>` clamp at 0 and 6 instead of raising; `helper_mml_note.rs` (`__audio_mmlNote`) shows accidentals are `+`/`-` (not `#`), the length suffix is 1..=64, dots follow; `helper_mml_expand.rs` (`__audio_mmlExpand`) shows the repeat count follows the closing brace as `}3`, must be >= 1, and nests; `helper_mml_tokens.rs` splits on single spaces only, so `CEG` is one token and an error, and `C E G` is required. A developer who reads the example `"T180 O2 L8 I triangle C G"` has no way to discover `square`, `saw` or `noise`, nor that `C#` is rejected.
SUGGESTED: Add a short grammar block to the `play` description (or a new guide topic the overview links): the five instrument names; per-track defaults T120 O4 L4 V10 sine; the ranges T 32..=255, O 0..=6, L 1..=64, V 0..=10, with out-of-range raising ErrInvalidArgument and `<`/`>` clamping at 0 and 6; accidentals spelt `+`/`-`; the repeat form `{ C E G }3`; tokens separated by spaces so `CEG` is an error; and that tempo/length/octave/volume/instrument do not carry from one track to the next in the multi-track overload (`func_play.rs:BODY_TRACKS` comment).

### 2. `play`'s "must be 48 kHz mono" is a silent requirement, and the overview's type-safety promise implies otherwise
UNIT:      man-pkg:audio
PAGE:      audio::play, overview
CATEGORY:  overview-mismatch
CLAIM:     overview: "Direction is part of the type ... so passing the wrong stream is a compile error rather than a runtime one." play: "output must be an AudioOutput opened with sampleRate = 48000 and channels = 1."
VERDICT:   misleading
EVIDENCE:  Nothing checks the stream's format. `helper_play_samples.rs` (`__audio_playSamples`) just calls `audio::write(out, __audio_mmlEncode(samples))`; `grep -n -i "48000\|channels\|rate" func_play.rs helper_play_samples.rs` finds only prose and a comment. Probe `/tmp/plan-125-scratch/B-recheck/man-pkg-audio/proj/src/main.mfb` opened `audio::openOutput(44100, 2, 512)` and called `audio::play(out, "T240 O4 L32 I sine C")`; it printed `play on 44100/stereo returned without error` and exited 0. The play page lists ErrInvalidArgument but the reader cannot tell whether a wrong-format stream is one of the things that raises it. Reading the overview's "compile error" paragraph and then "must be" on the play page, a developer reasonably expects some guard.
SUGGESTED: On the play page, after "must be an AudioOutput opened with sampleRate = 48000 and channels = 1", add: "play does not check this: on any other stream the PCM is written as-is and sounds at the wrong pitch and speed (and, on a stereo stream, with left and right swapping every sample)." In the overview's render/play sentence, add "both are fixed at 48 kHz mono, and neither checks the stream it is written to."

### 3. Windows is a first-class platform in the overview but absent from the device pages
UNIT:      man-pkg:audio
PAGE:      audio::devices, types (AudioDevice), audio::write, audio::openOutput
CATEGORY:  consistency
CLAIM:     types: "canInput ... On Linux every hint reports TRUE." / "isDefaultInput ... Always FALSE on Linux." devices: "The id is a Core Audio device UID on macOS and an ALSA PCM hint name on Linux". write: "On macOS a tail too short ... on Linux an underrun bumps the audio::xruns counter".
VERDICT:   inconsistent
EVIDENCE:  The overview devotes two paragraphs to Windows (WASAPI, 0x88890004, exclusive-mode rates), yet every function and type page that lists per-platform behaviour names only macOS and Linux. `gen_windows_devices.rs` doc comment at the `lower_devices` emitter: "Every endpoint carries canInput/canOutput TRUE and both default flags FALSE", and `emit_device_record` at `can_input: DEV_ONE_OFF, ... is_default_input: DEV_ZERO_OFF, is_default_output: DEV_ZERO_OFF`; the id comes from `IMMDevice::GetId` (`com_call(SLOT_DEV_GET_ID...)` then `emit_string_from_wstr(symbol, "id", ...)`), and enumeration is `EnumAudioEndpoints(eAll, DEVICE_STATE_ACTIVE)`. So on Windows, as on Linux, a developer cannot find the default device through the flags and every device claims both directions, but the types page says "Always FALSE on Linux", which a Windows reader takes to mean Windows reports real defaults. My macOS probe shows the flags are meaningful there (`MacBook Pro Microphone in=TRUE out=FALSE defIn=TRUE`), so the omission matters.
SUGGESTED: types: "On Linux and Windows every device reports TRUE" and "Always FALSE on Linux and Windows; use the three-argument openInput/openOutput for the system default there." devices: "The id is a Core Audio device UID on macOS, an ALSA PCM hint name on Linux, and a WASAPI endpoint id on Windows." write: add one clause on Windows underrun behaviour (or say explicitly that only macOS and Linux differ), so the three platform lists agree in scope.

### 4. The overview promises a full-duplex pattern that no page demonstrates
UNIT:      man-pkg:audio
PAGE:      overview
CATEGORY:  coverage
CLAIM:     "full duplex means opening one stream of each direction and driving both from a single loop with audio::poll, audio::available, and the timed form of audio::read."
VERDICT:   missing
EVIDENCE:  `grep -n "openInput" all.txt` beside `openOutput` shows no example on any page that opens both directions; every example is single-direction (`available`, `poll`, `read` open only `mic`; `write`, `play`, `render`, `close`, `xruns` open only `out`). The overview names the three calls a duplex loop needs but the developer who arrives asking "how do I monitor my mic through the speakers" must invent the loop, including the non-obvious part the overview itself raises: how to keep the output fed while waiting on input without blocking in `audio::read`.
SUGGESTED: Put a minimal duplex loop on the `poll` or `available` page (open `mic` and `out`, loop: `IF audio::poll(mic, 0) THEN LET n = audio::available(mic); LET pcm = audio::read(mic, n, 0); IF len(pcm) > 0 THEN audio::write(out, pcm)`), and have the overview sentence point to it: "see the example on audio::poll."

### 5. Sibling open pages disagree on whether the device's capability flag is a precondition
UNIT:      man-pkg:audio
PAGE:      audio::openInput, audio::openOutput
CATEGORY:  consistency
CLAIM:     openOutput device: "from audio::devices() with canOutput set (four-argument form only)". openInput device: "from audio::devices() (four-argument form only)".
VERDICT:   inconsistent
EVIDENCE:  Rendered Parameters tables in `/tmp/plan-125-scratch/B-recheck/man-pkg-audio/all.txt` (OPENINPUT vs OPENOUTPUT). One sibling states a capability precondition and the other does not, and neither says what happens when it is violated (on macOS my probe shows "MacBook Pro Speakers in=FALSE", so passing it to openInput is a realistic mistake). The errors table for both lists ErrAudioDevice, which is presumably the outcome, but the reader cannot tell from the pair.
SUGGESTED: Make both say the same thing: "from audio::devices(); it should report canInput (canOutput) TRUE — a device that cannot capture (play back), or whose id no longer exists, raises ErrAudioDevice."
