import importlib.util
from pathlib import Path
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


if __name__ == "__main__":
    unittest.main()
