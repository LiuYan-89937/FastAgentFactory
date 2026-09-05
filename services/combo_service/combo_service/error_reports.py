from __future__ import annotations

import json
import sqlite3
from typing import Any
from uuid import uuid4

from combo_service.audit import record_audit
from combo_service.database import Database, utc_now


ERROR_REPORT_STATUSES = frozenset({"new", "reviewed", "resolved"})
MAX_CONTEXT_JSON_CHARS = 24_000


class ErrorReportError(RuntimeError):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


class ErrorReportRegistry:
    def __init__(self, database: Database) -> None:
        self.database = database

    def create(
        self,
        *,
        source: str,
        app_version: str,
        platform: str,
        architecture: str,
        summary: str,
        error_code: str = "",
        request_id: str = "",
        diagnostic_ref: str = "",
        context: dict[str, Any] | None = None,
        log_excerpt: str = "",
    ) -> dict[str, Any]:
        normalized_source = _required_text(source, "source")
        normalized_version = _required_text(app_version, "app_version")
        normalized_platform = _required_text(platform, "platform")
        normalized_architecture = _required_text(architecture, "architecture")
        normalized_summary = _required_text(summary, "summary")
        context_json = json.dumps(context or {}, ensure_ascii=False, separators=(",", ":"))
        if len(context_json) > MAX_CONTEXT_JSON_CHARS:
            raise ErrorReportError("error_report_context_too_large", "error report context is too large")
        report_id = uuid4().hex
        now = utc_now()
        with self.database.connect() as connection:
            connection.execute(
                """
                insert into error_reports(
                  error_report_id, status, source, app_version, platform,
                  architecture, error_code, summary, request_id, diagnostic_ref,
                  context_json, log_excerpt, created_at, updated_at
                ) values (?, 'new', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    report_id,
                    normalized_source,
                    normalized_version,
                    normalized_platform,
                    normalized_architecture,
                    error_code,
                    normalized_summary,
                    request_id,
                    diagnostic_ref,
                    context_json,
                    log_excerpt,
                    now,
                    now,
                ),
            )
        return self.get(report_id)

    def list(self, *, status: str = "", limit: int = 100) -> list[dict[str, Any]]:
        normalized_status = status.strip().casefold()
        if normalized_status and normalized_status not in ERROR_REPORT_STATUSES:
            raise ErrorReportError("error_report_status_invalid", "invalid error report status")
        where = "where status = ?" if normalized_status else ""
        parameters: tuple[Any, ...] = (normalized_status, limit) if normalized_status else (limit,)
        with self.database.connect() as connection:
            rows = connection.execute(
                f"""
                select error_report_id, status, source, app_version, platform,
                       architecture, error_code, summary, request_id,
                       diagnostic_ref, created_at, updated_at
                from error_reports
                {where}
                order by created_at desc
                limit ?
                """,
                parameters,
            ).fetchall()
        return [dict(row) for row in rows]

    def get(self, report_id: str) -> dict[str, Any]:
        with self.database.connect() as connection:
            row = connection.execute(
                "select * from error_reports where error_report_id = ?",
                (report_id,),
            ).fetchone()
        if row is None:
            raise ErrorReportError("error_report_not_found", "error report was not found")
        return _report_view(row)

    def update_status(
        self,
        report_id: str,
        *,
        status: str,
        admin: dict[str, Any],
    ) -> dict[str, Any]:
        normalized_status = status.strip().casefold()
        if normalized_status not in ERROR_REPORT_STATUSES:
            raise ErrorReportError("error_report_status_invalid", "invalid error report status")
        now = utc_now()
        with self.database.connect() as connection:
            connection.execute("begin immediate")
            cursor = connection.execute(
                "update error_reports set status = ?, updated_at = ? where error_report_id = ?",
                (normalized_status, now, report_id),
            )
            if cursor.rowcount != 1:
                connection.rollback()
                raise ErrorReportError("error_report_not_found", "error report was not found")
            record_audit(
                connection,
                actor_user_id=str(admin["user_id"]),
                action="error_report.status_updated",
                target_type="error_report",
                target_id=report_id,
                detail={"status": normalized_status},
            )
            connection.commit()
        return self.get(report_id)


def _report_view(row: sqlite3.Row) -> dict[str, Any]:
    value = dict(row)
    try:
        value["context"] = json.loads(str(value.pop("context_json") or "{}"))
    except json.JSONDecodeError:
        value["context"] = {}
    return value


def _required_text(value: str, field: str) -> str:
    normalized = value.strip()
    if not normalized:
        raise ErrorReportError("error_report_field_invalid", f"{field} must not be empty")
    return normalized
