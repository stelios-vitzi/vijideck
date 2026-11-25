# Third-party notices

VijiDeck's MIT License covers its original material only. Dependencies and
vendored assets retain their own licenses; the root LICENSE does not relicense
those components.

## Devicon

Language and ecosystem logos in `assets/dev-tools/` are vendored, unmodified,
from [Devicon v2.17.0](https://github.com/devicons/devicon/tree/v2.17.0/icons).
Their MIT license and copyright notice are in
[assets/dev-tools/LICENSE.devicon](assets/dev-tools/LICENSE.devicon).
Logo trademarks belong to their respective owners. `build.svg`, `packages.svg`,
and `tools.svg` are original VijiDeck illustrations.

## Geist fonts

`Geist-Regular`, `Geist-Light`, `Geist-Medium`, `GeistMono-Regular`, and
`GeistMono-Light` in `assets/fonts/` are vendored, unmodified, from the
[`geist` npm package v1.3.1](https://www.npmjs.com/package/geist) and embedded in
the binary as the interface typeface. Copyright (c) 2023 Vercel, in collaboration
with basement.studio. They are licensed under the SIL Open Font License 1.1, whose
full text is in [assets/fonts/OFL.txt](assets/fonts/OFL.txt).

## Rust and native dependencies

`Cargo.lock` and `xtask/Cargo.lock` record resolved Rust dependencies. Cargo
metadata and each upstream package's license files identify their terms.
Dependencies include egui/eframe, sysinfo, netstat2, rusqlite and bundled SQLite,
and platform integration libraries. This document is not a complete transitive
dependency license inventory. Before distributing binaries, complete the license
and notice audit in [the release checklist](docs/releasing.md), including native
and bundled dependencies, and ship any additional required notices or sources.
