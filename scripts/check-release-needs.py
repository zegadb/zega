#!/usr/bin/env python3
"""Fail the release check when required jobs fail or skip on release refs."""

import json
import os
import sys


def failed_jobs(needs: dict, full_matrix: bool) -> dict:
    # changes is PR-only; package-types is also intentionally PR-only.
    allowed_skips = {"changes", "package-types"}
    if not full_matrix:
        allowed_skips.update({"package", "native", "publish"})
    return {
        name: job["result"]
        for name, job in needs.items()
        if job["result"] != "success"
        and not (job["result"] == "skipped" and name in allowed_skips)
    }


def main() -> int:
    needs = json.loads(os.environ["NEEDS_JSON"])
    full_matrix = os.environ["FULL_MATRIX"] == "true"
    bad = failed_jobs(needs, full_matrix)
    if bad:
        print("failed upstream jobs:", bad)
        return 1
    print("ok:", {name: job["result"] for name, job in needs.items()})
    return 0


if __name__ == "__main__":
    sys.exit(main())
