# Third-party notices

Rustload itself is released under the [MIT license](LICENSE). It ships with the following
third-party work, under their own licenses.

## Bundled in the binary and the browser extension

- **Archivo** typeface, copyright 2020 The Archivo Project Authors
  (<https://github.com/Omnibus-Type/Archivo>). SIL Open Font License 1.1; the full text is in
  [`extension/fonts/OFL.txt`](extension/fonts/OFL.txt). The web interface embeds it through
  [`@fontsource-variable/archivo`](https://fontsource.org/fonts/archivo).
- **Lucide** icons, copyright Lucide Icons and Contributors (<https://lucide.dev>). ISC License.
  Some icons are derived from **Feather**, copyright 2013-present Cole Bemis, MIT License. The
  license text is reproduced at the top of [`extension/lib/icons.js`](extension/lib/icons.js).
- Rust crates and npm packages compiled into Rustload, each under its own license (mostly MIT
  and/or Apache-2.0). They are listed with their versions in [`Cargo.lock`](Cargo.lock) and
  [`web/pnpm-lock.yaml`](web/pnpm-lock.yaml).

## Bundled in the Docker image only

| Component                                                           | License                                                                                   |
| ------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| [7-Zip](https://www.7-zip.org) (`7zz`, official static build)       | GNU LGPL 2.1+ with parts under the BSD 3-clause license and the unRAR license restriction |
| [yt-dlp](https://github.com/yt-dlp/yt-dlp)                          | The Unlicense                                                                             |
| [FFmpeg](https://ffmpeg.org), from Alpine Linux packages            | GNU LGPL 2.1+ / GNU GPL 2+, depending on the build options                                |
| [QuickJS](https://bellard.org/quickjs/), from Alpine Linux packages | MIT                                                                                       |
| [tini](https://github.com/krallin/tini), from Alpine Linux packages | MIT                                                                                       |
| [Alpine Linux](https://alpinelinux.org) base image and packages     | Various, see each package                                                                 |
