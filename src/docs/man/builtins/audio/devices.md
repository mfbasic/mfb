# devices

Enumerate the audio devices the operating system reports.

## Synopsis

```
audio::devices() AS List OF AudioDevice
```

## Package

audio

## Imports

```
IMPORT audio
```

`audio` is a built-in package, so no manifest dependency is required. A program
that does not `IMPORT audio` gains no audio symbol and no dynamic-library
dependency. [[src/builtins/audio.rs:augmented_project]]

## Description

`audio::devices` takes no arguments and returns every audio device the host
reports, each as an `AudioDevice` record. Each record carries an opaque `id`, a
human-readable `name`, the `canInput`/`canOutput` capability flags, and the
`isDefaultInput`/`isDefaultOutput` flags marking the system defaults.
[[src/builtins/audio.rs:builtin_type_fields]]

The `id` is a Core Audio device UID on macOS and an ALSA PCM hint `NAME` on
Linux. It is opaque: pass it to `audio::openInput` or `audio::openOutput` to open
that specific device; never construct it. A device whose `id` no longer exists
when opened (unplugged between `devices()` and the open) raises `ErrAudioDevice`
from the open call, not from `devices`.

The records report no channel counts and no supported sample rates. Discover a
working rate/channel combination by attempting to open the device and handling
the error. On macOS the `canInput`/`canOutput` flags reflect the device's actual
input/output stream configuration; on Linux both flags are reported as `TRUE`
for every hint, and `isDefaultInput`/`isDefaultOutput` are always `FALSE`
because ALSA hints do not distinguish a system default. [[src/target/shared/code/audio/alsa.rs:lower_devices]]

macOS drives Core Audio directly; the returned list has exactly one record with
`isDefaultOutput` set when a default output exists, and likewise for
`isDefaultInput`. An empty device list on macOS raises `ErrAudioUnavailable`
rather than returning an empty list. [[src/target/shared/code/audio/macos.rs:lower_devices]]
Linux drives ALSA's `snd_device_name_hint` enumeration through a `libasound.so.2`
resolved at runtime with `dlopen`, so a binary that imports `audio` still starts
on a Linux host without alsa-lib and `devices` there raises `ErrAudioUnavailable`.
Unlike macOS, the Linux path returns a successful empty `List OF AudioDevice`
when ALSA reports no PCM hints; it does not raise. [[src/target/shared/code/audio/alsa.rs:lower_devices]]

## Parameters

(none) [[src/builtins/audio.rs:arity]]

## Return value

| Type | Description |
| --- | --- |
| `List OF AudioDevice` | Every audio device the host reports, in the order the operating system returns them. On macOS the list is always nonempty on success — an empty enumeration raises `ErrAudioUnavailable` instead; on Linux the list may be empty when ALSA reports no PCM hints. [[src/builtins/audio.rs:call_return_type_name]][[src/target/shared/code/audio/alsa.rs:lower_devices]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050017` | `ErrAudioUnavailable` | No audio device is present (macOS reports an empty device list), or, on Linux, `libasound.so.2` could not be resolved or the ALSA hint enumeration failed. [[src/target/shared/code/audio/macos.rs:lower_devices]][[src/target/shared/code/audio/alsa.rs:lower_devices]] |
| `77050018` | `ErrAudioDevice` | The Core Audio device-enumeration API failed while querying the device list, a device UID, or a device name (macOS only). [[src/target/shared/code/audio/macos.rs:lower_devices]] |
| `77010001` | `ErrOutOfMemory` | Allocation of the returned list, a device record, or a device name/id string failed. [[src/target/shared/code/audio/macos.rs:lower_devices]][[src/target/shared/code/audio/alsa.rs:lower_devices]] |

## Examples

List every device and mark its capabilities:

```
IMPORT audio
IMPORT io

SUB main()
  FOR EACH d IN audio::devices()
    io::print(d.name & " in=" & toString(d.canInput) & " out=" & toString(d.canOutput))
  NEXT
END SUB
```

Open the default output, or fall back to the first output-capable device:

```
IMPORT audio

SUB main()
  FOR EACH d IN audio::devices()
    IF d.isDefaultOutput THEN
      RES out AS AudioOutput = audio::openOutput(d, 48000, 2, 512)
      audio::close(out)
    END IF
  NEXT
END SUB
```

## See also

- `mfb man audio openInput`
- `mfb man audio openOutput`
- `mfb man audio types`
