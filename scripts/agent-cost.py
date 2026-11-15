#!/usr/bin/env python3
"""What a coding agent pays to answer each rg-compare question with `monokl search` against ripgrep alone. No model, no
timing: bytes, estimated tokens and turns, all deterministic for a pinned checkout and a binary.

Rows are the rg-compare harness's: sampled identifiers (rg-compare.repos), concept questions and member lookups
(rg-compare.questions), on every repository including the held-out and report-only ones. Sets mirror the harness's
lines: identifiers, questions, members, rust (fd identifiers, questions and members pooled), held-out questions,
held-out members, held-out rust and report-only (the repositories marked `report-only`), plus all.

monokl cost. One `monokl search <words> .` with default caps, as `rg-compare.py` runs it. Bytes are the size of its
stdout, tokens are ceil(bytes / 3) (the product's own estimate), turns are 1. The row is answerable in one turn when
one of the first 3 rows (blocks) is in the expected file, spans the expected declaration's line and shows the
declaration's text, so the agent needs no follow-up read. `monokl+read` is the same cost with one more turn for a
file read of the expected block when the row is not answerable, so a miss is not counted as free.

rg-policy cost. A fixed policy standing in for an agent that only has ripgrep. Every step favors rg:

  1. Best term. For an identifier, the identifier. For a question, the word whose `rg -n` output has the fewest
     non-test lines (at least one; ties go to the longer word, then alphabetical order), which an agent could only
     know by trying every word.
  2. Turn 1: `rg -n -S -w <best term>` over the language's source files. Test files are dropped, as monokl drops
     them. The agent reads the first 200 lines, each cut to 300 bytes, with no `./` prefix; matches are sorted by path.
  3. Turn 2, only when turn 1 matched more than 50 lines: a narrower `rg -n`, read the same way. For an identifier,
     the declaration pattern of that identifier. For a question, lines holding the two best words in either order.
     A question with fewer than two words that match has no narrower pattern and skips turn 2.
  4. Last turn: a file read of the lines of the block that holds the declaration. The extent is monokl's own, from
     the first and last line of the rows of `monokl search <symbol>` with the lean cut off (`--min-score 0`, no row
     or token cap, `--snippet-lines 0`); the declaration line is found by rg, and ranking plays no part, so the
     measurement does not depend on where monokl ranks the answer. For each declaration line the block is the
     innermost block holding it (a class does not stand in for its method), and the largest of those counts, so an
     implementation is read in place of its overload signatures. Across several expected files the smallest counts.
     A declaration with no body, such as Rust's `mod name;`, has no block and is read as its own line; the report
     lists them. The extent is monokl's, so a TypeScript block starts at the declaration and a Rust block also holds
     the doc comments and attributes above it. An analyzer extent bug would raise or lower the read cost and monokl's
     own rows alike. rg is never charged for failing to find the file: the policy always succeeds, and `rg_located`
     reports how often the expected file was in the lines the agent actually read. `scripts/test_agent_cost.py`
     checks the extents with exact bytes; `just agent-cost` runs it first.

Turns are the rg calls plus the read; bytes are the sum of the lines read in each call plus the read. `rg-early`
repeats the policy but stops, with no read, as soon as a turn shows the expected declaration's line; it is the cheaper
agent that trusts a matching line, and tokens use the same ceil(bytes / 3) as monokl.

Report only. A line starting `FAIL` is printed when a set's median monokl bytes exceed its median rg-policy bytes or
when its answerable-in-one-turn rate is below 0.8; the exit status stays 0. This is not a model evaluation: it says
what each route costs if the agent does what the policy says, not whether a model would find the answer.

Repositories are the clones `rg-compare.py` keeps in target/rg-compare, fetched on demand at the pinned shas.
"""

import argparse
import importlib.util
import json
import math
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# rg-compare.py owns the repositories, questions, declaration patterns and monokl's file rules; load it by path.
_spec = importlib.util.spec_from_file_location("rg_compare", ROOT / "scripts" / "rg-compare.py")
RC = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(RC)

TOP_ROWS = 3
FIRST_LINES = 200
LINE_CAP = 300
NARROW_ABOVE = 50
MIN_ONE_TURN = 0.8
ROW = re.compile(r"^  ([^,\n]+),(\d+),(\d+),")
BARE = str.maketrans("", "", '"\\')
TOOLS = ("monokl", "monokl+read", "rg-policy", "rg-early")
SETS = ("identifiers", "questions", "members", "rust", "held-out questions", "held-out members", "held-out rust",
        "report-only", "all")


def tokens(byte_count):
    return math.ceil(byte_count / 3)


def percentile(values, share):
    """Nearest rank: the smallest value with at least `share` of the values at or below it."""
    ordered = sorted(values)
    return ordered[max(math.ceil(share * len(ordered)) - 1, 0)]


def median(values):
    ordered = sorted(values)
    mid = len(ordered) // 2
    return ordered[mid] if len(ordered) % 2 else (ordered[mid - 1] + ordered[mid]) / 2


def rg_lines(path, lang, pattern, extra=()):
    """Non-test `file:line:text` lines of one `rg -n` call, sorted by path so the first lines are stable."""
    args = ["rg", "-n", "--no-heading", "--color", "never", "--sort", "path", *extra]
    for glob in lang.globs:
        args += ["--glob", glob]
    out = RC.run([*args, "-e", pattern, "."], cwd=path, check=False).stdout
    lines = [line.removeprefix("./") for line in out.splitlines()]
    return [line for line in lines if not RC.is_test_path(line.split(":", 1)[0], lang)]


def read_cost(lines):
    """Bytes of the first lines an agent reads from one call, each cut at the cap, with its newline."""
    shown = [line[:LINE_CAP] for line in lines[:FIRST_LINES]]
    return shown, sum(len(line.encode()) + 1 for line in shown)


def word_lines(path, lang, word, cache):
    key = (str(path), word)
    if key not in cache:
        cache[key] = rg_lines(path, lang, re.escape(word), ["-S", "-w"])
    return cache[key]


def best_words(path, lang, words, cache):
    """The words that match at least once, most selective first."""
    counted = [(len(word_lines(path, lang, word, cache)), word) for word in words]
    return [word for count, word in sorted(counted, key=lambda c: (c[0], -len(c[1]), c[1])) if count]


def declaration_lines(path, lang, file, symbol, member):
    """(line number, text) of every declaration of `symbol` in `file`, as rg finds them."""
    pattern = RC.declaration_pattern(symbol, lang, member)
    out = RC.run(["rg", "-n", "--no-heading", "--color", "never", "-e", pattern, file], cwd=path, check=False).stdout
    found = []
    for line in out.splitlines():
        number, _, text = line.partition(":")
        found.append((int(number), text))
    return found


def monokl_spans(binary, symbol, path, file):
    """(first line, last line) of every block monokl returns for `file` when it searches `symbol` with the lean cut
    off (no score floor, no row cap, no token cap). Only extents are read: the declaration is found by rg, and monokl's
    ranking does not choose it."""
    out = RC.run([binary, "search", symbol, ".", "--min-score", "0", "--max-results", "1000000", "--max-tokens",
                  "1000000000", "--snippet-lines", "0"], cwd=path, check=False).stdout
    rows = [(m.group(1).removeprefix("./"), int(m.group(2)), int(m.group(3)))
            for line in out.splitlines() if (m := ROW.match(line))]
    return sorted({(start, end) for name, start, end in rows if name == file})


def declared_block(spans, declarations):
    """(first line, last line) of the block that is the declaration: for each declaration line, the innermost block
    that holds it (a class that holds a method does not stand in for the method), then the largest of those, so an
    implementation is read in place of its overload signatures. None when no block holds any declaration line."""
    innermost = set()
    for number, _text in declarations:
        holding = [(start, end) for start, end in spans if start <= number <= end]
        if holding:
            innermost.add(min(holding, key=lambda span: (span[1] - span[0], -span[0])))
    return max(innermost, key=lambda span: (span[1] - span[0], -span[0]), default=None)


def span_bytes(lines, span):
    """Bytes of the lines `span` covers (1-based, inclusive), each with its newline."""
    return sum(len(line.encode()) + 1 for line in lines[span[0] - 1:span[1]])


def read_block_bytes(binary, path, expected, symbol, gaps):
    """Bytes of the smallest file read that shows the expected declaration: per expected file the block of
    `declared_block`, then the smallest file. monokl makes no block for a declaration that has no body, such as Rust's
    `mod name;`; that declaration is read as its own line, and each such file is appended to `gaps` so the report
    counts them."""
    sizes = []
    for file, declarations in expected.items():
        lines = (path / file).read_text(errors="replace").splitlines()
        span = declared_block(monokl_spans(binary, symbol, path, file), declarations)
        if span is None:
            gaps.append(f"{path.name}:{file}:{declarations[0][0]}")
            span = (declarations[0][0], declarations[0][0])
        sizes.append(span_bytes(lines, span))
    return min(sizes)


def declaration_prefix(text):
    """The first 40 characters of a declaration line without its export keywords, as rg-compare compares them."""
    bare = re.sub(r"^(?:export\s+|default\s+|declare\s+)+", "", text).rstrip(" {;").strip()
    return bare[:40].translate(BARE)


def monokl_cost(binary, query, path, expected):
    """Runs monokl once; returns (bytes, answerable in the first rows, how many rows were printed)."""
    out = RC.run([binary, "search", query, "."], cwd=path, check=False).stdout
    rows = [(m.group(1).removeprefix("./"), int(m.group(2)), int(m.group(3)), line)
            for line in out.splitlines() if (m := ROW.match(line))]
    answerable = False
    for file, start, end, text in rows[:TOP_ROWS]:
        shown = text.translate(BARE)
        if file in expected and any(
                start <= number <= end and declaration_prefix(decl) in shown for number, decl in expected[file]):
            answerable = True
    return len(out.encode()), answerable, len(rows)


def rg_policy(path, lang, words, identifier, expected, cache, read):
    """Simulates the rg-only agent; returns its turns, bytes, whether the expected file was read, and the early-stop
    turns and bytes."""
    usable = best_words(path, lang, words, cache)
    turns = []
    if usable:
        first = word_lines(path, lang, usable[0], cache)
        turns.append(first)
        if len(first) > NARROW_ABOVE:
            if identifier:
                narrower = rg_lines(path, lang, RC.declaration_pattern(identifier, lang))
            elif len(usable) > 1:
                a, b = (re.escape(w) for w in usable[:2])
                narrower = rg_lines(path, lang, rf"\b{a}\b.*\b{b}\b|\b{b}\b.*\b{a}\b", ["-i"])
            else:
                narrower = None
            if narrower is not None:
                turns.append(narrower)
    spent = 0
    early = None
    located = False
    for number, lines in enumerate(turns, 1):
        shown, cost = read_cost(lines)
        spent += cost
        for line in shown:
            file, _, rest = line.partition(":")
            if file in expected:
                located = True
                if early is None and int(rest.partition(":")[0]) in {n for n, _ in expected[file]}:
                    early = (number, spent)
    full = (len(turns) + 1, spent + read)
    return full, located, early or full


def identifier_rows(binary, repo_filter, count):
    for name, url, sha, lang, held_out in RC.repositories(repo_filter):
        if held_out:
            continue
        path = RC.checkout(name, url, sha)
        for term in RC.sample_terms(path, count, lang):
            files, _ = RC.split(RC.definitions(term, path, lang), lang)
            if files:
                yield {"repo": name, "lang": lang, "path": path, "kind": "identifier", "query": term,
                       "words": [term], "identifier": term, "symbol": term, "member": False,
                       "files": sorted(files)}


def question_rows(repo_filter):
    checkouts = {name: (RC.checkout(name, url, sha), lang) for name, url, sha, lang, _ in RC.repositories(repo_filter)}
    for repo, words, file, symbol, member, held_out in RC.question_rows(repo_filter):
        path, lang = checkouts[repo]
        yield {"repo": repo, "lang": lang, "path": path, "kind": "member" if member else "question", "query": words,
               "words": words.split(), "identifier": None, "symbol": symbol, "member": member, "files": [file],
               "held_out": held_out}


def set_names(row, report_only):
    rust = row["lang"] is RC.LANGS["rust"]
    held = row.get("held_out", False)
    if held and row["repo"] in report_only:
        name = "report-only"
    elif held:
        name = "held-out rust" if rust else "held-out members" if row["member"] else "held-out questions"
    elif rust:
        name = "rust"
    else:
        name = {"identifier": "identifiers", "question": "questions", "member": "members"}[row["kind"]]
    return [name, "all"]


def score(binary, row, cache, gaps):
    path, lang = row["path"], row["lang"]
    expected = {}
    for file in row["files"]:
        found = declaration_lines(path, lang, file, row["symbol"], row["member"])
        if found:
            expected[file] = found
    if not expected:
        sys.exit(f"{row['repo']}: `{row['symbol']}` has no declaration in {row['files']}; run `just rg-compare`")
    monokl_bytes, answerable, printed = monokl_cost(binary, row["query"], path, expected)
    block = read_block_bytes(binary, path, expected, row["symbol"], gaps)
    (rg_turns, rg_bytes), located, (early_turns, early_bytes) = rg_policy(
        path, lang, row["words"], row["identifier"], expected, cache, block)
    return {
        "repo": row["repo"], "kind": row["kind"], "query": row["query"], "symbol": row["symbol"],
        "expected": sorted(expected), "read": block,
        "answerable": answerable, "rg_located": located, "monokl_rows": printed,
        "monokl": (1, monokl_bytes),
        "monokl+read": (1 if answerable else 2, monokl_bytes + (0 if answerable else block)),
        "rg-policy": (rg_turns, rg_bytes),
        "rg-early": (early_turns, early_bytes),
    }


def table(results, detail):
    print(f"{'set':<19} {'n':>3} {'tool':<11} {'turns med/total':>15} {'bytes median':>13} {'p90':>8} {'total':>10} "
          f"{'tokens median':>14} {'p90':>7} {'total':>9}  answered")
    failures = []
    for name in SETS:
        rows = [r for r in results if name in r["sets"]]
        if not rows:
            continue
        stats = {}
        for tool in TOOLS:
            turns = [r[tool][0] for r in rows]
            sizes = [r[tool][1] for r in rows]
            stats[tool] = median(sizes)
            answered = {"monokl": "answerable in one turn", "rg-policy": "rg located the file"}.get(tool)
            count = sum(r["answerable" if tool == "monokl" else "rg_located"] for r in rows)
            print(f"{name if tool == 'monokl' else '':<19} {len(rows) if tool == 'monokl' else '':>3} {tool:<11} "
                  f"{f'{median(turns):g}/{sum(turns)}':>15} {median(sizes):>13,.0f} {percentile(sizes, 0.9):>8,} "
                  f"{sum(sizes):>10,} {tokens(median(sizes)):>14,} {tokens(percentile(sizes, 0.9)):>7,} "
                  f"{tokens(sum(sizes)):>9,}  "
                  f"{f'{count}/{len(rows)} = {count / len(rows):.0%} {answered}' if answered else ''}")
        rate = sum(r["answerable"] for r in rows) / len(rows)
        if stats["monokl"] > stats["rg-policy"]:
            failures.append(f"{name}: monokl median {stats['monokl']:,.0f} bytes exceeds rg-policy median "
                            f"{stats['rg-policy']:,.0f}")
        if rate < MIN_ONE_TURN:
            failures.append(f"{name}: answerable in one turn {rate:.0%} is below {MIN_ONE_TURN:.0%}")
    if detail:
        print()
        print("per question: repo | kind | query -> symbol | monokl bytes answerable | rg-policy turns bytes located | "
              "rg-early turns bytes")
        for r in results:
            print(f"{r['repo']} | {r['kind']} | {r['query']!r} -> {r['symbol']} | {r['monokl'][1]} "
                  f"{'yes' if r['answerable'] else 'no'} | {r['rg-policy'][0]} {r['rg-policy'][1]} "
                  f"{'yes' if r['rg_located'] else 'no'} | {r['rg-early'][0]} {r['rg-early'][1]}")
    print()
    for failure in failures:
        print(f"FAIL (report-only, exit status unchanged) {failure}")
    return failures


def summary(results, failures):
    rows = results
    monokl = median([r["monokl"][1] for r in rows])
    rg = median([r["rg-policy"][1] for r in rows])
    one_turn = sum(r["answerable"] for r in rows)
    located = sum(r["rg_located"] for r in rows)
    print(f"summary: {len(rows)} questions; median bytes monokl {monokl:,.0f} ({tokens(monokl):,} tokens, 1 turn) vs "
          f"rg-policy {rg:,.0f} ({tokens(rg):,} tokens, {median([r['rg-policy'][0] for r in rows]):g} turns); "
          f"monokl answerable in one turn {one_turn}/{len(rows)} = {one_turn / len(rows):.0%}; "
          f"rg-policy located the file {located}/{len(rows)}; "
          f"{'FAIL x' + str(len(failures)) if failures else 'no failing marker'} (report-only)")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", default=str(ROOT / "target" / "release" / "monokl"))
    parser.add_argument("--repo", action="append", help="limit to a repository name from rg-compare.repos")
    parser.add_argument("--terms", type=int, default=25, help="identifiers sampled per repository, as rg-compare.py")
    parser.add_argument("--detail", action="store_true", help="print one line per question after the table")
    parser.add_argument("--json", help="write every row to this path")
    args = parser.parse_args()

    if not Path(args.binary).exists():
        sys.exit(f"{args.binary} not found; run `cargo build --release` or pass --binary")
    RC.configure(args.binary)
    report_only = RC.report_only_repositories()

    cache = {}
    gaps = []
    results = []
    for row in [*identifier_rows(args.binary, args.repo, args.terms), *question_rows(args.repo)]:
        result = score(args.binary, row, cache, gaps)
        result["sets"] = set_names(row, report_only)
        results.append(result)
    if not results:
        sys.exit("no rows to score")

    print("agent workflow cost: one monokl search vs a fixed rg policy; bytes, tokens = ceil(bytes / 3), no timing")
    failures = table(results, args.detail)
    print(f"read sized from monokl's block extents; {len(gaps)} declaration(s) have no block and are read as their own "
          f"line: {', '.join(gaps) or 'none'}")
    summary(results, failures)
    if args.json:
        Path(args.json).write_text(json.dumps(results, indent=2, default=str))


if __name__ == "__main__":
    main()
