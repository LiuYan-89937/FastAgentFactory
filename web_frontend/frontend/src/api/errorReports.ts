import { invoke, isTauri } from '@tauri-apps/api/core'

export interface ErrorReportInput {
  summary: string
  errorCode?: string
  requestId?: string
  diagnosticRef?: string
  context?: Record<string, unknown>
}

export interface ErrorReportReceipt {
  error_report_id: string
  status: string
  created_at: string
}

export const errorReportsApi = {
  available(): boolean {
    return isTauri()
  },

  submit(input: ErrorReportInput): Promise<ErrorReportReceipt> {
    if (!isTauri()) {
      return Promise.reject(new Error('Error reporting is available in the Combo desktop app'))
    }
    return invoke<ErrorReportReceipt>('report_error', { input })
  },
}
