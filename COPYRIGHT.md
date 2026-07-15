# Copyright and licenses

Booklet's own source is covered by the repository's license. This file covers
the third-party files **bundled in this repository**.

## Bundled fonts

The typefaces in `src/booklet/fonts/` are third-party works, redistributed under
the **SIL Open Font License 1.1** (OFL-1.1). They are compiled into the binary as
a Qt resource by `build.rs` and loaded at startup (see `src/booklet/fonts.qrc`
and `Theme.qml`).

| Font | Files | Copyright | Source |
|---|---|---|---|
| EB Garamond | `EBGaramond.ttf` | Copyright 2017 The EB Garamond Project Authors | <https://github.com/octaviopardo/EBGaramond12> |
| Alegreya Sans | `AlegreyaSans-Regular.ttf`, `AlegreyaSans-Medium.ttf` | Copyright 2013 The Alegreya Sans Project Authors | <https://github.com/huertatipografica/Alegreya-Sans> |
| Spectral | `Spectral-Regular.ttf`, `Spectral-Italic.ttf`, `Spectral-Bold.ttf`, `Spectral-BoldItalic.ttf` | Copyright 2017 The Spectral Project Authors | <https://github.com/productiontype/Spectral> |
| JetBrains Mono | `JetBrainsMono.ttf` | Copyright 2020 The JetBrains Mono Project Authors | <https://github.com/JetBrains/JetBrainsMono> |

All four were obtained from the Google Fonts repository
(<https://github.com/google/fonts>).

The full license text for each family is kept alongside the fonts, as the OFL
requires:

    src/booklet/fonts/licenses/EBGaramond-OFL.txt
    src/booklet/fonts/licenses/AlegreyaSans-OFL.txt
    src/booklet/fonts/licenses/Spectral-OFL.txt
    src/booklet/fonts/licenses/JetBrainsMono-OFL.txt

Under the OFL these fonts may be bundled and redistributed, including with
commercial software. They must remain under the OFL, must ship with the license
text above, and must not be sold on their own. Reserved Font Names, where a
family declares them, prevent redistributing a *modified* version under the same
name — Booklet ships them unmodified.

## Qt and qtbridge

Booklet links against **Qt 6** and uses **Qt Bridges for Rust** (`qtbridge`).
These are not vendored in this repository; they are resolved at build time from
your Qt installation and from crates.io. Qt is available under the LGPL-3.0 or a
commercial Qt license, and `qtbridge` is published by The Qt Company under
`LicenseRef-Qt-Commercial OR LGPL-3.0-only`. Distributing a Booklet binary means
complying with whichever of those applies to you.
