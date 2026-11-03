# Security Policy

## Reporting a Vulnerability

Email **security@orin-dx.com** — don't open a public issue for anything exploitable before a fix ships.

Include:
- Which subsystem is affected (parsing, path resolution, a specific CLI command)
- The concrete failure scenario — what an attacker could achieve, and how
- A minimal reproduction, if you have one — a crafted source file or path is usually enough

Expect an acknowledgment within 5 business days. We'll keep you posted as a fix moves through triage, and credit you in release notes unless you'd rather stay anonymous.

## Scope

monokl parses source code with real parsers ([OXC](https://oxc.rs) for TypeScript/JavaScript, [`ra_ap_syntax`](https://crates.io/crates/ra_ap_syntax) for Rust) and returns structured JSON, built specifically to be consumed by AI agents rather than read by a human. That shapes the actual threat model:

- **Parsing untrusted source as an attack surface** — monokl is designed to run against arbitrary target codebases, including code the operator doesn't control or hasn't reviewed. A crafted file that exploits a parser bug (crash, resource exhaustion, pathological AST depth) or that's specifically shaped to poison search rankings or produce a misleading structural answer is in scope.
- **Path handling** — The `monokl search` path argument resolves filesystem paths (via `camino::Utf8Path`). Any way a crafted path or symlink escapes the intended search root is in scope.
- **Output consumed by an agent, not a human** — a result JSON is not just displayed, it typically drives an agent's next action. A crafted source file that manipulates monokl's output to mislead a consuming agent (a structural-search analogue of prompt injection) is a real finding here, not a theoretical one.
- **Supply chain** — the OXC and `ra_ap_syntax` parser dependencies, and anything in `Cargo.toml`/`Cargo.lock` that could resolve to unintended code.

Out of scope: vulnerabilities in OXC or `ra_ap_syntax` themselves — report those upstream; we'll track and update once a fix is available.

## Supported Versions

The `monokl` crate is published on crates.io. Security fixes land on `main` first and ship in the next release; only the latest release is supported.
