import copy
import importlib.util
from pathlib import Path
import unittest
import tempfile

SOURCE = Path(__file__).resolve().parents[1] / "performance/compare.py"
spec = importlib.util.spec_from_file_location("performance", SOURCE)
performance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(performance)


class PerformanceOracle(unittest.TestCase):
    def test_prepared_file_and_launcher_changes_invalidate_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            executable = root / "runtime"
            executable.write_bytes(b"runtime")
            prepared = root / "prepared"
            prepared.mkdir()
            binding = prepared / "binding.json"
            binding.write_text("{}")
            sdk = prepared / "sdk.py"
            sdk.write_text("original")
            configuration = {"worker_argv": [str(executable)], "cold_argv": [str(executable)]}
            baseline = performance.artifact_snapshot(executable, binding, configuration)
            sdk.write_text("changed")
            self.assertNotEqual(baseline, performance.artifact_snapshot(executable, binding, configuration))
            sdk.write_text("original")
            executable.write_bytes(b"different runtime")
            self.assertNotEqual(baseline, performance.artifact_snapshot(executable, binding, configuration))

    def report(self, mode):
        return {"artifact_id": "fixture", "outcome": "success", "infrastructure_failure": None,
                "cleanup": {"outcome": "clean"}, "author_status": {"exit_code": 0, "signal": None},
                "warm_calls": 2 if mode == "fast" else 0,
                "cold_calls": 0 if mode == "fast" else 2,
                "worker_starts": 1 if mode == "fast" else 0,
                "cold_reasons": {} if mode == "fast" else {"explicit_cold": 2},
                "application_statuses": [{"route": "warm" if mode == "fast" else "cold",
                                          "exit_code": 0, "signal": None}] * 2}

    def test_route_cleanup_identity_and_effect_count_are_independent_of_elapsed_time(self):
        for mode in ("fast", "cold"):
            original = self.report(mode)
            performance.validate_report(original, mode, 2, "fixture")
            for key, bad in [("worker_starts", 2), ("artifact_id", "different"),
                             ("infrastructure_failure", "lost worker"), ("outcome", "failure"),
                             ("cleanup", {"outcome": "uncertain"}), ("application_statuses", []),
                             ("cold_reasons", {"environment": 2})]:
                with self.subTest(mode=mode, field=key):
                    report = copy.deepcopy(original)
                    report[key] = bad
                    with self.assertRaises(ValueError):
                        performance.validate_report(report, mode, 2, "fixture")

    def test_workload_changes_input_for_every_call_and_sample(self):
        first = performance.document("tool", Path("/tmp/input.txt"), 2, 0)
        second = performance.document("tool", Path("/tmp/input.txt"), 2, 1)
        self.assertNotEqual(first, second)
        self.assertEqual(first.count(b"tool read_file"), 2)
        self.assertIn(b"sample=0 call=0", first)
        self.assertIn(b"sample=0 call=1", first)
        self.assertEqual(first.count(b"```output"), 2)


if __name__ == "__main__":
    unittest.main()
