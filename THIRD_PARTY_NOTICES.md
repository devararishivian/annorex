# Third-party notices

Annorex contains code from the projects below. Each vendored file starts with a header that gives its origin and the
changes for Annorex.

## plannotator-tui

- Project: plannotator-tui
- URL: <https://github.com/plannotator/plannotator-tui>
- Commit: `83b0cd91e287df391d5b27029140ff72c32c085d`
- License: MIT
- Vendored paths in Annorex:
    - `crates/annorex-hosts/src/` (from `crates/plannotator-tui-hosts/src/` and
      `crates/plannotator-tui/src/last/roots.rs`)
    - `crates/annorex-hosts/tests/` and `crates/annorex-hosts/tests/fixtures/` (from
      `crates/plannotator-tui-hosts/tests/`)
    - The lint table in the root `Cargo.toml` and the root `rustfmt.toml`

```text
MIT License

Copyright (c) 2026 Michael Ramos

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
