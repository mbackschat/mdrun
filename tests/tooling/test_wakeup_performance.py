import importlib.util
from pathlib import Path
import sys
import unittest

SOURCE = Path(__file__).resolve().parents[1] / "performance/wakeup.py"
sys.path.insert(0, str(SOURCE.parent))
spec = importlib.util.spec_from_file_location("wakeup_performance", SOURCE)
performance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(performance)


class WakeupOracle(unittest.TestCase):
    def test_payload_and_call_count_must_match_before_timing_is_accepted(self):
        valid = {"latency_ms": [10.0, 12.0], "stream_ms": 1000.0,
                 "stream_bytes": 134217728,
                 "stream_sha256": "06233646f01770d62d9318d0228528a8097936a5d25d96484a45526f48ded01c"}
        performance.validate_observations(valid, 2)
        for key, value in [("latency_ms", [10.0]), ("stream_bytes", 1),
                           ("stream_sha256", "wrong"), ("stream_ms", 0),
                           ("stream_ms", float("nan")), ("latency_ms", [10.0, float("inf")])]:
            with self.subTest(field=key, value=value), self.assertRaises(ValueError):
                performance.validate_observations({**valid, key: value}, 2)


if __name__ == "__main__":
    unittest.main()
