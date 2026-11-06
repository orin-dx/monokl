#!/usr/bin/env python3
"""Compares `monokl search` with ripgrep on pinned public repositories.

For each sampled identifier, ripgrep's whole-word file list is the ground truth. Recall is the share of those files that
`monokl search` also returns, so a file the tool silently loses shows up as a miss. Precision and wall time are reported
but not gated. Repositories are cloned on demand into target/rg-compare.
"""

import argparse
import json
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "scripts" / "rg-compare.repos"
CACHE = ROOT / "target" / "rg-compare"
GLOBS = ["*.ts", "*.tsx", "*.mts", "*.cts", "*.js", "*.jsx", "*.mjs", "*.cjs"]
DECLARATION = r"\b(?:function|class|interface|type|enum|const)\s+([A-Za-z_][A-Za-z0-9_]{3,})"
UNCAPPED = "1000000"


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


def rg_files(term, path):
    args = ["rg", "--files-with-matches", "--fixed-strings", "--ignore-case", "--word-regexp"]
    for glob in GLOBS:
        args += ["--glob", glob]
    out = run([*args, "--", term, "."], cwd=path, check=False).stdout
    return {line.removeprefix("./") for line in out.splitlines() if line}


def monokl_files(binary, term, path):
    out = run(
        [binary, "search", term, ".", "--tests", "--max-results", UNCAPPED, "--max-tokens", UNCAPPED,
         "--max-bytes", UNCAPPED],
        cwd=path,
        check=False,
    ).stdout
    files = set()
    for line in out.splitlines():
        if line.startswith("  ") and "," in line:
            files.add(line.strip().split(",", 1)[0])
    return files


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


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", default=str(ROOT / "target" / "release" / "monokl"))
    parser.add_argument("--repo", action="append", help="limit to a repository name from rg-compare.repos")
    parser.add_argument("--terms", type=int, default=25, help="identifiers sampled per repository")
    parser.add_argument("--min-recall", type=float, default=1.0, help="fail when overall recall is lower")
    parser.add_argument("--json", help="write the full result to this path")
    args = parser.parse_args()

    if not Path(args.binary).exists():
        sys.exit(f"{args.binary} not found; run `cargo build --release` or pass --binary")

    report = []
    for name, url, sha in repositories(args.repo):
        path = checkout(name, url, sha)
        rows = []
        for term in sample_terms(path, args.terms):
            truth, rg_ms = timed(rg_files, term, path)
            found, monokl_ms = timed(monokl_files, args.binary, term, path)
            rows.append(
                {
                    "term": term,
                    "truth": len(truth),
                    "hit": len(truth & found),
                    "extra": len(found - truth),
                    "missed": sorted(truth - found),
                    "rg_ms": rg_ms,
                    "monokl_ms": monokl_ms,
                }
            )
        report.append({"repo": name, "sha": sha, "rows": rows})

    truth_total = hit_total = 0
    for repo in report:
        rows = repo["rows"]
        truth = sum(r["truth"] for r in rows)
        hit = sum(r["hit"] for r in rows)
        truth_total += truth
        hit_total += hit
        extra = sum(r["extra"] for r in rows)
        rg_ms = statistics.median(r["rg_ms"] for r in rows)
        monokl_ms = statistics.median(r["monokl_ms"] for r in rows)
        print(
            f"{repo['repo']:<10} terms={len(rows):<3} recall={hit}/{truth} ({hit / max(truth, 1):.1%}) "
            f"extra_files={extra} median_ms rg={rg_ms:.0f} monokl={monokl_ms:.0f}"
        )
        misses = {}
        for row in rows:
            for file in row["missed"]:
                misses.setdefault(file, []).append(row["term"])
        for file, terms in sorted(misses.items(), key=lambda kv: -len(kv[1]))[:10]:
            print(f"    missed {file}  ({len(terms)} terms, e.g. {terms[0]})")

    recall = hit_total / max(truth_total, 1)
    print(f"overall recall {hit_total}/{truth_total} = {recall:.2%} (gate {args.min_recall:.2%})")
    if args.json:
        Path(args.json).write_text(json.dumps(report, indent=2))
    sys.exit(0 if recall >= args.min_recall else 1)


if __name__ == "__main__":
    main()
