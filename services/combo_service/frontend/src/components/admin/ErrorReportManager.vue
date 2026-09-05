<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { ApiError } from '@/api/client'
import {
  fetchErrorReport,
  listErrorReports,
  updateErrorReportStatus,
} from '@/api/errorReports'
import type { ErrorReport, ErrorReportStatus, ErrorReportSummary } from '@/api/types'
import BaseButton from '@/components/base/BaseButton.vue'
import CopyButton from '@/components/base/CopyButton.vue'
import StateBlock from '@/components/base/StateBlock.vue'

const filters: Array<{ value: ErrorReportStatus | ''; label: string }> = [
  { value: '', label: '全部' },
  { value: 'new', label: '待处理' },
  { value: 'reviewed', label: '已查看' },
  { value: 'resolved', label: '已解决' },
]

const reports = ref<ErrorReportSummary[]>([])
const selected = ref<ErrorReport | null>(null)
const statusFilter = ref<ErrorReportStatus | ''>('')
const loading = ref(false)
const detailLoading = ref(false)
const updating = ref(false)
const error = ref('')

const contextText = computed(() => JSON.stringify(selected.value?.context || {}, null, 2))

onMounted(load)
watch(statusFilter, load)

async function load(): Promise<void> {
  loading.value = true
  error.value = ''
  try {
    reports.value = await listErrorReports(statusFilter.value)
    if (selected.value && !reports.value.some(item => item.error_report_id === selected.value?.error_report_id)) {
      selected.value = null
    }
  } catch (reason) {
    error.value = errorMessage(reason)
  } finally {
    loading.value = false
  }
}

async function selectReport(report: ErrorReportSummary): Promise<void> {
  detailLoading.value = true
  error.value = ''
  try {
    selected.value = await fetchErrorReport(report.error_report_id)
  } catch (reason) {
    error.value = errorMessage(reason)
  } finally {
    detailLoading.value = false
  }
}

async function setStatus(status: ErrorReportStatus): Promise<void> {
  if (!selected.value || selected.value.status === status || updating.value) return
  updating.value = true
  error.value = ''
  try {
    selected.value = await updateErrorReportStatus(selected.value.error_report_id, status)
    await load()
  } catch (reason) {
    error.value = errorMessage(reason)
  } finally {
    updating.value = false
  }
}

function errorMessage(value: unknown): string {
  if (value instanceof ApiError) return value.message
  return value instanceof Error ? value.message : '请求失败'
}

function statusLabel(status: ErrorReportStatus): string {
  return { new: '待处理', reviewed: '已查看', resolved: '已解决' }[status]
}

function dateTime(value: string): string {
  return new Intl.DateTimeFormat('zh-CN', {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(value))
}
</script>

<template>
  <section class="reports-panel">
    <header class="reports-header">
      <div>
        <span class="eyebrow">Diagnostics</span>
        <h2>错误上报</h2>
        <p>查看桌面应用主动提交的脱敏诊断信息。</p>
      </div>
      <div class="reports-actions">
        <label>
          <span class="visually-hidden">处理状态</span>
          <select v-model="statusFilter">
            <option v-for="filter in filters" :key="filter.value" :value="filter.value">
              {{ filter.label }}
            </option>
          </select>
        </label>
        <BaseButton variant="secondary" size="sm" :loading="loading" @click="load">刷新</BaseButton>
      </div>
    </header>

    <StateBlock v-if="error" kind="error" title="无法加载错误上报" :body="error" />

    <div v-else class="reports-layout">
      <div class="report-list" aria-label="错误上报列表">
        <StateBlock v-if="loading && !reports.length" kind="loading" title="正在加载" />
        <StateBlock v-else-if="!reports.length" kind="empty" title="暂无错误上报" />
        <template v-else>
          <button
            v-for="report in reports"
            :key="report.error_report_id"
            type="button"
            class="report-row"
            :class="{ 'report-row--active': selected?.error_report_id === report.error_report_id }"
            @click="selectReport(report)"
          >
            <span class="report-row__top">
              <strong>{{ report.summary }}</strong>
              <small :class="`status-${report.status}`">{{ statusLabel(report.status) }}</small>
            </span>
            <span class="report-row__meta">
              v{{ report.app_version }} · {{ report.platform }}/{{ report.architecture }} · {{ dateTime(report.created_at) }}
            </span>
          </button>
        </template>
      </div>

      <article class="report-detail">
        <StateBlock v-if="detailLoading" kind="loading" title="正在加载详情" />
        <StateBlock v-else-if="!selected" kind="empty" title="选择一条上报查看详情" />
        <template v-else>
          <header class="detail-header">
            <div>
              <span :class="`detail-status status-${selected.status}`">{{ statusLabel(selected.status) }}</span>
              <h3>{{ selected.summary }}</h3>
              <p>{{ dateTime(selected.created_at) }} · v{{ selected.app_version }} · {{ selected.platform }}/{{ selected.architecture }}</p>
            </div>
            <div class="detail-actions">
              <BaseButton
                v-if="selected.status === 'new'"
                variant="secondary"
                size="sm"
                :loading="updating"
                @click="setStatus('reviewed')"
              >标为已查看</BaseButton>
              <BaseButton
                v-if="selected.status !== 'resolved'"
                size="sm"
                :loading="updating"
                @click="setStatus('resolved')"
              >标为已解决</BaseButton>
              <BaseButton
                v-else
                variant="secondary"
                size="sm"
                :loading="updating"
                @click="setStatus('reviewed')"
              >重新打开</BaseButton>
            </div>
          </header>

          <dl class="detail-grid">
            <div><dt>错误代码</dt><dd>{{ selected.error_code || '—' }}</dd></div>
            <div><dt>来源</dt><dd>{{ selected.source }}</dd></div>
            <div class="detail-grid__wide">
              <dt>请求 ID</dt>
              <dd><code>{{ selected.request_id || '—' }}</code><CopyButton v-if="selected.request_id" :value="selected.request_id" /></dd>
            </div>
            <div class="detail-grid__wide"><dt>诊断引用</dt><dd><code>{{ selected.diagnostic_ref || '—' }}</code></dd></div>
          </dl>

          <details v-if="contextText !== '{}'" class="detail-section">
            <summary>上下文</summary>
            <pre>{{ contextText }}</pre>
          </details>
          <details class="detail-section" :open="Boolean(selected.log_excerpt)">
            <summary>脱敏日志</summary>
            <pre>{{ selected.log_excerpt || '没有可用日志' }}</pre>
          </details>
        </template>
      </article>
    </div>
  </section>
</template>

<style scoped>
.reports-panel { display: grid; gap: var(--space-6); }
.reports-header { display: flex; align-items: end; justify-content: space-between; gap: var(--space-6); }
.reports-header h2 { margin-top: var(--space-1); color: var(--text-strong); font-size: 28px; }
.reports-header p { margin-top: var(--space-1); color: var(--text-secondary); }
.reports-actions, .detail-actions { display: flex; align-items: center; gap: var(--space-2); }
.reports-actions select { min-height: 36px; padding: 0 var(--space-8) 0 var(--space-3); border: 1px solid var(--border-strong); border-radius: var(--radius-pill); background: var(--surface); color: var(--text); font: inherit; }
.reports-layout { display: grid; grid-template-columns: minmax(280px, .8fr) minmax(0, 1.4fr); height: min(680px, calc(100vh - 300px)); min-height: 440px; border: 1px solid var(--border); border-radius: var(--radius-lg); overflow: hidden; }
.report-list { overflow-y: auto; border-right: 1px solid var(--border); background: var(--surface-subtle); }
.report-row { display: grid; width: 100%; gap: 5px; padding: var(--space-3) var(--space-4); border: 0; border-bottom: 1px solid var(--border); background: transparent; color: inherit; text-align: left; cursor: pointer; }
.report-row:hover, .report-row--active { background: var(--surface); }
.report-row--active { box-shadow: inset 2px 0 var(--text-strong); }
.report-row__top { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--space-2); }
.report-row__top strong { min-width: 0; overflow: hidden; color: var(--text-strong); font-size: 14px; text-overflow: ellipsis; white-space: nowrap; }
.report-row__top small, .detail-status { flex: none; padding: 1px 7px; border-radius: var(--radius-pill); font-size: 11px; }
.report-row__meta { color: var(--text-secondary); font-size: 12px; }
.status-new { color: var(--danger); background: var(--danger-surface); }
.status-reviewed { color: var(--warning); background: var(--warning-surface); }
.status-resolved { color: var(--success); background: var(--success-surface); }
.report-detail { min-width: 0; overflow-y: auto; padding: var(--space-6); }
.detail-header { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--space-4); }
.detail-header h3 { margin-top: var(--space-2); color: var(--text-strong); font-size: 20px; line-height: 1.4; overflow-wrap: anywhere; }
.detail-header p { margin-top: var(--space-1); color: var(--text-secondary); font-size: 13px; }
.detail-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--space-3); margin-top: var(--space-6); }
.detail-grid > div { min-width: 0; padding: var(--space-3); border-radius: var(--radius-sm); background: var(--surface-subtle); }
.detail-grid__wide { grid-column: 1 / -1; }
.detail-grid dt { color: var(--text-secondary); font-size: 12px; }
.detail-grid dd { display: flex; align-items: center; gap: var(--space-2); margin-top: 4px; min-width: 0; overflow-wrap: anywhere; }
.detail-grid code { min-width: 0; color: var(--text); font-family: var(--font-mono); font-size: 12px; overflow-wrap: anywhere; }
.detail-section { margin-top: var(--space-4); border: 1px solid var(--border); border-radius: var(--radius-md); overflow: hidden; }
.detail-section summary { padding: var(--space-3) var(--space-4); color: var(--text-strong); font-weight: 600; cursor: pointer; }
.detail-section pre { max-height: 360px; overflow: auto; padding: var(--space-4); border-top: 1px solid var(--border); background: var(--surface-subtle); color: var(--text); font: 12px/1.6 var(--font-mono); white-space: pre-wrap; overflow-wrap: anywhere; }
@media (max-width: 820px) {
  .reports-header, .detail-header { align-items: stretch; flex-direction: column; }
  .reports-layout { grid-template-columns: 1fr; height: auto; }
  .report-list { max-height: 320px; border-right: 0; border-bottom: 1px solid var(--border); }
  .report-detail { min-height: 400px; }
  .detail-grid { grid-template-columns: 1fr; }
}
</style>
