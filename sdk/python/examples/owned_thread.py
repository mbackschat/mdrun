"""Application-owned cooperative work, with explicit reset attestation and cleanup."""

import threading

from mdrun_sdk import Application, Eligible, Refused


class OwnedThreadApplication(Application):
    """Own one known thread; do not infer ownership from a process-wide inventory."""

    def __init__(self):
        self.thread = None
        self._stop = threading.Event()
        self._started = threading.Event()
        self._expired = threading.Event()

    def classify(self, argv):
        return Eligible() if argv in (("owned_joined",), ("owned_live",)) else Refused("unsupported_command")

    def invoke(self, request):
        if self.thread is not None:
            raise RuntimeError("previous owned work remains")
        self._stop.clear()
        self._started.clear()
        self._expired.clear()

        def work():
            self._started.set()
            if not self._stop.wait(5):
                self._expired.set()

        # SDK callbacks use a daemon thread; owned work must have an explicit lifetime.
        self.thread = threading.Thread(target=work, name="example-owned-work", daemon=False)
        self.thread.start()
        if not self._started.wait(1) or not self.thread.is_alive():
            raise RuntimeError("owned work did not start")
        if request.argv[0] == "owned_joined":
            self._stop_and_join()
        return 0

    def reset(self, request):
        if self._expired.is_set():
            raise RuntimeError("owned work expired instead of being stopped")
        if self.thread is not None:
            raise RuntimeError("owned work remains unjoined")

    def _stop_and_join(self):
        if self.thread is not None:
            self._stop.set()
            self.thread.join(1)
            if self.thread.is_alive():
                raise RuntimeError("owned work did not stop")
            self.thread = None

    def close(self):
        self._stop_and_join()
