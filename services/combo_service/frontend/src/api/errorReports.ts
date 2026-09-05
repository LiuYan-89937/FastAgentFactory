import { request } from './client'
import type { ErrorReport, ErrorReportStatus, ErrorReportSummary } from './types'

export function listErrorReports(
  status: ErrorReportStatus | '' = '',
  limit = 100,
  signal?: AbortSignal,
): Promise<ErrorReportSummary[]> {
  return request<ErrorReportSummary[]>('/admin/error-reports', {
    query: { status, limit },
    signal,
  })
}

export function fetchErrorReport(
  reportId: string,
  signal?: AbortSignal,
): Promise<ErrorReport> {
  return request<ErrorReport>(`/admin/error-reports/${encodeURIComponent(reportId)}`, { signal })
}

export function updateErrorReportStatus(
  reportId: string,
  status: ErrorReportStatus,
): Promise<ErrorReport> {
  return request<ErrorReport>(`/admin/error-reports/${encodeURIComponent(reportId)}`, {
    method: 'PATCH',
    body: { status },
  })
}
