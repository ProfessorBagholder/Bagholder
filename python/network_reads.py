"""Shared pacing for read-only broker and market-data requests.

Respect the configured DNS resolver: no alternate resolvers or cached-IP
bypass. A temporary DNS refusal slows every reader, including fallback loops.
Order mutations and authentication exchanges never pass through this retryer.
"""
import socket
import threading
import time
from urllib.error import URLError


class ReadGate:
    def __init__(self, interval=.25, cooldown=45, clock=time.monotonic, sleep=time.sleep):
        self.interval = interval
        self.cooldown = cooldown
        self.clock = clock
        self.sleep = sleep
        self.lock = threading.Lock()
        self.next_at = 0

    def wait(self):
        while True:
            with self.lock:
                now = self.clock()
                delay = self.next_at - now
                if delay <= 0:
                    self.next_at = now + self.interval
                    return
            # Recheck the shared deadline, since another reader may have
            # extended the DNS cooldown while this reader was asleep.
            self.sleep(min(delay, 1))

    def defer(self):
        with self.lock:
            self.next_at = max(self.next_at, self.clock() + self.cooldown)

    def call(self, read):
        for attempt in range(3):
            self.wait()
            try:
                return read()
            except (URLError, socket.gaierror) as exc:
                reason = getattr(exc, 'reason', exc)
                if not isinstance(reason, socket.gaierror):
                    raise
                self.defer()
                if attempt == 2:
                    raise


gate = ReadGate()


def call(read):
    return gate.call(read)
