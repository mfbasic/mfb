# Pipeline

Native executable output flows through these stages:

```text
IR project
  -> target NIR
  -> native plan
  -> native code plan
  -> encoded architecture image
  -> target linker
  -> executable file(s)
```

The target backend owns the pipeline. The linker is the final target-specific
stage: it takes an encoded image containing text, data, symbols, relocations,
and imports, patches relocations, emits the executable container format, and
writes the executable to disk.

The linker does not decide semantic language behavior. It only materializes the
symbols, imports, and relocations requested by earlier native lowering stages.
