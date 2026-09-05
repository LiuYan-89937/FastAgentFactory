from __future__ import annotations

from pathlib import Path
from pathlib import PurePosixPath

from combo.runtime_defaults import DEFAULT_BUILTIN_WORKSPACE_ROOT


def workspace_path_candidate(value: str, *, root: Path) -> Path:
    """Resolve relative paths and the stable virtual workspace alias against a runtime root."""
    raw_value = str(value or "").strip()
    virtual_requested = PurePosixPath(raw_value.replace("\\", "/"))
    virtual_root = PurePosixPath(DEFAULT_BUILTIN_WORKSPACE_ROOT)
    try:
        virtual_relative = virtual_requested.relative_to(virtual_root)
    except ValueError:
        virtual_relative = None
    if virtual_relative is not None:
        return root.joinpath(*virtual_relative.parts)

    requested = Path(raw_value).expanduser()
    if not requested.is_absolute():
        return root / requested
    return requested


def resolve_workspace_path(
    value: str,
    *,
    root: Path,
    allow_external: bool = False,
    allowed_roots: tuple[Path, ...] = (),
) -> Path:
    """Translate a workspace reference and enforce access after resolving symlinks."""
    resolved_root = root.expanduser().resolve()
    resolved = workspace_path_candidate(value, root=resolved_root).resolve()
    if allow_external or any(
        resolved.is_relative_to(allowed_root.expanduser().resolve())
        for allowed_root in (resolved_root, *allowed_roots)
    ):
        return resolved
    raise ValueError(f"path escapes workspace root: {value}")


def workspace_virtual_relative_path(value: str) -> str:
    raw_value = str(value or "").strip()
    virtual_requested = PurePosixPath(raw_value.replace("\\", "/"))
    virtual_root = PurePosixPath(DEFAULT_BUILTIN_WORKSPACE_ROOT)
    try:
        relative = virtual_requested.relative_to(virtual_root)
    except ValueError:
        requested = Path(raw_value)
        if requested.is_absolute():
            raise ValueError(
                f"workspace transaction path must use the {DEFAULT_BUILTIN_WORKSPACE_ROOT} alias or a relative path"
            )
        relative = PurePosixPath(raw_value.replace("\\", "/"))
    if any(part in {"", ".", ".."} for part in relative.parts):
        raise ValueError(f"workspace path contains an invalid segment: {value}")
    normalized = relative.as_posix()
    if normalized in {"", "."}:
        raise ValueError("workspace path must identify a file")
    return normalized
