# Package Linking

Package files are architecture-independent `.mfp` files. During executable
native linking, the backend reads installed package metadata and emits native
functions for reachable package exports.

Package export symbols are derived as:

```text
_mfb_pkg_<package>_<export>
```

Package-to-package calls resolve through package identity and ABI metadata, not
raw binary representation function ids. The linker must not assume package-local function ids
are globally unique.

When package binary representation uses runtime helpers, the final executable must include
the helper implementation and any platform imports required by that helper. This
is true even if the app package does not directly call the helper.
