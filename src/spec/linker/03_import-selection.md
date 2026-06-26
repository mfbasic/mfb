# Import And Library Selection

Library decisions are made before final linking, primarily in the target native
plan and target codegen platform.

The native plan collects imports from:

- Program entry support, such as process exit and entry error output.
- Built-in runtime helpers used by the app package.
- Built-in runtime helpers used only inside package binary representation.
- Native built-in calls, such as math functions lowered to platform libraries.
- Platform runtime requirements, such as thread creation.

Each import has:

```text
library
symbol
requiredBy
```

`library` is the target library name that must appear in the final executable's
dynamic dependency metadata. `symbol` is the dynamic symbol name used by codegen.
`requiredBy` records which function or runtime helper caused the import.

Runtime helper import selection is target-specific. For example:

- `io::print` and `io::write` require a platform `write` function.
- `fs::readText` requires file functions such as `open`, `read`, `close`, and
  error access.
- `thread::start` requires a target thread creation primitive.
- `math::sin` requires the target math library symbol.

Package binary representation matters even when the app package does not directly call a
helper. If an imported package export uses `fs::readText`, `io::print`,
`thread::start`, or another runtime helper, the final executable must still
include the platform imports required by that package export.
