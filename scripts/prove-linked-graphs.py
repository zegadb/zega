#!/usr/bin/env python3
"""Revert-prove the APS 39 push, mailbox replay, and coalescing effects.

Runs sequentially in this clone. Only three specific implementation lines are
mutated, one at a time, and restored in finally. Never changes Git history.
Logs and compiler output stay under the clone's .tmp directory.
"""
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
os.chdir(ROOT)
TEMP = ROOT / ".tmp"
TEMP.mkdir(mode=0o700, exist_ok=True)
env = dict(os.environ, TMPDIR=str(TEMP), CARGO_TARGET_DIR=str(ROOT / ".target"))
CASES = [
    ("b", "zega-server/src/sync.rs", "match receive(&state, &runtime, diff).await {",
     "match { let _ = diff; Ok::<(), String>(()) } {",
     "b_update_arrives_by_push_within_six_seconds"),
    ("c", "zega-server/src/sync.rs", "receive(state, runtime, item.diff).await?;",
     "let _ = item.diff; // Revert proof: delete without applying.",
     "c_offline_subscriber_drains_mailbox_before_live_pushes"),
    ("e", "zega/src/linked.rs", "changes: nodes.into_values().collect(),",
     "changes: records.iter().flat_map(|r| r.changes.clone()).collect(),",
     "e_five_edits_send_exactly_one_field_level_diff"),
]
for scenario, relative, original, reverted, test in CASES:
    path = ROOT / relative
    before = path.read_text()
    assert before.count(original) == 1, (relative, original)
    try:
        path.write_text(before.replace(original, reverted))
        with (TEMP / f"revert-{scenario}.log").open("w") as log:
            result = subprocess.run(
                ["cargo", "test", "--locked", "-p", "zega-server", "--test", "linked_proof",
                 test, "--", "--exact", "--nocapture"],
                env=env, stdout=log, stderr=subprocess.STDOUT, check=False,
            )
        output = (TEMP / f"revert-{scenario}.log").read_text()
        assert result.returncode == 101 and f"test {test} ... FAILED" in output, output
        assert "could not compile" not in output, output
        print(f"REVERT {scenario} exit={result.returncode} expected_failure=true", flush=True)
    finally:
        path.write_text(before)
