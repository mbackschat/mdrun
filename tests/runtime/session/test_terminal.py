import argparse
import importlib.util
import json
from pathlib import Path
import tempfile
import time
import unittest
from unittest import mock


SPEC = importlib.util.spec_from_file_location("terminal_qualification", Path(__file__).with_name("terminal.py"))
terminal = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(terminal)


class PinnedChildObservation(unittest.TestCase):
    parent = {"pid": 101, "parent": 1, "group": 100, "state": "T", "identity": [111]}
    child = {"pid": 102, "parent": 101, "group": 200, "state": "live", "identity": [222]}

    def test_already_zombie_child_needs_no_native_identity_or_signal(self):
        def native(pid):
            self.assertEqual(pid, self.parent["pid"], "zombie child must not need a native identity query")
            return self.parent.copy()
        with mock.patch.object(terminal, "process_info", side_effect=native), \
             mock.patch.object(terminal.subprocess, "check_output", return_value="102 101 200 Z+\n"), \
             mock.patch.object(terminal.os, "kill", side_effect=AssertionError("zombie child must never be signalled")):
            visited = []
            terminal.terminate_pinned_tree(self.parent, self.child, visited, time.monotonic() + 3)
        self.assertEqual(visited, [102])

    def test_live_reference_can_become_zombie_when_native_identity_disappears(self):
        def native(pid):
            return self.parent.copy() if pid == self.parent["pid"] else None
        with mock.patch.object(terminal, "process_info", side_effect=native), \
             mock.patch.object(terminal.subprocess, "check_output", side_effect=["102 101 200 S+\n", "102 101 200 Z+\n"]), \
             mock.patch.object(terminal.os, "kill", side_effect=AssertionError("zombie child must never be signalled")):
            visited = []
            terminal.terminate_pinned_tree(self.parent, self.child, visited, time.monotonic() + 3)
        self.assertEqual(visited, [102])

    def test_native_disappearance_without_explicit_zombie_is_not_death(self):
        def native(pid):
            return self.parent.copy() if pid == self.parent["pid"] else None
        with mock.patch.object(terminal, "process_info", side_effect=native), \
             mock.patch.object(terminal.subprocess, "check_output", return_value="102 101 200 S+\n"), \
             mock.patch.object(terminal.os, "kill") as kill:
            with self.assertRaises(AssertionError):
                terminal.terminate_pinned_tree(self.parent, self.child, [], time.monotonic() + 3)
        kill.assert_not_called()

    def test_zombie_does_not_release_frozen_parent_identity_requirement(self):
        resumed = {**self.parent, "state": "live"}
        with mock.patch.object(terminal, "process_info", side_effect=[self.parent, resumed]), \
             mock.patch.object(terminal.subprocess, "check_output", return_value="102 101 200 Z+\n"), \
             mock.patch.object(terminal.os, "kill") as kill:
            with self.assertRaises(AssertionError):
                terminal.terminate_pinned_tree(self.parent, self.child, [], time.monotonic() + 3)
        kill.assert_not_called()


class BackgroundDiagnostics(unittest.TestCase):
    reference = {"pid": 102, "parent": 101, "group": 200, "state": "live", "identity": [222]}

    def test_inner_driver_observation_survives_a_later_teardown_failure(self):
        with tempfile.TemporaryDirectory(prefix="mdrun-terminal-driver-test-") as temporary:
            directory = Path(temporary)
            args = argparse.Namespace(directory=directory, case="background_group", mdrun=Path("/fixture/mdrun"))
            terminal.publish(directory, "job.json", {"status": 0})
            terminal.publish(directory, "report.json", {"cleanup": {"outcome": "clean"}})
            terminal.publish(directory, "background.json", self.reference)
            observation = {"before": {"state": "live"}, "after": {"state": "live"},
                           "heartbeat_advanced": True, "listener_available": True}
            child = mock.Mock(pid=120)
            child.poll.return_value = None
            reservation = mock.MagicMock()
            reservation.__enter__.return_value.getsockname.return_value = ("127.0.0.1", 1234)
            original_close = terminal.os.close

            def close(descriptor):
                if descriptor not in (1050, 1051):
                    original_close(descriptor)

            with mock.patch.object(terminal.os, "getpgrp", return_value=100), \
                 mock.patch.object(terminal.os, "tcgetpgrp", side_effect=[100, 120, 100]), \
                 mock.patch.object(terminal, "descriptors", return_value=[]), \
                 mock.patch.object(terminal, "foreground"), \
                 mock.patch.object(terminal.socket, "socket", return_value=reservation), \
                 mock.patch.object(terminal.os, "pipe", return_value=(1050, 1051)), \
                 mock.patch.object(terminal.os, "close", side_effect=close), \
                 mock.patch.object(terminal.os, "write"), \
                 mock.patch.object(terminal.subprocess, "Popen", return_value=child), \
                 mock.patch.object(terminal, "observe_background", return_value=observation), \
                 mock.patch.object(terminal, "known_groups", return_value={200}), \
                 mock.patch.object(terminal, "wait_for", side_effect=TimeoutError("inner teardown failed")):
                with self.assertRaisesRegex(TimeoutError, "inner teardown failed"):
                    terminal.driver(args)
            self.assertTrue((directory / "result.json").exists(), "driver cleanup discarded its pre-cleanup observation")
            saved = json.loads((directory / "result.json").read_text())
            self.assertEqual(saved["background_observation"], observation)
            self.assertIn("inner teardown failed", saved["cleanup_error"])

    def test_primary_and_cleanup_errors_are_saved_before_scratch_removal(self):
        with tempfile.TemporaryDirectory(prefix="mdrun-terminal-diagnostic-test-") as temporary:
            results = Path(temporary) / "results"
            results.mkdir()
            args = argparse.Namespace(results=results, mdrun=Path("/fixture/mdrun"))
            child = mock.Mock(returncode=0)
            child.poll.return_value = 0
            child.wait.return_value = 0
            scratch = []

            def spawn(*_, **options):
                directory = Path(options["cwd"])
                scratch.append(directory)
                terminal.publish(directory, "binding.json", {"resources": [{"port": 1234}]})
                terminal.publish(directory, "fact.json", {"observed": "before failure"})
                return child

            original_close = terminal.os.close

            def close(descriptor):
                if descriptor == 100:
                    raise OSError("cleanup close failed")
                if descriptor != 101:
                    original_close(descriptor)

            with mock.patch.object(terminal.pty, "openpty", return_value=(100, 101)), \
                 mock.patch.object(terminal.termios, "tcgetattr", return_value=[0, 0, 0, 0]), \
                 mock.patch.object(terminal.termios, "tcsetattr"), \
                 mock.patch.object(terminal.subprocess, "Popen", side_effect=spawn), \
                 mock.patch.object(terminal.select, "select", side_effect=RuntimeError("primary read failed")), \
                 mock.patch.object(terminal.os, "close", side_effect=close):
                try:
                    terminal.capture(args, "background_group")
                except BaseException as error:
                    failure = error
                else:
                    self.fail("injected primary failure unexpectedly succeeded")

            saved = results / "background_group.json"
            self.assertTrue(saved.exists(), "cleanup failure discarded the collected observations")
            result = json.loads(saved.read_text())
            self.assertIn("primary read failed", result["primary_error"])
            self.assertTrue(any("cleanup close failed" in error["error"] for error in result["cleanup_errors"]))
            self.assertEqual(result["files"]["fact.json"], {"observed": "before failure"})
            self.assertIsInstance(failure, RuntimeError)
            self.assertTrue(scratch and not scratch[0].exists())

    def test_stopped_or_unknown_helper_without_heartbeat_does_not_certify_cleanup(self):
        for expected, native in [("live", {**self.reference, "state": "T"}), ("unknown", None)]:
            with self.subTest(observation=expected), tempfile.TemporaryDirectory(prefix="mdrun-background-observation-") as temporary:
                directory = Path(temporary)
                terminal.publish(directory, "background.json", self.reference)
                result = {}

                def bounded_wait(condition, *_):
                    self.assertFalse(condition(), "missing heartbeat or close marker must not certify death")
                    raise TimeoutError("helper remains live or unknown")

                with mock.patch.object(terminal, "process_info", return_value=native), \
                     mock.patch.object(terminal, "process_rows", return_value=[{**self.reference, "state": "T"}]), \
                     mock.patch.object(terminal, "wait_for", side_effect=bounded_wait):
                    with self.assertRaisesRegex(TimeoutError, "live or unknown"):
                        terminal.cleanup_background(directory, result)
                self.assertEqual(result["background_cleanup"]["observation"]["state"], expected)
                self.assertFalse(result["background_cleanup"]["cooperative_close"])
                self.assertFalse((directory / "heartbeat.json").exists())

    def test_independently_dead_helper_does_not_require_a_cooperative_close_marker(self):
        with tempfile.TemporaryDirectory(prefix="mdrun-background-observation-") as temporary:
            directory = Path(temporary)
            terminal.publish(directory, "background.json", self.reference)
            result = {}
            with mock.patch.object(terminal, "process_info", return_value=None), \
                 mock.patch.object(terminal, "process_rows", return_value=[]):
                terminal.cleanup_background(directory, result)
            self.assertEqual(result["background_cleanup"]["observation"]["state"], "dead")
            self.assertFalse(result["background_cleanup"]["cooperative_close"])


if __name__ == "__main__":
    unittest.main()
