# Third-party notices

## Libraries

### GPUI

The interface is built with [GPUI](https://www.gpui.rs/), from the [Zed](https://zed.dev) project
(`gpui` 0.2.2 from crates.io), which is linked statically.

> Licensed under the Apache License, Version 2.0 (see `LICENSE-APACHE` in the `gpui` crate).

### Other Rust crates

The app statically links other open-source Rust crates, all under permissive licences (MIT,
Apache-2.0, BSD, Zlib, ISC, Unicode and similar). The exact crates and versions are listed in
`Cargo.lock`. A few are under other licences:

- `hexf-parse` (Linux build): CC0-1.0, a public domain dedication.
- `option-ext` (Linux build): MPL-2.0. Its source is published unchanged on crates.io.
- `tiny-keccak`: CC0-1.0. It is only used while compiling and is not part of the app.

## Bundled code

### Chakra Petch font

`third_party/fonts/chakra-petch/` contains Chakra Petch Regular and SemiBold, built into the app
with `include_bytes!`.

> Copyright 2018 The Chakra Petch Project Authors (https://github.com/m4rc1e/Chakra-Petch.git)
>
> Licensed under the SIL Open Font License, Version 1.1. See
> `third_party/fonts/chakra-petch/OFL.txt` for the full licence text.

## Protocol information (no code copied)

The camera command bytes in `src/protocol.rs` and `docs/PROTOCOL.md`
are facts learnt from these projects. Thank you to their authors:

- [lxman/obsbot-mcp](https://github.com/lxman/obsbot-mcp) (MIT)
- [mitchelloharawild/obsbot-tiny-2-control](https://github.com/mitchelloharawild/obsbot-tiny-2-control) (MIT)
- [cgevans/tiny2](https://github.com/cgevans/tiny2) (EUPL-1.2)
- [me-tony/nod](https://github.com/me-tony/nod)
- [samliddicott/meet4k](https://github.com/samliddicott/meet4k) (LGPL-2.1)
- [vampyren/obsbot4linux](https://github.com/vampyren/obsbot4linux) (EUPL-1.2), the inspiration for this project

## Not included

- The **OBSBOT SDK** (`libdev`) is proprietary, is not used by this app, and is never
  committed to this repository.
