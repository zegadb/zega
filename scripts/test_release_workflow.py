import importlib.util
import re
import unittest
from pathlib import Path


SPEC = importlib.util.spec_from_file_location(
    "check_release_needs", Path(__file__).with_name("check-release-needs.py")
)
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


def job_condition(workflow, job_name):
    lines = (Path(__file__).parents[1] / ".github/workflows" / workflow).read_text().splitlines()
    start = lines.index(f"  {job_name}:") + 1
    for line in lines[start:]:
        if line.startswith("  ") and not line.startswith("    ") and line.rstrip().endswith(":"):
            break
        match = re.match(r"    if: (.+)$", line)
        if match:
            return match.group(1)
    raise AssertionError(f"no inline if condition for {job_name}")


def evaluate_condition(expression, needs, github):
    expression = expression.replace("always()", "True").replace("&&", "and").replace("||", "or")
    pattern = r"needs\.([a-z-]+)\.(result|outputs\.([a-z_]+))|github\.([a-z_]+)"

    def replace(match):
        if match.group(1):
            value = needs[match.group(1)]
            value = value["result"] if match.group(2) == "result" else value["outputs"][match.group(3)]
        else:
            value = github[match.group(4)]
        return repr(value)

    expression = re.sub(pattern, replace, expression)
    return bool(eval(expression, {"__builtins__": {}}, {}))


class ReleaseRequiredChecks(unittest.TestCase):
    def test_dispatch_with_tag_runs_native_and_publish_after_changes_skip(self):
        needs = {
            "resolve": {"result": "success", "outputs": {"tag": "v0.2.0-canary-f673aa7", "full_matrix": "true"}},
            "changes": {"result": "skipped", "outputs": {}},
            "plan": {"result": "success", "outputs": {"run_native": "true"}},
            "package": {"result": "success", "outputs": {}},
            "native": {"result": "success", "outputs": {}},
        }
        github = {"event_name": "workflow_dispatch", "ref_type": "tag", "repository": "zegadb/zega"}
        native = job_condition("release.yml", "native")
        publish = job_condition("release.yml", "publish")
        self.assertTrue(evaluate_condition(native, needs, github))
        self.assertTrue(evaluate_condition(publish, needs, github))

    def test_dispatch_with_tag_rejects_skipped_native_and_publish(self):
        needs = {
            "resolve": {"result": "success"},
            "changes": {"result": "skipped"},
            "plan": {"result": "success"},
            "package": {"result": "success"},
            "package-types": {"result": "skipped"},
            "native": {"result": "skipped"},
            "publish": {"result": "skipped"},
        }
        self.assertEqual(
            CHECK.failed_jobs(needs, full_matrix=True),
            {"native": "skipped", "publish": "skipped"},
        )

    def test_pull_request_path_filter_can_skip_native_and_package(self):
        needs = {
            "resolve": {"result": "success"},
            "changes": {"result": "success"},
            "plan": {"result": "success"},
            "package": {"result": "skipped"},
            "package-types": {"result": "success"},
            "native": {"result": "skipped"},
            "publish": {"result": "skipped"},
        }
        self.assertEqual(CHECK.failed_jobs(needs, full_matrix=False), {})


if __name__ == "__main__":
    unittest.main()
