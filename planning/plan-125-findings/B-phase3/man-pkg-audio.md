### 1. Backend-unavailable promise has documented exceptions
UNIT:      man-pkg:audio  
PAGE:      package overview  
CATEGORY:  overview-mismatch  
CLAIM:     “every `audio::` call there raises `ErrAudioUnavailable`.”  
VERDICT:   wrong  
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man audio` prints the claim. `src/codegen/builtins/audio/func_xruns.rs:DESC` explicitly says `audio::xruns` “never raises `ErrAudioUnavailable”; `func_render.rs:BODY`/`DESC` implement and describe `audio::render` without opening hardware or a stream. `gen_alsa_io.rs`’s `Query::Xruns` arm reads the stored counter without `emit_dlopen`.  
SUGGESTED: “On Linux, calls that need the ALSA backend raise `ErrAudioUnavailable` when `libasound.so.2` is unavailable. `audio::render` needs no device, and `audio::xruns` can still report its stored count.”

### 2. Overview denies mixing that `play` exposes
UNIT:      man-pkg:audio  
PAGE:      package overview  
CATEGORY:  overview-mismatch  
CLAIM:     “There is no audio file, container, codec, mixing, resampling, or channel-conversion API at any layer”  
VERDICT:   misleading  
EVIDENCE:  The overview is printed by `mfb man audio`. `src/codegen/builtins/audio/func_play.rs:BODY_TRACKS` accepts `List OF String` tracks and calls `__audio_mmlMix`; `helper_mml_mix.rs:__audio_mmlMix` sums several tracks with clamping. The rendered `audio::play` page likewise says it “mixes the tracks by summing.”  
SUGGESTED: “There is no general raw-PCM mixing, resampling, channel-conversion, codec, container, or audio-file API. `audio::play` does mix its MML tracks internally.”