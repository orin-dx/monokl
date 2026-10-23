# Architecture decision records

Short, dated records of architectural decisions: ones that shape system structure, boundaries, or contracts other code depends on. Not a changelog, not a design doc. Comments, `ARCHITECTURE.md` and `docs/architecture/` explain what the code does.

The spec's own decision log, [`docs/spec/05-research-and-decisions.md`](../spec/05-research-and-decisions.md), records how the spec was reached. ADRs record decisions made since, including any that depart from the spec.

## Index

One line per ADR, with what it guards. Add a line when you add an ADR.

- [1. Keep FileIoError in monokl instead of a shared io-errors crate](0001-file-io-error-lives-in-monokl.md): do not reintroduce a private registry; move `FileIoError` to a shared crate only when a second tool needs it.
- [2. Define Digest in monokl until wisp-contracts exists](0002-digest-lives-in-monokl-until-wisp-contracts.md): do not change `Digest`'s text or serde form; when `wisp-contracts` ships it, re-export and delete monokl's copy.

Changing any of these needs a new ADR.

## When to write one

Two gates, both required:

1. **Architectural.** It affects system structure (module or crate boundaries, how components interact), a dependency or technology choice that ripples across the codebase, a data format or contract other code depends on, or a cross-cutting constraint (performance model, correctness discipline, security model).
2. **Expensive to reverse and not obvious from the code.** If it gets reverted in a year, what breaks, and would anyone know why?

If either answer is no, it's not an ADR. A single function's algorithm is a code comment; a bug fix or refactor is a commit message; a naming or style convention belongs in `AGENTS.md` or a lint.

## Format

One file per decision: `docs/adr/NNNN-short-title.md`, numbered sequentially, never reused. Copy `0000-template.md` to start.

- **Never edit history.** If a decision changes, write a new ADR and mark the old one's status `Superseded by NNNN`.
- **Status** is one of `Accepted`, `Superseded by NNNN`, `Deprecated`.
- Keep it to a screen. Put technical depth in `docs/architecture/` and link it.

## Writing it

Write it like you're explaining it to the next engineer over their shoulder. One line per paragraph, no hard wrapping. Sentence-case headers.

- Cut filler. If a sentence means the same without a word, delete the word.
- Banned by default: _leverage, utilize, robust, seamless, cutting-edge, essentially, basically, in order to, it's worth noting, it is important to note, at the end of the day._
- Concrete beats vague. Name the file, the number, the command.
- Active voice. "We chose X because Y."
- Show the trade-off. A decision with no downside is a fact, not a decision.
