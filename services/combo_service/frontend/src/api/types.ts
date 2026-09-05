/*
 * API contract types. Defined once, mirrored from the backend registry views.
 * Pages and stores must import from here rather than redeclaring shapes.
 */

/** Signed OSS PUT request returned by POST /uploads. */
export interface UploadRequest {
  method: string
  url: string
  headers: Record<string, string>
  expires_in_seconds: number
}

export type AppReleaseStatus =
  | 'draft'
  | 'queued'
  | 'publishing'
  | 'published'
  | 'failed'
  | 'withdrawn'

export type AppAssetStatus =
  | 'awaiting_upload'
  | 'uploaded'
  | 'publishing'
  | 'published'
  | 'failed'

export interface AppReleaseAsset {
  asset_id: string
  asset_kind: 'installer' | 'updater'
  platform: 'macos' | 'windows'
  architecture: string
  filename: string
  content_type: string
  size_bytes: number
  sha256: string
  status: AppAssetStatus
  download_url: string
  download_count: number
  created_at: string
  updated_at: string
  expected_size?: number
  progress_bytes?: number
  progress_ratio?: number
  has_updater_signature?: boolean
  error?: { code: string; message: string } | null
}

export interface AppReleaseJob {
  job_id: string
  job_type: 'publish' | 'sync_metadata'
  status: 'queued' | 'running' | 'succeeded' | 'failed'
  stage: string
  progress_bytes: number
  total_bytes: number
  progress_ratio: number
  error: { code: string; message: string } | null
  created_at: string
  updated_at: string
}

export interface AppRelease {
  app_release_id: string
  version: string
  tag_name: string
  title: string
  notes_markdown: string
  status: AppReleaseStatus
  github_url: string
  created_at: string
  published_at: string
  updated_at: string
  assets: AppReleaseAsset[]
  error?: { code: string; message: string } | null
  latest_job?: AppReleaseJob | null
}

export interface CreateAppAssetResponse {
  asset: AppReleaseAsset
  upload_request: UploadRequest
}

/** Machine-readable API error envelope. */
export interface ApiErrorBody {
  code: string
  message: string
  request_id?: string
}

export type ErrorReportStatus = 'new' | 'reviewed' | 'resolved'

export interface ErrorReportSummary {
  error_report_id: string
  status: ErrorReportStatus
  source: string
  app_version: string
  platform: string
  architecture: string
  error_code: string
  summary: string
  request_id: string
  diagnostic_ref: string
  created_at: string
  updated_at: string
}

export interface ErrorReport extends ErrorReportSummary {
  context: Record<string, unknown>
  log_excerpt: string
}
