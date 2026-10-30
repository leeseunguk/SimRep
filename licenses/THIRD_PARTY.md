# Third-party notices

## Windows installer runtime (October 30, 2026)

The Windows installer includes the unmodified Windows x64 Node.js **24.14.1**
`node.exe` as `runtime/node.exe`. Node.js is MIT licensed, Copyright Node.js
contributors. Its complete upstream `LICENSE`, including the notices for V8,
OpenSSL, libuv and the other embedded dependencies, is installed alongside it
as `runtime/LICENSE`. The staging script downloads the pinned official archive
and checks its SHA-256 before extracting only these two files; npm and a global
Node installation are not required at runtime. Source and release:
https://nodejs.org/dist/v24.14.1/ and https://github.com/nodejs/node/tree/v24.14.1.

The unchanged JavaScript bridge and its Windows-installed production npm tree
are redistributed under `ai/`, preserving package licence/notice files. The
exact versions are recorded in installed `ai/package-lock.json`. Direct
production packages are `@anthropic-ai/claude-agent-sdk` 0.3.311 (Anthropic
commercial terms), `@openai/codex` 0.164.0 (Apache-2.0),
`@code-yeongyu/senpi-ai` 2026.10.25-2 (MIT), and `jose` 5.10.0 (MIT).
Transitive licences, including `@bufbuild/protobuf` Apache-2.0 AND BSD-3-Clause,
remain in their packages. Native provider executables are installed by Windows
npm, not copied from Linux. No account stores or credentials are bundled.
The adapter notices below do not relicense commercial SDK binaries.

Packaging uses a separate Node executable, rather than SEA or a JavaScript
bundler, to preserve dynamic imports, native SDK executable discovery and
upstream notices. Node and its licence add 91,586,176 uncompressed bytes
(91,426,304 for node.exe and 159,872 for LICENSE). The installer additionally
includes the release CLI, production npm packages, OCCT closure and C++ runtimes.
The Windows overlay places the OCCT DLLs beside both executables so the CLI's
startup imports resolve without a global PATH change or a duplicate DLL tree;
OCCT notices remain in `occt/`. Compressed package and complete installed-tree
sizes are recorded in the installer QA report.

## Automatic refinement policy (October 27, 2026)

`crates/simrep-cli/src/auto_refine.rs` adapts the dyadic spatial marking,
sorted fixed-point balance policy and size-relative clearance ideas from
[using76/Iteration-CFD](https://github.com/using76/Iteration-CFD):
`rust/src/automesher/octree.rs`, `rust/src/adapt.rs`, and
`docs/08-site-mesh-playbook.md`, retrieved October 27, 2026. The last file's
`g/2.5` rule and distinction between unresolved gaps and refinement targets
inform the policy. The implementation is new Rust code for SimRep's disjoint
nodal boxes, not a copy of the upstream polyhedral mesh implementation.
The upstream feature, quality, castellate and driver stages and adaptive
rebuild/transfer modules were read as design references; their field-transfer,
snapping and polyhedral topology code is not used.

This adapted policy retains the Prosperity Public License 3.0.0 notice:
Copyright (c) 2026 Iterations Co., Ltd.; collaborator and contributor:
Meteo Simulation Co., Ltd. Source: https://github.com/using76/Iteration-CFD.
The complete licence text is reproduced below. The selector's tests,
CLI wiring, solver profiling and study measurements are original SimRep code.

## AI authentication and provider adapters (October 27, 2026)

The following files adapt the MIT provider package installed as
`@earendil-works/pi-ai`, whose package identity is
`@code-yeongyu/senpi-ai` version `2026.10.25-2`
([source repository](https://github.com/code-yeongyu/senpi/tree/main/packages/ai)).
Every adapted file includes the MIT notice below and “modified for SimRep”.
SimRep's credential storage, RPC/UI integration and test fixtures are original.

| SimRep file | Installed source paths under `dist/` |
| --- | --- |
| `ai/auth/providers.mjs` | `auth/oauth/anthropic.js`, `anthropic-callback-listener.js`, `github-copilot.js`, `xai.js`, `openrouter.js`, `kimi-coding.js`, `kimi-region.js`, `kimi-identity.js`, `devin.js`, `devin-token.js`, `devin-callback.js`, `meta.js`, `cursor.js`, `radius.js`, `device-code.js`, `pkce.js`, `callback-server.js`; `api/github-copilot-endpoint.js` |
| `ai/auth/chatgpt.mjs` | `auth/oauth/chatgpt-subscription.js`, `chatgpt-subscription-callback.js`; `utils/chatgpt-subscription-auth.js` |
| `ai/providers/table.mjs` | `providers/*.js` and `auth/oauth/*.js` endpoint/catalog metadata |
| `ai/providers/client.mjs` | `types.js`, `providers/devin.js`, `api/devin-agent.js` provider context/tool mapping; native Devin wire implementation is the pinned MIT dependency, not copied |

The installed README's `## License` section says only `MIT`; it has no copyright
line. Its published file list excludes a LICENSE file. The canonical
[senpi root LICENSE](https://github.com/code-yeongyu/senpi/blob/main/LICENSE)
was read separately and supplies the two copyright lines reproduced below.
The [badlogic/pi-mono root LICENSE](https://github.com/badlogic/pi-mono/blob/main/LICENSE)
read on the same date says `Copyright (c) 2025 Mario Zechner`.
No auth/inference code from the differently licensed omo top-level plugin is
copied. These notices do not relicense the Anthropic Agent SDK or its bundled
native executables; their commercial terms must remain in redistributed
`node_modules`. `@bufbuild/protobuf` retains its Apache-2.0 AND BSD-3-Clause
notices. Exact direct and transitive versions are in `ai/package-lock.json`.

```text
MIT License

Copyright (c) 2025 Mario Zechner (upstream pi-mono)
Copyright (c) 2026 Yeongyu Kim and senpi contributors

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

### Kiro community adapter

`ai/auth/kiro.mjs` and `ai/providers/kiro.mjs` adapt
[`eddieparc/omo-accounts`](https://github.com/eddieparc/omo-accounts) at commit
`3df128a51458c829e61a4da62d6545ed55bcc1c6`:
`src/providers/kiro/config.ts`, `oauth.ts`, `vendor/kiro.ts`, and
`vendor/eventstream.ts`. The source's MIT LICENSE was read at that exact commit.
SimRep removes account-pool failover and debug logging, binds the auth listener
to IPv4 loopback, validates callback host/state/path, checks event CRCs strictly,
and routes all tools through the workspace MCP approval proxy. No existing omo
credential file is read or imported.

```text
MIT License

Copyright (c) 2026 eddieparc

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

## Numerical verification and verdict registry

`crates/simrep-core/src/uncertainty.rs` adapts `rust/src/vv.rs` from
[using76/Iteration-CFD](https://github.com/using76/Iteration-CFD/blob/main/rust/src/vv.rs)
(retrieved 2026-10-27). It retains Celik's triplet procedure and Eca-Hoekstra's
Appendix A weighted/fixed-exponent fit selection; SimRep uses CPU f64 and adds
input checks and independent worked-example tests.
`crates/simrep-core/src/validation.rs` adapts the explicit registry concept from
[`rust/src/capture/registry.rs`](https://github.com/using76/Iteration-CFD/blob/main/rust/src/capture/registry.rs),
with new runtime numerical records and a generated Markdown table instead of
CUDA capture gates. No GPU capture code was copied.

These adapted files are under the Prosperity Public License 3.0.0, whose
complete upstream text and contributor/source notices appear below.
Copyright (c) 2026 Iterations Co., Ltd.
Collaborator and contributor: Meteo Simulation Co., Ltd.
Source code: https://github.com/using76/Iteration-CFD
The Python wrapper, manufactured solutions, generator and new tests are
original SimRep code.

## CAD face-set measurement reference (October 27, 2026)

The complete-face measurement contract in `crates/simrep-cad/native/bridge.cpp`
and `crates/simrep-cad/src/occt.rs` was independently implemented from the description of
`wall_min_tagged` and its coverage, touch and cross-check guards in
[Iteration-CFD docs/16a-amagine-borrow.md](https://github.com/using76/Iteration-CFD/blob/1475bbf781e08f33f681661ec40e00727f1e7021/docs/16a-amagine-borrow.md).
No implementation code or prose was copied or adapted from Iteration-CFD or
Amagine3D for the CAD measurement feature. The triangle minimum and BVH in
`crates/simrep-cad/src/measure_mesh.rs` are original. The existing MCP transport
adaptation described below retains its existing licence.

Primary source licences and notices read for this work:

- Iteration-CFD [LICENSE](https://github.com/using76/Iteration-CFD/blob/1475bbf781e08f33f681661ec40e00727f1e7021/LICENSE)
  and [NOTICE](https://github.com/using76/Iteration-CFD/blob/1475bbf781e08f33f681661ec40e00727f1e7021/NOTICE):
  Prosperity Public License 3.0.0; Iterations Co., Ltd., with contributions
  from Meteo Simulation Co., Ltd. The licence is reproduced below.
- Amagine3D at `e608dc61d5ac70238d96d9c4f3fba00b6cc9dc21`
  [LICENSE](https://github.com/amagine-ai/Amagine3D/blob/e608dc61d5ac70238d96d9c4f3fba00b6cc9dc21/LICENSE)
  and [NOTICE](https://github.com/amagine-ai/Amagine3D/blob/e608dc61d5ac70238d96d9c4f3fba00b6cc9dc21/NOTICE):
  Apache License 2.0; Copyright 2026 amagine-ai. Its NOTICE separately
  identifies the build123d contributors and CadQuery ancestry. None of those
  components is copied into or newly linked by this feature.

## Existing MCP transport adaptation

The files listed below adapt code from
[using76/Iteration-CFD](https://github.com/using76/Iteration-CFD)
`mcp/` at commit `1475bbf781e08f33f681661ec40e00727f1e7021`
(2026-10-27, `main`).

| File | Source path | Source URL | Blob |
|---|---|---|---|
| `mcp/server.mjs` | `mcp/server.mjs` | https://github.com/using76/Iteration-CFD/blob/1475bbf781e08f33f681661ec40e00727f1e7021/mcp/server.mjs | `c6097254d905e7c78c7e7731ca2bbbdd45490431` |
| `mcp/smoke.mjs` | `mcp/smoke.mjs` | https://github.com/using76/Iteration-CFD/blob/1475bbf781e08f33f681661ec40e00727f1e7021/mcp/smoke.mjs | `ecc53242a7120341faea5035bf3a6270a778c8b2` |

What was adapted is the stdio JSON-RPC loop (`initialize`, `tools/list`,
`tools/call`, `ping`, notifications), workspace path refusal, and argv-only
process spawn. SimRep tool definitions, CLI flag mapping, and the einzel
pipeline checks are not from that repository. `mcp/README.md` is original.

Those two files are under the Prosperity Public License 3.0.0.
Copyright Iterations Co., Ltd. (주식회사 이터레이션즈).
Collaborator and contributor: 주식회사 메테오시뮬레이션 (Meteo Simulation Co., Ltd.).
Source code: https://github.com/using76/Iteration-CFD

Prosperity is not an OSI open-source licence. Non-commercial use is free.
Commercial use is a thirty-day trial, then a separate licence from the
contributor (simul@msimul.com). The licence text below is the upstream
`LICENSE` file, unchanged, including the licensor's additions.

## Prosperity Public License 3.0.0

```
# Prosperity Public License 3.0.0

Contributor: 주식회사 이터레이션즈 (Iterations Co., Ltd.)

Collaborator and contributor: 주식회사 메테오시뮬레이션 (Meteo Simulation Co., Ltd.)

Source Code: https://github.com/using76/Iteration-CFD

## Purpose

This license allows you to use and share this software for noncommercial purposes for free and to try this software for commercial purposes for thirty days.

## Agreement

In order to receive this license, you have to agree to its rules.  Those rules are both obligations under that agreement and conditions to your license.  Don't do anything with this software that triggers a rule you can't or won't follow.

## Notices

Make sure everyone who gets a copy of any part of this software from you, with or without changes, also gets the text of this license and the contributor and source code lines above.

## Commercial Trial

Limit your use of this software for commercial purposes to a thirty-day trial period.  If you use this software for work, your company gets one trial period for all personnel, not one trial per person.

## Contributions Back

Developing feedback, changes, or additions that you contribute back to the contributor on the terms of a standardized public software license such as [the Blue Oak Model License 1.0.0](https://blueoakcouncil.org/license/1.0.0), [the Apache License 2.0](https://spdx.org/licenses/Apache-2.0.html), [the MIT license](https://spdx.org/licenses/MIT.html), or [the two-clause BSD license](https://spdx.org/licenses/BSD-2-Clause.html) doesn't count as use for a commercial purpose.

## Personal Uses

Personal use for research, experiment, and testing for the benefit of public knowledge, personal study, private entertainment, hobby projects, amateur pursuits, or religious observance, without any anticipated commercial application, doesn't count as use for a commercial purpose.

## Noncommercial Organizations

Use by any charitable organization, educational institution, public research organization, public safety or health organization, environmental protection organization, or government institution doesn't count as use for a commercial purpose regardless of the source of funding or obligations resulting from the funding.

## Defense

Don't make any legal claim against anyone accusing this software, with or without changes, alone or with other technology, of infringing any patent.

## Copyright

The contributor licenses you to do everything with this software that would otherwise infringe their copyright in it.

## Patent

The contributor licenses you to do everything with this software that would otherwise infringe any patents they can license or become able to license.

## Reliability

The contributor can't revoke this license.

## Excuse

You're excused for unknowingly breaking [Notices](#notices) if you take all practical steps to comply within thirty days of learning you broke the rule.

## No Liability

***As far as the law allows, this software comes as is, without any warranty or condition, and the contributor won't be liable to anyone for any damages related to this software or this license, under any kind of legal claim.***

---

*Everything above this line is the Prosperity Public License 3.0.0, unchanged.
Everything below it is added by the licensor.*

## What "public research organization" means here

[Noncommercial Organizations](#noncommercial-organizations) makes use by a
public research organization noncommercial. The licensor reads **public** there
as the *purpose of the work*, not the ownership of the body doing it.

**Free.** Research whose purpose is public: fire and rescue, medicine and public
health, occupational and public safety, disaster response, environmental
protection. This is free wherever it is done, including at a government-funded
research institute.

**Not free.** Research and development aimed at a particular product, or at
technology to be transferred to industry, whoever performs it — electric-vehicle
technology, rail and propulsion technology, aircraft engine technology and
anything comparable. Work of that kind at a government-funded research institute
is commercial use.

**The test the licensor applies:** if the work is subject to a technology fee
(기술료), or is carried out for a company or under a programme that transfers the
result to one, it is commercial use and needs a licence after the thirty-day
trial.

Universities and schools, and the institutes and laboratories that belong to
them, are educational institutions and remain free.

한국어: 「공공 연구기관」의 *공공* 은 기관의 소유 주체가 아니라 **연구의 목적** 을
뜻합니다. 소방·의료·보건·안전·재난·환경 보호를 목적으로 하는 연구는 정부출연
연구기관에서 수행하더라도 무료입니다. 특정 제품이나 산업 이전을 목적으로 하는
기술개발·기반연구는 — 전기차, 철도·추진, 항공기 엔진 기술 등 — 정부출연연구기관에서
수행하더라도 상업적 이용입니다. **기술료 징수 대상이거나 기업을 위해 또는 결과를
기업에 이전하는 사업으로 수행되는 연구는 상업적 이용** 이며, 30일 시험 후 라이선스가
필요합니다. 대학·학교 및 그 소속 연구소는 교육기관으로서 무료입니다.

## Commercial licensing

Commercial use beyond the thirty-day trial requires a separate commercial
licence from the contributor.

    주식회사 이터레이션즈 (Iterations Co., Ltd.)
    simul@msimul.com

See [LICENSING.md](LICENSING.md) for what counts as commercial use, what a
commercial licence covers, and how to ask for one.
```
