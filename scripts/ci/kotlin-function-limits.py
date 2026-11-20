#!/usr/bin/env python3
"""Fails when detekt's function and class limits of the Kotlin code are loosened, or when the code
excused from them grows.

The limits are detekt's defaults for `LongMethod`, `LongParameterList`, `CyclomaticComplexMethod`,
`NestedBlockDepth`, `LargeClass` and `TooManyFunctions`; `scripts/kotlin-plugin-tests.sh` runs them.
Three ways to loosen them are refused here:

- the config may only switch rules off (`active: false`), never set a value, and never switch off
  one of the limits or their rule set, `complexity`;
- a `@Suppress` naming a limit stays silent forever, so the one way to excuse code is the baseline;
- the baseline's count of each rule may not grow against the commit where HEAD leaves
  `origin/main` (their merge-base): moving an excused function to another file, as a file split
  does, keeps the count, and a release of an older tag keeps the excuses it was built with.

Usage: kotlin-function-limits.py [checkout] [main]   (defaults: the current directory, origin/main)
Exit:  0 the limits hold · 1 a setting, a suppression or an excuse breaks them, one line each
       2 cannot judge: not a git checkout, or main is missing or shares no history with HEAD
"""

from __future__ import annotations

import re
import sys
from collections import Counter
from typing import List, Optional, Tuple

from ratchet import git, merge_base, run_check

TESTS_ROOT = "crates/tauri-plugin-erplora-android/android-tests"
CONFIG = f"{TESTS_ROOT}/detekt.yml"
BASELINE = f"{TESTS_ROOT}/detekt-baseline.xml"
RULE_SET = "complexity"
LIMITS = {
    "LongMethod",
    "LongParameterList",
    "CyclomaticComplexMethod",
    "NestedBlockDepth",
    "LargeClass",
    "TooManyFunctions",
}
# detekt reads `Rule`, `detekt:Rule`, `complexity:Rule`, the rule set and `all` as suppressions.
SILENCED = LIMITS | {RULE_SET, "all"}
SUPPRESS = re.compile(r"@(?:file:)?Suppress(?:Warnings)?\s*\(([^)]*)\)")
BASELINE_ID = re.compile(r"<ID>\s*([A-Za-z]+):")
KEY = re.compile(r"^\s*([A-Za-z][\w-]*)\s*:\s*(.*?)\s*$")

Problem = Tuple[str, str]


def config_problems() -> List[Problem]:
    try:
        with open(CONFIG, encoding="utf-8") as handle:
            lines = handle.read().splitlines()
    except FileNotFoundError:
        return [(CONFIG, f"{CONFIG} is missing: detekt would run every default rule, not only the limits")]
    problems = []
    keys: List[Tuple[int, str]] = []
    for number, line in enumerate(lines, start=1):
        content = line.split("#", 1)[0].rstrip()
        if not content:
            continue
        indent = len(content) - len(content.lstrip())
        while keys and keys[-1][0] >= indent:
            keys.pop()
        problem = config_line_problem([key for _, key in keys], content.strip())
        if problem:
            problems.append((f"{CONFIG}:{number}", problem))
        match = KEY.match(content)
        if match:
            keys.append((indent, match.group(1)))
    return problems


def config_line_problem(parents: List[str], content: str) -> Optional[str]:
    match = KEY.match(content)
    if match and not match.group(2):
        return None
    if not match or (match.group(1), match.group(2)) != ("active", "false"):
        return f"detekt.yml only switches rules off (active: false), never sets a value: {content}"
    switched_off = parents[-1] if parents else ""
    if parents[:1] == [RULE_SET] and (len(parents) == 1 or switched_off in LIMITS):
        return f"detekt.yml switches off {switched_off}, one of the function and class limits or their rule set"
    return None


def suppression_problems() -> List[Problem]:
    problems = []
    listed = git("-c", "core.quotepath=off", "ls-files", "-z", "*.kt", "*.kts").stdout.decode()
    for path in filter(None, listed.split("\0")):
        try:
            with open(path, encoding="utf-8") as handle:
                text = handle.read()
        except FileNotFoundError:
            continue
        problems += file_suppressions(path, text)
    return problems


def file_suppressions(path: str, text: str) -> List[Problem]:
    problems = []
    for match in SUPPRESS.finditer(text):
        line_start = text.rfind("\n", 0, match.start()) + 1
        if text[line_start : match.start()].lstrip().startswith(("//", "/*", "*")):
            continue
        number = text.count("\n", 0, match.start()) + 1
        for name in re.findall(r'"([^"]*)"', match.group(1)):
            rule = re.split(r"[:.]", name)[-1]
            if rule in SILENCED:
                problems.append(
                    (f"{path}:{number}", f"@Suppress silences {rule}: an excuse goes in {BASELINE}, and only shrinks")
                )
    return problems


def excused_by_rule(text: str) -> Counter:
    return Counter(BASELINE_ID.findall(text))


def baseline_problems(main: str) -> List[Problem]:
    base = merge_base(main, "the excuses")
    base_label = f"{main}@{git('rev-parse', '--short', base).stdout.decode().strip()}"
    shown = git("cat-file", "blob", f"{base}:{BASELINE}")
    if shown.returncode != 0:
        # Only true on the commit that brings the baseline: after it, the base always has one.
        print(f"{base_label} has no {BASELINE}: the excused code starts here")
        return []
    before = excused_by_rule(shown.stdout.decode())
    try:
        with open(BASELINE, encoding="utf-8") as handle:
            now = excused_by_rule(handle.read())
    except FileNotFoundError:
        return []
    return [
        (BASELINE, f"{rule}: {count} excused in {BASELINE}, {base_label} has {before[rule]}")
        for rule, count in sorted(now.items())
        if count > before[rule]
    ]


def judge(main: str) -> int:
    problems = config_problems() + suppression_problems() + baseline_problems(main)
    for where, problem in problems:
        path, _, line = where.partition(":")
        location = f"file={path},line={line}" if line else f"file={path}"
        print(f"::error {location}::{where}: {problem}")
    if problems:
        return 1
    print("detekt's function and class limits run at their defaults, and the excused code only shrinks")
    return 0


if __name__ == "__main__":
    sys.exit(run_check(sys.argv, judge))
