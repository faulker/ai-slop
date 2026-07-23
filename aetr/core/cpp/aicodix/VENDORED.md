# Vendored aicodix headers

Header-only C++17 libraries by Ahmet Inan (aicodix), all under the same
permissive zero-clause-BSD-style license (see the LICENSE file in each
subdirectory). Vendored 2026-07-22 from the upstream `main` branches:

| Directory | Upstream | Pinned commit |
|---|---|---|
| `modem/` | https://github.com/aicodix/modem | `a4a8b57769679cf0fd3f5b4941f246b5e3718ee9` |
| `code/`  | https://github.com/aicodix/code  | `e6cfc5b0f71d8e82d6cba2184b1edf0486f64238` |
| `dsp/`   | https://github.com/aicodix/dsp   | `8246c5b5dd9c35124bbf88538c8b75e748adb9c9` |

All `.hh` files from each repo are copied verbatim (plus each repo's LICENSE),
with one exception:

## Local modifications

- `modem/schmidl_cox.hh`: two tuning constants changed to the values used by
  the COFDMTV protocol (Rattlegram): Schmitt trigger thresholds `0.07/0.09`
  -> `0.17/0.19` of `match_len`, and the fine-timing sanity bound
  `abs(pos_err) > guard_len` -> `guard_len / 2`. These are the field-proven
  values for the push-to-talk FM audio path this project targets.

## Protocol provenance

The `aetr` shim (`core/cpp/shim.cc`) implements the COFDMTV burst format
(Schmidl-Cox sync symbol, BCH(255,71)+OSD preamble, 4 QPSK payload symbols,
CA-SCL polar coding at 2048 bits) as designed by Ahmet Inan for
Rattlegram (https://github.com/aicodix/rattlegram, commit
`56bba44527f37963deefc675cc909a97dbb6b149`). Rattlegram's LICENSE file was
verified to carry the same permissive zero-clause text as the three repos
above (not GPL). The shim is an independent implementation written against
the vendored headers; the three polar frozen-bit tables in
`core/cpp/cofdm_tables.hh` are numeric code-construction data taken from
that protocol definition.
