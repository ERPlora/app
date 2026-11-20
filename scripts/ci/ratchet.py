"""Where a limit check of scripts/ci runs, and the base its exception list only shrinks against.

A check runs from the top of a git checkout and compares its list with the commit where HEAD leaves
`main` (their merge-base, not main's tip), so a release of an older tag keeps the exceptions it was
built with. When the checkout or that commit is missing it exits 2: a check that cannot judge never
passes in silence.
"""

from __future__ import annotations

import os
import subprocess
import sys
from typing import Callable, List


class Unjudgeable(Exception):
    """The check cannot judge this checkout; exits 2."""


def git(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(["git", *args], capture_output=True, check=False)


def merge_base(main: str, what: str) -> str:
    """The commit where HEAD leaves `main`; `what` names the list in the error, as in "the list"."""
    if git("rev-parse", "--verify", "--quiet", f"{main}^{{commit}}").returncode != 0:
        raise Unjudgeable(f"{main} is not in this checkout: run git fetch origin main ({what} can only shrink against it)")
    found = git("merge-base", "HEAD", main)
    if found.returncode != 0:
        raise Unjudgeable(
            f"HEAD and {main} share no history in this checkout: fetch the whole history "
            f"(git fetch --unshallow origin) so {what} has a base to shrink against"
        )
    return found.stdout.decode().strip()


def run_check(argv: List[str], judge: Callable[[str], int]) -> int:
    """Runs `judge(main)` from the top of the checkout: `argv` is `[program, checkout, main]`, both
    optional (the current directory and origin/main)."""
    root = argv[1] if len(argv) > 1 else "."
    main_ref = argv[2] if len(argv) > 2 else "origin/main"
    top = subprocess.run(["git", "-C", root, "rev-parse", "--show-toplevel"], capture_output=True, check=False)
    if top.returncode != 0:
        print(f"::error::{root} is not a git checkout", file=sys.stderr)
        return 2
    try:
        os.chdir(top.stdout.decode().strip())
        return judge(main_ref)
    except Unjudgeable as error:
        print(f"::error::{error}", file=sys.stderr)
        return 2
