from __future__ import annotations

from dataclasses import dataclass
import json
import os
import socket
from threading import RLock
from typing import Any
from uuid import UUID


COMPUTER_HOST_ADDRESS_ENV = "COMBO_COMPUTER_HOST_ADDRESS"
COMPUTER_HOST_TOKEN_ENV = "COMBO_COMPUTER_HOST_TOKEN"


@dataclass(frozen=True, slots=True)
class ApplicationDescriptor:
    application_id: str
    display_name: str
    process_id: int
    windows: tuple[dict[str, Any], ...]


@dataclass(frozen=True, slots=True)
class ApplicationTarget:
    application_id: str
    display_name: str
    process_id: int
    window_id: int
    window_title: str
    bounds: dict[str, Any]


@dataclass(frozen=True, slots=True)
class WindowObservation:
    frame_id: int
    width: int
    height: int
    mime_type: str
    image: bytes
    stable: bool
    change_score: float
    target: ApplicationTarget
    accessibility: dict[str, Any]


class ComputerHostClient:
    """Persistent low-overhead client for Combo's native desktop host."""

    def __init__(self, *, address: str, token: str, timeout_seconds: float = 5.0) -> None:
        host, separator, port_text = str(address or "").strip().rpartition(":")
        if not separator or not host or not port_text.isdigit():
            raise ValueError("computer host address must use host:port")
        normalized_token = str(token or "").strip()
        if not normalized_token:
            raise ValueError("computer host token must not be empty")
        self._address = (host, int(port_text))
        self._token = normalized_token
        self._timeout_seconds = float(timeout_seconds)
        self._socket: socket.socket | None = None
        self._reader = None
        self._request_lock = RLock()
        self._connection_lock = RLock()
        self._session_lock = RLock()
        self._session_id: str | None = None

    @classmethod
    def from_environment(cls) -> "ComputerHostClient | None":
        address = str(os.getenv(COMPUTER_HOST_ADDRESS_ENV) or "").strip()
        token = str(os.getenv(COMPUTER_HOST_TOKEN_ENV) or "").strip()
        if not address and not token:
            return None
        if not address or not token:
            raise RuntimeError("native computer host environment is incomplete")
        return cls(address=address, token=token)

    def start(self) -> str:
        response = self._request({"op": "start"})
        session_id = _required_session_id(response)
        with self._session_lock:
            self._session_id = session_id
        return session_id

    def list_applications(self, session_id: str) -> tuple[ApplicationDescriptor, ...]:
        response = self._session_request(session_id, {"op": "list_applications"})
        values = response.get("applications")
        if not isinstance(values, list):
            raise RuntimeError("native computer host response requires applications")
        return tuple(_application_descriptor(value) for value in values)

    def attach_application(self, session_id: str, application_id: str) -> ApplicationTarget:
        normalized = str(application_id or "").strip()
        if not normalized:
            raise ValueError("application_id must not be empty")
        response = self._session_request(
            session_id,
            {"op": "attach_application", "application_id": normalized}
        )
        return _application_target(response.get("target"))

    def stop(self, session_id: str) -> None:
        try:
            self._request({"op": "stop", "session_id": _normalized_session_id(session_id)})
        finally:
            if self._clear_session(session_id):
                self.close()

    def cancel_session(self, session_id: str | None = None) -> bool:
        """Revoke the active native session without waiting for the request connection."""

        target_session_id = (
            _normalized_session_id(session_id)
            if session_id is not None
            else self._current_session_id()
        )
        if target_session_id is None:
            return False
        request = {
            "token": self._token,
            "op": "cancel_session",
            "session_id": target_session_id,
        }
        encoded = (
            json.dumps(request, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
            + b"\n"
        )
        try:
            with socket.create_connection(self._address, timeout=self._timeout_seconds) as stream:
                stream.settimeout(self._timeout_seconds)
                stream.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                stream.sendall(encoded)
                response_line = stream.makefile("rb", buffering=0).readline()
                if not response_line:
                    raise ConnectionError("native computer host closed the cancellation connection")
                response = json.loads(response_line.decode("utf-8"))
                if not isinstance(response, dict) or not bool(response.get("ok")):
                    raise RuntimeError(
                        str(response.get("error") if isinstance(response, dict) else "")
                        or "native computer host cancellation failed"
                    )
                return bool(response.get("cancelled", False))
        finally:
            if self._clear_session(target_session_id):
                self.close()

    def observe(
        self,
        session_id: str,
        *,
        after_frame_id: int | None = None,
        settle: bool = False,
    ) -> WindowObservation:
        response, image = self._session_request(
            session_id,
            {
                "op": "observe",
                "after_frame_id": after_frame_id,
                "settle": settle,
            },
            expect_binary=True,
        )
        return WindowObservation(
            frame_id=_required_int(response, "frame_id"),
            width=_required_int(response, "width"),
            height=_required_int(response, "height"),
            mime_type=str(response.get("mime_type") or "image/jpeg"),
            image=image,
            stable=bool(response.get("stable", False)),
            change_score=float(response.get("change_score") or 0.0),
            target=_application_target(response.get("target")),
            accessibility=_accessibility_snapshot(response),
        )

    def act(self, session_id: str, actions: list[dict[str, Any]]) -> None:
        self._session_request(session_id, {"op": "act", "actions": actions})

    def close(self) -> None:
        with self._connection_lock:
            reader = self._reader
            stream = self._socket
            self._reader = None
            self._socket = None
        if reader is not None:
            try:
                reader.close()
            except OSError:
                pass
        if stream is not None:
            try:
                stream.close()
            except OSError:
                pass

    def _connect(self) -> tuple[socket.socket, Any]:
        with self._connection_lock:
            if self._socket is not None and self._reader is not None:
                return self._socket, self._reader
            stream = socket.create_connection(self._address, timeout=self._timeout_seconds)
            stream.settimeout(self._timeout_seconds)
            stream.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            reader = stream.makefile("rb", buffering=0)
            self._socket = stream
            self._reader = reader
            return stream, reader

    def _session_request(
        self,
        session_id: str,
        payload: dict[str, Any],
        *,
        expect_binary: bool = False,
    ) -> tuple[dict[str, Any], bytes] | dict[str, Any]:
        normalized_session_id = _normalized_session_id(session_id)
        return self._request(
            {**payload, "session_id": normalized_session_id},
            expect_binary=expect_binary,
        )

    def _current_session_id(self) -> str | None:
        with self._session_lock:
            return self._session_id

    def _clear_session(self, session_id: str) -> bool:
        with self._session_lock:
            if self._session_id != session_id:
                return False
            self._session_id = None
            return True

    def _request(
        self,
        payload: dict[str, Any],
        *,
        expect_binary: bool = False,
    ) -> tuple[dict[str, Any], bytes] | dict[str, Any]:
        request = {"token": self._token, **payload}
        encoded = (
            json.dumps(request, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
            + b"\n"
        )
        with self._request_lock:
            try:
                stream, reader = self._connect()
                stream.sendall(encoded)
                line = reader.readline()
                if not line:
                    raise ConnectionError("native computer host closed the connection")
                response = json.loads(line.decode("utf-8"))
                if not isinstance(response, dict):
                    raise RuntimeError("native computer host returned a non-object response")
                if not bool(response.get("ok")):
                    raise RuntimeError(str(response.get("error") or "native computer host request failed"))
                if not expect_binary:
                    return response
                length = _required_int(response, "content_length")
                image = _read_exact(reader, length)
                return response, image
            except (OSError, ValueError, json.JSONDecodeError, ConnectionError):
                self.close()
                raise


def _required_int(payload: dict[str, Any], key: str) -> int:
    value = payload.get(key)
    if isinstance(value, bool) or not isinstance(value, int):
        raise RuntimeError(f"native computer host response requires integer {key}")
    return value


def _required_session_id(payload: dict[str, Any]) -> str:
    value = _required_text(payload, "session_id")
    try:
        return str(UUID(value))
    except ValueError as exc:
        raise RuntimeError("native computer host returned an invalid session_id") from exc


def _normalized_session_id(value: str) -> str:
    try:
        return str(UUID(str(value or "").strip()))
    except ValueError as exc:
        raise ValueError("computer host session_id must be a UUID") from exc


def _accessibility_snapshot(payload: dict[str, Any]) -> dict[str, Any]:
    value = payload.get("accessibility")
    if not isinstance(value, dict):
        raise RuntimeError("native computer host response requires an accessibility snapshot")
    nodes = value.get("nodes")
    if not isinstance(nodes, list):
        raise RuntimeError("native computer host accessibility snapshot requires nodes")
    return value


def _application_descriptor(value: Any) -> ApplicationDescriptor:
    if not isinstance(value, dict):
        raise RuntimeError("native computer host application must be an object")
    windows = value.get("windows")
    if not isinstance(windows, list) or not all(isinstance(item, dict) for item in windows):
        raise RuntimeError("native computer host application requires windows")
    return ApplicationDescriptor(
        application_id=_required_text(value, "application_id"),
        display_name=_required_text(value, "display_name"),
        process_id=_required_int(value, "process_id"),
        windows=tuple(windows),
    )


def _application_target(value: Any) -> ApplicationTarget:
    if not isinstance(value, dict):
        raise RuntimeError("native computer host target must be an object")
    bounds = value.get("bounds")
    if not isinstance(bounds, dict):
        raise RuntimeError("native computer host target requires bounds")
    return ApplicationTarget(
        application_id=_required_text(value, "application_id"),
        display_name=_required_text(value, "display_name"),
        process_id=_required_int(value, "process_id"),
        window_id=_required_int(value, "window_id"),
        window_title=str(value.get("window_title") or ""),
        bounds=bounds,
    )


def _required_text(payload: dict[str, Any], key: str) -> str:
    value = payload.get(key)
    if not isinstance(value, str) or not value.strip():
        raise RuntimeError(f"native computer host response requires text {key}")
    return value


def _read_exact(reader: Any, length: int) -> bytes:
    remaining = length
    chunks: list[bytes] = []
    while remaining > 0:
        chunk = reader.read(remaining)
        if not chunk:
            raise ConnectionError("native computer host ended a frame early")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)
