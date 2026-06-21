"""Host side of the Spectrum-native chatbot trap protocol (``docs/04-spectrum-
native-chat-spec.md``).

A Z80 terminal app calls the ``CHAT_*`` host syscalls (over the ``ED FE`` trap
ABI); the host runs an LLM (or a stub) and streams the reply back as events the
Z80 drains one at a time — a teletype with two decoupled clocks. This module is
the *host half*, testable on its own; the Z80 terminal app (asm) is separate.

Protocol (id in ``A``):
  * ``CHAT_BEGIN`` (0x30): ``HL`` → input bytes, ``B`` = length. Run the
    responder, queue the reply as events.
  * ``CHAT_POLL``  (0x31): ``HL`` → output buffer, ``B`` = capacity. Dequeue one
    event → ``A`` = event code, ``BC`` = bytes written.
  * ``CHAT_CANCEL`` (0x32) / ``CHAT_RESET`` (0x33): drop the queue / wipe history.
Carry = error on every call.
"""

from __future__ import annotations

from collections import deque
from typing import Callable, Optional

CHAT_BEGIN, CHAT_POLL, CHAT_CANCEL, CHAT_RESET = 0x30, 0x31, 0x32, 0x33
EV_NONE, EV_TEXT, EV_DONE, EV_ERROR = 0, 1, 2, 3

# A responder turns the conversation so far into a reply string.
Responder = Callable[[list[tuple[str, str]]], str]


def echo_responder(history: list[tuple[str, str]]) -> str:
    """A dependency-free stub: echoes the last user line. Swap in a chuk-llm
    backed responder for the real chatbot."""
    last = next((t for who, t in reversed(history) if who == "user"), "")
    return f"You said: {last}" if last else "Hello!"


def to_spectrum(s: str) -> bytes:
    """Clamp to the printable Spectrum charset (ASCII 32..126)."""
    return bytes(ord(c) if 32 <= ord(c) <= 126 else ord("?") for c in s)


class ChatSession:
    """One conversation: history + a queue of reply events to teletype out."""

    def __init__(self, responder: Optional[Responder] = None, chunk: int = 16) -> None:
        self.responder: Responder = responder or echo_responder
        self.chunk = chunk
        self.history: list[tuple[str, str]] = []
        self.queue: "deque[tuple[int, bytes]]" = deque()

    def begin(self, prompt: str) -> None:
        self.history.append(("user", prompt))
        try:
            reply = self.responder(self.history)
        except Exception as e:  # a failing backend becomes an error event
            self.queue.append((EV_ERROR, to_spectrum(str(e))[:64]))
            return
        self.history.append(("assistant", reply))
        data = to_spectrum(reply)
        for i in range(0, len(data), self.chunk):
            self.queue.append((EV_TEXT, data[i : i + self.chunk]))
        self.queue.append((EV_DONE, b""))

    def poll(self) -> tuple[int, bytes]:
        return self.queue.popleft() if self.queue else (EV_NONE, b"")

    def cancel(self) -> None:
        self.queue.clear()

    def reset(self) -> None:
        self.history.clear()
        self.queue.clear()


def make_dispatcher(session: ChatSession):
    """A host-trap callback wiring the ``CHAT_*`` ids to `session`. Pass it to
    ``Machine.register_host_dispatcher`` (optionally ``with_math=True``)."""

    def on_trap(ctx) -> None:
        sid = ctx.a
        if sid == CHAT_BEGIN:
            addr, n = ctx.hl, ctx.bc >> 8  # HL = input, B = length
            text = bytes(ctx.read(addr, n)).decode("latin-1")
            session.begin(text)
            ctx.set_carry(False)
        elif sid == CHAT_POLL:
            addr, cap = ctx.hl, ctx.bc >> 8  # HL = buffer, B = capacity
            code, payload = session.poll()
            payload = payload[:cap]
            ctx.write(addr, payload)
            ctx.set_a(code)
            ctx.set_bc(len(payload))
            ctx.set_carry(False)
        elif sid == CHAT_CANCEL:
            session.cancel()
            ctx.set_carry(False)
        elif sid == CHAT_RESET:
            session.reset()
            ctx.set_carry(False)
        else:
            ctx.set_carry(True)

    return on_trap
