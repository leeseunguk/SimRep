# Release-pinned omission sources

The package map supplements missing package notice files with upstream text,
not author metadata. The manifest hashes the exact downloaded bytes. Refresh
only these entries with:

```sh
node tools/win/refresh-notice-sources.mjs aws-sdk-license aws-sdk-notice lambda-invoke-store-license lambda-invoke-store-notice data-uri-to-buffer-license proxy-agent-negotiate-license standardwebhooks-license alloc-stdlib-license
```

## Release resolution

- `@aws-sdk/credential-provider-http@3.972.79`,
  `@aws-sdk/credential-provider-login@3.972.84`, and
  `@aws-sdk/nested-clients@3.997.51` have no npm registry `gitHead`.
  All three were published on October 26, 2026 from the `Publish v3.1157.0`
  commit `e5988ecad1aded47fa75f5e18c9cc00ddcb608de` in `aws/aws-sdk-js-v3`.
  The source `packages-internal/{credential-provider-http,credential-provider-login,nested-clients}/package.json`
  files at that commit contain those exact versions. The nested-clients npm
  repository directory is stale: the package had moved to packages-internal.
  The root LICENSE is preserved, including Amazon's copyright in its appendix.
  There is no root NOTICE. The repository's only NOTICE, `codegen/NOTICE`, is
  included conservatively and remains labelled with its actual upstream path.
- `@aws/lambda-invoke-store@0.3.0`: npm `gitHead`
  `4b2c7152b3f48975059585a09a59c8007ab008fa` in
  `awslabs/aws-lambda-invoke-store`; root LICENSE and NOTICE.
- `data-uri-to-buffer@4.0.1`: npm `gitHead`
  `85cd8c854aefbf1bb636789d80364cfac8ea1583` in
  `TooTallNate/node-data-uri-to-buffer`. This revision has no separate LICENSE
  or NOTICE; README.md contains the full MIT text with the 2014 Nathan Rajlich
  copyright. The complete README is preserved, without editing its attribution.
- `proxy-agent-negotiate@1.1.0`: npm has no `gitHead`; its annotated release
  tag `proxy-agent-negotiate@1.1.0` resolves through tag object
  `04af2dd5365059650ad7209cc46457adca9b5ecb` to commit
  `b7e5f7ccce1a3ac5b339cc4c587974e8989cbc16` in `TooTallNate/proxy-agents`.
  The package.json at `packages/negotiate` confirms version 1.1.0 and MIT.
  That package has no dedicated licence file and the repository has no root
  licence or NOTICE. The mapped source is the monorepo's companion
  `packages/proxy-agent/LICENSE`, preserved verbatim with its 2013 Nathan
  Rajlich copyright. This is a companion-package notice, not a newly found
  negotiate-specific copyright declaration; upstream does not provide one
  at this revision. Its applicability to negotiate is an attribution assumption
  that release reviewers must retain, not a claim established by author metadata.
- `standardwebhooks@1.1.1`: npm `gitHead`
  `b4d2c14fc5b4ccff3ff271e3b087dff812254c59` in
  `standard-webhooks/standard-webhooks`; libraries/LICENSE identifies 2023 Svix.
  There is no NOTICE in that revision.
- `alloc-stdlib@0.3.0`: the review's published-crate `.cargo_vcs_info.json`
  points to `0a81fd6928ea3b33c8cd484aa4575d50ffb98012` in
  `dropbox/rust-alloc-no-stdlib`; root LICENSE identifies 2016 Dropbox, Inc.
  The mapping also removes this Cargo package's author-derived fallback.
- `@tauri-apps/plugin-dialog@2.8.3`: npm `gitHead`
  `44293a5722efcaf27060504e63b17aeba721b40f` in
  `tauri-apps/plugins-workspace`. The npm package ships `LICENSE.spdx`,
  not the full licence text. The snapshot preserves the release's root
  `LICENSE_MIT` verbatim for the MIT option of its dual-licence declaration.
- `mlua-sys@0.10.0`, `profiling@1.0.18`,
  `spirv@0.4.0+sdk-1.4.341.0`, and `tauri-plugin@2.7.2` omit licence
  files from their published crates. Their `.cargo_vcs_info.json` files pin
  commits `e7fa8d75bb3947648605562694d03c1ce4c82b20`,
  `8271551172eb6fa4cba47369aedd93790c623df9`,
  `8afc3d0ac8e158128cd1410bb2e4b4c26ab11bb4`, and
  `59b63ab500da9859f12d8888c3298bc29bba0b92`, respectively.
  Root upstream LICENSE or LICENSE-MIT files are preserved verbatim, using
  the MIT option for dual-licensed crates. No copyright is inferred.
- `selectors@0.38.0`: `.cargo_vcs_info.json` pins
  `572ecba2d1600e7c3d490586692a209faf703baa` in `servo/stylo`.
  Neither the crate nor this repository root carries the full MPL text.
  The complete `selectors/lib.rs` is retained as the source of its explicit
  MPL-2.0 declaration, together with the existing full SPDX MPL-2.0 text.
  This package-specific mapping is not an author-derived fallback.
- `dunce@1.0.5` already ships the complete CC0 legal code. The full-text
  recognizer now requires its four sections and closing disclaimer rather
  than rejecting this valid shipped notice. No supplemental text is needed.

Exact raw URLs and SHA-256 content hashes are in manifest.json; every new
upstream source URL includes the full commit SHA. Builds read these local
snapshots offline and fail when a package has neither complete shipped text
nor a mapped source. A new package version does not inherit an older
version's mapping.
