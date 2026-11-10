#!/usr/bin/env python3
"""Times `monokl search` against `rg` on pinned repositories and on a synthetic stress set. Not a correctness check.

Per (repo, query class, tool): `--runs` timed runs after `--warmups` untimed ones, reported as p50 and p95 wall ms, the
peak resident set size over the runs (from `/usr/bin/time`, macOS `-l` or Linux `-v`) and the bytes of output.
Everything is warm: the warm-ups and repeated runs leave the files in the OS page cache, and cold-cache runs are not
measured. A run that exits with a status above 1 or dies is a crash and a run over 60 s is a TIMEOUT; either makes the
script exit non-zero. There are no thresholds.

Query classes: identifier (a declared name), words (three declared names), broad (a very common word), regex
(`regex:\\bPREFIX\\w+`), nomatch (a word that occurs nowhere) and path (the broad word restricted to the directory
that declares the identifier). Identifiers are sampled from top-level declarations as rg-compare.py does, so the choice
is deterministic for a pinned checkout.

The stress set is generated into target/bench/stress: a 5 MB single-line JavaScript file, a 50,000-line generated
TypeScript file and 20,000 small files that each contain `needle`. `needle` is searched in each, with rg for comparison.

Repositories are cloned shallow into target/bench on demand.
"""

import argparse
import json
import os
import re
import signal
import statistics
import importlib.util
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "scripts" / "bench.repos"
CACHE = ROOT / "target" / "bench"
# rg-compare.py holds the mirror of monokl's test-path rule; its file name has a hyphen, so load it by path.
_spec = importlib.util.spec_from_file_location("rg_compare", ROOT / "scripts" / "rg-compare.py")
RG_COMPARE = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(RG_COMPARE)
TIMEOUT_S = 60
NO_MATCH = "zqxjvkwpnomatchzz"
DARWIN = sys.platform == "darwin"
TIME = ["/usr/bin/time", "-l" if DARWIN else "-v"]

TS_GLOBS = ["*.ts", "*.tsx", "*.mts", "*.cts", "*.js", "*.jsx", "*.mjs", "*.cjs", "!*.d.ts", "!*.d.mts", "!*.d.cts"]
# language -> (rg globs, top-level declaration regex, broad word). The first two are copied from rg-compare.py.
LANGS = {
    "ts": (
        TS_GLOBS,
        r"^(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(?:function\*?|class|interface|type|enum|const|let|var)\s+",
        "string",
    ),
    "rust": (
        ["*.rs"],
        r"^(?:pub(?:\([a-z:]+\))?\s+)?(?:(?:const|async|unsafe|extern(?:\s+\"[A-Za-z]+\")?)\s+)*(?:fn|struct|enum|trait|type|const|static|mod|macro_rules!)\s+",
        "error",
    ),
}


def run(args, cwd=None, check=True):
    return subprocess.run(args, cwd=cwd, capture_output=True, text=True, check=check)


def repositories(only):
    for line in MANIFEST.read_text().splitlines():
        if line.strip() and not line.startswith("#"):
            name, url, sha, *language = line.split()
            if not only or name in only:
                yield name, url, sha, language[0] if language else "ts"


def checkout(name, url, sha):
    path = CACHE / name
    if (path / ".git").exists() and run(["git", "rev-parse", "HEAD"], cwd=path).stdout.strip() == sha:
        return path
    path.mkdir(parents=True, exist_ok=True)
    run(["git", "init", "-q"], cwd=path)
    run(["git", "fetch", "-q", "--depth", "1", url, sha], cwd=path)
    run(["git", "checkout", "-q", "FETCH_HEAD"], cwd=path)
    return path


def glob_args(globs):
    return [arg for glob in globs for arg in ("--glob", glob)]


def sample_terms(path, count, globs, declaration, language):
    """Declared names from files monokl searches: test paths are skipped, and declaration files are not globbed."""
    args = ["rg", "--with-filename", "--only-matching", "--no-line-number", "--replace", "$1", *glob_args(globs)]
    out = run([*args, "-e", declaration + r"([A-Za-z_][A-Za-z0-9_]{3,})", "."], cwd=path, check=False).stdout
    lang = RG_COMPARE.LANGS[language]
    names = set()
    for line in out.splitlines():
        file, _, name = line.rpartition(":")
        if not RG_COMPARE.is_test_path(file.removeprefix("./"), lang):
            names.add(name)
    names = sorted(names - {"tests"})
    if len(names) <= count:
        return names
    step = len(names) / count
    return [names[int(i * step)] for i in range(count)]


def declaring_dir(term, path, globs, declaration):
    out = run(["rg", "--files-with-matches", *glob_args(globs), "-e", declaration + re.escape(term) + r"\b", "."],
              cwd=path, check=False).stdout
    files = sorted(line.removeprefix("./") for line in out.splitlines() if line)
    return (os.path.dirname(files[0]) or ".") if files else "."


def queries(path, language):
    """The six query classes as (class, monokl args, rg argv), all relative to `path`, and the chosen terms."""
    globs, declaration, broad = LANGS[language]
    terms = sample_terms(path, 7, globs, declaration, language)
    if len(terms) < 6:
        sys.exit(f"{path}: fewer than 6 declared identifiers to sample")
    ident = terms[3]
    words = [terms[1], terms[3], terms[5]]
    pattern = rf"\b{ident[:4]}\w+"
    subdir = declaring_dir(ident, path, globs, declaration)
    g = glob_args(globs)
    cases = [
        ("identifier", ["search", ident, "."], ["rg", "-n", *g, "-e", ident, "."]),
        ("words", ["search", " ".join(words), "."], ["rg", "-n", *g, *[a for w in words for a in ("-e", w)], "."]),
        ("broad", ["search", broad, "."], ["rg", "-n", *g, "-e", broad, "."]),
        ("regex", ["search", f"regex:{pattern}", "."], ["rg", "-n", *g, "-e", pattern, "."]),
        ("nomatch", ["search", NO_MATCH, "."], ["rg", "-n", *g, "-e", NO_MATCH, "."]),
        ("path", ["search", broad, subdir], ["rg", "-n", *g, "-e", broad, subdir]),
    ]
    return cases, {"identifier": ident, "words": words, "regex": pattern, "broad": broad, "path": subdir}


def parse_rss_mb(stderr):
    if DARWIN:
        m = re.search(r"(\d+)\s+maximum resident set size", stderr)
        return int(m.group(1)) / 1e6 if m else None
    m = re.search(r"Maximum resident set size \(kbytes\): (\d+)", stderr)
    return int(m.group(1)) / 1e3 if m else None


def once(argv, cwd):
    """One run under /usr/bin/time. Returns (status, wall ms, peak RSS MB, stdout bytes); status is ok, TIMEOUT or crash."""
    start = time.perf_counter()
    proc = subprocess.Popen([*TIME, *argv], cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            start_new_session=True)
    try:
        out, err = proc.communicate(timeout=TIMEOUT_S)
    except subprocess.TimeoutExpired:
        os.killpg(proc.pid, signal.SIGKILL)
        proc.communicate()
        return "TIMEOUT", TIMEOUT_S * 1000.0, None, 0
    wall = (time.perf_counter() - start) * 1000
    # 0 is a match and 1 is no match for rg; anything else is a crash or a signal.
    status = "ok" if proc.returncode in (0, 1) else f"crash({proc.returncode})"
    return status, wall, parse_rss_mb(err.decode(errors="replace")), len(out)


def measure(argv, cwd, runs, warmups):
    for _ in range(warmups):
        status, *_ = once(argv, cwd)
        if status != "ok":
            return {"status": status}
    walls, rss, size = [], [], 0
    for _ in range(runs):
        status, wall, mb, size = once(argv, cwd)
        if status != "ok":
            return {"status": status}
        walls.append(wall)
        if mb is not None:
            rss.append(mb)
    walls.sort()
    return {
        "status": "ok",
        "p50_ms": statistics.median(walls),
        "p95_ms": walls[min(len(walls) - 1, max(0, round(0.95 * len(walls)) - 1))],
        "peak_rss_mb": max(rss) if rss else None,
        "output_bytes": size,
    }


def format_row(r):
    if r["status"] != "ok":
        return f"{r['repo']:<16} {r['query']:<11} {r['tool']:<7} {r['status']}"
    rss = f"{r['peak_rss_mb']:.0f}" if r["peak_rss_mb"] is not None else "-"
    return (f"{r['repo']:<16} {r['query']:<11} {r['tool']:<7} {r['p50_ms']:>9.1f} {r['p95_ms']:>9.1f} {rss:>8} "
            f"{r['output_bytes']:>10}")


def bench_set(label, cases, cwd, binary, runs, warmups, rows):
    """`cases` is (class, monokl args, rg argv); each runs for both tools."""
    for cls, monokl_args, rg_argv in cases:
        for tool, argv in (("monokl", [binary, *monokl_args]), ("rg", rg_argv)):
            rows.append({"repo": label, "query": cls, "tool": tool, **measure(argv, cwd, runs, warmups)})
            print(format_row(rows[-1]), flush=True)


def generate_stress():
    """Writes the deterministic stress files once; the marker file says they are complete."""
    root = CACHE / "stress"
    marker = root / ".generated"
    if marker.exists():
        return root
    bigline = root / "bigline"
    bigline.mkdir(parents=True, exist_ok=True)
    parts, size, i = [], 0, 0
    while size < 5_000_000:
        piece = f"var v{i}=function(a,b){{return a+b+{i}}};"
        parts.append(piece)
        size += len(piece)
        i += 1
    (bigline / "bundle.js").write_text("".join(parts) + "var needle=1;")
    biggen = root / "biggen"
    biggen.mkdir(exist_ok=True)
    lines = [f"export function fn{i}(a: number, b: string): string {{ return b + a + {i}; }}" for i in range(49_999)]
    lines.append("export const needle = 1;")
    (biggen / "generated.ts").write_text("\n".join(lines) + "\n")
    for d in range(200):
        folder = root / "many" / f"d{d:03}"
        folder.mkdir(parents=True, exist_ok=True)
        for f in range(100):
            (folder / f"f{f:03}.ts").write_text(f"export const value{d}_{f} = 'needle';\n")
    marker.write_text("")
    return root


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", default=str(ROOT / "target" / "release" / "monokl"))
    parser.add_argument("--repo", action="append",
                        help="limit to a repository name from bench.repos, or `stress` for the synthetic set")
    parser.add_argument("--runs", type=int, default=15)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--json", help="write the full result to this path")
    args = parser.parse_args()
    if not Path(args.binary).exists():
        sys.exit(f"{args.binary} not found; run `cargo build --release` or pass --binary")
    if not os.access(TIME[0], os.X_OK):
        sys.exit(f"{TIME[0]} not found")

    print(f"warm cache only; {args.runs} runs after {args.warmups} warm-ups; rss is the peak over the runs")
    print(f"{'repo':<16} {'query':<11} {'tool':<7} {'p50 ms':>9} {'p95 ms':>9} {'rss MB':>8} {'out bytes':>10}")
    rows, chosen = [], {}
    for name, url, sha, language in repositories(args.repo):
        path = checkout(name, url, sha)
        cases, chosen[name] = queries(path, language)
        bench_set(name, cases, path, args.binary, args.runs, args.warmups, rows)
    if not args.repo or "stress" in args.repo:
        root = generate_stress()
        for sub in ("bigline", "biggen", "many"):
            cases = [("needle", ["search", "needle", "."], ["rg", "-n", "-e", "needle", "."])]
            bench_set(f"stress/{sub}", cases, root / sub, args.binary, args.runs, args.warmups, rows)

    if args.json:
        Path(args.json).write_text(json.dumps({"platform": sys.platform, "queries": chosen, "rows": rows}, indent=2))
    bad = [r for r in rows if r["status"] != "ok"]
    for r in bad:
        print(f"FAILED {r['repo']} {r['query']} {r['tool']}: {r['status']}", file=sys.stderr)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
