# hello_world

The smallest MFBASIC program: prints "Hello World" in 38 languages and exits.
No input, no interaction — a build/run smoke test.

## Build and run

```sh
mfb build examples/hello_world
```

- macOS / Linux — `./examples/hello_world/build/hello_world.out`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/hello_world`,
  then run `examples/hello_world/build/hello_world.exe`

No dependencies beyond `io`.
