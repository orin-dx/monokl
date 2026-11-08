#!/usr/bin/env python3
"""Compares `monokl search` with ripgrep on pinned public repositories.

monokl is meant to return the few right answers, not everything that matches. So ripgrep supplies the ground truth for
what is right, and the score rewards targeting:

- definitions: files that declare the identifier. The gate is whether one of them is among monokl's first `--top` files
  (hit@top), because an agent that asks for a symbol must be shown where it is declared, early.
- declaration_in_output: whether the declaration's own line appears in monokl's output, so the agent needs no second
  read of the file. Reported, not gated.
- cost: files and output size monokl returns with its default caps, against one `rg -n` call. rg wins on a bare
  identifier, so this is the floor to beat; monokl's case is the question that takes rg several calls.
- test files are left out of the lists, as monokl leaves them out by default. `--tests` also runs monokl with its test
  flag and reports how many test files with the word it returned, ungated.

Questions (scripts/rg-compare.questions) score the case monokl exists for: two to four words that take several rg calls,
with a known answer file and symbol. Each reports the answer file's rank, whether the declaration line is in the output,
the rg route's narrowing cost (files with any word, files with all words, bytes of one `rg -n` call for any word) and
monokl's output bytes. The gate is the answer file in monokl's first `--top` files for at least `--min-question-recall`
of the questions.

Member questions use the same file format with the symbol cell written `member:name`: the answer is a method,
constructor, accessor or arrow-function property declared on an indented line inside a class. The script re-checks that
line with rg and scores them on their own line, apart from the top-level questions. A member question is a hit when the
answer file is in monokl's first `--top` files and the member's declaration line is in the output; the gate is
`--min-member-recall`.

Wall time is reported. Repositories are cloned on demand into target/rg-compare.
"""

import argparse
import json
import re
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "scripts" / "rg-compare.repos"
QUESTIONS = ROOT / "scripts" / "rg-compare.questions"
CACHE = ROOT / "target" / "rg-compare"
# Declaration files are not searched by design (see `supports` in crates/monokl/src/analysis/ts/mod.rs).
GLOBS = ["*.ts", "*.tsx", "*.mts", "*.cts", "*.js", "*.jsx", "*.mjs", "*.cjs", "!*.d.ts", "!*.d.mts", "!*.d.cts"]
# A top-level declaration starts its line. Indented ones are locals and loop variables, which monokl does not index.
TOP_LEVEL = r"^(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(?:function\*?|class|interface|type|enum|const|let|var)\s+"
DECLARATION = TOP_LEVEL + r"([A-Za-z_][A-Za-z0-9_]{3,})"
# An indented class member: optional modifiers, then the name followed by a call signature or an arrow-function value.
MEMBER_MODIFIERS = r"^\s+(?:(?:static|async|public|private|protected|readonly|override|get|set)\s+)*\*?"
UNCAPPED ="1000000"
TEST_DIRS = ("/__tests__/", "/test/", "/tests/", "/spec/")
TEST_SUFFIXES = tuple(
    f".{kind}.{ext}" for kind in ("test", "spec") for ext in ("ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts")
)


def is_test_path(relative):
    """Mirrors `is_test_path` in crates/monokl/src/text_search.rs; change both together."""
    path = "/" + relative
    return any(d in path for d in TEST_DIRS) or path.endswith(TEST_SUFFIXES)


def run(args, cwd=None, check=True):
    return subprocess.run(args, cwd=cwd, capture_output=True, text=True, check=check)


def repositories(only):
    for line in MANIFEST.read_text().splitlines():
        if line.strip() and not line.startswith("#"):
            name, url, sha = line.split()
            if not only or name in only:
                yield name, url, sha


def checkout(name, url, sha):
    path = CACHE / name
    if (path / ".git").exists() and run(["git", "rev-parse", "HEAD"], cwd=path).stdout.strip() == sha:
        return path
    path.mkdir(parents=True, exist_ok=True)
    run(["git", "init", "-q"], cwd=path)
    run(["git", "fetch", "-q", "--depth", "1", url, sha], cwd=path)
    run(["git", "checkout", "-q", "FETCH_HEAD"], cwd=path)
    return path


def rg_files(pattern, path, extra=()):
    args = ["rg", "--files-with-matches", *extra]
    for glob in GLOBS:
        args += ["--glob", glob]
    out = run([*args, "-e", pattern, "."], cwd=path, check=False).stdout
    return {line.removeprefix("./") for line in out.splitlines() if line}


def mentions(term, path):
    return rg_files(re.escape(term), path, ["--ignore-case", "--word-regexp"])


def declaration_pattern(term, member=False):
    name = re.escape(term)
    if member:
        call = rf"{MEMBER_MODIFIERS}{name}\s*(?:<[^>]*>)?\("
        arrow = rf"{MEMBER_MODIFIERS}{name}\s*(?::[^=]+)?=\s*(?:async\s*)?(?:\(|\w+\s*=>)"
        return f"{call}|{arrow}"
    return rf"{TOP_LEVEL}{name}\b"


def definitions(term, path, member=False):
    return rg_files(declaration_pattern(term, member), path)


def monokl_files(binary, term, path, tests):
    """Runs monokl with its default caps, as an agent would, and returns the files in rank order and the output size."""
    args = [binary, "search", term, "."]
    if tests:
        args.append("--tests")
    out = run(args, cwd=path, check=False).stdout
    files = []
    for line in out.splitlines():
        if line.startswith("  ") and "," in line:
            file = line.strip().split(",", 1)[0].removeprefix("./")
            if file not in files:
                files.append(file)
    return files, out


def declaration_line(term, files, path, member=False):
    """The text of the first top-level (or, for a member, indented) declaration of `term` in `files`, as ripgrep finds it."""
    for file in sorted(files):
        out = run(["rg", "--no-filename", "--max-count", "1", "-e", declaration_pattern(term, member), file], cwd=path,
                  check=False).stdout
        if out.strip():
            return out.strip()
    return None


def rg_output_bytes(term, path):
    """Size of the lines an agent reads from one `rg -n` call, the cheapest rg route to the same files."""
    args = ["rg", "-n", "--word-regexp", "--ignore-case"]
    for glob in GLOBS:
        args += ["--glob", glob]
    return len(run([*args, "-e", re.escape(term), "."], cwd=path, check=False).stdout.encode())


def sample_terms(path, count):
    args = ["rg", "--no-filename", "--only-matching", "--no-line-number", "--replace", "$1"]
    for glob in GLOBS:
        args += ["--glob", glob]
    out = run([*args, "-e", DECLARATION, "."], cwd=path, check=False).stdout
    names = sorted(set(out.split()))
    if len(names) <= count:
        return names
    step = len(names) / count
    return [names[int(i * step)] for i in range(count)]


def timed(fn, *args):
    start = time.perf_counter()
    result = fn(*args)
    return result, (time.perf_counter() - start) * 1000


def split(files):
    tests = {f for f in files if is_test_path(f)}
    return set(files) - tests, tests


def first_rank(ordered, wanted):
    """1-based position of the first file in `ordered` that is in `wanted`, or None."""
    return next((i for i, f in enumerate(ordered, 1) if f in wanted), None)


def question_rows(only):
    for line in QUESTIONS.read_text().splitlines():
        if line.strip() and not line.startswith("#"):
            repo, words, file, symbol = (cell.strip() for cell in line.split("|"))
            member = symbol.startswith("member:")
            if not only or repo in only:
                yield repo, words, file, symbol.removeprefix("member:"), member


def score_questions(binary, only):
    """Runs every question and returns one row each. Fails if the answer's declaration is not in the pinned checkout."""
    checkouts = {name: checkout(name, url, sha) for name, url, sha in repositories(only)}
    rows = []
    for repo, words, file, symbol, member in question_rows(only):
        path = checkouts[repo]
        if file not in definitions(symbol, path, member):
            kind = "a class member" if member else "declared"
            sys.exit(f"{repo}: `{symbol}` is not {kind} in {file} at the pinned sha; fix rg-compare.questions")
        terms = words.split()
        per_word = [split(rg_files(re.escape(t), path, ["--ignore-case", "--word-regexp"]))[0] for t in terms]
        rg_args = ["rg", "-n", "--ignore-case", "--word-regexp"]
        for glob in GLOBS:
            rg_args += ["--glob", glob]
        for term in terms:
            rg_args += ["-e", re.escape(term)]
        rg_out = run([*rg_args, "."], cwd=path, check=False).stdout
        rg_lines = [l for l in rg_out.splitlines() if not is_test_path(l.removeprefix("./").split(":", 1)[0])]
        ordered, output = monokl_files(binary, words, path, False)
        non_test = [f for f in ordered if not is_test_path(f)]
        declaration = declaration_line(symbol, {file}, path, member)
        bare = str.maketrans("", "", '"\\')
        bare_declaration = re.sub(r"^(?:export\s+|default\s+|declare\s+)+", "", declaration).rstrip(" {;")
        rows.append(
            {
                "repo": repo,
                "words": words,
                "file": file,
                "symbol": symbol,
                "member": member,
                "rank": non_test.index(file) + 1 if file in non_test else None,
                "declaration_in_output": bare_declaration[:40].translate(bare) in output.translate(bare),
                "rg_any_files": len(set.union(*per_word)),
                "rg_all_files": len(set.intersection(*per_word)),
                "rg_n_bytes": len("\n".join(rg_lines).encode()),
                "monokl_bytes": len(output.encode()),
            }
        )
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", default=str(ROOT / "target" / "release" / "monokl"))
    parser.add_argument("--repo", action="append", help="limit to a repository name from rg-compare.repos")
    parser.add_argument("--terms", type=int, default=25, help="identifiers sampled per repository")
    parser.add_argument("--top", type=int, default=3, help="a declaration must appear within this many files")
    parser.add_argument("--min-recall", type=float, default=0.9, help="fail when the share of hits is lower")
    parser.add_argument("--min-question-recall", type=float, default=0.75,
                        help="fail when the share of questions with the answer file in the top files is lower")
    parser.add_argument("--min-member-recall", type=float, default=0.125,
                        help="fail when the share of member questions with the answer file in the top files and the "
                             "member's declaration in the output is lower")
    parser.add_argument("--no-questions", action="store_true", help="skip the question set")
    parser.add_argument("--tests", action="store_true", help="also score test files, ungated")
    parser.add_argument("--json", help="write the full result to this path")
    args = parser.parse_args()

    if not Path(args.binary).exists():
        sys.exit(f"{args.binary} not found; run `cargo build --release` or pass --binary")

    report = []
    for name, url, sha in repositories(args.repo):
        path = checkout(name, url, sha)
        rows = []
        for term in sample_terms(path, args.terms):
            defs, def_tests = split(definitions(term, path))
            seen, seen_tests = split(mentions(term, path))
            (ordered, output), monokl_ms = timed(monokl_files, args.binary, term, path, args.tests)
            monokl_bytes = len(output.encode())
            declaration = declaration_line(term, defs, path)
            # The first 40 characters of the declaration's line, because a signature or snippet may cut a long one.
            # TOON quotes and escapes cells, so compare with quotes and backslashes removed on both sides.
            bare = str.maketrans("", "", '"\\')
            # A signature drops the opening brace, so compare without trailing braces and semicolons.
            # and its export, default and declare keywords.
            wanted = None
            if declaration:
                bare_declaration = re.sub(r"^(?:export\s+|default\s+|declare\s+)+", "", declaration).rstrip(" {;")
                wanted = bare_declaration[:40].translate(bare)
            answered = wanted is not None and wanted in output.translate(bare)
            non_test = [f for f in ordered if not is_test_path(f)]
            found, found_tests = split(ordered)
            _, rg_ms = timed(mentions, term, path)
            rank = first_rank(non_test, defs)
            rows.append(
                {
                    "term": term,
                    "defs": len(defs),
                    "def_rank": rank,
                    "answered": answered,
                    "returned": len(non_test),
                    "rg_files": len(seen | defs),
                    "monokl_bytes": monokl_bytes,
                    "rg_bytes": rg_output_bytes(term, path),
                    "tests": len(seen_tests | def_tests),
                    "tests_hit": len((seen_tests | def_tests) & found_tests),
                    "rg_ms": rg_ms,
                    "monokl_ms": monokl_ms,
                    "top": non_test[:3],
                }
            )
        report.append({"repo": name, "sha": sha, "rows": rows})

    scored = hits = 0
    for repo in report:
        rows = [r for r in repo["rows"] if r["defs"]]
        repo_hits = sum(1 for r in rows if r["def_rank"] is not None and r["def_rank"] <= args.top)
        scored += len(rows)
        hits += repo_hits
        ranks = [r["def_rank"] for r in rows if r["def_rank"] is not None]
        returned = sum(r["returned"] for r in repo["rows"])
        rg_files = sum(r["rg_files"] for r in repo["rows"])
        line = (
            f"{repo['repo']:<10} defined_terms={len(rows):<3} hit@{args.top}={repo_hits}/{len(rows)} "
            f"median_rank={statistics.median(ranks) if ranks else '-'} "
            f"declaration_in_output={sum(1 for r in rows if r['answered'])}/{len(rows)} "
            f"files={returned} vs rg={rg_files} "
            f"output_kb monokl={sum(r['monokl_bytes'] for r in repo['rows']) / 1024:.0f} "
            f"rg_n={sum(r['rg_bytes'] for r in repo['rows']) / 1024:.0f} "
            f"median_ms rg={statistics.median(r['rg_ms'] for r in repo['rows']):.0f} "
            f"monokl={statistics.median(r['monokl_ms'] for r in repo['rows']):.0f}"
        )
        if args.tests:
            line += f" tests={sum(r['tests_hit'] for r in repo['rows'])}/{sum(r['tests'] for r in repo['rows'])}"
        print(line)
        for row in rows:
            if row["def_rank"] is None or row["def_rank"] > args.top:
                rank = row["def_rank"] or "absent"
                print(f"    declaration at rank {rank}: {row['term']} (top: {', '.join(row['top']) or 'nothing'})")

    # A gate with nothing to score is skipped and says so, instead of failing on a rate of 0/0.
    identifiers_pass = True
    if scored:
        rate = hits / scored
        identifiers_pass = rate >= args.min_recall
        print(f"declaration in top {args.top}: {hits}/{scored} = {rate:.2%} (gate {args.min_recall:.2%})")
    else:
        print("declaration in top: no declarations to score, gate skipped")

    scored_questions = [] if args.no_questions else score_questions(args.binary, args.repo)
    questions = [q for q in scored_questions if not q["member"]]
    members = [q for q in scored_questions if q["member"]]
    question_hits = 0
    for q in questions:
        in_top = q["rank"] is not None and q["rank"] <= args.top
        question_hits += in_top
        print(
            f"  {'ok  ' if in_top else 'MISS'} {q['repo']}: {q['words']!r} -> {q['symbol']} "
            f"file_rank={q['rank'] or 'absent'} declaration_in_output={'yes' if q['declaration_in_output'] else 'no'} "
            f"rg_files any={q['rg_any_files']} all={q['rg_all_files']} rg_n_bytes={q['rg_n_bytes']} "
            f"monokl_bytes={q['monokl_bytes']}"
        )
    questions_pass = True
    if questions:
        question_rate = question_hits / len(questions)
        questions_pass = question_rate >= args.min_question_recall
        print(f"question answer file in top {args.top}: {question_hits}/{len(questions)} = {question_rate:.2%} "
              f"(gate {args.min_question_recall:.2%})")
    else:
        print("question answer file in top: no questions to score, gate skipped")
    member_hits = 0
    for q in members:
        hit = q["rank"] is not None and q["rank"] <= args.top and q["declaration_in_output"]
        member_hits += hit
        print(
            f"  {'ok  ' if hit else 'MISS'} {q['repo']}: {q['words']!r} -> member {q['symbol']} "
            f"file_rank={q['rank'] or 'absent'} declaration_in_output={'yes' if q['declaration_in_output'] else 'no'} "
            f"monokl_bytes={q['monokl_bytes']}"
        )
    members_pass = True
    if members:
        member_rate = member_hits / len(members)
        members_pass = member_rate >= args.min_member_recall
        print(f"member questions: {member_hits}/{len(members)} = {member_rate:.2%} (gate {args.min_member_recall:.2%})")
    else:
        print("member questions: no member questions to score, gate skipped")
    if args.json:
        Path(args.json).write_text(json.dumps({"identifiers": report, "questions": questions, "members": members}, indent=2))
    sys.exit(0 if identifiers_pass and questions_pass and members_pass else 1)


if __name__ == "__main__":
    main()
